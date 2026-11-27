#![cfg(feature = "client")]
use super::*;
use crate::game::remote::interaction_event_tests::predicted_use_tests::*;

#[cfg(feature = "client")]
pub(crate) fn matrix(mods: mod_api::Mods) {
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_interaction_client_game_at(root.prepared().to_owned(), |game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            let air = game.res.facts().air;
            // Literal packet [UseItemOn, UseItem, Interact], placement and swing outcomes.
            for path in 1..=4 {
                for (answer, consumes, success_swing) in [
                    (1, true, 1),
                    (2, true, 0),
                    (3, true, 0),
                    (4, false, 0),
                    (5, false, 0),
                    (6, false, 0),
                ] {
                    for server_only in [false, true] {
                        game.set_local_mode(sunrise_command::args::GameMode::Survival);
                        game.common.inventory[game.common.selected] =
                            Some(ItemStack::new("minecraft:paper", 1));
                        let target_state = if path >= 3 { air } else { stone };
                        game.world.set(0, 64, 0, target_state).unwrap();
                        game.offhand = Some(ItemStack::new("minecraft:torch", 2));
                        game.world.set(0, 65, 0, air).unwrap();
                        game.set_test_forced_hit((path == 1).then(hit));
                        game.side_release_tick_targets();
                        game.mobs.clear();
                        game.clear_device_keys();
                        game.yaw = 0.0;
                        game.pitch = 0.0;
                        let player_z = if path >= 3 { 0.5 } else { 3.5 };
                        game.player = Some(
                            PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, player_z)).unwrap(),
                        );
                        if path >= 3 {
                            game.spawn_entity(42, "minecraft:zombie", [0.5, 64.0, 2.5], [0.0; 3]);
                        }
                        game.refresh_client_pick(1.0);
                        game.swing = Default::default();
                        fixture::begin(path, answer, server_only);
                        let swings = game.swings_started;
                        sunrise_client::interaction_input::use_on_target(
                            &mut sunrise_client_gameplay::game::client_side::remote_client::RemoteUseWorld(
                                game,
                                mod_api::InteractionAttempt::Press,
                            ),
                            path >= 3,
                        );
                        let stops = !server_only && (consumes || (path == 1 && answer == 4));
                        let packets = match (path, stops) {
                            (1, true) => [1, 0, 0],
                            (1, false) => [2, 1, 0],
                            (2, true) => [0, 1, 0],
                            (2, false) => [0, 2, 0],
                            (_, true) => [0, 0, 1],
                            (_, false) => [0, 2, 2],
                        };
                        assert_eq!(
                            fixture::finish(),
                            packets,
                            "path={path} answer={answer} server_only={server_only}"
                        );
                        let placed = path == 1 && !stops;
                        assert_eq!(game.world.block(0, 65, 0) != air, placed);
                        if placed {
                            assert_eq!(&*game.block_name([0, 65, 0]), "minecraft:torch");
                        }
                        assert_eq!(
                            game.offhand.as_ref().unwrap().count,
                            if placed { 1 } else { 2 }
                        );
                        assert_eq!(
                            game.common.inventory[game.common.selected]
                                .as_ref()
                                .unwrap()
                                .count,
                            1
                        );
                        assert_eq!(game.world.block(0, 64, 0), target_state);
                        assert!(game.mobs.iter().all(|mob| !mob.removed && mob.alive()));
                        assert_eq!(
                            game.swings_started - swings,
                            if server_only {
                                u32::from(placed)
                            } else if consumes {
                                success_swing
                            } else {
                                u32::from(placed)
                            }
                        );
                    }
                }
            }
        });
    });
}

use sunrise_gameplay_common::game::side_ops::player::PlayerSideGameExt as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
