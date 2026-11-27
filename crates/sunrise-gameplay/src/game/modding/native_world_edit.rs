//! Bulk block edits, chunk relighting and chunk resends for server mods.
use crate::game::ServerGame;
use sunrise_level::chunk_streamer::ChunkStreamer;
use sunrise_level::world::World;
use sunrise_sim::lan::hub::Hub;

impl ServerGame<'_> {
    /// For a server mod: the runs made at once, straight into the chunks of this server level
    /// (active or parked), with what `fill ... strict` does for block entities (a block replaced
    /// by another loses its block entity, and a new block gets the one `set_block` would give it,
    /// a mod block's included); no neighbour is told, and changed chunks go to players whole.
    /// Runs in unloaded chunks and parts of runs outside the column are skipped.
    pub(in crate::game) fn level_edit_blocks(
        &mut self,
        level: mod_api::LevelRef,
        edit: &mod_api::BlockEdit,
    ) -> Result<u64, mod_api::BlockEditError> {
        let started = std::time::Instant::now();
        let changed = self.with_server_level_mut(
            level,
            mod_api::BlockEditError::Unavailable,
            mod_api::BlockEditError::Stale,
            None,
            |game| game.apply_block_edit(edit),
        )?;
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self
            .common
            .options
            .spikes
            .is_some_and(|threshold| ms > threshold)
        {
            log::info!(
                "Sunrise: slow mod block edit {ms:.1} ms ({} runs, {changed} blocks)",
                edit.runs.len()
            );
        }
        Ok(changed)
    }
    /// Resolve an open server level, its streaming scheduler, and its server's hub, without
    /// switching levels.
    fn resolve_chunk_level_mut(
        &mut self,
        level: mod_api::LevelRef,
    ) -> Result<(&mut World, &mut ChunkStreamer, Option<&Hub>), mod_api::ChunkRefusal> {
        use mod_api::ChunkRefusal;
        if !self.common.is_logical_server() {
            return Err(ChunkRefusal::Client);
        }
        if !self.level_handle_current(level) {
            return Err(ChunkRefusal::Stale);
        }
        let (world, streaming) = if level.dimension() == self.common.dim.key() {
            (&mut self.common.world, &mut self.server.streaming)
        } else {
            let parked = self
                .server
                .levels
                .0
                .parked_mut_of(level.dimension_id())
                .ok_or(ChunkRefusal::Stale)?;
            (&mut parked.world, &mut parked.streaming)
        };
        Ok((
            world,
            streaming,
            self.server.lan.as_ref().map(|host| &host.hub),
        ))
    }
    /// `chunks` of any open server level (active or parked), and the loaded ring around each
    /// loaded chunk, queued to be lit again whole on that level's light engine
    /// (`World::dirty_light`, matching vanilla `ThreadedLevelLightEngine` queueing and batching):
    /// light jobs run asynchronously on the owning level's light updates, never synchronously on
    /// this call. Returns how many columns were newly queued (ring included, columns already
    /// waiting not counted).
    pub(in crate::game) fn level_relight_chunks(
        &mut self,
        level: mod_api::LevelRef,
        chunks: &[[i32; 2]],
    ) -> Result<u32, mod_api::ChunkRefusal> {
        let (world, _, _) = self.resolve_chunk_level_mut(level)?;
        let mut queued = 0u32;
        for &[x, z] in chunks {
            if !world.chunks.contains_key(&(x, z)) {
                continue;
            }
            for dz in -1..=1 {
                let Some(nz) = z.checked_add(dz) else {
                    continue;
                };
                for dx in -1..=1 {
                    let Some(nx) = x.checked_add(dx) else {
                        continue;
                    };
                    let key = (nx, nz);
                    if world.chunks.contains_key(&key) && world.dirty_light.insert(key) {
                        queued = queued.saturating_add(1);
                    }
                }
            }
        }
        Ok(queued)
    }
    /// Queue a loaded chunk to go whole to the players who hold it, on its level's block-change
    /// broadcast after the column's pending light work (`ChunkMap.waitForLightBeforeSending`).
    pub(in crate::game) fn chunk_resend(
        &mut self,
        chunk: mod_api::ChunkRef,
    ) -> Result<(), mod_api::ChunkRefusal> {
        let level = chunk.level();
        let [x, z] = chunk.pos();
        let (world, streaming, hub) = self.resolve_chunk_level_mut(level)?;
        if !world.chunks.contains_key(&(x, z)) {
            return Err(mod_api::ChunkRefusal::NotLoaded);
        }
        if hub.is_some_and(|hub| hub.has_chunk_resend_target(level.dimension_id(), (x, z))) {
            streaming.queue_chunk_resend(world, (x, z));
        }
        Ok(())
    }
}
