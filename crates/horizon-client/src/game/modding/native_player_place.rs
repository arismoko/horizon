//! A block a mod writes as a player placed it: `BlockItem.place`'s write from a chosen state.
use super::Game;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use mod_api::{LevelRef, PlayerRef};

impl Game<'_> {
    /// `Behaviours::place_block_state` in the player's turn, then the block entity a new block
    /// brings, as a placement makes it; the same block in another state keeps its own.
    pub(super) fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> bool {
        if !self.is_logical_server() || self.pack.assets.registry.state(state).is_none() {
            return false;
        }
        let mut placed = false;
        let ran = self.native_as_player_handle(player, |game| {
            if game.native_level_world(level).is_none() || level.dimension() != game.dim.key() {
                return;
            }
            let previous = game.world.block(pos[0], pos[1], pos[2]);
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            let Some(now) = game.with_level(|b, level, ticks| {
                b.place_block_state(level, ticks, at, state, update_neighbour_shapes)
            }) else {
                let _ = game.process_level_events();
                return;
            };
            let key = (pos[0], pos[1], pos[2]);
            let facts = game.res.facts();
            let same_block = facts.get(previous).name == facts.get(now).name;
            if !same_block
                && !game.world.block_entities.stateful_entity(now.0)
                && let Some(entity) = game.new_block_entity_for_state(now)
            {
                game.world.block_entities.insert(key, entity);
            }
            let _ = game.process_level_events();
            placed = true;
        });
        ran && placed
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// A bed written for a player gets its head (`setPlacedBy`), and nothing is written for
    /// another level or an unknown state.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_block_placed_as_a_player_runs_set_placed_by() {
        with_guests(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let b = game.player(13).unwrap();
            let level = game.level(game.dim.id()).unwrap();
            let registry = &game.pack.assets.registry;
            let bed = registry.default_state("minecraft:red_bed").unwrap();
            let foot = [2, 70, 2];
            assert!(game.player_place_block(b, level, foot, bed, true));
            assert_eq!(game.world.block(2, 70, 2), bed);
            let head = game.world.block(2, 70, 1);
            let facts = game.res.facts();
            assert_eq!(facts.get(head).name, facts.get(bed).name, "the head was placed");
            assert!(!game.player_place_block(
                b,
                level,
                foot,
                horizon_world::registry::BlockStateId(u32::MAX),
                true
            ));
        });
    }
}
