use super::*;

/// `RegionFileStorage.MAX_CACHE_SIZE`: the region files of a folder kept
/// open. Each holds a file descriptor: kept for every region ever visited,
/// a server whose players travel runs out of them (`Too many open files`).
pub(super) const MAX_OPEN_REGIONS: usize = 256;
/// A folder's region files, and the ones known to be missing, by region,
/// with when each was last asked for.
#[derive(Default)]
pub(super) struct RegionFiles {
    pub(super) regions: HashMap<(i32, i32), (Option<Arc<Mutex<RegionFile>>>, u64)>,
    pub(super) clock: u64,
    /// Each region's write stamp (`far_regions::writing`).
    pub(super) written: HashMap<(i32, i32), u64>,
    #[cfg(test)]
    pub(super) close_on_evict: Option<fn(RegionFile) -> std::io::Result<()>>,
}
impl RegionFiles {
    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }
    /// The open region files.
    #[cfg(test)]
    pub(super) fn open(&self) -> usize {
        self.regions.values().filter(|(r, _)| r.is_some()).count()
    }
    /// `RegionFileStorage.cache`: remove the least recently used region before closing it.
    /// A region a reader or the writer still holds stays: closing it under them would let a
    /// second handle read a header older than its sectors.
    pub(super) fn make_room(&mut self) -> Vec<((i32, i32), std::io::Error)> {
        let close = RegionFile::close;
        #[cfg(test)]
        let close = self.close_on_evict.unwrap_or(close);
        let mut failures = Vec::new();
        while self.regions.len() >= MAX_OPEN_REGIONS {
            let oldest = self
                .regions
                .iter()
                .filter(|(_, (r, _))| r.as_ref().is_none_or(|r| Arc::strong_count(r) == 1))
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key);
            let Some(key) = oldest else {
                break;
            };
            let (file, _) = self.regions.remove(&key).unwrap();
            if let Some(file) = file {
                // The cache owns the only reference, as checked above under its lock.
                let closed = Arc::into_inner(file)
                    .unwrap()
                    .into_inner()
                    .map_err(|_| std::io::Error::other("region file poisoned"))
                    .and_then(close);
                if let Err(error) = closed {
                    failures.push((key, error));
                }
            }
        }
        failures
    }
}
/// Write `records` into the region files of `folder` in place, as vanilla's
/// `RegionFile.write` does: each chunk takes free sectors and the header
/// then points to it; the region's other chunks are not touched. `None`
/// clears a chunk. The chunks are encoded and compressed before their region
/// is locked for the writes (`compress_records`), and each region is flushed
/// to the disk once. An unencodable record is skipped, leaving its old slot alone.
pub(super) fn write_region_records(
    storage: RegionStorage<'_>,
    records: &[((i32, i32), &Record)],
    sync_data: &impl Fn(&fs::File) -> std::io::Result<()>,
) -> RegionWriteOutcome {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let compressed = compress_records(records, timestamp);
    write_prepared_region_records(storage, timestamp, records, compressed, sync_data)
}

#[derive(Default)]
pub(super) struct RegionWriteOutcome {
    pub(super) failed_records: BTreeMap<(i32, i32), std::io::Error>,
    pub(super) sync_error: Option<std::io::Error>,
}

pub(super) fn write_prepared_region_records(
    storage: RegionStorage<'_>,
    timestamp: u32,
    records: &[((i32, i32), &Record)],
    compressed: Vec<compression_pool::Prepared>,
    sync_data: &impl Fn(&fs::File) -> std::io::Result<()>,
) -> RegionWriteOutcome {
    type RegionWrite<'a> = (ChunkPos, &'a Record, Option<CompressedWrite>);
    let mut by_region: BTreeMap<(i32, i32), Vec<RegionWrite<'_>>> = BTreeMap::new();
    let mut outcome = RegionWriteOutcome::default();
    let failed = &mut outcome.failed_records;
    for (&((x, z), source), record) in records.iter().zip(compressed) {
        match record {
            Ok(record) => by_region.entry((x >> 5, z >> 5)).or_default().push((
                ChunkPos::new(x, z),
                source,
                record,
            )),
            Err(error) => {
                failed.insert((x, z), error);
            }
        }
    }
    for ((rx, rz), mut list) in by_region {
        // Clearing chunks of a region that doesn't exist changes nothing.
        let create = list.iter().any(|(_, _, r)| r.is_some());
        let result: std::io::Result<()> = (|| {
            let Some(region) = storage.open_cached((rx, rz), create)? else {
                return Ok(());
            };
            // Keep far readers on the exact locked read path until the flush completes.
            let _writing = far_regions::writing(storage.cache, (rx, rz));
            let mut region = region
                .lock()
                .map_err(|_| std::io::Error::other("region file poisoned"))?;
            let mut needs_sync = false;
            for (pos, source, record) in &mut list {
                let changed = match record.take() {
                    Some(record) => {
                        write_chunk_record(&mut region, *pos, source, record).map(|()| true)
                    }
                    None => region.clear(*pos, timestamp),
                };
                match changed {
                    Ok(changed) => needs_sync |= changed,
                    Err(error) => {
                        failed.insert((pos.x, pos.z), error);
                    }
                }
            }
            if needs_sync {
                let synced = match region.sync_handle() {
                    Ok(handle) => {
                        drop(region);
                        sync_data(&handle)
                    }
                    // Out of descriptors: flush under the lock rather than skip durability.
                    Err(_) => region.sync(),
                };
                if let Err(error) = synced {
                    log::warn!(
                        "Failed to synchronize {} region [{rx}, {rz}]: {error}",
                        storage.folder.name()
                    );
                    outcome.sync_error.get_or_insert(error);
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            // Open or lock failed before any record reached this region.
            let error = Arc::new(error);
            for (pos, _, _) in list {
                failed.insert(
                    (pos.x, pos.z),
                    std::io::Error::new(error.kind(), error.clone()),
                );
            }
        }
    }
    outcome
}

pub(super) fn write_chunk_record(
    region: &mut RegionFile,
    pos: ChunkPos,
    source: &Record,
    CompressedWrite { record, primary }: CompressedWrite,
) -> std::io::Result<()> {
    region.write(pos, &record)?;
    if primary {
        source.accept_block_entity_edits();
    }
    Ok(())
}

pub(super) fn region_path(root: &Path, folder: &str, rx: i32, rz: i32) -> PathBuf {
    root.join(format!("{folder}/r.{rx}.{rz}.mca"))
}

/// A queued POI record as it is written: claims kept from the saved one, and
/// a chunk that lost its last POI gets an empty one, as vanilla trusts a
/// saved section. None when the chunk never had any.
pub(super) fn resolve_poi(
    storage: RegionStorage<'_>,
    (x, z): (i32, i32),
    record: &Record,
) -> Option<Record> {
    let Record::Poi(chunk, tables, professions) = record else {
        return Some(record.clone());
    };
    let old = read_region_record(storage, x, z)
        .and_then(|old| {
            old.map(|mut old| {
                upgrade_record(Folder::Poi, &mut old, || None)?;
                Ok(old)
            })
            .transpose()
        })
        .unwrap_or_else(|e| {
            log::warn!("Sunrise: could not read the poi record of chunk {x} {z}: {e}");
            None
        });
    let tag = match (encode_poi(chunk, tables, professions.as_deref()), old) {
        (Some(mut tag), Some(old)) => {
            keep_claims(&mut tag, &old);
            tag
        }
        (Some(tag), None) => tag,
        (None, Some(_)) => poi_root(Compound::new()),
        (None, None) => return None,
    };
    Some(Record::Tag(Arc::new(tag)))
}
pub(super) fn read_region_record(
    storage: RegionStorage<'_>,
    x: i32,
    z: i32,
) -> Result<Option<NamedTag>> {
    let Some(region) = storage.open_cached((x >> 5, z >> 5), false)? else {
        return Ok(None);
    };
    let record = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .read(ChunkPos::new(x, z))?;
    Ok(match record {
        Some(r) => Some(r.nbt(Limits::saved())?),
        None => None,
    })
}

fn region_write_stamp(cache: &RegionCache, region: (i32, i32)) -> Result<u64> {
    let files = cache.lock().map_err(|_| "region cache poisoned")?;
    Ok(files.written.get(&region).copied().unwrap_or(0))
}

/// Read a single chunk's NBT record by opening its `.mca` file read-only for this call,
/// bypassing the gameplay `RegionCache` so scans across many regions do not evict active ones.
/// Falls back to locked `read_region_record` whenever the region's write stamp is odd (write in
/// progress) or changes during the read.
pub(super) fn read_region_record_readonly(
    storage: RegionStorage<'_>,
    x: i32,
    z: i32,
) -> Result<Option<NamedTag>> {
    let key = (x >> 5, z >> 5);
    let before = region_write_stamp(storage.cache, key)?;
    if before & 1 == 0 {
        let path = region_path(storage.root, storage.folder.name(), key.0, key.1);
        let direct: Result<Option<NamedTag>> = (|| {
            let mut region = match RegionFile::open(&path) {
                Ok(region) => region,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            match region.read(ChunkPos::new(x, z))? {
                Some(record) => Ok(Some(record.nbt(Limits::saved())?)),
                None => Ok(None),
            }
        })();
        if region_write_stamp(storage.cache, key)? == before {
            return direct;
        }
    }
    read_region_record(storage, x, z)
}

/// List chunk coordinates present in region `(rx, rz)`'s 8 KiB sector header without decompressing
/// payloads or entering the gameplay `RegionCache`. Falls back to the locked cached region handle
/// when a concurrent write is in flight.
pub(super) fn list_region_chunks_readonly(
    storage: RegionStorage<'_>,
    rx: i32,
    rz: i32,
) -> Result<Vec<[i32; 2]>> {
    let key = (rx, rz);
    let before = region_write_stamp(storage.cache, key)?;
    if before & 1 == 0 {
        let path = region_path(storage.root, storage.folder.name(), rx, rz);
        let direct: Result<Vec<[i32; 2]>> = match RegionFile::open(&path) {
            Ok(region) => Ok(region.chunks().map(|(pos, _)| [pos.x, pos.z]).collect()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        };
        if region_write_stamp(storage.cache, key)? == before {
            return direct;
        }
    }
    let Some(region) = storage.open_cached(key, false)? else {
        return Ok(Vec::new());
    };
    let chunks = region
        .lock()
        .map_err(|_| "region file poisoned")?
        .chunks()
        .map(|(pos, _)| [pos.x, pos.z])
        .collect();
    Ok(chunks)
}
