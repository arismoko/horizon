//! Ordered region writer and durable completion ownership.
use super::*;

impl SaveDir {
    pub(super) fn queue(
        &self,
        folder: Folder,
        records: Vec<((i32, i32), Record)>,
        wait: bool,
    ) -> Result<()> {
        if !records.is_empty() {
            let mut writer = self.writer.lock().map_err(|_| "writer poisoned")?;
            let records = if self.disk.get().is_some()
                || mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
            {
                let keys: Vec<_> = records.iter().map(|(key, _)| *key).collect();
                match records
                    .into_iter()
                    .map(|(key, record)| {
                        self.capture_disk_record(record).map(|record| (key, record))
                    })
                    .collect::<Result<Vec<_>>>()
                {
                    Ok(records) => records,
                    Err(error) => {
                        // A refused capture must leave its chunks dirty.
                        self.note_refused_saves(folder, &keys);
                        return Err(error);
                    }
                }
            } else {
                records
            };
            let mut batch = Vec::with_capacity(records.len());
            {
                let mut pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
                for (key, record) in records {
                    let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
                    pending.insert((folder, key.0, key.1), (seq, record.clone()));
                    batch.push((key, record, seq));
                }
            }
            *self
                .outstanding
                .count
                .lock()
                .map_err(|_| "writer poisoned")? += 1;
            let writer = writer.get_or_insert_with(|| self.start_writer());
            let chunks = if folder == Folder::Region {
                batch.len()
            } else {
                0
            };
            self.outstanding.chunks.fetch_add(chunks, Ordering::Relaxed);
            if writer.send.send((folder, batch)).is_err() {
                self.outstanding.chunks.fetch_sub(chunks, Ordering::Relaxed);
                *self
                    .outstanding
                    .error
                    .lock()
                    .map_err(|_| "writer poisoned")? = Some("the region writer stopped".into());
                *self
                    .outstanding
                    .count
                    .lock()
                    .map_err(|_| "writer poisoned")? -= 1;
                self.outstanding.done.notify_all();
                return Err("the region writer stopped".into());
            }
        }
        if wait {
            self.flush()?;
        }
        Ok(())
    }
    fn start_writer(&self) -> Writer {
        let (send, receive) = mpsc::channel::<WriteBatch>();
        let root = self.root.clone();
        let (regions, entity_regions, poi_regions) = (
            self.regions.clone(),
            self.entity_regions.clone(),
            self.poi_regions.clone(),
        );
        let pending = self.pending.clone();
        let outstanding = self.outstanding.clone();
        let thread = std::thread::Builder::new()
            .name("horizon-region-writer".into())
            .spawn(move || {
                for (kind, batch) in receive {
                    let cache = match kind {
                        Folder::Region => &regions,
                        Folder::Entities => &entity_regions,
                        Folder::Poi => &poi_regions,
                    };
                    let folder = kind.name();
                    // IOWorker.pendingWrites: a chunk queued again since is
                    // written once, by its newest batch.
                    let newest = |k: &(i32, i32), seq: u64| {
                        pending.lock().map_or(true, |p| {
                            p.get(&(kind, k.0, k.1)).is_none_or(|p| p.0 <= seq)
                        })
                    };
                    let resolved: Vec<_> = batch
                        .iter()
                        .filter(|(k, _, seq)| newest(k, *seq))
                        .filter_map(|(k, r, _)| Some((*k, resolve_poi(&root, cache, *k, r)?)))
                        .collect();
                    let records: Vec<_> = resolved.iter().map(|(k, r)| (*k, r)).collect();
                    let result = (|| -> Result<()> {
                        chunk_disk::prepare_batch(&batch)?;
                        write_region_records(&root, cache, folder, &records)
                    })();
                    if let Err(e) = &result {
                        chunk_disk::note_failed(&batch);
                        let message = format!("failed to write {folder} files: {e}");
                        log::warn!("Horizon: {message}");
                        *outstanding.error.lock().unwrap() = Some(message);
                        // `reportChunkSaveFailure` for each chunk of the batch.
                        for (k, _, _) in &batch {
                            crate::chunk_io::report_save_failure(&root, k.0, k.1, e);
                        }
                    }
                    // Keep failed snapshots readable; never report them as durable.
                    if result.is_ok()
                        && let Ok(mut pending) = pending.lock()
                    {
                        for (k, _, seq) in &batch {
                            if pending.get(&(kind, k.0, k.1)).is_some_and(|p| p.0 == *seq) {
                                pending.remove(&(kind, k.0, k.1));
                            }
                        }
                    }
                    if kind == Folder::Region {
                        outstanding.chunks.fetch_sub(batch.len(), Ordering::Relaxed);
                    }
                    if let Ok(mut count) = outstanding.count.lock() {
                        *count -= 1;
                        outstanding.done.notify_all();
                    }
                }
            })
            .expect("spawn region writer");
        Writer { send, thread }
    }
}
