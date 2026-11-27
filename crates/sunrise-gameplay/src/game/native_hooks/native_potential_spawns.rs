use sunrise_level::biome::SpawnEntry;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// Possible natural spawns at a server position (NeoForge's
    /// `LevelEvent.PotentialSpawns`, `NaturalSpawner.mobsAt`). Mods may add or
    /// remove entries, or cancel to make the list empty. Mods can read the world,
    /// but cannot write it during the spawn pass. 388.
    pub(in crate::game) fn native_potential_spawns(
        &self,
        dimension: &str,
        category: &str,
        pos: [i32; 3],
        entries: &mut Vec<SpawnEntry>,
        canceled: &mut bool,
    ) {
        self.fire_potential_spawns(dimension, category, pos, entries, canceled);
    }
}
