//! Bulk block edits for server mods: a helper on the level being ticked, as `native_set_block` is, that makes many
//! runs of blocks up columns at once, straight into the chunks.
use super::Game;

impl Game<'_> {
    /// For a server mod: the runs made at once, straight into the chunks of the authoritative level `dimension`
    /// (the one being ticked, as for `native_set_block`), with what `fill ... strict` does for block entities (a
    /// block replaced by another loses its block entity, and a new block gets the one `native_set_block` would
    /// give it, a mod block's included); no neighbour is told unless `neighbour_updates`, and the chunks changed
    /// go to the players whole.
    /// Runs in chunks not loaded, and the parts of runs outside the column, are left out. The blocks changed, or
    /// why the edit was left out, with nothing changed: another dimension, a client of a server, a palette entry
    /// or filter that doesn't read or carries NBT (refused), a run naming a palette entry or filter past the end,
    /// or more than 65536 filters.
    pub(super) fn edit_blocks(
        &mut self,
        dimension: &str,
        edit: &mod_api::BlockEdit,
    ) -> Result<u64, String> {
        if !self.native_level_open(dimension) {
            return Err(format!("{dimension} is not the level being ticked"));
        }
        let started = std::time::Instant::now();
        let changed = self.apply_block_edit(edit)?;
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        if self.options.spikes.is_some_and(|threshold| ms > threshold) {
            log::info!(
                "Horizon: slow mod block edit {ms:.1} ms ({} runs, {changed} blocks)",
                edit.runs.len()
            );
        }
        Ok(changed)
    }
    /// `chunks` of the level in place, and the loaded ring around them, lit again now
    /// (`Streaming::relight_now`): how many were lit. None for another level or a stale handle.
    pub(super) fn level_relight_chunks(
        &mut self,
        level: mod_api::LevelRef,
        chunks: &[[i32; 2]],
    ) -> Option<u32> {
        if level.dimension() != self.dim.key() {
            return None;
        }
        self.native_level_world(level)?;
        let mut columns: Vec<(i32, i32)> = chunks
            .iter()
            .flat_map(|&[x, z]| {
                (-1..=1).flat_map(move |dz| {
                    (-1..=1).map(move |dx| (x.wrapping_add(dx), z.wrapping_add(dz)))
                })
            })
            .filter(|key| self.world.chunks.contains_key(key))
            .collect();
        columns.sort_unstable();
        columns.dedup();
        Some(self.streaming.relight_now(&mut self.world, &columns) as u32)
    }
    /// The chunk queued to go whole to its watchers at the end of the tick, as an
    /// `edit_blocks` chunk does (`World::bulk_log`). False for a chunk not loaded or not in
    /// the level in place.
    pub(super) fn chunk_resend(&mut self, chunk: mod_api::ChunkRef) -> bool {
        let level = chunk.level();
        if level.dimension() != self.dim.key() || self.native_level_world(level).is_none() {
            return false;
        }
        let [x, z] = chunk.pos();
        if !self.world.chunks.contains_key(&(x, z)) {
            return false;
        }
        if self.world.block_log.is_some() {
            self.world.bulk_log.insert((x, z));
        }
        true
    }
    /// Whether `dimension` is the authoritative level being ticked: where a server mod's world helpers act.
    pub(super) fn native_level_open(&self, dimension: &str) -> bool {
        (self.serving.is_some() || self.options.dedicated.is_some()) && dimension == self.dim.id()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::chunk::Chunk;
    use std::sync::Arc;

    /// A stone roof written straight into a chunk is lit at once by
    /// `level_relight_chunks`, and `chunk_resend` queues the chunk to go out whole.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_relights_and_resends_chunks_it_wrote() {
        with_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let mut chunk = Chunk::empty(100 + dx, 100 + dz);
                    if dx == 0 && dz == 0 {
                        for (x, z) in (0..16).flat_map(|x| (0..16).map(move |z| (x, z))) {
                            chunk.set(x, 100, z, stone.0);
                        }
                    }
                    game.world
                        .chunks
                        .insert((100 + dx, 100 + dz), Arc::new(chunk));
                }
            }
            let level = game.level(game.dim.id()).unwrap();
            let (x, z) = (100 * 16 + 8, 100 * 16 + 8);
            // Only the middle chunk has all eight neighbours loaded.
            assert_eq!(level.relight_chunks(game, &[[100, 100]]), Some(1));
            assert!(game.world.light(x, 90, z) >> 4 < 15, "shaded under the roof");
            assert_eq!(game.world.light(x, 110, z) >> 4, 15, "open sky above it");
            assert_eq!(game.level_relight_chunks(level, &[[400, 400]]), Some(0));
            let stale = mod_api::LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                level.dimension(),
            );
            assert_eq!(game.level_relight_chunks(stale, &[[100, 100]]), None);

            game.world.block_log = Some(Vec::new());
            game.world.bulk_log.clear();
            let chunk = game.chunk(level, [100, 100]).unwrap();
            assert!(chunk.resend(game));
            assert!(game.world.bulk_log.contains(&(100, 100)));
            let unloaded = mod_api::__private::game::chunk_ref(level, [400, 400]);
            assert!(!game.chunk_resend(unloaded));
            assert!(!game.world.bulk_log.contains(&(400, 400)));
        });
    }
}
