#![cfg(feature = "client")]
use crate::game::remote::movement_input_tests::Native;
use crate::game::remote::movement_input_tests::reset_player;
use crate::game::remote::movement_input_tests::tick;
use crate::game::remote::movement_input_tests::*;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_gameplay_common::game_side;
use sunrise_sim::motion::MotionWorld;
use sunrise_sim::sprint::Keys;
use winit::keyboard::KeyCode;

#[test]
#[ignore = "daily"]
fn native_movement_input_keys_axes_and_sprint() {
    let mods = vec![mod_api::ModEntry::new("movement", Box::new(Native))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            // Key-only edits don't replace the sampled controller vector.
            let keys = game.update_movement_input(Keys {
                right: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert!(keys.forward && keys.right);
            assert_eq!(keys.direction(), [0.2, 0.8]);

            // Edited vectors (including large ones whose squared length overflows) apply directly
            // without reverting to the sampled vector, alongside independent key edits.
            let keys = game.update_movement_input(Keys {
                jump: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [f32::MAX, 0.25]);
            assert!(keys.jump && keys.shift);

            // Consumer-visible acceptance: no physical input still moves the
            // local player; the final keys and eligible sprint reach the server.
            for food in [0, 6, 7, 20] {
                reset_player(game, food);
                let mut replayed = game.player.as_ref().unwrap().clone();
                tick(game);
                let player = game.player.as_ref().unwrap();
                assert_eq!(player.move_vector, [0.0, 1.0]);
                assert!(
                    player.position.z > 8.0,
                    "injected forward must move, food={food}"
                );
                assert_eq!(player.sprinting, food > 6, "food={food}");
                assert_eq!(game.client.remote.as_ref().unwrap().last_input, 1 | 64);
                assert_eq!(game.client.remote.as_ref().unwrap().was_sprinting, food > 6);
                // What the client sent is enough to play its tick again: the input packet's keys
                // and the sprint state, through `MotionWorld::replayed_tick` (the tick a mod's
                // `native_simulate_player_move` plays), without asking the listener again, move
                // the player as the client's own tick did.
                let position = player.position;
                let sprinting = player.sprinting;
                let sent_keys = sunrise_gameplay::game::lan_vehicles::input_keys(
                    game.client.remote.as_ref().unwrap().last_input,
                );
                let meta = sunrise_gameplay_common::game::side_ops::world::WorldSideGameExt::side_game_side_meta(&*game);
                let side = game_side!(
                    game.common,
                    meta,
                    sunrise_gameplay_common::game::side_ops::tables::SideOps::side_level_features(
                        &game.client,
                        game.common.level.enabled_features.as_arc(),
                    ),
                    sunrise_gameplay_common::game::side_ops::tables::SideOps::side_game_tag_items(
                        &game.client,
                        &game.common.res,
                    )
                );
                MotionWorld {
                    side: &side,
                    entities: &[],
                    player: &mut replayed,
                    scene: &mut game.common.world,
                    assets: &game.common.pack.assets,
                    collision_binding: game.common.collision,
                    resources: game.common.motion,
                    health: &game.common.health_resources,
                    loop_context: &game.common.loop_context,
                    damage_hooks: &game.common.damage_hooks,
                    tag_items: None,
                    block_registries: None,
                }
                .replayed_tick(sent_keys, sprinting)
                .unwrap();
                assert_eq!(replayed.move_vector, [0.0, 1.0]);
                assert_eq!(replayed.sprinting, sprinting);
                assert!((replayed.position.z - position.z).abs() < 1.0e-10);
            }

            // Both edits can stand at once, including all four direction flags being held.
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyS);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.5, 0.25]);
            assert_eq!(
                game.client.remote.as_ref().unwrap().last_input,
                1 | 2 | 4 | 8 | 64
            );
            // Vector-only edits don't affect what gets sent, even if the vector points backward.
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyA);
            tick(game);
            assert_eq!(game.player.as_ref().unwrap().move_vector, [0.25, -0.5]);
            assert_eq!(game.client.remote.as_ref().unwrap().last_input, 4);

            // A mod's sprint request still uses the actual player's vanilla
            // crouching and slow-item eligibility, not only the hunger check.
            reset_player(game, 20);
            game.player.as_mut().unwrap().shift_key_down = true;
            game.insert_device_key(KeyCode::ShiftLeft);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.inventory[0] = Some(sunrise_items::items::ItemStack::new("minecraft:apple", 1));
            assert!(game.start_using_item());
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);

            server.shutdown();
        });
    });
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
