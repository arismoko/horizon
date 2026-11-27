#![cfg(test)]
use mod_api::LevelRef;
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::DimensionKey;
    use sunrise_core::position::ChunkPos;
    use sunrise_sim::ticket_levels::Sources;
    use {
        sunrise_gameplay::game::native_entity_refs::tests::prepare_entity_ticking_neighborhood,
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
    };

    fn assert_reads(
        game: &sunrise_gameplay::game::ServerGame<'_>,
        level: LevelRef,
        pos: [i32; 3],
        expected: Option<bool>,
    ) {
        assert_eq!(level.entity_ticking(game, pos), expected);
        assert_eq!(level.should_tick_blocks_at(game, pos), expected);
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_follow_live_tickets_bounds_and_residency() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(-1, -1));
            game.server.tickets.0.update(Some(Sources {
                players: vec![(-1, -1)],
                view_distance: 2,
                simulation_distance: 2,
                ..Default::default()
            }));
            let before = game.world.chunks.len();
            for pos in [[-1, 64, -1], [-17, 64, -17], [-1, -64, -1], [-1, 319, -1]] {
                assert_reads(game, level, pos, Some(true));
            }
            // Resident x/z and live tickets isolate the height check.
            for pos in [[-1, -65, -1], [-1, 320, -1], [30_000_000, 64, -1]] {
                assert_reads(game, level, pos, Some(false));
            }
            // A missing center chunk inside the live ticket range.
            game.world.chunks.remove(&(-1, -1)).unwrap();
            assert_reads(game, level, [-1, 64, -1], Some(false));
            assert_eq!(game.world.chunks.len(), before - 1);
            game.world.entities_pending.insert((-2, -2));
            assert_eq!(
                level.should_tick_blocks_at(game, [-17, 64, -17]),
                Some(false)
            );
            game.world.entities_pending.clear();
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.server.tickets.0.update(None);
            assert_reads(game, level, [-17, 64, -17], Some(true));
            game.server.tickets.0.update(Some(Sources::default()));
            assert_reads(game, level, [-17, 64, -17], Some(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn ticking_reads_refuse_each_unavailable_handle_independently() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.server.tickets.0.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            let stale = LevelRef::new(
                mod_api::WorldEpoch::new(level.epoch().get() + 1),
                DimensionKey::OVERWORLD,
            );
            assert_reads(game, stale, [0, 64, 0], None);
            let unopened = LevelRef::new(level.epoch(), DimensionKey::NETHER);
            assert_reads(game, unopened, [0, 64, 0], None);
            game.enter_level(sunrise_gameplay_common::game::Dim::Nether)
                .unwrap();
            assert_reads(game, level, [0, 64, 0], None);
        });
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
            game.server.tickets.0.update(None);
            assert_reads(game, level, [0, 64, 0], Some(true));
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_reads(game, level, [0, 64, 0], None);
        });
    }
}
