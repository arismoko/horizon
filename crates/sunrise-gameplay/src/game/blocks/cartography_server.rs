//! Server-side cartography map loading and registration (`MapItem.scaleMap`, `lockMap`).
use super::{cartography::LevelMaps, maps::map_id};
use std::collections::HashMap;
use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// The maps of `stacks` (loaded as needed) for a menu over this level.
    pub fn level_maps<'s>(&mut self, stacks: impl IntoIterator<Item = &'s ItemStack>) -> LevelMaps {
        let ids: Vec<i32> = stacks.into_iter().filter_map(map_id).collect();
        let mut states = HashMap::new();
        for id in ids {
            if let Some(m) = self.map(id) {
                states.insert(id, (m.locked, m.scale));
            }
        }
        LevelMaps::new(states, self.peek_map_id())
    }

    /// The scaled and locked copies a menu made (`level.setMapData`), staged if a transaction is open.
    pub fn register_made_maps(&mut self, maps: LevelMaps) {
        for (id, source, scale) in maps.into_made() {
            let Some(data) = self
                .map(source)
                .map(|m| if scale { m.scaled() } else { m.locked_copy() })
            else {
                continue;
            };
            let new = self.new_map_id(data);
            debug_assert_eq!(new, id, "map ids follow peek_map_id");
        }
    }
}
