use crate::game::ServerGame;
use sunrise_gameplay_common::game::debug_lines::ServerDebug;
use sunrise_world::heightmap::HeightmapType;

impl ServerGame<'_> {
    /// The server thread's half of the debug entries, in the level of
    /// `dim` while it is in place: the server chunk's heightmaps, the local
    /// difficulty, the last spawn state and the chunk source's counts.
    pub(in crate::game) fn server_debug_lines(&self, feet: [i32; 3]) -> ServerDebug {
        let [x, y, z] = feet;
        let key = (x >> 4, z >> 4);
        let loaded = self.common.world.chunks.contains_key(&key);
        let inside = y >= self.common.world.bottom_y() && y < self.common.world.top_y();
        let heights = loaded.then(|| {
            [
                HeightmapType::WorldSurface,
                HeightmapType::OceanFloor,
                HeightmapType::MotionBlocking,
                HeightmapType::MotionBlockingNoLeaves,
            ]
            .map(|kind| self.common.column_height(kind, x, z))
        });
        let difficulty = (loaded && inside).then(|| {
            let at = [f64::from(x), f64::from(y), f64::from(z)];
            self.common.local_difficulty(at).effective
        });
        let spawn_counts = self.server.spawn_counts.get(&self.common.dim).copied();
        // ServerLevel.gatherChunkSourceStats: the chunk map's size, then
        // PersistentEntitySectionManager's known and visible entities, its
        // sections, the chunks it loaded and sees, and its two queues
        // (Sunrise loads a chunk's entities with it: both stay empty).
        let (entities, loaded) = (
            self.common.debug_entity_count(),
            self.common.world.chunks.len(),
        );
        let chunks = Some((loaded, entities, self.common.debug_entity_sections()));
        ServerDebug {
            heights,
            difficulty,
            spawn_counts,
            chunks,
        }
    }
}
