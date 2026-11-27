use std::sync::Arc;
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::EventLoopCtl;
use sunrise_gameplay_common::game::Screen;
use winit::event::ElementState;
use winit::event::MouseButton;
use winit::event::WindowEvent;

#[path = "incoming_tests/pet_order_client.rs"]
pub(crate) mod pet_order_tests;

#[test]
#[ignore = "daily"]
fn client_modifier_snapshot_tracks_focus_loss() {
    use mod_api::ClientHost;
    use winit::{event_loop::ControlFlow, keyboard::ModifiersState};
    struct TestLoop;
    impl EventLoopCtl for TestLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: ControlFlow) {}
    }
    super::native_entity_refs::tests::with_client_game(|game| {
        game.client.modifiers = ModifiersState::SHIFT | ModifiersState::ALT;
        let snapshot = (game as &dyn ClientHost).modifiers();
        assert!(snapshot.shift());
        assert!(snapshot.alt());
        assert!(!snapshot.control());
        assert!(!snapshot.super_key());
        game.handle_window_event_inner(&TestLoop, WindowEvent::Focused(false), None, None);
        assert_eq!((game as &dyn ClientHost).modifiers().bits, 0);
        assert_eq!(snapshot.bits, 5);
    });
}

pub(crate) fn exercise_blank_inventory_presses() {
    use mod_api::{ClientHost, Hooks, ModEntry, ScreenMouseButtonPressedPostEvent};
    use std::sync::Mutex;
    use winit::{event::DeviceId, event_loop::ControlFlow};

    struct TestLoop;
    impl EventLoopCtl for TestLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: ControlFlow) {}
    }
    type Post = (&'static str, bool, bool);
    struct Observer(Arc<Mutex<Vec<Post>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn screen_mouse_button_pressed_post(
            &self,
            _: &dyn ClientHost,
            event: &mut ScreenMouseButtonPressedPostEvent<'_>,
        ) {
            self.0.lock().unwrap().push((
                event.screen.id,
                event.was_click_handled,
                event.double_click,
            ));
        }
    }

    let posts = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "inventory_clicks",
            Box::new(Observer(posts.clone())),
        )]
        .into(),
        || {
            super::native_entity_refs::tests::with_client_game(|game| {
                let _window =
                    sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene(
                        game,
                    );
                let mut millis = 0;
                for (screen, id) in [
                    (Screen::Inventory, "inventory"),
                    (Screen::Creative, "creative"),
                ] {
                    game.set_screen(screen);
                    let renderer = game.client.renderer.0.as_ref().unwrap();
                    let batch = HudBatch::new(
                        [renderer.config.width, renderer.config.height],
                        game.client.settings.gui_scale_inputs(),
                    );
                    let (x, y) = game.inventory_origin(&batch);
                    game.client.cursor = (x + 3.0, y + 3.0);
                    assert_eq!(game.inventory_target(), (None, false));
                    for button in [
                        MouseButton::Left,
                        MouseButton::Right,
                        MouseButton::Middle,
                        MouseButton::Other(300),
                    ] {
                        for state in [ElementState::Pressed, ElementState::Released].repeat(2) {
                            let timestamp = (state == ElementState::Pressed).then(|| {
                                millis += 100;
                                millis
                            });
                            game.handle_window_event_inner(
                                &TestLoop,
                                WindowEvent::MouseInput {
                                    device_id: DeviceId::dummy(),
                                    state,
                                    button,
                                },
                                None,
                                timestamp,
                            );
                        }
                        assert_eq!(
                            std::mem::take(&mut *posts.lock().unwrap()),
                            [(id, true, false), (id, true, true)],
                            "{id} with {button:?}",
                        );
                    }
                }
            });
        },
    );
}
