//! The writer thread: what a folder queues to be written, and waiting for it to be on disk.
use super::*;
mod saved_portals;

/// The background thread that encodes chunks and writes them into the region files, grouped
/// by folder within each drained pass. Submission order is preserved within each folder.
pub(super) struct Writer {
    send: mpsc::Sender<WriteRequest>,
    thread: std::thread::JoinHandle<()>,
}

enum WriteRequest {
    Batch(WriteBatch),
    Flush(mpsc::Sender<std::io::Result<()>>),
}

struct WriteBatch {
    folder: Folder,
    jobs: Vec<QueuedWrite>,
    completion: BatchCompletion,
}

#[derive(Default)]
struct WriteGroup {
    jobs: Vec<QueuedWrite>,
    completion: Option<BatchCompletion>,
}

/// Owns a submission's contribution to both writer counts from send to completion.
/// On unwind, this accounts for the current group, unprocessed groups and queued messages.
struct BatchCompletion {
    outstanding: Arc<Outstanding>,
    chunks: usize,
    batches: usize,
}

impl BatchCompletion {
    fn merge(&mut self, mut other: Self) {
        self.chunks += other.chunks;
        self.batches += std::mem::take(&mut other.batches);
    }
}

impl Drop for BatchCompletion {
    fn drop(&mut self) {
        if self.batches == 0 {
            return;
        }
        let outstanding = &self.outstanding;
        if std::thread::panicking() {
            outstanding
                .error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get_or_insert_with(|| "the region writer panicked".into());
        }
        let mut count = outstanding
            .count
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        outstanding.chunks.fetch_sub(self.chunks, Ordering::Relaxed);
        *count -= self.batches;
        outstanding.done.notify_all();
    }
}

/// The sole owner of completion for a queued record. Pending readers borrow
/// only the record; dropping this job releases them even on cancellation,
/// coalescing, or an unwind before the record reaches the writer.
pub(super) struct QueuedWrite {
    pub(super) key: (i32, i32),
    pub(super) record: Record,
    seq: u64,
    eviction: Option<EvictionWrites>,
}

/// Records skipped or refused in one idle eviction, independent of other saves.
#[derive(Clone, Default)]
pub(super) struct EvictionWrites(Arc<Mutex<std::collections::HashSet<(i32, i32)>>>);

impl EvictionWrites {
    pub(super) fn refuse(&self, key: (i32, i32)) {
        self.0.lock().unwrap().insert(key);
    }

    pub(super) fn refused(&self) -> std::collections::HashSet<(i32, i32)> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl QueuedWrite {
    pub(super) fn new(key: (i32, i32), record: Record, seq: u64) -> Self {
        Self {
            key,
            record: record.track_block_entity_edits(),
            seq,
            eviction: None,
        }
    }

    fn refuse_eviction(&self) {
        if let Some(eviction) = &self.eviction {
            eviction.refuse(self.key);
        }
    }
}

impl Drop for QueuedWrite {
    fn drop(&mut self) {
        if let Some(disk) = self.record.disk_job() {
            disk.cancel();
        }
        self.record.resolve_block_entity_edits(false);
    }
}

/// Writes still queued or running, with a condition to wait for none.
#[derive(Default)]
pub(super) struct Outstanding {
    count: Mutex<usize>,
    chunks: AtomicUsize,
    pub(super) error: Mutex<Option<String>>,
    // A storage sync failure is consumed by the barrier that reports it.
    sync_error: Mutex<Option<std::io::Error>>,
    done: Condvar,
    /// What the folder's writes make stale: the portal blocks a search read from saved chunks.
    portals: saved_portals::SavedPortals,
}

#[derive(Clone)]
pub(super) struct SaveReporting {
    world_root: PathBuf,
    dimension: &'static str,
    pub(super) reporter: Arc<crate::chunk_io::SaveFailureReporter>,
    pub(super) inbox: Arc<crate::chunk_io::Reports>,
}

impl SaveReporting {
    pub(super) fn new(
        world_root: PathBuf,
        dimension: &'static str,
        reporter: Arc<crate::chunk_io::SaveFailureReporter>,
        inbox: Arc<crate::chunk_io::Reports>,
    ) -> Self {
        Self {
            world_root,
            dimension,
            reporter,
            inbox,
        }
    }

    pub(super) fn report(&self, folder: Folder, key: (i32, i32), error: &std::io::Error) {
        let label = match folder {
            Folder::Region => "chunk",
            Folder::Entities => "entities",
            Folder::Poi => "poi",
        };
        self.inbox.report_save_failure(
            &self.reporter,
            &self.world_root,
            self.dimension,
            label,
            key,
            error,
        );
    }
}

impl SaveDir {
    pub(super) fn report_record_failure(
        &self,
        folder: Folder,
        key: (i32, i32),
        error: &std::io::Error,
    ) {
        self.save_reporting.report(folder, key, error);
    }

    pub fn report_entity_record_failure(&self, key: (i32, i32), error: &std::io::Error) {
        self.save_reporting.report(Folder::Entities, key, error);
    }

    /// Queue `records` for the writer thread. With `wait`, wait for these and all earlier
    /// write attempts. Individual failures are reported; a failed sync fails the wait.
    pub(super) fn write_records(
        &self,
        folder: &str,
        records: Vec<((i32, i32), Option<NamedTag>)>,
        wait: bool,
    ) -> Result<()> {
        let records = records
            .into_iter()
            .map(|(k, t)| (k, t.map_or(Record::Delete, |t| Record::Tag(Arc::new(t)))))
            .collect();
        self.queue(Folder::of(folder), records, wait)
    }
    pub(super) fn queue(
        &self,
        folder: Folder,
        records: Vec<((i32, i32), Record)>,
        wait: bool,
    ) -> Result<()> {
        self.enqueue_records(folder, records, None)?;
        if wait {
            self.flush()?;
        }
        Ok(())
    }

    pub(super) fn enqueue_records(
        &self,
        folder: Folder,
        records: Vec<((i32, i32), Record)>,
        eviction: Option<&EvictionWrites>,
    ) -> Result<()> {
        if !records.is_empty() {
            // Run snapshot callbacks before publishing pending records or taking writer locks.
            let records: Vec<_> = if folder == Folder::Region {
                records
                    .into_iter()
                    .filter_map(|(key, record)| match self.capture_disk_record(record) {
                        Ok(record) => Some((key, record)),
                        Err(error) => {
                            if let Some(eviction) = eviction {
                                eviction.refuse(key);
                            }
                            self.report_record_failure(
                                Folder::Region,
                                key,
                                &std::io::Error::other(error),
                            );
                            None
                        }
                    })
                    .collect()
            } else {
                records
            };
            if records.is_empty() {
                return Ok(());
            }
            let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
            let writer = writer.get_or_insert_with(|| {
                self.start_writer(fs::File::sync_data, RegionFile::sync_all)
            });
            let mut batch = Vec::with_capacity(records.len());
            {
                let mut pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
                for (key, record) in records {
                    let mut job = QueuedWrite::new(
                        key,
                        record,
                        self.sequence.fetch_add(1, Ordering::Relaxed),
                    );
                    job.eviction = eviction.cloned();
                    pending.insert((folder, key.0, key.1), (job.seq, job.record.clone()));
                    batch.push(job);
                }
            }
            // After the pending writes: a search reads the new chunks from here on.
            self.outstanding
                .portals
                .written(folder, batch.iter().map(|job| job.key));
            *self
                .outstanding
                .count
                .lock()
                .map_err(|_| "writer poisoned")? += 1;
            let chunks = if folder == Folder::Region {
                batch.len()
            } else {
                0
            };
            self.outstanding.chunks.fetch_add(chunks, Ordering::Relaxed);
            if let Err(unsent) = writer.send.send(WriteRequest::Batch(WriteBatch {
                folder,
                jobs: batch,
                completion: BatchCompletion {
                    outstanding: self.outstanding.clone(),
                    chunks,
                    batches: 1,
                },
            })) {
                self.outstanding
                    .error
                    .lock()
                    .map_err(|_| "writer poisoned")?
                    .get_or_insert_with(|| "the region writer stopped".into());
                drop(unsent);
                return Err("the region writer stopped".into());
            }
        }
        Ok(())
    }
    /// Queue a durable barrier behind all earlier writes. The window only polls
    /// its receiver; draining writes and forcing regions stay off the event loop.
    pub fn flush_later(&self) -> Result<mpsc::Receiver<std::io::Result<()>>> {
        let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
        let writer = writer
            .get_or_insert_with(|| self.start_writer(fs::File::sync_data, RegionFile::sync_all));
        let (send, receive) = mpsc::channel();
        writer
            .send
            .send(WriteRequest::Flush(send))
            .map_err(|_| "the region writer stopped")?;
        Ok(receive)
    }

    pub(super) fn start_writer(
        &self,
        sync_data: impl Fn(&fs::File) -> std::io::Result<()> + Send + 'static,
        mut force: impl FnMut(&RegionFile) -> std::io::Result<()> + Send + 'static,
    ) -> Writer {
        let (send, receive) = mpsc::channel::<WriteRequest>();
        let root = self.root.clone();
        let reporting = self.save_reporting.clone();
        let (regions, entity_regions, poi_regions) = (
            self.regions.clone(),
            self.entity_regions.clone(),
            self.poi_regions.clone(),
        );
        let pending = self.pending.clone();
        let outstanding = self.outstanding.clone();
        // Callbacks have run before handoff; the writer owns region I/O and completion.
        let thread = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("sunrise-region-writer".into())
            .spawn(move || {
                const MAX_PASS_CHUNKS: usize = 512;
                const KINDS: [Folder; 3] = [Folder::Region, Folder::Entities, Folder::Poi];
                let slot = |kind: Folder| match kind {
                    Folder::Region => 0,
                    Folder::Entities => 1,
                    Folder::Poi => 2,
                };
                while let Ok(first) = receive.recv() {
                    let mut groups: [WriteGroup; 3] = Default::default();
                    let mut total = 0;
                    let mut next = Some(first);
                    let mut flush = None;
                    while let Some(request) = next.take() {
                        let WriteBatch {
                            folder,
                            jobs,
                            completion,
                        } = match request {
                            WriteRequest::Batch(batch) => batch,
                            WriteRequest::Flush(reply) => {
                                flush = Some(reply);
                                break;
                            }
                        };
                        total += jobs.len();
                        let group = &mut groups[slot(folder)];
                        group.jobs.extend(jobs);
                        if let Some(group) = &mut group.completion {
                            group.merge(completion);
                        } else {
                            group.completion = Some(completion);
                        }
                        if total >= MAX_PASS_CHUNKS {
                            break;
                        }
                        next = receive.try_recv().ok();
                    }
                    for (i, group) in groups.into_iter().enumerate() {
                        let batch = &group.jobs;
                        if batch.is_empty() {
                            continue;
                        }
                        let kind = KINDS[i];
                        let cache = match kind {
                            Folder::Region => &regions,
                            Folder::Entities => &entity_regions,
                            Folder::Poi => &poi_regions,
                        };
                        let storage = RegionStorage {
                            root: &root,
                            folder: kind,
                            cache,
                            reporting: &reporting,
                        };
                        let newest = |k: &(i32, i32), seq: u64| {
                            pending.lock().map_or(true, |p| {
                                p.get(&(kind, k.0, k.1)).is_none_or(|p| p.0 <= seq)
                            })
                        };
                        let resolved: Vec<_> = batch
                            .iter()
                            .filter_map(|job| {
                                let newest = newest(&job.key, job.seq);
                                // The owner already ran the callback for every capture, even
                                // when a newer write replaces this snapshot.
                                if !newest {
                                    if let Some(capture) = job.record.disk_job() {
                                        capture.cancel();
                                    }
                                    job.refuse_eviction();
                                    return None;
                                }
                                match compression_pool::catch_encoding_panic(|| {
                                    Ok(resolve_poi(storage, job.key, &job.record))
                                }) {
                                    Ok(record) => record.map(|record| (job, record)),
                                    Err(error) => {
                                        job.refuse_eviction();
                                        reporting.report(kind, job.key, &error);
                                        None
                                    }
                                }
                            })
                            .collect();
                        let records: Vec<_> =
                            resolved.iter().map(|(job, r)| (job.key, r)).collect();
                        let outcome = write_region_records(storage, &records, &sync_data);
                        if !outcome.failed_records.is_empty() {
                            for (job, _) in &resolved {
                                if outcome.failed_records.contains_key(&job.key) {
                                    job.refuse_eviction();
                                }
                            }
                        }
                        for (key, error) in outcome.failed_records {
                            reporting.report(kind, key, &error);
                        }
                        if let Some(error) = outcome.sync_error {
                            outstanding
                                .sync_error
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .get_or_insert(error);
                        }
                        if let Ok(mut pending) = pending.lock() {
                            for job in batch {
                                let key = (kind, job.key.0, job.key.1);
                                if pending.get(&key).is_some_and(|p| p.0 == job.seq) {
                                    // Like IOWorker, retire both successful and failed stores. A
                                    // reader that already borrowed the snapshot still owns it.
                                    pending.remove(&key);
                                }
                            }
                        }
                    }
                    if let Some(reply) = flush {
                        let result = force_cached_regions(
                            [&regions, &entity_regions, &poi_regions],
                            &mut force,
                            outstanding.write_result(true),
                        );
                        let _ = reply.send(result);
                    }
                }
            })
            .expect("spawn region writer");
        Writer { send, thread }
    }
    /// Number of failed records reported for this world. Checkpoints use a before/after
    /// difference for diagnostics; this count does not change their save result.
    pub fn record_save_failures(&self) -> u64 {
        self.save_failure_reporter().reported_failures()
    }

    /// Drain queued writes and force every cached writable region. Individual record failures
    /// are reported by the writer; a storage sync failure fails the barrier that reports it.
    pub fn flush(&self) -> Result<()> {
        self.flush_with(RegionFile::sync_all)
    }

    #[doc(hidden)]
    pub fn inject_sync_error_for_test(&self, error: std::io::Error) {
        self.outstanding
            .sync_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_or_insert(error);
    }

    /// Wait for every queued write batch to finish on the writer thread without
    /// calling `sync_all` across all cached regions (used by idle chunk eviction;
    /// full durability barriers use [`Self::flush`]).
    pub(super) fn wait_for_writes(&self) -> Result<()> {
        let mut count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        while *count > 0 {
            count = self
                .outstanding
                .done
                .wait(count)
                .map_err(|_| "writer poisoned")?;
        }
        self.outstanding.write_result(true)?;
        Ok(())
    }

    fn flush_with(&self, mut force: impl FnMut(&RegionFile) -> std::io::Result<()>) -> Result<()> {
        let mut count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        while *count > 0 {
            count = self
                .outstanding
                .done
                .wait(count)
                .map_err(|_| "writer poisoned")?;
        }
        let result = self.outstanding.write_result(true);
        // A completion poll must not block behind this explicit flush's disk I/O.
        drop(count);
        force_cached_regions(
            [&self.regions, &self.entity_regions, &self.poi_regions],
            &mut force,
            result,
        )?;
        Ok(())
    }
    /// Wait until at most `low` chunk records are still queued, without waiting
    /// for all of them; a save pass resuming at half the cap keeps the writer fed
    /// without stalling for records it did not have to.
    ///
    /// The predicate is `chunks`, not `count`: `count` holds queued batches and
    /// one batch can carry many records, so waiting on it would let the record
    /// cap (`MAX_ASYNC_CHUNK_WRITES`) drift with how the caller batches.
    pub(super) fn wait_for_chunk_writes_below(&self, low: usize) -> Result<()> {
        let mut count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        // The writer lowers `chunks` before it takes this same mutex to notify,
        // so a wakeup cannot fall between the check and the wait.
        while self.outstanding.chunks.load(Ordering::Relaxed) > low {
            count = self
                .outstanding
                .done
                .wait(count)
                .map_err(|_| "writer poisoned")?;
        }
        // Backpressure is not a durability barrier; continue submitting independent work.
        self.outstanding.write_result(false)?;
        Ok(())
    }
    /// `ChunkMap.activeChunkWrites`: chunk records, excluding entity/POI IO.
    pub fn active_chunk_writes(&self) -> usize {
        self.outstanding.chunks.load(Ordering::Relaxed)
    }
    /// Poll completion of queued write attempts without blocking. This does not force cached
    /// regions; callers that need a durable barrier use `flush` or `flush_later`.
    pub fn writes_finished(&self) -> Result<bool> {
        let count = self
            .outstanding
            .count
            .lock()
            .map_err(|_| "writer poisoned")?;
        self.outstanding.write_result(*count == 0)?;
        Ok(*count == 0)
    }
}

impl Outstanding {
    fn write_result(&self, drained: bool) -> std::io::Result<()> {
        if let Some(error) = &*self
            .error
            .lock()
            .map_err(|_| std::io::Error::other("writer poisoned"))?
        {
            return Err(std::io::Error::other(error.clone()));
        }
        if drained
            && let Some(error) = self
                .sync_error
                .lock()
                .map_err(|_| std::io::Error::other("writer poisoned"))?
                .take()
        {
            return Err(error);
        }
        Ok(())
    }
}

fn force_cached_regions(
    caches: [&RegionCache; 3],
    force: &mut impl FnMut(&RegionFile) -> std::io::Result<()>,
    mut result: std::io::Result<()>,
) -> std::io::Result<()> {
    // Force every cache even after an error, as RegionFileStorage.close does.
    for (folder, cache) in [Folder::Region, Folder::Entities, Folder::Poi]
        .into_iter()
        .zip(caches)
    {
        let Ok(cache) = cache.lock() else {
            if result.is_ok() {
                result = Err(std::io::Error::other("region cache poisoned"));
            }
            continue;
        };
        let regions: Vec<_> = cache
            .regions
            .iter()
            .filter_map(|(&key, (file, _))| Some((key, file.as_ref()?.clone())))
            .collect();
        drop(cache);
        for ((rx, rz), region) in regions {
            let synced = region
                .lock()
                .map_err(|_| std::io::Error::other("region file poisoned"))
                .and_then(|region| force(&region));
            if let Err(error) = synced {
                log::warn!(
                    "Failed to synchronize {} region [{rx}, {rz}]: {error}",
                    folder.name()
                );
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
    }
    result
}

impl Drop for SaveDir {
    /// Finish queued writes before the world is let go.
    fn drop(&mut self) {
        if let Some(writer) = self.writer.get_mut().ok().and_then(Option::take) {
            drop(writer.send);
            let _ = writer.thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(debug_assertions)]
    fn a_poi_resolution_panic_refuses_only_that_record() {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let tables = Arc::new(SaveTables {
            block_names: Vec::new(),
            block_ids: HashMap::new(),
            default_state: Vec::new(),
            block_entity: Vec::new(),
            heightmaps: crate::assets::Properties::for_tests(&[]),
            air: 0,
            poi: vec![Some(("minecraft:home", 1))],
        });
        // Resolving a POI for this synthetic out-of-world chunk overflows its block position.
        let key = (i32::MAX, 0);
        let invalid = Record::Poi(Arc::new(Chunk::empty(key.0, key.1)), tables, None);
        let good = Arc::new(NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(3))]),
        });
        let save = open();
        save.queue(
            Folder::Poi,
            vec![(key, invalid), ((0, 0), Record::Tag(good.clone()))],
            true,
        )
        .unwrap();
        assert_eq!(save.record_save_failures(), 1);
        let disk = open();
        assert_eq!(
            disk.read_record_raw("poi", 0, 0).as_ref(),
            Some(good.as_ref())
        );
        assert!(disk.read_record_raw("poi", key.0, key.1).is_none());
    }

    #[test]
    fn a_region_sync_failure_fails_the_barrier_but_siblings_still_sync() {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let save = open();
        let sync_attempts = Arc::new(AtomicUsize::new(0));
        let attempts = sync_attempts.clone();
        let kind = std::io::ErrorKind::PermissionDenied;
        *save.writer.lock().unwrap() = Some(save.start_writer(
            move |file| {
                if attempts.fetch_add(1, Ordering::Relaxed) == 0 {
                    return Err(kind.into());
                }
                file.sync_data()
            },
            RegionFile::sync_all,
        ));
        let record = || NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(3))]),
        };
        let error = save
            .write_records(
                "region",
                vec![((0, 0), Some(record())), ((32, 0), Some(record()))],
                true,
            )
            .unwrap_err();
        assert_eq!(error.downcast_ref::<std::io::Error>().unwrap().kind(), kind);
        assert_eq!(save.record_save_failures(), 0);
        assert_eq!(sync_attempts.load(Ordering::Relaxed), 2);

        // Nothing new was queued: the next flush must still force both regions, including
        // whichever one failed above. It must not carry the old error into this barrier.
        let mut forced = Vec::new();
        save.flush_with(|region| {
            forced.push(region.position().x);
            region.sync_all()
        })
        .unwrap();
        forced.sort_unstable();
        assert_eq!(forced, [0, 1]);
        assert_eq!(sync_attempts.load(Ordering::Relaxed), 2);
        assert!(save.writes_finished().unwrap());
        drop(save);
        let disk = open();
        assert_eq!(disk.read_record_raw("region", 0, 0), Some(record()));
        assert_eq!(disk.read_record_raw("region", 32, 0), Some(record()));
    }

    #[test]
    fn eviction_closes_a_region_after_its_force_failed() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let save = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let path = save.region_path("region", 0, 0);
        let storage = save.region_storage(Folder::Region);
        storage.open_cached((0, 0), true).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(&[1])
            .unwrap();
        let failure = std::io::ErrorKind::PermissionDenied;
        let error = save.flush_with(|_| Err(failure.into())).unwrap_err();
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            failure
        );
        assert_eq!(fs::metadata(&path).unwrap().len(), 1);

        for x in 1..=MAX_OPEN_REGIONS as i32 {
            assert!(storage.open_cached((x, 0), false).unwrap().is_none());
        }
        assert!(!save.regions.lock().unwrap().regions.contains_key(&(0, 0)));
        // The production eviction path pads the file before forcing and closing it.
        assert_eq!(fs::metadata(&path).unwrap().len(), 4096);
        save.flush().unwrap();
    }

    #[test]
    fn a_failed_eviction_does_not_regenerate_the_requested_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let open = || SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let key = (32, 0);
        let saved = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("Status", string("minecraft:full")),
                ("xPos", Tag::Int(key.0)),
                ("zPos", Tag::Int(key.1)),
                ("sections", Tag::List(Vec::new())),
                ("marker", Tag::Int(42)),
            ]),
        };
        open()
            .write_records("region", vec![(key, Some(saved))], true)
            .unwrap();
        let save = open();
        let storage = save.region_storage(Folder::Region);
        for x in (0..=MAX_OPEN_REGIONS as i32).filter(|&x| x != 1) {
            storage.open_cached((x, 0), x == 0).unwrap();
        }
        save.regions.lock().unwrap().close_on_evict = Some(|region| {
            assert_eq!(region.position(), ChunkPos::new(0, 0));
            Err(std::io::Error::other("injected close failure"))
        });
        let (registry, biomes) = super::super::loot_tests::fixture();
        let loaded = save
            .load(key.0, key.1, &registry, &biomes)
            .expect("load B, not generation");
        assert_eq!(
            loaded.saved.as_ref().unwrap().root.get(&"marker".into()),
            Some(&Tag::Int(42))
        );
        assert_eq!(save.record_save_failures(), 1);
        assert!(
            save.save_failure_reporter()
                .suppressed_summary()
                .contains("Failed to close region [0, 0] in region: injected close failure")
        );
        assert!(!save.regions.lock().unwrap().regions.contains_key(&(0, 0)));
        save.flush().unwrap();
    }

    #[test]
    fn a_second_staged_barrier_forces_the_failed_region_without_replaying_writes() {
        let dir = tempfile::tempdir().unwrap();
        let save = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let writes = Arc::new(AtomicUsize::new(0));
        let writes_on_worker = writes.clone();
        let (forced, attempts) = mpsc::channel();
        let failure = std::io::ErrorKind::PermissionDenied;
        let mut failed = false;
        *save.writer.lock().unwrap() = Some(save.start_writer(
            move |file| {
                writes_on_worker.fetch_add(1, Ordering::Relaxed);
                file.sync_data()
            },
            move |region| {
                let x = region.position().x;
                forced.send(x).unwrap();
                if x == 0 && !failed {
                    failed = true;
                    return Err(failure.into());
                }
                region.sync_all()
            },
        ));
        let record = || NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(3))]),
        };
        save.write_records(
            "region",
            vec![((0, 0), Some(record())), ((32, 0), Some(record()))],
            false,
        )
        .unwrap();
        let barrier = || {
            let result = save
                .flush_later()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let mut forced: Vec<_> = attempts.try_iter().collect();
            forced.sort_unstable();
            assert_eq!(forced, [0, 1]);
            assert_eq!(writes.load(Ordering::Relaxed), 2);
            result
        };
        let error = barrier().unwrap_err();
        assert_eq!(error.kind(), failure);
        barrier().unwrap();
    }

    #[test]
    fn a_failed_send_leaves_the_pending_disk_snapshot_readable() {
        let dir = tempfile::tempdir().unwrap();
        let (registry, biomes) = super::super::loot_tests::fixture();
        let registry = Box::leak(Box::new(registry));
        let biomes = Arc::new(biomes);
        super::super::chunk_disk_tests::with_listener(|saves, _| {
            let save = SaveDir::new(
                dir.path().to_path_buf(),
                LevelShape::OVERWORLD,
                Some(Default::default()),
            );
            super::super::chunk_disk_tests::context(&save);
            let (send, receive) = mpsc::channel();
            drop(receive);
            *save.writer.lock().unwrap() = Some(Writer {
                send,
                thread: std::thread::spawn(|| {}),
            });

            let key = (2, -3);
            let mut chunk = Chunk::empty(key.0, key.1);
            let mut extras = ChunkExtras::default();
            extras.root.insert("test:marker".into(), Tag::Int(42));
            chunk.saved = Some(Arc::new(extras));
            assert!(
                save.save_later(&[Arc::new(chunk)], registry, &biomes)
                    .is_err()
            );
            assert!(
                save.pending.lock().unwrap()[&(Folder::Region, key.0, key.1)]
                    .1
                    .disk_job()
                    .is_some()
            );

            let (send, receive) = mpsc::channel();
            let reader = std::thread::spawn(move || {
                send.send(save.raw_chunk(key.0, key.1)).unwrap();
            });
            let snapshot = receive
                .recv_timeout(std::time::Duration::from_secs(1))
                .expect("pending reader waited for a disconnected writer")
                .unwrap();
            reader.join().unwrap();
            assert_eq!(snapshot.tag.get("test:marker"), Some(&Tag::Int(42)));
            assert_eq!(saves.load(Ordering::Relaxed), 1);
        });
    }

    #[test]
    fn an_unexpected_writer_panic_releases_both_waits() {
        let dir = tempfile::tempdir().unwrap();
        let save = SaveDir::new(dir.path().to_path_buf(), LevelShape::OVERWORLD, None);
        let outstanding = save.outstanding.clone();
        *outstanding.count.lock().unwrap() = 1;
        outstanding.chunks.store(1, Ordering::Relaxed);
        let writer = std::thread::spawn(move || {
            let _completion = BatchCompletion {
                outstanding,
                chunks: 1,
                batches: 1,
            };
            panic!("injected unexpected writer panic");
        });
        let (send, receive) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            assert!(save.flush().is_err());
            assert_eq!(save.active_chunk_writes(), 0);
            assert!(save.wait_for_chunk_writes_below(0).is_err());
            send.send(()).unwrap();
        });
        assert!(writer.join().is_err());
        receive
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn save_eviction_drains_writes_without_syncing_all_cached_regions() {
        let dir = tempfile::tempdir().unwrap();
        let (registry, biomes) = super::super::loot_tests::fixture();
        let registry = Box::leak(Box::new(registry));
        let biomes = Arc::new(biomes);
        let save = SaveDir::new(
            dir.path().to_path_buf(),
            LevelShape::OVERWORLD,
            Some(Default::default()),
        );
        super::super::chunk_disk_tests::context(&save);
        let sync_data_calls = Arc::new(AtomicUsize::new(0));
        let force_calls = Arc::new(AtomicUsize::new(0));
        let sd = sync_data_calls.clone();
        let fc = force_calls.clone();
        *save.writer.lock().unwrap() = Some(save.start_writer(
            move |file| {
                sd.fetch_add(1, Ordering::Relaxed);
                file.sync_data()
            },
            move |region| {
                fc.fetch_add(1, Ordering::Relaxed);
                region.sync_all()
            },
        ));
        let record = NamedTag {
            name: "".into(),
            tag: Tag::compound([("marker", Tag::Int(7))]),
        };
        // Seed two unrelated cached regions.
        save.write_records(
            "region",
            vec![((0, 0), Some(record.clone())), ((32, 0), Some(record))],
            false,
        )
        .unwrap();
        save.wait_for_writes().unwrap();
        let before_sync_data = sync_data_calls.load(Ordering::Relaxed);
        let before_force = force_calls.load(Ordering::Relaxed);

        let chunk = Arc::new(Chunk::empty(64, 0));
        let refused = save
            .save_eviction(&[chunk], registry, &biomes, Vec::new())
            .unwrap();
        assert!(refused.is_empty());
        assert_eq!(
            force_calls.load(Ordering::Relaxed),
            before_force,
            "save_eviction must not call RegionFile::sync_all across cached regions"
        );
        // Only the evicted column's own region (not the two unrelated cached regions) synced.
        assert_eq!(
            sync_data_calls.load(Ordering::Relaxed) - before_sync_data,
            1
        );
    }
}
