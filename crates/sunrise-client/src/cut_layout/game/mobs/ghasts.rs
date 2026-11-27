#![cfg(test)]
#[cfg(test)]
pub(crate) fn with_headless_game(run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    use {
        crate::game::load_resources, crate::settings::Settings,
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
    };

    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
    let save = tempfile::tempdir().unwrap();
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.game_dir = Some(save.path().into());
    options.execution_role = ExecutionRole::IntegratedServer;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    run(&mut game);
}

#[cfg(test)]
pub(crate) mod hatch_tests {
    use super::with_headless_game;
    use sunrise_core::{nbt::Tag, position::BlockPos};
    use sunrise_gameplay_common::game::LevelEvent;

    #[test]
    #[ignore = "daily"]
    fn ghastling_and_tadpole_hatches_match_vanilla_persistence() {
        with_headless_game(|game| {
            for (block, kind) in [
                ("minecraft:dried_ghast", "spawn_ghastling"),
                ("minecraft:frogspawn", "hatch"),
            ] {
                game.common.level_events.push(LevelEvent::BlockEvent {
                    pos: BlockPos::new(4, 64, 4),
                    state: game
                        .common
                        .pack
                        .assets
                        .registry
                        .default_state(block)
                        .unwrap(),
                    kind,
                });
            }
            game.server_process_level_events().unwrap();

            let (ghastling, tadpoles) = game.mobs.split_first().unwrap();
            assert_eq!(ghastling.kind.id, "minecraft:happy_ghast");
            assert!(ghastling.is_baby());
            assert!(!ghastling.persistent);
            assert!((2..=5).contains(&tadpoles.len()));
            for tadpole in tadpoles {
                assert_eq!(tadpole.kind.id, "minecraft:tadpole");
                assert!(tadpole.persistent);
            }
            let (_, saved) = game.save_mob(0, &[(0, 0)].into()).unwrap();
            assert_eq!(saved.get("Age"), Some(&Tag::Int(-24000)));
            assert_eq!(saved.get("PersistenceRequired"), Some(&Tag::Byte(0)));
        });
    }
}

#[cfg(test)]
pub(crate) mod item_collision_tests {
    use super::with_headless_game;
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_level::chunk::Chunk;

    /// An item dropped onto a minecart falls through it to the floor, and one
    /// dropped onto a boat rests on the boat (`canBeCollidedWith`).
    #[test]
    #[ignore = "daily"]
    fn a_dropped_item_rests_on_a_boat_but_falls_through_a_minecart() {
        with_headless_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap()
                .0;
            let mut chunk = Chunk::empty(0, 0);
            for x in 0..16 {
                for z in 0..16 {
                    chunk.set(x, 199, z, stone);
                }
            }
            game.world.chunks.insert((0, 0), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.load_ticking_neighbourhood(0, 0);
            assert!(game.run_command("forceload add 0 0"));
            assert!(game.run_command("summon minecart 4.5 200 4.5"));
            assert!(game.run_command("summon oak_boat 10.5 200 10.5"));
            for (x, z) in [(4.5, 4.5), (10.5, 10.5)] {
                assert!(game.run_command(&format!(
                "summon item {x} 201.5 {z} {{Item:{{id:\"minecraft:stick\",count:1}},Motion:[0.0d,0.0d,0.0d]}}"
            )));
            }
            for _ in 0..60 {
                game.tick_world(true).unwrap();
            }
            let rest = |x: f64| {
                game.dropped
                    .iter()
                    .find(|d| (d.position.x - x).abs() < 0.5)
                    .map(|d| d.position.y)
                    .unwrap()
            };
            assert_eq!(rest(4.5), 200.0, "through the minecart to the stone");
            let boat_top = game.boats[0].bounds().max_position().y;
            assert_eq!(rest(10.5), boat_top, "on the boat");
        });
    }
}
