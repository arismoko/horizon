/// The hook point. Does nothing in Sunrise without a listener.
impl crate::game::ServerGame<'_> {
    /// `ModifyCustomSpawnersEvent`: the logical server `level` builds its custom spawner list
    /// once when it opens. `spawners` edits the ordered list, which starts with vanilla's
    /// built-ins for the Overworld and empty for other dimensions; a mod inserts, removes and
    /// moves built-ins and owned `CustomSpawner` objects.
    pub(in crate::game) fn native_modify_custom_spawners(
        &self,
        level: mod_api::LevelRef,
        spawners: &mut mod_api::CustomSpawnersEditor,
    ) {
        self.fire_modify_custom_spawners(level, spawners);
    }
}
