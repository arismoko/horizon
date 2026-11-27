//! Exercise the real local-player movement caller with and without a native listener.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{ClientHost, Hooks, MovementInputUpdateEvent};
use sunrise_gameplay_common::game::remote::{Keys, Vec3};
use sunrise_sim::motion::PlayerMotion;
use winit::keyboard::KeyCode;

pub(crate) struct Native;
#[mod_api::__subscriptions]
impl Hooks for Native {
    fn movement_input_update(&self, _: &dyn ClientHost, event: &mut MovementInputUpdateEvent<'_>) {
        let input = &mut *event.input;
        let keys = &mut input.key_presses;
        if keys.right {
            keys.forward = true;
            keys.jump = false;
        } else if keys.left {
            input.move_vector = [0.25, -0.5];
        } else if keys.backward {
            keys.forward = true;
            keys.left = true;
            keys.right = true;
            keys.sprint = true;
            input.move_vector = [0.5, 0.25];
        } else if keys.jump {
            input.move_vector = [f32::MAX, 0.25];
            keys.sneak = true;
        } else if !keys.forward {
            keys.forward = true;
            keys.sprint = true;
            input.move_vector = [0.0, 1.0];
        }
    }
}

pub(crate) fn reset_player(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, food: i32) {
    let mut player = PlayerMotion::new(game.motion, Vec3::new(8.0, 64.0, 8.0)).unwrap();
    player.client_side = true;
    player.health.food.food = food;
    player.ground.on_ground = true;
    game.player = Some(player);
    game.clear_device_keys();
    game.using = None;
    let remote = game.client.remote.as_mut().unwrap();
    remote.health = (20.0, food, 5.0);
    remote.was_sprinting = false;
    remote.last_input = 0;
}

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::movement_input_tests::connect;

pub(crate) fn tick(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    if let Some(input) = game.tick_local_player(Vec::new(), 7, None).unwrap() {
        game.remote_send_state(input);
    }
}

#[test]
#[ignore = "daily"]
fn movement_input_passive_listener_preserves_sprint() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    struct Observe(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn movement_input_update(
            &self,
            _: &dyn ClientHost,
            event: &mut MovementInputUpdateEvent<'_>,
        ) {
            if event.input.key_presses.forward && !event.input.key_presses.sprint {
                self.0.store(true, Ordering::Relaxed);
            }
        }
    }
    let saw_released_sprint = Arc::new(AtomicBool::new(false));
    let mods = vec![mod_api::ModEntry::new(
        "observer",
        Box::new(Observe(saw_released_sprint.clone())),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            game.insert_device_key(KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting);
            game.remove_device_key(&KeyCode::ControlLeft);
            tick(game);
            assert!(saw_released_sprint.load(Ordering::Relaxed));
            assert!(game.player.as_ref().unwrap().sprinting, "vanilla latch");
            assert_eq!(game.client.remote.as_ref().unwrap().last_input, 1);
            assert!(game.client.remote.as_ref().unwrap().was_sprinting);

            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            game.clear_device_keys();
            tick(game);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(
                game.player.as_ref().unwrap().sprinting,
                "vanilla double tap"
            );
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_sends_auto_jump_with_the_edited_keys() {
    let mods = vec![mod_api::ModEntry::new("movement", Box::new(Native))];
    mod_platform::testing::with_mods(mods.into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            // Right selects the listener's edits: add forward and clear jump.
            game.insert_device_key(KeyCode::KeyD);
            game.insert_device_key(KeyCode::Space);
            game.player.as_mut().unwrap().auto_jump_time = 1;
            tick(game);
            let player = game.player.as_ref().unwrap();
            assert!(player.was_auto_jump);
            assert_eq!(player.move_vector, [-1.0, 0.0]);
            assert_eq!(game.client.remote.as_ref().unwrap().last_input, 1 | 8 | 16);
            tick(game);
            assert!(!game.player.as_ref().unwrap().was_auto_jump);
            assert_eq!(game.client.remote.as_ref().unwrap().last_input, 1 | 8);
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_without_listeners_keeps_vanilla_latch_double_tap_and_controller() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            let keys = game.update_movement_input(Keys {
                forward: true,
                analog: Some([0.2, 0.8]),
                ..Keys::default()
            });
            assert_eq!(keys.direction(), [0.2, 0.8]);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            game.insert_device_key(KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting);
            game.remove_device_key(&KeyCode::ControlLeft);
            tick(game);
            assert!(game.player.as_ref().unwrap().sprinting, "vanilla latch");
            game.clear_device_keys();
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            reset_player(game, 20);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(!game.player.as_ref().unwrap().sprinting);
            game.clear_device_keys();
            tick(game);
            game.insert_device_key(KeyCode::KeyW);
            tick(game);
            assert!(
                game.player.as_ref().unwrap().sprinting,
                "vanilla double tap"
            );
            server.shutdown();
        });
    });
}

#[test]
#[ignore = "daily"]
fn movement_input_disabled_listener_keeps_vanilla_keys() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("movement", Box::new(Native))].into(),
        || {
            assert!(mod_api::__private::set_mod_enabled("movement", false));
            with_client_game(|game| {
                let keys = game.update_movement_input(Keys {
                    forward: true,
                    analog: Some([0.2, 0.8]),
                    ..Keys::default()
                });
                assert_eq!(keys.direction(), [0.2, 0.8]);
                assert!(!keys.sprint);
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint);
                assert!(mod_api::__private::set_mod_enabled("movement", true));
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 1.0]);
                assert!(keys.forward && keys.sprint);
                assert!(mod_api::__private::set_mod_enabled("movement", false));
                let keys = game.update_movement_input(Keys::default());
                assert_eq!(keys.direction(), [0.0, 0.0]);
                assert!(!keys.forward && !keys.sprint);
            });
        },
    );
}

#[path = "local_rotation_tests.rs"]
pub(crate) mod local_rotation_tests;

/// LocalPlayer.isShiftKeyDown is the input's sneak while the player rides too, so the
/// sneak held to get off already counts on the first tick out of the vehicle.
#[test]
#[ignore = "daily"]
fn the_sneak_key_counts_while_riding() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let server = connect(game);
            reset_player(game, 20);
            let boat = sunrise_gameplay_common::game::boats_common::Boat::new(
                5,
                "oak".into(),
                Vec3::new(8.0, 64.0, 8.0),
                0.0,
            );
            game.boats.push(boat);
            game.riding = Some(5);
            game.client.remote.as_mut().unwrap().first_passenger = false;
            game.insert_device_key(KeyCode::ShiftLeft);
            tick(game);
            assert_eq!(game.riding, Some(5));
            assert!(game.player.as_ref().unwrap().shift_key_down);
            server.shutdown();
        });
    });
}
