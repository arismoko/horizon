use crate::game::ServerGame;

impl ServerGame<'_> {
    /// Install the open world's fluid interactions, replacing the last load's: vanilla's two
    /// lava rules for each fluid of the world's `minecraft:lava` tag (water read from its
    /// `minecraft:water` tag; a data pack may edit both), then native mods' declarations, then
    /// content mods' files in load order and file name. Tags are read again on each call. An
    /// id that names no fluid or block refuses the world by name.
    pub fn install_fluid_interactions(&self) -> Result<(), String> {
        let items = self
            .server
            .world_tables
            .0
            .as_ref()
            .map_or_else(|| self.items(), |t| &t.items);
        self.common.install_fluid_interactions_from(items)
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
