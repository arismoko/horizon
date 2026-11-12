//! The chunk writer's isolated mod attachment callback boundary.
//! Server code captures encoded envelopes; only workers execute mod code.
use super::*;
use horizon_mod_runtime::attachments::Attachments;
use mod_api::{ChunkDiskContextWire, ChunkDiskJob, ChunkDiskKind, ChunkDiskPhase, WorldEpoch};

pub(super) struct DiskRuntime {
    refused_loads: Mutex<HashSet<(i32, i32)>>,
    /// Chunks whose captured save did not reach disk; the game marks them dirty again.
    failed_saves: Mutex<Vec<(i32, i32)>>,
    world: WorldEpoch,
    dimension: mod_api::Identifier,
    resources: Arc<horizon_world::native_load_context::WorldgenContext>,
}

impl DiskRuntime {
    fn capture(
        &self,
        chunk: &Chunk,
        kind: ChunkDiskKind,
        phase: ChunkDiskPhase,
    ) -> Option<ChunkDiskJob> {
        let context = ChunkDiskContextWire::new(
            self.world,
            self.dimension.clone(),
            [chunk.x, chunk.z],
            kind,
            phase,
        );
        let empty = Attachments::default();
        ChunkDiskJob::capture(
            &context,
            chunk
                .saved
                .as_ref()
                .map_or(&empty, |saved| &saved.attachments),
        )
    }

    fn run(&self, job: &ChunkDiskJob) -> std::io::Result<Compound> {
        let registry = |name: &str| self.resources.registry(name);
        let setting = |owner: &str, key: &str| self.resources.setting(owner, key);
        job.run(&mod_api::OwningWorldgen::new(&registry, &setting))
            .clone()
            .map_err(|failure| {
                std::io::Error::other(format!("chunk disk callback refused: {failure:?}"))
            })
    }
}

pub(super) struct DiskRecord {
    original: Record,
    job: ChunkDiskJob,
    runtime: Arc<DiskRuntime>,
    encoded: OnceLock<std::result::Result<Option<Arc<NamedTag>>, String>>,
}
impl DiskRecord {
    fn encode(&self) -> std::io::Result<Option<Arc<NamedTag>>> {
        self.encoded
            .get_or_init(|| {
                let entries = self.runtime.run(&self.job).map_err(|e| e.to_string())?;
                let mut tag = self
                    .original
                    .tag()
                    .ok_or("captured chunk has no disk record")?;
                replace_entries(Arc::make_mut(&mut tag), entries).map_err(|e| e.to_string())?;
                Ok(Some(tag))
            })
            .clone()
            .map_err(std::io::Error::other)
    }

    // Readers may wait for the owning writer, but never execute save callbacks.
    pub(super) fn read(&self) -> Option<Arc<NamedTag>> {
        self.encoded.wait().as_ref().ok().cloned().flatten()
    }
}

/// The shared save gate: an open action savepoint holds tentative state, so no
/// subscribed save may capture, and no callback runs, until it settles.
fn admit_disk_save() -> Result<()> {
    if crate::join_seq::visibility::passes_settled() {
        Ok(())
    } else {
        Err("chunk save refused: an action savepoint is open".into())
    }
}

pub(super) fn encode_record(record: &Record) -> std::io::Result<Option<Arc<NamedTag>>> {
    match record {
        Record::Disk(record) => record.encode(),
        _ => Ok(record.tag()),
    }
}

pub(super) fn prepare_batch(batch: &WriteBatchRecords) -> std::io::Result<()> {
    // Coalescing may skip a disk write, never the callback of a captured record.
    let mut refusal = None;
    for (_, record, _) in batch {
        if let Record::Disk(record) = record
            && let Err(error) = record.encode()
        {
            refusal.get_or_insert(error);
        }
    }
    refusal.map_or(Ok(()), Err)
}

type WriteBatchRecords = [((i32, i32), Record, u64)];

fn replace_entries(tag: &mut NamedTag, entries: Compound) -> std::io::Result<()> {
    let Tag::Compound(root) = &mut tag.tag else {
        return Err(std::io::Error::other("invalid chunk root"));
    };
    if entries.is_empty() {
        root.remove(&ATTACHMENTS_KEY.into());
    } else {
        root.insert(ATTACHMENTS_KEY.into(), Tag::Compound(entries));
    }
    Ok(())
}

impl SaveDir {
    /// Bind disk callbacks before this server level starts any chunk workers.
    /// The world epoch belongs to the existing lifecycle owner. Remote caches
    /// never call this method. With no subscribers no context is retained.
    pub fn set_chunk_disk_context(
        &self,
        world: WorldEpoch,
        dimension: mod_api::Identifier,
        resources: Arc<horizon_world::native_load_context::WorldgenContext>,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
            && !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
        {
            return;
        }
        let _ = self.disk.set(Arc::new(DiskRuntime {
            refused_loads: Mutex::default(),
            failed_saves: Mutex::default(),
            world,
            dimension,
            resources,
        }));
    }

    /// A failed decode/callback never becomes a generator's missing chunk.
    pub fn chunk_disk_load_refused(&self, x: i32, z: i32) -> bool {
        self.disk.get().is_some_and(|runtime| {
            runtime
                .refused_loads
                .lock()
                .map_or(true, |refused| refused.contains(&(x, z)))
        })
    }

    pub(super) fn capture_disk_record(&self, record: Record) -> Result<Record> {
        let (chunk, kind) = match &record {
            Record::Chunk(chunk, ..) => (&**chunk, ChunkDiskKind::Full),
            Record::Proto(proto, ..) => (&proto.column, ChunkDiskKind::Proto),
            _ => return Ok(record),
        };
        if self.chunk_disk_load_refused(chunk.x, chunk.z) {
            return Err("chunk disk load was refused; prior record retained".into());
        }
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataSave) {
            return Ok(record);
        }
        admit_disk_save()?;
        let Some(runtime) = self.disk.get() else {
            if !self.trusted_fluid_load {
                return Ok(record);
            }
            return Err("chunk disk worker context is not bound".into());
        };
        let Some(job) = runtime.capture(chunk, kind, ChunkDiskPhase::Save) else {
            return Ok(record);
        };
        Ok(Record::Disk(Arc::new(DiskRecord {
            original: record,
            job,
            runtime: runtime.clone(),
            encoded: OnceLock::new(),
        })))
    }

    pub(super) fn load_disk_attachments(
        &self,
        chunk: &mut Chunk,
        kind: ChunkDiskKind,
    ) -> Result<()> {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataLoad) {
            return Ok(());
        }
        let Some(runtime) = self.disk.get() else {
            // This save directory can also be a non-authoritative remote cache.
            return Ok(());
        };
        let Some(job) = runtime.capture(chunk, kind, ChunkDiskPhase::Load) else {
            return Ok(());
        };
        match runtime.run(&job) {
            Ok(entries) => {
                Arc::make_mut(chunk.saved.get_or_insert_with(Default::default)).attachments =
                    Attachments::from_nbt(entries);
                Ok(())
            }
            Err(error) => {
                runtime
                    .refused_loads
                    .lock()
                    .map_err(|_| "chunk disk refusals poisoned")?
                    .insert((chunk.x, chunk.z));
                let message = error.to_string();
                *self
                    .outstanding
                    .error
                    .lock()
                    .map_err(|_| "writer poisoned")? = Some(message);
                crate::chunk_io::report_load_failure(&self.root, chunk.x, chunk.z, &error);
                Err(error.into())
            }
        }
    }

    pub(super) fn save_disk_sync(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<Result<usize>> {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataSave) {
            return None;
        }
        if let Err(error) = admit_disk_save() {
            return Some(Err(error));
        }
        Some((|| {
            let runtime = self
                .disk
                .get()
                .ok_or("chunk disk worker context is not bound")?;
            let jobs: Vec<_> = chunks
                .iter()
                .map(|chunk| runtime.capture(chunk, ChunkDiskKind::Full, ChunkDiskPhase::Save))
                .collect();
            let work = || -> std::io::Result<Vec<_>> {
                chunks
                    .iter()
                    .zip(&jobs)
                    .map(|(chunk, job)| {
                        let entries = job.as_ref().map(|job| runtime.run(job)).transpose()?;
                        let blending = self.calculated_blending(chunk.x, chunk.z);
                        let mut tag = encode_saved(
                            chunk,
                            registry,
                            biomes,
                            self.tables.get().map(|t| &**t),
                            self.shape,
                            blending.as_deref(),
                        );
                        if let Some(entries) = entries {
                            replace_entries(&mut tag, entries)?;
                        }
                        Ok(((chunk.x, chunk.z), Some(tag)))
                    })
                    .collect()
            };
            let records = std::thread::scope(|scope| scope.spawn(work).join())
                .map_err(|_| "chunk disk worker unwound")??;
            self.write_records("region", records, true)?;
            self.save_poi(chunks, true)?;
            Ok(chunks.len())
        })())
    }
}

/// Remember the chunks of a failed write so the game can mark them dirty again.
pub(super) fn note_failed(batch: &WriteBatchRecords) {
    for (key, record, _) in batch {
        if let Record::Disk(record) = record
            && let Ok(mut failed) = record.runtime.failed_saves.lock()
        {
            failed.push(*key);
        }
    }
}

impl SaveDir {
    /// Chunks whose subscribed save was refused or failed, once each. The
    /// caller marks them modified again, so dirty state is never lost to a
    /// save that did not complete. Empty without a subscriber.
    pub fn take_failed_chunk_saves(&self) -> Vec<(i32, i32)> {
        self.disk.get().map_or_else(Vec::new, |runtime| {
            runtime
                .failed_saves
                .lock()
                .map_or_else(|_| Vec::new(), |mut failed| std::mem::take(&mut *failed))
        })
    }

    pub(super) fn note_refused_saves(&self, folder: Folder, keys: &[(i32, i32)]) {
        if folder == Folder::Region
            && let Some(runtime) = self.disk.get()
            && let Ok(mut failed) = runtime.failed_saves.lock()
        {
            failed.extend_from_slice(keys);
        }
    }
}
