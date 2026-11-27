//! Server-level binding and save admission for chunk disk callbacks.
use super::*;

impl crate::game::ServerGame<'_> {
    /// Bind a newly opened level's save directory before publishing its streamer.
    /// `Persistent` and the server share this directory; only its submitted records
    /// become disk jobs.
    pub(in crate::game::server_side) fn bind_chunk_disk_worker(
        &self,
        save: &sunrise_sim::save::SaveDir,
        dim: super::Dim,
    ) {
        if self.common.options.execution_role.is_client()
            || (!mod_api::__private::listened(mod_api::HookId::ChunkDataLoad)
                && !mod_api::__private::listened(mod_api::HookId::ChunkDataSave))
        {
            return;
        }
        if let (Some(world), Some(resources)) = (
            self.common.native_lifecycle.epoch(),
            sunrise_world::native_worldgen_hooks::context(),
        ) {
            save.set_chunk_disk_context(
                world,
                mod_api::Identifier::parse(dim.id()).expect("registered dimension"),
                resources,
            );
        }
    }
}

impl crate::game::ServerGame<'_> {
    /// Defer eager saves, streaming/unload and closing-save polling at their
    /// gameplay call sites while an action savepoint holds an entity pass open.
    /// With no save listener these call sites do not take this deferral path.
    pub(in crate::game) fn defer_chunk_disk_save(&mut self) -> bool {
        if self.common.options.execution_role.is_client()
            || !mod_api::__private::listened(mod_api::HookId::ChunkDataSave)
        {
            return false;
        }
        sunrise_gameplay_common::game::CommonGame::admit_entity_save().is_err()
    }

    /// `ChunkMap.saveChunksEagerly`: snapshot only while the tick and writer
    /// have room. NBT encoding, compression and durable writes stay on workers.
    pub fn save_chunks_eagerly(&mut self) {
        self.save_chunks_eagerly_at(Instant::now());
    }

    pub fn save_chunks_eagerly_at(&mut self, now: Instant) {
        // No world open (`world_root` is set by `finish_open_world`): the save
        // is `Game::new`'s placeholder under the working directory.
        if self.common.world.carrier_fences.is_broken()
            || self.server.world_root.as_os_str().is_empty()
            || self.server.hold_server
            || self.common.clock.sprinting()
            || self
                .server
                .server_lists
                .0
                .as_ref()
                .is_some_and(|lists| !lists.lock().unwrap().autosave)
        {
            return;
        }
        if self.defer_chunk_disk_save() {
            return;
        }
        let deadline = self
            .server
            .server_tick_deadline
            .unwrap_or_else(|| Instant::now() + self.common.clock.tick_duration());
        let mut modified = std::mem::take(&mut self.common.world.modified);
        modified.save_eagerly(
            now,
            || Instant::now() < deadline && self.server.save.0.active_chunk_writes() < 128,
            // Only a lit column is a LevelChunk. An unlit one stays queued.
            |key| self.common.world.chunks.get(&key).map(|chunk| chunk.lit),
            |key, _| {
                let chunks = self.with_entities([&self.common.world.chunks[&key]]);
                match self.server.save.0.save_later(
                    &chunks,
                    self.common.pack.assets.registry,
                    world_biomes!(self),
                ) {
                    Ok(_) => true,
                    Err(e) => {
                        log::warn!("Sunrise: failed to save changed chunks: {e}");
                        false
                    }
                }
            },
        );
        self.common.world.modified = modified;
    }
}
