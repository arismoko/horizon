//! Server capture seams. Disk callbacks run only in the persistence owner.
use super::*;
use horizon_client::chunk::Chunk;
use horizon_client::streaming::{ProtoSave, ProtoSnapshot};
use std::sync::Arc;

impl Game<'_> {
    /// Bind the existing world's identity and immutable resources, before workers start.
    pub(super) fn bind_chunk_disk_worker(
        &self,
        save: &horizon_client::save::SaveDir,
        dim: super::Dim,
    ) {
        if self.options.execution_role.is_client()
            || (!mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
                && !mod_api::__private::listened(mod_api::HookId::ChunkDataSave))
        {
            return;
        }
        if let (Some(world), Some(resources)) = (
            self.native_lifecycle.epoch(),
            horizon_world::native_worldgen_hooks::context(),
        ) {
            save.set_chunk_disk_context(
                world,
                mod_api::Identifier::parse(dim.id()).expect("registered dimension"),
                resources,
            );
        }
    }

    /// Observe the exact full-chunk save copies on the server thread.
    pub(super) fn native_full_chunk_snapshots(&mut self, chunks: &[Arc<Chunk>]) {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() {
            return;
        }
        for chunk in chunks {
            self.native_full_chunk_snapshot(chunk);
        }
    }

    pub(super) fn native_late_full_chunk_snapshot(&mut self, chunk: &Chunk) {
        if self.remote.is_none() && !mod_api::__private::mods().is_empty() {
            self.native_full_chunk_snapshot(chunk);
        }
    }

    fn native_full_chunk_snapshot(&mut self, _chunk: &Chunk) {
        self.bind_chunk_disk_worker(&self.save, self.dim);
    }

    /// The proto stays checked out until its immutable save record is submitted.
    pub(super) fn native_proto_snapshots(&mut self, protos: &[ProtoSave]) {
        if self.remote.is_some() || mod_api::__private::mods().is_empty() {
            return;
        }
        let access = self.streaming.proto_access();
        let dimension = self.dim.id();
        for save in protos {
            let Some(status) = access.status(&save.proto) else {
                continue;
            };
            self.native_proto_snapshot(ProtoSnapshot {
                dimension,
                pos: [save.pos.0, save.pos.1],
                proto: &save.proto,
                access: access.as_ref(),
                status,
            });
        }
    }

    fn native_proto_snapshot(&mut self, _snapshot: ProtoSnapshot<'_>) {
        self.bind_chunk_disk_worker(&self.save, self.dim);
    }
}

impl Game<'_> {
    /// Chunks whose subscribed save was refused or failed become dirty again.
    /// Without a `chunk_data_save` listener this does nothing.
    fn requeue_failed_chunk_saves(&mut self) {
        if mod_api::__private::listened(mod_api::HookId::ChunkDataSave) {
            for key in self.save.take_failed_chunk_saves() {
                self.world.modified.insert(key);
            }
        }
    }

    /// With a `chunk_data_save` listener, saves wait while an action savepoint
    /// is open, so tentative state is never captured: unloads, and the plan
    /// that causes them, are deferred until it closes (the design's "defer
    /// unload when the gate refuses"). Never defers without a listener.
    pub(super) fn defer_chunk_disk_save(&mut self) -> bool {
        if self.options.execution_role.is_client()
            || !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
        {
            return false;
        }
        self.requeue_failed_chunk_saves();
        Self::admit_entity_save().is_err()
    }

    /// Save the level in place: its changed chunks, its entities and its raids.
    pub(super) fn save_level(&mut self, wait: bool) -> Result<()> {
        self.admit_level_save()?;
        self.requeue_failed_chunk_saves();
        self.save_border();
        self.save_chunk_tickets()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match self.streaming.prepare_save(&mut self.world, wait, deadline) {
                horizon_client::streaming::SaveStep::Full(chunk) => {
                    self.native_late_full_chunk_snapshot(&chunk);
                    self.streaming.save_finished(chunk);
                }
                horizon_client::streaming::SaveStep::Protos(protos) => {
                    self.native_proto_snapshots(&protos);
                    self.streaming.finish_save(protos, wait);
                    break;
                }
                horizon_client::streaming::SaveStep::Pending => unreachable!(),
            }
        }
        let keys: Vec<_> = self.world.modified.iter().copied().collect();
        self.save.save_attachments(&self.world.attachments)?;
        for key in &keys {
            self.world.modified.remove(key);
        }
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.native_full_chunk_snapshots(&chunks);
        self.save
            .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        if !wait {
            self.world.modified.autosaved(&keys, Instant::now());
        }
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        if mod_api::__private::listened(mod_api::HookId::LevelSave) {
            self.native_level_save(self.dim.id());
        }
        Ok(())
    }
    /// `ChunkMap.saveChunksEagerly`: snapshot only while the tick and writer
    /// have room. NBT encoding, compression and durable writes stay on workers.
    pub(super) fn save_chunks_eagerly(&mut self) {
        // No world open (`world_root` is set by `finish_open_world`): the save
        // is `Game::new`'s placeholder under the working directory.
        if self.world.carrier_fences.is_broken()
            || self.remote.is_some()
            || self.world_root.as_os_str().is_empty()
            || self.agent.hold_server
            || self.clock.sprinting()
            || self
                .server_lists
                .as_ref()
                .is_some_and(|lists| !lists.lock().unwrap().autosave)
        {
            return;
        }
        if self.defer_chunk_disk_save() {
            return;
        }
        let deadline = self
            .server_tick_deadline
            .unwrap_or_else(|| Instant::now() + self.clock.tick_duration());
        let mut modified = std::mem::take(&mut self.world.modified);
        modified.save_eagerly(
            Instant::now(),
            || Instant::now() < deadline && self.save.active_chunk_writes() < 128,
            |key| self.world.chunks.contains_key(&key),
            |key| {
                // `saveChunkIfNeeded` saves only a LevelChunk, so a lit one; a
                // column still waiting for its light keeps its queue entry.
                let Some(chunk) = self.world.chunks.get(&key).filter(|c| c.lit) else {
                    return false;
                };
                let chunks = self.with_entities([chunk]);
                match self
                    .save
                    .save_later(&chunks, self.pack.assets.registry, world_biomes!(self))
                {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!("Horizon: failed to save changed chunks: {e}");
                        false
                    }
                }
            },
        );
        self.world.modified = modified;
    }
}
