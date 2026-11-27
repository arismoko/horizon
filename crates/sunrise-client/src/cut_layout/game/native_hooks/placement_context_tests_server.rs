use crate::game::native_hooks::placement_context_tests::Notice;
use crate::game::native_hooks::placement_context_tests::PlacementSubstitution;
#[cfg(feature = "client")]
use crate::game::native_hooks::placement_context_tests::SourceSpy;
use crate::game::native_hooks::placement_context_tests::VetoProbe;
use crate::game::native_hooks::placement_context_tests::VetoTrace;
#[cfg(feature = "client")]
use mod_api::PlacementSource;
use std::sync::{Arc, Mutex};
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use sunrise_items::items::ItemStack;
use sunrise_sim::motion::PlayerMotion;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};
use {sunrise_gameplay_common::game::GuestUse, sunrise_gameplay_common::game::Hit};

#[test]
#[ignore = "daily"]
fn player_and_dispenser_apply_only_the_context_seen_by_the_veto() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_hooks_server::placement_context_tests::player_and_dispenser_apply_only_the_context_seen_by_the_veto",
        || {
            use sunrise_gameplay_common::game::api_values::TryIntoApi;

            let trace = Arc::new(Mutex::new(VetoTrace::default()));
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "placement_veto",
                Box::new(VetoProbe(trace.clone())),
            )]
            .into();
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                        .unwrap();
                    let registry = game.pack.assets.registry;
                    let state = |name| registry.default_state(name).unwrap();
                    let stone = state("minecraft:stone");
                    let glass = state("minecraft:glass");
                    let air = state("minecraft:air");
                    let obsidian = state("minecraft:obsidian");
                    for x in [4, 10] {
                        for z in [2, 4, 7, 9] {
                            game.world.set(x, 63, z, stone).unwrap();
                        }
                        for z in [4, 9] {
                            game.world.set(x, 65, z, glass).unwrap();
                        }
                    }
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
                    bind_local_player(game, 20, 123);
                    game.server.lan_acting_name = Some("Placer".into());
                    game.server.lan_acting_permission = Some(0);
                    game.set_mode_id(0);

                    for (occupy_target, z) in [(false, 2), (true, 7)] {
                        trace.lock().unwrap().occupy_target = occupy_target;
                        game.world.set(4, 64, z, stone).unwrap();
                        *game.held_mut() = Some(ItemStack::new("minecraft:stone", 2));
                        game.server.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
                        let hit = Hit {
                            pos: [4, 63, z],
                            face: [0, 1, 0],
                            point: [4.5, 64.0, f64::from(z) + 0.5],
                            inside: false,
                        };
                        // Mark the swapped-in body as a server guest's active turn for the click.
                        game.server.lan_acting = Some(1);
                        assert!(game.use_hand(Some(&hit)).unwrap());
                        game.server.lan_acting = None;
                        let left = game
                            .dispense_item(
                                (9, 64, z),
                                Direction::East,
                                ItemStack::new("minecraft:shulker_box", 2),
                            )
                            .unwrap()
                            .continue_value()
                            .unwrap()
                            .unwrap();
                        let count = if occupy_target { 2 } else { 1 };
                        assert_eq!(game.held().unwrap().count, count);
                        assert_eq!(left.count, count);
                        assert_eq!(
                            game.world.block(4, 64, z + 2),
                            if occupy_target { obsidian } else { stone }
                        );
                        let dispenser_state = game.world.block(10, 64, z + 2);
                        if occupy_target {
                            assert_eq!(dispenser_state, obsidian);
                        } else {
                            assert_eq!(
                                registry.state(dispenser_state).unwrap().name.as_str(),
                                "minecraft:shulker_box"
                            );
                        }
                        assert_eq!(
                            game.world.block(4, 64, z),
                            stone,
                            "occupied original destination"
                        );
                        assert_eq!(game.world.block(10, 64, z), air, "dispenser's front cell");
                        for x in [4, 10] {
                            assert_eq!(game.world.block(x, 64, z + 3), air, "second candidate");
                        }
                    }
                    game.world.set(10, 64, 12, stone).unwrap();
                    let left = game
                        .dispense_item(
                            (9, 64, 12),
                            Direction::East,
                            ItemStack::new("minecraft:shulker_box", 2),
                        )
                        .unwrap()
                        .continue_value()
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        left.count, 2,
                        "a blocked dispenser still fails before the hook"
                    );
                    let trace = trace.lock().unwrap();
                    let targets = [
                        BlockPos::new(4, 64, 4),
                        BlockPos::new(10, 64, 4),
                        BlockPos::new(4, 64, 9),
                        BlockPos::new(10, 64, 9),
                    ];
                    let against = glass.try_into_api().unwrap();
                    assert_eq!(trace.decisions, targets);
                    assert_eq!(trace.vetoes, targets.map(|pos| (pos, against)));
                    assert_eq!(
                        trace.notices,
                        [
                            Notice {
                                pos: [4, 64, 4],
                                face: mod_api::BlockFace::Down,
                                click: [4.125, 64.75, 4.875]
                            },
                            Notice {
                                pos: [10, 64, 4],
                                face: mod_api::BlockFace::Down,
                                click: [10.125, 64.75, 4.875]
                            },
                        ]
                    );
                });
            });
        },
    );
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn state_for_placement_distinguishes_player_and_dispenser_sources() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::server_side::native_hooks_server::placement_context_tests::state_for_placement_distinguishes_player_and_dispenser_sources",
        || {
            let spy = SourceSpy::default();
            let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
                "placement_source",
                Box::new(spy.clone()),
            )]
            .into();
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                with_game(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                        .unwrap();
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    game.world.set(4, 63, 4, stone).unwrap();
                    game.world.set(8, 63, 5, stone).unwrap();
                    game.player =
                        Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
                    bind_local_player(game, 20, 123);
                    game.server.lan_acting_name = Some("Placer".into());
                    game.server.lan_acting_permission = Some(0);
                    game.set_mode_id(0);

                    // Server player placement looking south (yaw 0, pitch 15) must report `Player`,
                    // distinct from a dispenser facing south.
                    game.yaw = 0.0;
                    game.pitch = 15.0;
                    *game.held_mut() = Some(ItemStack::new("minecraft:stone", 2));
                    game.server.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
                    let hit = Hit {
                        pos: [4, 63, 4],
                        face: [0, 1, 0],
                        point: [4.5, 64.0, 4.5],
                        inside: false,
                    };
                    game.server.lan_acting = Some(1);
                    assert!(game.use_hand(Some(&hit)).unwrap());
                    game.server.lan_acting = None;

                    // Dispenser shulker-box placement facing south and up must report `Directional`.
                    let left = game
                        .dispense_item(
                            (8, 64, 4),
                            Direction::South,
                            ItemStack::new("minecraft:shulker_box", 2),
                        )
                        .unwrap()
                        .continue_value()
                        .unwrap()
                        .unwrap();
                    assert_eq!(left.count, 1);
                    let left = game
                        .dispense_item(
                            (10, 64, 4),
                            Direction::Up,
                            ItemStack::new("minecraft:shulker_box", 2),
                        )
                        .unwrap()
                        .continue_value()
                        .unwrap()
                        .unwrap();
                    assert_eq!(left.count, 1);
                });

                // Client placement prediction also reports `Player`.
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(pipe, "SourceClient").unwrap();
                    let remote = game.client.remote.as_mut().unwrap();
                    remote.logged_in = true;
                    remote.entity_id = 7;
                    remote.profile = 42;
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(6, 63, 4, stone).unwrap();
                    game.yaw = 90.0;
                    game.pitch = -20.0;
                    let client_hit = Hit {
                        pos: [6, 63, 4],
                        face: [0, 1, 0],
                        point: [6.5, 64.0, 4.5],
                        inside: false,
                    };
                    assert!(
                        game.remote_can_place(&client_hit, &ItemStack::new("minecraft:stone", 1))
                            .is_some()
                    );
                });

                let seen = spy.0.lock().unwrap();
                assert_eq!(
                    seen.as_slice(),
                    &[
                        (
                            true,
                            BlockPos::new(4, 64, 4),
                            PlacementSource::Player {
                                yaw: 0.0,
                                pitch: 15.0,
                                sneaking: false,
                            },
                        ),
                        (
                            true,
                            BlockPos::new(8, 64, 5),
                            PlacementSource::Directional {
                                direction: Direction::South,
                            },
                        ),
                        (
                            true,
                            BlockPos::new(10, 65, 4),
                            PlacementSource::Directional {
                                direction: Direction::Up,
                            },
                        ),
                        (
                            false,
                            BlockPos::new(6, 64, 4),
                            PlacementSource::Player {
                                yaw: 90.0,
                                pitch: -20.0,
                                sneaking: false,
                            },
                        ),
                    ]
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn substituted_placement_uses_the_held_item_preclear_and_selected_placed_by() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "placement_substitution",
        Box::new(PlacementSubstitution),
    )]
    .into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.player =
                Some(PlayerMotion::new(game.motion, Vec3::new(12.5, 64.0, 12.5)).unwrap());
            bind_local_player(game, 20, 123);
            game.server.lan_acting_name = Some("Placer".into());
            game.server.lan_acting_permission = Some(0);
            game.set_mode_id(0);

            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let roof = registry.default_state("minecraft:obsidian").unwrap();
            let door = registry.default_state("minecraft:oak_door").unwrap();
            let mut upper = registry.state(door).unwrap().clone();
            upper.properties.insert("half".into(), "upper".into());
            let upper = registry.id(&upper).unwrap();
            for (item, z, remaining, primary, above) in [
                ("minecraft:furnace", 5, 1, door, upper),
                ("minecraft:oak_door", 7, 1, stone, air),
                ("minecraft:oak_door", 9, 2, air, roof),
            ] {
                game.world.set(4, 63, z, stone).unwrap();
                game.world.set(4, 65, z, roof).unwrap();
                let hit = Hit {
                    pos: [4, 63, z],
                    face: [0, 1, 0],
                    point: [4.5, 64.0, f64::from(z) + 0.5],
                    inside: false,
                };
                *game.held_mut() = Some(ItemStack::new(item, 2));
                game.server.guest_use = Some(GuestUse::on_block(mod_api::Hand::Main));
                assert!(game.use_hand(Some(&hit)).unwrap());
                assert_eq!(game.held().unwrap().count, remaining);
                assert_eq!(game.world.block(4, 64, z), primary);
                assert_eq!(game.world.block(4, 65, z), above);
            }
        });
    });
}
