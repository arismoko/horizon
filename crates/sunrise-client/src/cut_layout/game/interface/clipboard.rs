#![cfg(test)]
#[cfg(test)]
mod tests {
    use mod_api::{
        ClientHost, ClientTickPreEvent, Flow, Hooks, ModEntry, MouseButtonPreEvent,
        RenderGuiPreEvent,
    };
    use mod_platform::testing::with_mods;
    use std::sync::{Arc, Mutex};
    use sunrise_client_gameplay::game::ClientGame;
    use sunrise_gameplay_common::game::execution_role::ExecutionRole;

    /// What each hook read from the clipboard, and whether its copy was taken.
    #[derive(Clone, Default)]
    struct Reads(Arc<Mutex<Vec<(&'static str, Option<String>, bool)>>>);
    impl Reads {
        fn read(&self, at: &'static str, host: &dyn ClientHost) {
            let text = host.clipboard_text();
            let copied = host.set_clipboard_text(at);
            self.0.lock().unwrap().push((at, text, copied));
        }
    }
    struct Mod(Reads);
    /// A step of an event loop that is never asked to stop.
    struct NoEventLoop;
    impl sunrise_client_gameplay::game::event_loop::EventLoopCtl for NoEventLoop {
        fn exit(&self) {}
        fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
    }

    fn with_server_role_clipboard(role: ExecutionRole, test: impl FnOnce(&mut ClientGame<'_>)) {
        // Only ClientGame implements ClientHost, so exercise the clipboard's role guard there.
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            let previous = std::mem::replace(&mut game.options.execution_role, role);
            test(game);
            game.options.execution_role = previous;
        });
    }

    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
            self.0.read("tick", host);
        }
        fn render_gui_pre(&self, host: &dyn ClientHost, _: &mut RenderGuiPreEvent) -> Flow {
            self.0.read("frame", host);
            Flow::Continue
        }
        fn mouse_button_pre(&self, host: &mut dyn ClientHost, _: &mut MouseButtonPreEvent) -> Flow {
            self.0.read("click", host);
            // The click stops here: the game has no world to click in.
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_mod_reaches_the_clipboard_from_any_client_callback() {
        let reads = Reads::default();
        let mods: mod_api::Mods = vec![ModEntry::new("paste", Box::new(Mod(reads.clone())))].into();
        with_mods(mods, || {
            sunrise_client_gameplay::game::native_entity_refs::tests::with_client_game(|game| {
                // Headless has no window, but a client role is what `native_window` asks.
                game.options.headless = true;
                game.client.clipboard.0.set_text("pasted");

                game.native_client_tick_pre();
                game.native_render_gui_pre(&mut false);
                game.handle_window_event(
                    &NoEventLoop,
                    winit::window::WindowId::dummy(),
                    winit::event::WindowEvent::MouseInput {
                        device_id: winit::event::DeviceId::dummy(),
                        state: winit::event::ElementState::Pressed,
                        button: winit::event::MouseButton::Left,
                    },
                    sunrise_client_gameplay::input_trace::InputOrigin::User,
                );
                let got = reads.0.lock().unwrap().clone();
                assert_eq!(
                    got,
                    [
                        ("tick", Some("pasted".to_owned()), true),
                        ("frame", Some("tick".to_owned()), true),
                        ("click", Some("frame".to_owned()), true),
                    ]
                );
                assert_eq!(game.client.clipboard.0.text(), "click");

                // The Create-shaped case: a payload handler copies reply text, and a read
                // after it returns that text.
                let host: &mut dyn ClientHost = game;
                assert!(host.set_clipboard_text("reply text"));
                assert_eq!(host.clipboard_text().as_deref(), Some("reply text"));

                // Empty text is ignored.
                assert!(host.set_clipboard_text(""));
                assert_eq!(host.clipboard_text().as_deref(), Some("reply text"));
            });
            for role in [
                ExecutionRole::IntegratedServer,
                ExecutionRole::DedicatedServer,
            ] {
                with_server_role_clipboard(role, |game| {
                    game.client.clipboard.0.set_text("kept");
                    let host: &dyn ClientHost = &*game;
                    assert_eq!(host.clipboard_text(), None);
                    assert!(!host.set_clipboard_text("lost"));
                    assert_eq!(game.client.clipboard.0.text(), "kept");
                });
            }
        });
    }
}
