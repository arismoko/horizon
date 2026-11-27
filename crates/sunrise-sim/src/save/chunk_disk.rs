//! Server-owned chunk-disk callbacks and the captured envelopes handed to region writers.
use super::*;
use mod_api::{ChunkDiskContext, ChunkDiskJob, ChunkDiskKind, ChunkDiskPhase, WorldEpoch};
use sunrise_level::api_context::WorldgenReader;
use sunrise_mod_runtime::attachments::{AttachmentEncodeError, Attachments};

pub(super) struct DiskRuntime {
    world: WorldEpoch,
    dimension: mod_api::Identifier,
    resources: Arc<sunrise_world::native_load_context::WorldgenContext>,
    load: bool,
    save: bool,
}

impl DiskRuntime {
    fn capture(
        &self,
        chunk: &Chunk,
        kind: ChunkDiskKind,
        phase: ChunkDiskPhase,
    ) -> std::result::Result<Option<ChunkDiskJob>, AttachmentEncodeError> {
        let context = ChunkDiskContext::new(
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

    fn run(&self, job: &ChunkDiskJob) -> Compound {
        let reader = WorldgenReader(&self.resources);
        // A disk job has no block-state index (that is the game's, published for worldgen), so
        // `state_with_property` answers None, as worldgen does before the index is published.
        job.run(
            &mod_api::OwningWorldgen::new(&reader, &|_, _, _| None, &|_, _| None),
            sunrise_world::datafix::upgrade_compound,
        )
        .clone()
    }
}

pub(super) struct DiskRecord {
    pub(super) original: Record,
    entries: Compound,
    encoded: OnceLock<std::result::Result<Option<Arc<NamedTag>>, String>>,
}
impl DiskRecord {
    pub(super) fn cancel(&self) {
        let _ = self
            .encoded
            .set(Err("chunk save cancelled before encoding".into()));
    }

    fn encode(&self) -> std::io::Result<Option<Arc<NamedTag>>> {
        self.encoded
            .get_or_init(|| {
                // Cache the encoded record so pending readers share the writer's outcome.
                super::compression_pool::catch_encoding_panic(|| {
                    self.original
                        .encode_tag(true, Some(&self.entries))?
                        .map(Some)
                        .ok_or_else(|| std::io::Error::other("captured chunk has no disk record"))
                })
                .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(std::io::Error::other)
    }

    pub(super) fn without_block_entity_edits(&self) -> Option<Arc<NamedTag>> {
        self.original.stored_block_entities(Some(&self.entries))
    }

    // Readers may wait for the owning writer, but never execute save callbacks.
    pub(super) fn read(&self) -> Option<Arc<NamedTag>> {
        match self.encoded.wait() {
            Ok(tag) => tag.clone(),
            // A reader that borrowed this snapshot before completion still owns that read.
            Err(_) => self.original.tag(),
        }
    }
}

pub(super) fn encode_record(record: &Record) -> std::io::Result<Option<Arc<NamedTag>>> {
    match record.disk_job() {
        Some(record) => record.encode(),
        None => record.tag_for_write(),
    }
}

impl SaveDir {
    /// Bind disk callbacks before this server level starts any chunk workers.
    /// The world epoch belongs to the existing lifecycle owner. Remote caches
    /// never call this method. With no subscribers no context is retained.
    pub fn set_chunk_disk_context(
        &self,
        world: WorldEpoch,
        dimension: mod_api::Identifier,
        resources: Arc<sunrise_world::native_load_context::WorldgenContext>,
    ) {
        let load = mod_api::__private::listened(mod_api::HookId::ChunkDataLoad);
        let save = mod_api::__private::listened(mod_api::HookId::ChunkDataSave);
        if !load && !save {
            return;
        }
        let _ = self.disk.set(Arc::new(DiskRuntime {
            world,
            dimension,
            resources,
            load,
            save,
        }));
    }

    pub(super) fn capture_disk_record(
        &self,
        record: Record,
    ) -> std::result::Result<Record, AttachmentEncodeError> {
        let (chunk, kind) = match &record {
            Record::Chunk(chunk, ..) => (&**chunk, ChunkDiskKind::Full),
            Record::Proto(proto, ..) => (&proto.column, ChunkDiskKind::Proto),
            _ => return Ok(record),
        };
        let entries = if let Some(runtime) = self.disk.get()
            && let Some(job) = runtime.capture(chunk, kind, ChunkDiskPhase::Save)?
        {
            runtime.run(&job)
        } else if let Some(extras) = &chunk.saved
            && extras.attachments.has_live_persistence()
        {
            extras.attachments.try_to_nbt()?
        } else {
            return Ok(record);
        };
        Ok(Record::Disk(Arc::new(DiskRecord {
            original: record,
            entries,
            encoded: OnceLock::new(),
        })))
    }

    pub(super) fn has_chunk_disk_hook(&self, phase: ChunkDiskPhase) -> bool {
        self.disk.get().is_some_and(|runtime| match phase {
            ChunkDiskPhase::Load => runtime.load,
            ChunkDiskPhase::Save => runtime.save,
            _ => false,
        })
    }

    /// Dispatch on the owner after disk decode and before publishing or restoring the chunk.
    pub(super) fn load_disk_attachments(&self, chunk: &mut Chunk, kind: ChunkDiskKind) {
        let Some(runtime) = self.disk.get() else {
            // Non-authoritative remote caches have no disk context.
            return;
        };
        match runtime.capture(chunk, kind, ChunkDiskPhase::Load) {
            Ok(Some(job)) => {
                Arc::make_mut(chunk.saved.get_or_insert_with(Default::default)).attachments =
                    Attachments::from_nbt(runtime.run(&job));
            }
            Ok(None) => {}
            Err(error) => {
                log::error!(
                    "Sunrise: chunk [{}, {}] disk-load attachment capture failed: {error}",
                    chunk.x,
                    chunk.z
                );
            }
        }
    }

    pub(super) fn save_disk_sync(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
        items: &crate::items::Items,
    ) -> Option<Result<usize>> {
        if !mod_api::__private::listened(mod_api::HookId::ChunkDataSave) {
            return None;
        }
        let runtime = self.disk.get()?;
        Some((|| {
            let edits: Vec<_> = chunks
                .iter()
                .map(|chunk| {
                    runtime
                        .capture(chunk, ChunkDiskKind::Full, ChunkDiskPhase::Save)
                        .map(|job| job.map(|job| runtime.run(&job)))
                })
                .collect();
            let work = || {
                chunks
                    .iter()
                    .zip(edits)
                    .filter_map(|(chunk, entries)| {
                        let key = (chunk.x, chunk.z);
                        let entries = match entries {
                            Ok(entries) => entries,
                            Err(error) => {
                                self.report_record_failure(
                                    Folder::Region,
                                    key,
                                    &std::io::Error::other(error),
                                );
                                return None;
                            }
                        };
                        let tag = compression_pool::catch_encoding_panic(|| {
                            let blending = self.calculated_blending(chunk.x, chunk.z);
                            let mut tag = encode_full(
                                chunk,
                                registry,
                                biomes,
                                self.tables.get().map(|t| &**t),
                                self.shape,
                                items,
                                ChunkEncodingEdits {
                                    block_entities: true,
                                    attachments: entries.as_ref(),
                                },
                            )
                            .map_err(std::io::Error::other)?;
                            if let Some(blending) = blending {
                                put_calculated_blending(&mut tag, &blending);
                            }
                            Ok(tag)
                        });
                        match tag {
                            Ok(tag) => Some((key, Some(tag))),
                            Err(error) => {
                                self.report_record_failure(Folder::Region, key, &error);
                                None
                            }
                        }
                    })
                    .collect()
            };
            let records = std::thread::scope(|scope| scope.spawn(work).join())
                .map_err(|_| "chunk disk worker unwound")?;
            self.write_records("region", records, true)?;
            self.save_poi(chunks, true)?;
            Ok(chunks.len())
        })())
    }
}
