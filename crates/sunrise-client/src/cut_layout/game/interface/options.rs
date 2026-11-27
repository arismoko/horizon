#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
mod resize_state_tests {
    use sunrise_client_gameplay::game::ResizeState;
    use winit::dpi::PhysicalSize;

    fn state(width: u32, unicode: bool) -> ResizeState {
        ResizeState {
            size: PhysicalSize::new(width, 720),
            gui_size: sunrise_client::window::GuiSize::new([width, 720], 3, unicode).size,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn viewport_resize_notifications_follow_each_player() {
        // Winit permits only one event loop per process, even after it is dropped.
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::options::resize_state_tests::viewport_resize_notifications_follow_each_player",
            "SUNRISE_RESIZE_STATE_CHILD",
            &["window"],
        ) {
            return;
        }
        use mod_api::{ClientHost, Hooks, ModEntry, WindowResizeEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{Arc, Mutex};
        struct Listener(Arc<Mutex<Vec<(u32, u32, u32, u32)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn window_resize(&self, _: &mut dyn ClientHost, event: &mut WindowResizeEvent) {
                self.0.lock().unwrap().push((
                    event.width,
                    event.height,
                    event.gui_width,
                    event.gui_height,
                ));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut builder = winit::event_loop::EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        let event_loop = builder.build().unwrap();
        #[allow(deprecated)]
        let window = Arc::new(
            event_loop
                .create_window(
                    winit::window::Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(PhysicalSize::new(1920, 1080)),
                )
                .unwrap(),
        );
        assert_eq!(window.inner_size(), PhysicalSize::new(1920, 1080));
        let mods = vec![ModEntry::new("resize", Box::new(Listener(events.clone())))].into();
        with_mods(mods, || {
            for (scale, expected) in [(0, (320, 360)), (2, (480, 540))] {
                super::super::native_entity_refs::tests::with_client_game(|game| {
                    game.options.offscreen = true;
                    game.client.window.0 = Some(window.clone());
                    let renderer = game.new_renderer(window.clone()).unwrap();
                    game.test_set_renderer(renderer);
                    game.client.renderer.0.as_mut().unwrap().resize(960, 1080);
                    game.client.settings.gui_scale = scale;
                    game.client.settings.force_unicode = false;
                    game.notify_window_resize();
                    game.notify_window_resize();
                    assert_eq!(
                        events.lock().unwrap().pop(),
                        Some((1920, 1080, expected.0, expected.1))
                    );
                    assert!(events.lock().unwrap().is_empty());
                    game.client.renderer.0.as_mut().unwrap().resize(1920, 1080);
                    game.notify_window_resize();
                    let gui = sunrise_client::window::GuiSize::new([1920, 1080], scale, false).size;
                    assert_eq!(
                        events.lock().unwrap().pop(),
                        Some((1920, 1080, gui[0], gui[1]))
                    );
                    game.notify_window_resize();
                    assert!(events.lock().unwrap().is_empty());
                });
            }
        });
    }

    /// `ClientOptionsClientHost::window_size` (`Window.getWidth`, `getHeight`): none without a
    /// window, then the size `window_resize` reports, before and after the window is resized.
    #[test]
    #[ignore = "daily"]
    fn window_size_reads_the_size_window_resize_reports() {
        // Winit permits only one event loop per process, even after it is dropped.
        if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::client_side::options::resize_state_tests::window_size_reads_the_size_window_resize_reports",
            "SUNRISE_WINDOW_SIZE_CHILD",
            &["window"],
        ) {
            return;
        }
        use mod_api::{ClientHost, Hooks, ModEntry, WindowResizeEvent};
        use mod_platform::testing::with_mods;
        use std::sync::{Arc, Mutex};
        use winit::platform::pump_events::EventLoopExtPumpEvents;
        struct Listener(Arc<Mutex<Vec<[u32; 2]>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn window_resize(&self, _: &mut dyn ClientHost, event: &mut WindowResizeEvent) {
                self.0.lock().unwrap().push([event.width, event.height]);
            }
        }
        /// Creates the hidden window when the loop resumes; the resize request is the
        /// test's own, so its events need no handling.
        #[derive(Default)]
        struct Hidden(Option<Arc<winit::window::Window>>);
        impl winit::application::ApplicationHandler for Hidden {
            fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
                if self.0.is_none() {
                    let attributes = winit::window::Window::default_attributes()
                        .with_visible(false)
                        .with_inner_size(PhysicalSize::new(1920, 1080));
                    self.0 = Some(Arc::new(event_loop.create_window(attributes).unwrap()));
                }
            }
            fn window_event(
                &mut self,
                _: &winit::event_loop::ActiveEventLoop,
                _: winit::window::WindowId,
                _: winit::event::WindowEvent,
            ) {
            }
        }
        let reported = Arc::new(Mutex::new(Vec::new()));
        let mut builder = winit::event_loop::EventLoop::builder();
        #[cfg(target_os = "linux")]
        {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11().with_any_thread(true);
        }
        // Libtest runs the test on a thread of its own.
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        let mut event_loop = builder.build().unwrap();
        let mut app = Hidden::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while app.0.is_none() && std::time::Instant::now() < deadline {
            event_loop.pump_app_events(Some(std::time::Duration::from_millis(10)), &mut app);
        }
        let window = app
            .0
            .clone()
            .expect("the event loop resumed and made the window");
        let mods = vec![ModEntry::new("size", Box::new(Listener(reported.clone())))].into();
        with_mods(mods, || {
            super::super::native_entity_refs::tests::with_client_game(|game| {
                game.options.offscreen = true;
                assert_eq!((&*game as &dyn ClientHost).window_size(), None);
                game.client.window.0 = Some(window.clone());
                assert_eq!(
                    (&*game as &dyn ClientHost).window_size(),
                    Some([1920, 1080])
                );
                game.notify_window_resize();
                assert_eq!(reported.lock().unwrap().pop(), Some([1920, 1080]));

                let _ = window.request_inner_size(PhysicalSize::new(1280, 720));
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while window.inner_size() != PhysicalSize::new(1280, 720)
                    && std::time::Instant::now() < deadline
                {
                    event_loop
                        .pump_app_events(Some(std::time::Duration::from_millis(10)), &mut app);
                }
                assert_eq!((&*game as &dyn ClientHost).window_size(), Some([1280, 720]));
                game.notify_window_resize();
                assert_eq!(reported.lock().unwrap().pop(), Some([1280, 720]));
                assert!(reported.lock().unwrap().is_empty());
            });
        });
    }

    /// One resize event that also flips fullscreen is observed twice (by the fullscreen
    /// observer and by the resize handler) but reported once.
    #[test]
    fn overlapping_observers_report_a_change_once() {
        let mut last = Some(state(640, false));
        let now = state(800, true);
        assert!(now.differs_from(&mut last));
        assert!(!now.differs_from(&mut last));
    }

    /// A settings-only change at an unchanged physical size still reports.
    #[test]
    fn a_settings_only_change_reports() {
        let mut last = Some(state(960, false));
        assert!(!state(960, false).differs_from(&mut last));
        assert!(state(960, true).differs_from(&mut last));
    }
    #[test]
    fn settings_that_leave_dimensions_unchanged_do_not_report() {
        let size = PhysicalSize::new(1280, 960);
        let before = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 2, false).size,
        };
        let mut last = Some(before);
        let unicode = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 2, true).size,
        };
        assert!(!unicode.differs_from(&mut last));
        let automatic = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 0, false).size,
        };
        assert!(automatic.differs_from(&mut last));
        let larger_limit = ResizeState {
            size,
            gui_size: sunrise_client::window::GuiSize::new([1280, 960], 100, false).size,
        };
        assert!(!larger_limit.differs_from(&mut last));
    }
}
