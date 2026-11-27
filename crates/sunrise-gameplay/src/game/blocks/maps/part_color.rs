//! The colour `MapItem.update` takes from a block: the state's own `MapColor` id, which the
//! block's `map_color` parts may change (`IBlockExtension.getMapColor`).
use crate::game::ServerGame;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, OnceLock},
};
use sunrise_core::position::BlockPos;
use sunrise_gameplay_common::game::api_values::block_state::block_listens;
use sunrise_world::registry::BlockStateId;

/// The highest `MapColor` id; the ids run from 0 (`NONE`).
const LAST_MAP_COLOR: u8 = 61;

impl ServerGame<'_> {
    /// `BlockState.getMapColor` for `state` at `pos`: the state's own colour, which its block's
    /// parts may change. An answer above `LAST_MAP_COLOR` is refused, logged once per block,
    /// and the state's own colour is kept.
    pub(super) fn map_color_at(&self, pos: BlockPos, state: BlockStateId) -> u8 {
        let facts = self.common.res.facts().get(state);
        let own = facts.map_color;
        if !block_listens(mod_api::HookId::MapColor, state) {
            return own;
        }
        let mut color = own;
        self.native_map_color(self.common.dim.level(), pos, state, &mut color);
        if color > LAST_MAP_COLOR {
            static REFUSED: OnceLock<Mutex<HashSet<Arc<str>>>> = OnceLock::new();
            let first = REFUSED
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(facts.name.clone());
            if first {
                log::warn!(
                    "Sunrise: a map_color part of {} answered {color}, above the last map colour \
                     {LAST_MAP_COLOR}; the block keeps its own colour",
                    facts.name
                );
            }
            return own;
        }
        color
    }

    /// `map_color_at` for the default state of the block `name` at `pos`; `fallback` when the
    /// game has no such block.
    pub(super) fn default_map_color(&self, name: &str, fallback: u8, pos: BlockPos) -> u8 {
        self.common
            .pack
            .assets
            .registry
            .default_state(name)
            .map_or(fallback, |state| self.map_color_at(pos, state))
    }
}

#[cfg(test)]
mod tests {
    use super::super::MapData;
    use crate::game::{ServerGame, native_entity_refs::tests::with_game};

    /// The colour id of the pixel a map centred on the origin shows for block (5, 5), after
    /// the player at (5.5, 5.5) held it over a floor of the block `floor` at y 63.
    fn pixel(game: &mut ServerGame<'_>, floor: &str) -> u8 {
        let state = game.pack.assets.registry.default_state(floor).unwrap();
        game.world.set(5, 63, 5, state).unwrap();
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(5.5, 64.0, 5.5),
            )
            .unwrap(),
        );
        let map = MapData::fresh(5.5, 5.5, 0, game.dimension_id());
        game.maps.data.insert(0, map);
        // A map reveals one column in sixteen per update.
        for _ in 0..16 {
            game.update_map(0);
        }
        game.maps.data[&0].colors[69 + 69 * 128] >> 2
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_decides_the_map_colour_of_its_block() {
        // The fixture's parts: stone answers 18, cobblestone (stone's colour, 11) answers 200.
        mod_platform::__private::part_tests::map_color_game_thread();
        with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            assert_eq!(pixel(game, "minecraft:stone"), 18);
            // A block with no part draws as before: dirt's own colour.
            assert_eq!(pixel(game, "minecraft:dirt"), 10);
            // An answer above 61 is refused and the block keeps its own colour.
            assert_eq!(pixel(game, "minecraft:cobblestone"), 11);
        });
    }
}
