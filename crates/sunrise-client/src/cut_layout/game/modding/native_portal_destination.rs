#![cfg(test)]
use mod_api::PortalAngle;
use mod_api::PortalError;
use mod_api::PortalRotation;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_level::chunk::Chunk;

    const SIZE: [f64; 2] = [0.98, 0.98];
    const ENTRY: [i32; 3] = [8, 70, 8];

    fn nether_portal_blocks(game: &sunrise_gameplay::game::ServerGame<'_>) -> usize {
        let b = game.res.behaviours();
        let side = game.portal_side(Dim::Nether).expect("the Nether is open");
        side.world
            .chunks
            .values()
            .map(|c| c.blocks_where(|s| b.portal_axis(s).is_some()).len())
            .sum()
    }

    fn prepare(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.build_registries();
        game.set_allow_commands(true);
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle.world_open_ready().unwrap();
        game.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
    }

    /// The chunks of the level in place that every answer reads around the entry, but `skip`.
    fn load_around(game: &mut sunrise_gameplay::game::ServerGame<'_>, skip: Option<(i32, i32)>) {
        for z in -3..=3 {
            for x in -3..=3 {
                if Some((x, z)) != skip && !game.world.chunks.contains_key(&(x, z)) {
                    game.server
                        .streaming
                        .insert_remote(&mut game.common.world, Chunk::empty(x, z));
                }
            }
        }
    }

    fn set(game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str, [x, y, z]: [i32; 3]) {
        let state = game.pack.assets.registry.default_state(name).unwrap();
        game.world.set(x, y, z, state).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn a_portal_is_asked_where_it_leads_and_find_or_create_builds_its_exit() {
        with_game(|game| {
            prepare(game);
            set(game, "minecraft:nether_portal", ENTRY);
            let overworld = game.level("minecraft:overworld").unwrap();
            let ask = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                game.level_portal_destination(overworld, ENTRY, SIZE)
            };
            load_around(game, None);
            // The Nether is not open.
            assert_eq!(ask(game), Err(PortalError::ChunksNotLoaded));
            // Open with none of the chunks the search reads.
            game.enter_level(Dim::Nether).unwrap();
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(ask(game), Err(PortalError::ChunksNotLoaded));
            // Its chunks loaded, nothing stands there: the read finds none and builds none.
            game.enter_level(Dim::Nether).unwrap();
            load_around(game, None);
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(ask(game), Err(PortalError::NoExit));
            assert_eq!(nether_portal_blocks(game), 0);
            // A block that is no portal; sizes no float placement can use.
            assert_eq!(
                game.level_portal_destination(overworld, [9, 70, 9], SIZE),
                Err(PortalError::NotAPortal)
            );
            for pos in [
                [i32::MAX, 70, 8],
                [i32::MIN, 70, 8],
                [8, 70, i32::MAX],
                [8, 70, i32::MIN],
            ] {
                assert_eq!(
                    game.level_portal_destination(overworld, pos, SIZE),
                    Err(PortalError::ChunksNotLoaded)
                );
            }
            for size in [
                [f64::NAN, 1.0],
                [1.0, f64::INFINITY],
                [-0.5, 1.0],
                [1.0, f64::from(f32::MAX) * 2.0],
            ] {
                assert_eq!(
                    game.level_portal_destination(overworld, ENTRY, size),
                    Err(PortalError::InvalidSize)
                );
            }
            // Not inside a transaction.
            game.storage_scope(|game, _| {
                assert_eq!(
                    game.level_find_or_create_portal_exit(overworld, ENTRY, SIZE),
                    Err(PortalError::InTransaction)
                );
                ((), false)
            });
            // Find or create builds one, and the read then finds it.
            let built = game
                .level_find_or_create_portal_exit(overworld, ENTRY, SIZE)
                .expect("the exit is built");
            assert_eq!(game.dim, Dim::Overworld);
            assert!(nether_portal_blocks(game) > 0);
            assert_eq!(built.level, game.level("minecraft:the_nether").unwrap());
            assert!(built.exit_portal.is_some());
            assert_eq!(ask(game), Ok(built));
        });
    }

    /// A two-wide portal across the edge of chunks (0, 0) and (1, 0) is read whole or not at
    /// all; the exit is the middle of the Nether portal's two blocks.
    #[test]
    #[ignore = "daily"]
    fn a_portal_across_a_chunk_edge_waits_for_its_far_chunk() {
        with_game(|game| {
            prepare(game);
            let overworld = game.level("minecraft:overworld").unwrap();
            let entry = [15, 70, 8];
            load_around(game, Some((1, 0)));
            for y in 70..73 {
                set(game, "minecraft:nether_portal", [15, y, 8]);
            }
            game.enter_level(Dim::Nether).unwrap();
            load_around(game, None);
            for x in 0..2 {
                for y in 70..73 {
                    set(game, "minecraft:nether_portal", [x, y, 1]);
                }
            }
            game.enter_level(Dim::Overworld).unwrap();
            assert_eq!(
                game.level_portal_destination(overworld, entry, SIZE),
                Err(PortalError::ChunksNotLoaded)
            );
            game.server
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(1, 0));
            for y in 70..73 {
                set(game, "minecraft:nether_portal", [16, y, 8]);
            }
            let exit = game
                .level_portal_destination(overworld, entry, SIZE)
                .expect("the portal is read whole");
            // The entity's 0.98 box centred at x 15.5 in the 2-wide frame starting at x 15 sits
            // 0.01 right of the frame's left end: (0.01 / 1.02) of the free width, which is
            // the middle of the 2-wide exit (0.5); forward 0.5 of the exit's z of 1.
            let [x, y, z] = exit.pos;
            assert!((x - 0.5).abs() < 1e-4 && (y - 70.0).abs() < 1e-4 && (z - 1.5).abs() < 1e-4);
            assert_eq!(
                exit.rotation,
                PortalRotation::new(PortalAngle::Turn(0.0), PortalAngle::Turn(0.0))
            );
            let shape = exit.exit_portal.expect("a nether portal");
            assert_eq!(
                (shape.bottom_left, shape.width, shape.height, shape.x_axis),
                ([0, 70, 1], 2, 3, true)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn end_portals_answer_the_fixed_spawn_with_the_travel_yaw() {
        with_game(|game| {
            prepare(game);
            game.level.world_spawn = Some([4, 64, 4]);
            game.level.world_spawn_angle = [30.0, 12.0];
            load_around(game, None);
            set(game, "minecraft:stone", [4, 63, 4]);
            set(game, "minecraft:end_portal", [8, 70, 8]);
            game.enter_level(Dim::End).unwrap();
            load_around(game, None);
            set(game, "minecraft:end_portal", [0, 60, 0]);
            game.enter_level(Dim::Overworld).unwrap();
            let overworld = game.level("minecraft:overworld").unwrap();
            let end = game.level("minecraft:the_end").unwrap();
            // Into the End: the End's spawn point, facing west.
            let into = game
                .level_portal_destination(overworld, [8, 70, 8], SIZE)
                .unwrap();
            assert_eq!(into.level, end);
            assert_eq!(into.pos, [100.5, 50.0, 0.5]);
            assert_eq!(
                into.rotation,
                PortalRotation::new(PortalAngle::Set(90.0), PortalAngle::Turn(0.0))
            );
            // Out of it: the top of the world spawn's column, turned by its angle.
            let out = game
                .level_portal_destination(end, [0, 60, 0], SIZE)
                .unwrap();
            assert_eq!(out.level, overworld);
            assert_eq!(out.pos, [4.5, 64.0, 4.5]);
            assert_eq!(
                out.rotation,
                PortalRotation::new(PortalAngle::Turn(30.0), PortalAngle::Turn(12.0))
            );
        });
    }
}
