use crate::game::native_hooks::base::screen_widgets_tests::DropCounter;
use crate::game::native_hooks::base::screen_widgets_tests::NoLoop;
use crate::game::native_hooks::base::screen_widgets_tests::move_mouse;
use crate::game::native_hooks::base::screen_widgets_tests::send_key_control;
use crate::game::native_hooks::base::screen_widgets_tests::send_mouse_button;
use crate::game::native_hooks::base::screen_widgets_tests::send_scroll;
use crate::game::native_hooks::base::screen_widgets_tests::take_log;
use mod_api::ScreenOpening;
use mod_api::client::widgets::{Button, TextField};
use mod_api::{
    ClientHost, DrawContext, Flow, Hooks, ModEntry, ScreenCharTypedPostEvent,
    ScreenCharTypedPreEvent, ScreenClosingEvent, ScreenInitPostEvent, ScreenKeyPressedPostEvent,
    ScreenKeyPressedPreEvent, ScreenKeyReleasedPostEvent, ScreenKeyReleasedPreEvent,
    ScreenMouseButtonPressedPostEvent, ScreenMouseButtonPressedPreEvent,
    ScreenMouseButtonReleasedPostEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPostEvent, ScreenMouseDraggedPreEvent, ScreenMouseScrolledPostEvent,
    ScreenMouseScrolledPreEvent, ScreenOpeningEvent, ScreenPreeditPostEvent, ScreenPreeditPreEvent,
    ScreenWidget, ScreenWidgetId, Text as ApiText,
};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::input_trace::InputOrigin;
use sunrise_gameplay_common::game::Screen;
use winit::dpi::PhysicalSize;
use winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use winit::event_loop::EventLoop;
use winit::keyboard::KeyCode;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

#[path = "screen_widgets_input_tests_client.rs"]
pub(crate) mod input_routing;

/// Attach a hidden offscreen window and renderer to `game` so screen hooks, `mod_screen_open`,
/// and real `WindowEvent` input paths execute end to end. Returns the `EventLoop` (kept alive
/// for the test's duration) and `WindowId`.
pub(crate) fn attach_offscreen_window(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> (EventLoop<()>, winit::window::WindowId) {
    let (event_loop, window) =
        sunrise_client_gameplay::game::native_entity_refs::tests::test_window([640, 480]);
    let id = window.id();
    game.options.offscreen = true;
    game.client.settings.gui_scale = 1;
    game.client.window.0 = Some(window.clone());
    {
        let renderer = game.new_renderer(window).unwrap();
        game.set_renderer(renderer);
    }
    game.client.title.0 = None;
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, sunrise_core::geometry::Vec3::ZERO)
            .unwrap(),
    );
    game.client.focused = true;
    (event_loop, id)
}

/// Button `on_press` & synchronous screen transitions (Audit [86]):
/// - Pressing a button mutates its `ButtonState` (`label`, `active`).
/// - Enter and Space activate a focused button.
/// - `host.mod_screen_open("mod_b")` inside a `mod_a` button `on_press` runs synchronously with
///   no borrow panic, switches to `mod_b`, mounts `mod_b`'s widgets, disposes the extracted
///   `mod_a` button, and never overwrites `mod_b`'s widget table.
/// - `host.close_screen()` inside a button `on_press` closes synchronously to `Playing`.
#[test]
#[ignore = "daily"]
fn button_on_press_state_mutation_keyboard_and_synchronous_transitions() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::button_on_press_state_mutation_keyboard_and_synchronous_transitions",
        "SUNRISE_WIDGETS_BUTTON_TRANSITION_CHILD",
        &["window"],
    ) {
        return;
    }

    struct ModA {
        log: Arc<Mutex<Vec<String>>>,
        mod_a_btn_drops: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for ModA {
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if host.mod_screen() != Some("mod_a") {
                return;
            }
            let log_toggle = self.log.clone();
            let toggle_id = event.widgets.add_button(Button::new(
                [20.0, 20.0, 100.0, 20.0],
                ApiText::literal("Count 0"),
                move |state, _host| {
                    let was_initial = state.label == ApiText::literal("Count 0");
                    log_toggle
                        .lock()
                        .unwrap()
                        .push(format!("toggle:{was_initial}"));
                    state.label = ApiText::literal("Count 1");
                    state.active = false;
                },
            ));
            let log_kb = self.log.clone();
            let kb_id = event.widgets.add_button(Button::new(
                [20.0, 50.0, 100.0, 20.0],
                ApiText::literal("Keyboard"),
                move |state, _host| {
                    log_kb.lock().unwrap().push("kb_pressed".into());
                    state.rect[2] = if state.label == ApiText::literal("Keyboard") {
                        f32::INFINITY
                    } else {
                        i32::MAX as f32
                    };
                    state.label = ApiText::literal("KB Done");
                },
            ));
            let log_switch = self.log.clone();
            let switch_drop = DropCounter(self.mod_a_btn_drops.clone());
            event.widgets.add_button(Button::new(
                [20.0, 80.0, 100.0, 20.0],
                ApiText::literal("To B"),
                move |_state, host| {
                    let _ = &switch_drop;
                    let opened = host.mod_screen_open("mod_b");
                    log_switch
                        .lock()
                        .unwrap()
                        .push(format!("switch_to_b:{opened}:{:?}", host.mod_screen()));
                },
            ));
            let invalid = event.widgets.add_button(Button::new(
                [f32::NAN, 20.0, 100.0, 20.0],
                ApiText::literal("Not mounted"),
                |_, _| {},
            ));
            assert!(!event.widgets.remove(invalid));
            let _ = toggle_id;
            assert!(event.widgets.set_initial_focus(kb_id));
        }
    }

    struct ModB {
        log: Arc<Mutex<Vec<String>>>,
        mod_b_btn_drops: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for ModB {
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if host.mod_screen() != Some("mod_b") {
                return;
            }
            self.log.lock().unwrap().push("init_mod_b".into());
            let log_b = self.log.clone();
            let drop_b = DropCounter(self.mod_b_btn_drops.clone());
            let close_id = event.widgets.add_button(Button::new(
                [20.0, 80.0, 100.0, 20.0],
                ApiText::literal("Close B"),
                move |_state, host| {
                    let _ = &drop_b;
                    let closed = host.close_screen();
                    log_b
                        .lock()
                        .unwrap()
                        .push(format!("close_b:{closed}:{:?}", host.mod_screen()));
                },
            ));
            assert!(event.widgets.set_initial_focus(close_id));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mod_a_btn_drops = Arc::new(AtomicUsize::new(0));
    let mod_b_btn_drops = Arc::new(AtomicUsize::new(0));
    let mods: Arc<[ModEntry]> = vec![
        ModEntry::new(
            "mod_a",
            Box::new(ModA {
                log: log.clone(),
                mod_a_btn_drops: mod_a_btn_drops.clone(),
            }),
        ),
        ModEntry::new(
            "mod_b",
            Box::new(ModB {
                log: log.clone(),
                mod_b_btn_drops: mod_b_btn_drops.clone(),
            }),
        ),
    ]
    .into();

    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            assert!(game.mod_screen_open("mod_a"));

            // Enter and Space activate the initially focused button.
            assert!(game.mounted_widget_key_pressed(KeyCode::Enter, false, 0));
            assert_eq!(
                game.mounted_widget_targets()[1].rect,
                [20.0, 50.0, 100.0, 20.0]
            );
            assert!(game.mounted_widget_key_pressed(KeyCode::Space, false, 0));
            assert_eq!(game.mounted_widget_targets()[1].rect[2], i32::MAX as f32);
            let mut batch = HudBatch::scratch([640, 480]);
            game.draw_mounted_screen_widgets(&mut batch);
            assert!(!batch.vertices.is_empty());
            assert!(
                batch
                    .vertices
                    .iter()
                    .all(|vertex| vertex.pos.into_iter().all(f32::is_finite))
            );
            assert_eq!(take_log(&log), vec!["kb_pressed", "kb_pressed"]);

            // Clicking the toggle button mutates its ButtonState (label + active = false).
            move_mouse(game, win_id, 50.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(take_log(&log), vec!["toggle:true"]);
            // Second click does nothing because the button disabled itself.
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(take_log(&log).is_empty());

            // Clicking "To B" calls host.mod_screen_open("mod_b") synchronously inside on_press.
            move_mouse(game, win_id, 50.0, 90.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_b"));
            assert_eq!(
                take_log(&log),
                vec!["init_mod_b", "switch_to_b:true:Some(\"mod_b\")"]
            );
            // Extracted mod_a button was disposed rather than written back into mod_b's table.
            assert_eq!(mod_a_btn_drops.load(Ordering::SeqCst), 1);
            assert_eq!(game.client.screen_state.0.widgets.len(), 1);
            assert_eq!(mod_b_btn_drops.load(Ordering::SeqCst), 0);

            // Pressing Enter on mod_b's focused button calls host.close_screen() synchronously.
            assert!(game.mounted_widget_key_pressed(KeyCode::Enter, false, 0));
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(game.mod_screen(), None);
            assert_eq!(take_log(&log), vec!["close_b:true:None"]);
            assert_eq!(mod_b_btn_drops.load(Ordering::SeqCst), 1);
            assert!(game.client.screen_state.0.widgets.is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn controller_activates_the_selected_mount_and_edits_its_field() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::controller_activates_the_selected_mount_and_edits_its_field",
        "SUNRISE_CONTROLLER_MOUNTS_CASE",
        &["controller"],
    ) {
        return;
    }
    use sunrise_client::{
        gamepad::{Button as PadButton, TextKeyboard},
        osk::Output,
    };

    struct Mounts(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Mounts {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id == "mod" {
                for name in ["first", "second"] {
                    let log = self.0.clone();
                    event.widgets.add_button(Button::new(
                        [30.0, 30.0, 90.0, 20.0],
                        ApiText::literal(name),
                        move |_, _| log.lock().unwrap().push(name.to_owned()),
                    ));
                }
            } else if event.screen.id != "chat" {
                return;
            }
            let log = self.0.clone();
            event
                .widgets
                .add_text_field(TextField::new([30.0, 70.0, 90.0, 20.0], 32).on_submit(
                    move |value, _| log.lock().unwrap().push(format!("submit:{value}")),
                ));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "controller_mounts",
        Box::new(Mounts(log.clone())),
    )]
    .into();
    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            game.client.settings.controller.keyboard = TextKeyboard::Sunrise;
            assert!(game.mod_screen_open("controller_mounts"));
            let targets = game.mounted_widget_targets();
            game.client.pad.0.menu.focus = Some(targets[1]);
            game.pad_menu_button(&NoLoop, PadButton::South, true)
                .unwrap();
            assert!(take_log(&log).is_empty());
            game.pad_menu_button(&NoLoop, PadButton::South, false)
                .unwrap();
            assert_eq!(take_log(&log), ["second"]);

            move_mouse(game, win_id, 40.0, 40.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(take_log(&log), ["first"]);

            game.client.pad.0.menu.focus = Some(targets[2]);
            game.pad_menu_button(&NoLoop, PadButton::South, true)
                .unwrap();
            assert!(game.client.pad.0.menu.osk.is_some());
            game.pad_osk_output(Output::Text("ab".into()));
            game.pad_osk_output(Output::Backspace);
            game.pad_osk_output(Output::Enter);
            assert_eq!(take_log(&log), ["submit:a"]);
            assert!(game.client.pad.0.menu.osk.is_none());

            game.client.chat_ui.chat.add_recent("from history");
            game.begin_chat_screen("native draft", false);
            let field = game.mounted_widget_targets()[0];
            game.client.pad.0.menu.focus = Some(field);
            game.pad_menu_button(&NoLoop, PadButton::South, true)
                .unwrap();
            assert!(game.client.pad.0.menu.osk.is_some());
            game.pad_osk_output(Output::Text("mounted draft".into()));
            game.pad_osk_output(Output::Previous);
            assert_eq!(
                game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                "native draft"
            );
            assert_eq!(
                game.mounted_focused_text_edit(None).unwrap().value,
                "mounted draft"
            );

            // The same keyboard can use Chat's history again when its field owns it.
            game.clear_mounted_widget_focus();
            game.pad_osk_output(Output::Previous);
            assert_eq!(
                game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                "from history"
            );
            game.sync_mounted_focus(field);
            game.pad_osk_output(Output::Next);
            assert_eq!(
                game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                "from history"
            );
            game.pad_osk_output(Output::Enter);
            assert_eq!(take_log(&log), ["submit:mounted draft"]);
        });
    });
}

/// Open `ScreenWidget` full callback surface & per-widget panic isolation:
/// - Verifies `render`, `mouse_clicked` (including `double_click`), `mouse_dragged`,
///   `mouse_released`, `mouse_scrolled`, `key_pressed`, `key_released`, `char_typed`, and
///   `set_focused`.
/// - Panics in a button `on_press`, a text field `on_change`, and a custom `ScreenWidget`
///   disable ONLY the panicking widget, queue `HookPanicReport`s, and leave sibling widgets
///   working.
/// - A non-panic refusal (`host.mod_screen_open(...)` returning `false`) does NOT disable the
///   widget.
#[test]
#[ignore = "daily"]
fn custom_screen_widget_callbacks_and_per_widget_panic_isolation() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::custom_screen_widget_callbacks_and_per_widget_panic_isolation",
        "SUNRISE_WIDGETS_CUSTOM_PANIC_CHILD",
        &["window"],
    ) {
        return;
    }

    struct RecordingWidget {
        bounds: [f32; 4],
        log: Arc<Mutex<Vec<String>>>,
        panic_on_click: Arc<AtomicBool>,
    }

    impl ScreenWidget for RecordingWidget {
        fn bounds(&self) -> [f32; 4] {
            self.bounds
        }
        fn render(
            &mut self,
            _host: &dyn ClientHost,
            draw: &mut dyn DrawContext,
            mouse: [f32; 2],
            hovered: bool,
            focused: bool,
            _partial_tick: f32,
        ) {
            draw.fill(self.bounds, 0xFF22_4466);
            self.log.lock().unwrap().push(format!(
                "render:{:.0},{:.0}:{hovered}:{focused}",
                mouse[0], mouse[1]
            ));
        }
        fn mouse_clicked(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            button: i32,
            _modifiers: u32,
            double_click: bool,
        ) -> bool {
            if self.panic_on_click.load(Ordering::SeqCst) {
                panic!("custom widget boom");
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("click:{button}:{double_click}"));
            true
        }
        fn mouse_dragged(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            button: i32,
            delta: [f32; 2],
            _modifiers: u32,
        ) -> bool {
            self.log
                .lock()
                .unwrap()
                .push(format!("drag:{button}:{:.0},{:.0}", delta[0], delta[1]));
            true
        }
        fn mouse_released(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            button: i32,
            _modifiers: u32,
        ) -> bool {
            self.log.lock().unwrap().push(format!("release:{button}"));
            true
        }
        fn mouse_scrolled(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            scroll: [f64; 2],
            _modifiers: u32,
        ) -> bool {
            self.log
                .lock()
                .unwrap()
                .push(format!("scroll:{:.0},{:.0}", scroll[0], scroll[1]));
            true
        }
        fn key_pressed(
            &mut self,
            host: &mut dyn ClientHost,
            key: &str,
            repeat: bool,
            _modifiers: u32,
        ) -> bool {
            self.log
                .lock()
                .unwrap()
                .push(format!("key_down:{key}:{repeat}"));
            if key == "key.keyboard.x" {
                host.close_screen();
            }
            true
        }
        fn key_released(&mut self, _host: &mut dyn ClientHost, key: &str, _modifiers: u32) -> bool {
            self.log.lock().unwrap().push(format!("key_up:{key}"));
            true
        }
        fn char_typed(&mut self, _host: &mut dyn ClientHost, c: char, _modifiers: u32) -> bool {
            self.log.lock().unwrap().push(format!("char:{c}"));
            true
        }
        fn set_focused(&mut self, focused: bool) {
            self.log.lock().unwrap().push(format!("focus:{focused}"));
        }
    }

    struct PanicMod {
        log: Arc<Mutex<Vec<String>>>,
        panic_custom: Arc<AtomicBool>,
        block_open: Arc<AtomicBool>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for PanicMod {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            if self.block_open.load(Ordering::SeqCst) && event.new.id == "mod" {
                Some(ScreenOpening::Cancel)
            } else {
                None
            }
        }

        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if host.mod_screen() != Some("panic_mod") {
                return;
            }
            // 0: Recording custom widget (can panic on click once armed)
            event.widgets.add_widget(Box::new(RecordingWidget {
                bounds: [20.0, 20.0, 80.0, 20.0],
                log: self.log.clone(),
                panic_on_click: self.panic_custom.clone(),
            }));
            // 1: Panicking button
            event.widgets.add_button(Button::new(
                [20.0, 50.0, 80.0, 20.0],
                ApiText::literal("BoomBtn"),
                |_state, _host| {
                    panic!("button boom");
                },
            ));
            // 2: Panicking text field
            event
                .widgets
                .add_text_field(TextField::new([20.0, 80.0, 80.0, 20.0], 16).on_change(
                    |_text, _host| {
                        panic!("textfield boom");
                    },
                ));
            // 3: Healthy sibling button that also exercises a non-panic refusal (`mod_screen_open` -> false)
            let log_sibling = self.log.clone();
            event.widgets.add_button(Button::new(
                [20.0, 110.0, 80.0, 20.0],
                ApiText::literal("Sibling"),
                move |_state, host| {
                    let opened = host.mod_screen_open("other_mod");
                    log_sibling
                        .lock()
                        .unwrap()
                        .push(format!("sibling_open:{opened}"));
                },
            ));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let panic_custom = Arc::new(AtomicBool::new(false));
    let block_open = Arc::new(AtomicBool::new(false));
    let mods: Arc<[ModEntry]> = vec![ModEntry::new(
        "panic_mod",
        Box::new(PanicMod {
            log: log.clone(),
            panic_custom: panic_custom.clone(),
            block_open: block_open.clone(),
        }),
    )]
    .into();

    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            let _ = mod_api::__private::take_hook_panics();
            assert!(game.mod_screen_open("panic_mod"));

            // Exercise every ScreenWidget callback on the recording widget.
            move_mouse(game, win_id, 40.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            move_mouse(game, win_id, 55.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            // Second click within 250ms delivers double_click = true.
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            send_scroll(game, win_id, 0.0, 1.0);
            assert!(game.mounted_widget_key_pressed(KeyCode::KeyG, false, 0));
            assert!(game.mounted_widget_key_released(KeyCode::KeyG, 0));
            assert!(game.mounted_widget_char_typed('g', 0));
            let mut batch = HudBatch::scratch([640, 480]);
            game.draw_mounted_screen_widgets(&mut batch);

            let events = take_log(&log);
            assert!(events.iter().any(|e| e == "click:0:false"), "{events:?}");
            assert!(events.iter().any(|e| e == "focus:true"), "{events:?}");
            assert!(events.iter().any(|e| e == "drag:0:15,0"), "{events:?}");
            assert!(events.iter().any(|e| e == "release:0"), "{events:?}");
            assert!(events.iter().any(|e| e == "click:0:true"), "{events:?}");
            assert!(
                events.iter().any(|e| e.starts_with("scroll:0,")),
                "{events:?}"
            );
            assert!(
                events.iter().any(|e| e == "key_down:key.keyboard.g:false"),
                "{events:?}"
            );
            assert!(
                events.iter().any(|e| e == "key_up:key.keyboard.g"),
                "{events:?}"
            );
            assert!(events.iter().any(|e| e == "char:g"), "{events:?}");
            assert!(
                events.iter().any(|e| e == "render:55,30:true:true"),
                "{events:?}"
            );

            // The focused custom child is blurred on a rebuild, on close, and
            // when temporarily extracted if it closes the screen itself.
            game.handle_window_event(
                &NoLoop,
                win_id,
                WindowEvent::Resized(PhysicalSize::new(800, 600)),
                InputOrigin::User,
            );
            assert_eq!(take_log(&log), ["focus:false"]);
            // Match the offscreen window's 640x480 inner size again so pointer projection stays 1:1.
            game.handle_window_event(
                &NoLoop,
                win_id,
                WindowEvent::Resized(PhysicalSize::new(640, 480)),
                InputOrigin::User,
            );
            assert!(take_log(&log).is_empty());
            game.sync_mounted_focus(game.mounted_widget_targets()[0]);
            assert!(game.close_screen());
            assert_eq!(take_log(&log), ["focus:true", "focus:false"]);
            assert!(game.mod_screen_open("panic_mod"));
            game.sync_mounted_focus(game.mounted_widget_targets()[0]);
            assert!(game.mounted_widget_key_pressed(KeyCode::KeyX, false, 0));
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(
                take_log(&log),
                ["focus:true", "key_down:key.keyboard.x:false", "focus:false"]
            );
            assert!(game.mod_screen_open("panic_mod"));

            // 1) Panic in Button on_press disables only the button and queues a panic notice.
            move_mouse(game, win_id, 40.0, 60.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(game.client.screen_state.0.widgets.is_disabled(1));

            // 2) Panic in TextField on_change disables only the text field.
            move_mouse(game, win_id, 40.0, 90.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(game.mounted_widget_char_typed('a', 0));
            assert!(game.client.screen_state.0.widgets.is_disabled(2));

            // 3) Panic in custom ScreenWidget mouse_clicked disables only the custom widget.
            panic_custom.store(true, Ordering::SeqCst);
            move_mouse(game, win_id, 40.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(game.client.screen_state.0.widgets.is_disabled(0));

            let notices = mod_api::__private::take_hook_panics();
            assert_eq!(notices.len(), 3, "{notices:?}");
            assert!(notices.iter().any(|n| n.hook == "button_press"));
            assert!(notices.iter().any(|n| n.hook == "text_field_change"));
            assert!(
                notices
                    .iter()
                    .any(|n| n.hook == "screen_widget_mouse_clicked")
            );

            // 4) Healthy sibling keeps working; non-panic refusal (`mod_screen_open` returning false)
            // does NOT disable the widget and queues no panic notice.
            block_open.store(true, Ordering::SeqCst);
            take_log(&log);
            move_mouse(game, win_id, 40.0, 120.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                take_log(&log),
                vec!["sibling_open:false", "sibling_open:false"]
            );
            assert!(!game.client.screen_state.0.widgets.is_disabled(3));
            assert!(mod_api::__private::take_hook_panics().is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn init_post_edits_ordered_children_and_discards_a_failed_stage() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::init_post_edits_ordered_children_and_discards_a_failed_stage",
        "SUNRISE_WIDGETS_EDIT_CHILD",
        &["window"],
    ) {
        return;
    }

    struct EditChildren {
        stage: u8,
        log: Arc<Mutex<Vec<String>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for EditChildren {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "paused" {
                return;
            }
            let widgets = &mut *event.widgets;
            let resume_label = ApiText::translate("menu.returnToGame", vec![]);
            let prior_label = ApiText::literal("Prior mount");
            let replacement_label = ApiText::literal("Replacement");
            match self.stage {
                0 => {
                    assert_eq!(widgets.children()[0].label.as_ref(), Some(&resume_label));
                    let prior = widgets.add_button(Button::new(
                        [20.0, 20.0, 80.0, 20.0],
                        prior_label,
                        |_, _| {},
                    ));
                    widgets.set_initial_focus(prior);
                    self.log.lock().unwrap().push("added".into());
                }
                1 => {
                    let prior = widgets.children().last().unwrap().id;
                    assert_eq!(
                        widgets.children().last().unwrap().label.as_ref(),
                        Some(&prior_label)
                    );
                    let resume = widgets.children()[0].id;
                    assert!(widgets.remove(resume));
                    assert!(widgets.remove(prior));
                    widgets.add_button(Button::inactive(
                        [20.0, 20.0, 80.0, 20.0],
                        ApiText::literal("Failed"),
                    ));
                    self.log.lock().unwrap().push("about to panic".into());
                    panic!("discard both removals and the new mount");
                }
                2 => {
                    let resume = &widgets.children()[0];
                    assert_eq!(resume.label.as_ref(), Some(&resume_label));
                    let (id, rect) = (resume.id, resume.rect);
                    assert_eq!(
                        widgets.children().last().unwrap().label.as_ref(),
                        Some(&prior_label)
                    );
                    let prior = widgets.children().last().unwrap().id;
                    assert!(widgets.remove(id));
                    assert!(widgets.remove(prior));
                    assert!(!widgets.remove(id));
                    let log = self.log.clone();
                    widgets.add_button(Button::new(rect, replacement_label, move |_, _| {
                        log.lock().unwrap().push("replacement pressed".into());
                    }));
                }
                _ => {
                    let children = widgets.children();
                    assert!(children.iter().all(|child| {
                        child
                            .label
                            .as_ref()
                            .is_none_or(|label| label != &resume_label && label != &prior_label)
                    }));
                    assert_eq!(
                        children.last().unwrap().label.as_ref(),
                        Some(&replacement_label)
                    );
                    self.log.lock().unwrap().push("published".into());
                }
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods: Vec<_> = ["child_a", "child_b", "child_c", "child_d"]
        .into_iter()
        .enumerate()
        .map(|(stage, id)| {
            ModEntry::new(
                id,
                Box::new(EditChildren {
                    stage: stage as u8,
                    log: log.clone(),
                }),
            )
        })
        .collect();
    with_mods(mods.into(), || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            game.client.pause_menu = true;
            game.set_screen(Screen::Paused);
            assert_eq!(take_log(&log), ["added", "about to panic", "published"]);
            assert_eq!(game.client.screen_state.0.widgets.len(), 1);
            assert!(!game.client.screen_state.0.widgets.has_focus());
            assert!(game.pause_child_removed(0));
            assert!(!game.pause_child_removed(1));
            let [x, y, _, _] = game.mounted_widget_targets()[0].rect;
            move_mouse(game, win_id, f64::from(x + 5.0), f64::from(y + 5.0));
            assert!(
                !game.pause_click().unwrap(),
                "removed vanilla action must not activate"
            );
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(game.client.screen, Screen::Paused);
            assert_eq!(take_log(&log), ["replacement pressed"]);
            let notices = mod_api::__private::take_hook_panics();
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].mod_id, "child_b");
        });
    });
}

#[test]
#[ignore = "daily"]
fn mounted_button_sprites_keep_their_pixels_and_accessibility() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::mounted_button_sprites_keep_their_pixels_and_accessibility",
        "SUNRISE_WIDGET_SPRITES_CHILD",
        &["sprites"],
    ) {
        return;
    }
    use mod_api::client::widgets::ButtonSprites;
    use sunrise_client::{
        font::Patch,
        gfx::entity_pixel_test::render_hud,
        hud::{GuiSprite, Rect},
        widgets::{self, Cursor, Kind, ScreenInfo, What},
    };

    const SIZE: [u32; 2] = [320, 240];
    const COUNT: usize = 50;
    fn bounds(i: usize) -> [f32; 4] {
        [
            10.0 + (i % 10) as f32 * 20.0,
            10.0 + (i / 10) as f32 * 20.0,
            14.0,
            15.0,
        ]
    }
    fn sprite(i: usize, state: u8) -> String {
        format!("fixture:button/{i}/{state}")
    }
    let color = |i: usize, state: u8| [64 + i as u8, 16 + state * 80, 230, 255];

    struct SpriteMod(Arc<Mutex<Vec<usize>>>);
    #[mod_api::__subscriptions]
    impl Hooks for SpriteMod {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "mod" {
                return;
            }
            for i in 0..COUNT {
                let mut art = ButtonSprites::new(sprite(i, 0));
                art.highlighted = Some(sprite(i, 1));
                art.disabled = Some(sprite(i, 2));
                let log = self.0.clone();
                event.widgets.add_button(
                    Button::new(bounds(i), ApiText::literal("iii"), move |_, _| {
                        log.lock().unwrap().push(i);
                    })
                    .active(i != 1)
                    .sprites(art),
                );
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "sprite_fixture",
            Box::new(SpriteMod(log.clone())),
        )]
        .into(),
        || {
            with_client_game(|game| {
                let (_event_loop, _win_id) = attach_offscreen_window(game);
                let atlas = &mut game.client.gui.atlas;
                let slot = atlas.sprites["sunrise/skin0"];
                let (aw, ah) = (atlas.width as f32, atlas.height as f32);
                let [ox, oy] = [(slot.u0 * aw).round() as u32, (slot.v0 * ah).round() as u32];
                let mut palette = Vec::new();
                for state in 0..3 {
                    for i in 0..COUNT {
                        let x = ox + i as u32;
                        let y = oy + u32::from(state);
                        palette.extend(color(i, state));
                        atlas.gui.insert(
                            sprite(i, state),
                            GuiSprite::Present {
                                rect: Rect {
                                    u0: x as f32 / aw,
                                    v0: y as f32 / ah,
                                    u1: (x + 1) as f32 / aw,
                                    v1: (y + 1) as f32 / ah,
                                    w: 1,
                                    h: 1,
                                },
                                scaling: None,
                            },
                        );
                    }
                }
                let patch = Patch {
                    x: ox,
                    y: oy,
                    width: COUNT as u32,
                    height: 3,
                    rgba: palette,
                };
                let pixel = |pixels: &[u8], [x, y, _, _]: [f32; 4]| {
                    let at = (((y as u32 + 1) * SIZE[0] + x as u32 + 1) * 4) as usize;
                    <[u8; 4]>::try_from(&pixels[at..at + 4]).unwrap()
                };
                let mut stock = HudBatch::scratch(SIZE);
                for i in 0..COUNT {
                    let [x, y, w, h] = bounds(i);
                    sunrise_client::ui::button(
                        &mut stock,
                        &game.client.gui.atlas,
                        x,
                        y,
                        w,
                        h,
                        &sunrise_game_data::text::Text::literal("iii"),
                        i != 1,
                        false,
                    );
                }
                widgets::end_frame(1);
                let stock = render_hud(&stock, &game.client.gui.atlas, SIZE).unwrap();
                assert!(game.mod_screen_open("sprite_fixture"));
                let frame = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                             at,
                             serial| {
                    game.client.cursor = at;
                    let mut batch = HudBatch::scratch(SIZE);
                    batch.patches.push(patch.clone());
                    widgets::begin_screen(|| ScreenInfo::new("fixture", "Mounted sprite buttons"));
                    game.draw_mounted_screen_widgets(&mut batch);
                    widgets::end_frame(serial);
                    render_hud(&batch, &game.client.gui.atlas, SIZE).unwrap()
                };
                let custom = frame(game, (1.0, 1.0), 2);
                for i in 0..COUNT {
                    let expected = color(i, if i == 1 { 2 } else { 0 });
                    assert_eq!(pixel(&custom, bounds(i)), expected);
                    assert_ne!(pixel(&stock, bounds(i)), expected);
                }
                for (i, (a, b)) in custom
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(stock.as_chunks::<4>().0)
                    .enumerate()
                {
                    let (x, y) = ((i as u32 % SIZE[0]) as f32, (i as u32 / SIZE[0]) as f32);
                    if !(0..COUNT).any(|n| {
                        let [l, t, w, h] = bounds(n);
                        (l..l + w).contains(&x) && (t..t + h).contains(&y)
                    }) {
                        assert_eq!(a, b, "outside mounted buttons at {x},{y}");
                    }
                }
                let buttons = widgets::last_labelled();
                assert_eq!(buttons.len(), COUNT);
                for (i, button) in buttons.iter().enumerate() {
                    assert_eq!([button.x, button.y, button.w, button.h], bounds(i));
                    assert_eq!(button.enabled, i != 1);
                    assert!(matches!(button.kind, Kind::Mounted { field: false, .. }));
                }

                let hovered = frame(game, (11.0, 11.0), 3);
                assert_eq!(pixel(&hovered, bounds(0)), color(0, 1));
                assert_eq!(widgets::take_cursor(), Cursor::PointingHand);
                let disabled = frame(game, (31.0, 11.0), 4);
                assert_eq!(pixel(&disabled, bounds(1)), color(1, 2));
                assert!(!game.mounted_widget_mouse_clicked([31.0, 11.0], 0, 0, false));
                assert!(game.mounted_widget_mouse_clicked([11.0, 11.0], 0, 0, false));
                assert!(game.mounted_widget_mouse_released([11.0, 11.0], 0, 0));
                assert!(game.key_focus_key(&NoLoop, KeyCode::Tab).is_some());
                assert_eq!(game.client.screen_state.0.widgets.focused_index(), Some(2));

                let _ = widgets::take_cursor();
                widgets::set_narrating(true);
                let focused = frame(game, (1.0, 1.0), 5);
                widgets::set_narrating(false);
                assert_eq!(widgets::take_cursor(), Cursor::Arrow);
                assert_eq!(pixel(&focused, bounds(2)), color(2, 1));
                let narration = widgets::take_last_narration();
                assert_eq!(narration.items.len(), COUNT);
                assert!(
                    matches!(&narration.items[2].what, What::Button { message, active: true } if message == "iii")
                );
                assert_eq!(narration.items[2].hovered, Some(true));
                assert!(
                    narration
                        .items
                        .iter()
                        .enumerate()
                        .all(|(i, item)| item.hovered == Some(i == 2))
                );
                assert_eq!(narration.items[2].rect, bounds(2));
                assert!(game.mounted_widget_key_pressed(KeyCode::Enter, false, 0));
                assert!(game.mounted_widget_key_pressed(KeyCode::Space, false, 0));
                assert_eq!(*log.lock().unwrap(), [0, 2, 2]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn mounted_widgets_draw_as_the_games_own() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::mounted_widgets_draw_as_the_games_own",
        "SUNRISE_WIDGET_STOCK_ART_CHILD",
        &["stock"],
    ) {
        return;
    }
    use std::time::{Duration, Instant};
    use sunrise_client_gameplay::game::screen_widgets::MountedWidgetKind;
    const LONG: &str = "a long name that does not fit in sixty pixels";
    struct StockWidgets;
    #[mod_api::__subscriptions]
    impl Hooks for StockWidgets {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "paused" {
                return;
            }
            event.widgets.add_button(Button::new(
                [10.0, 10.0, 100.0, 20.0],
                ApiText::literal("Done"),
                |_, _| {},
            ));
            let field = event
                .widgets
                .add_text_field(TextField::new([10.0, 40.0, 60.0, 20.0], 64).value(LONG));
            event.widgets.set_initial_focus(field);
        }
    }
    with_mods(
        vec![ModEntry::new("stock_widgets", Box::new(StockWidgets))].into(),
        || {
            with_client_game(|game| {
                game.set_screen(Screen::Paused);
                game.client.cursor = (11.0, 11.0);
                let focus_time = Instant::now();
                game.with_mounted_widget(1, |_, entry| {
                    let MountedWidgetKind::TextField { edit_box, .. } = &mut entry.kind else {
                        panic!("expected the initialized text field")
                    };
                    edit_box.reset_blink_at(focus_time);
                });
                let visible_at = focus_time + Duration::from_millis(299);
                let mut ours = HudBatch::scratch([320, 240]);
                game.draw_mounted_screen_widgets_at(&mut ours, visible_at);
                let mut stock = HudBatch::scratch([320, 240]);
                sunrise_client::ui::button(
                    &mut stock,
                    &game.client.gui.atlas,
                    10.0,
                    10.0,
                    100.0,
                    20.0,
                    &sunrise_game_data::text::Text::literal("Done"),
                    true,
                    true,
                );
                let n = stock.vertices.len();
                assert!(n > 0);
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&ours.vertices[..n]),
                    bytemuck::cast_slice::<_, u8>(&stock.vertices)
                );

                let mut stock_field = sunrise_client::ui::EditBox::new(10.0, 40.0, 60.0, 20.0);
                stock_field.max_length = 64;
                stock_field.set_value(LONG);
                stock_field.focused = true;
                stock_field.reset_blink_at(focus_time);
                stock_field.draw_at(&mut stock, &game.client.gui.atlas, None, visible_at);
                assert_eq!(
                    bytemuck::cast_slice::<_, u8>(&ours.vertices),
                    bytemuck::cast_slice::<_, u8>(&stock.vertices)
                );

                // Independent literal foreground glyph origins for "ixty pixels" + "_" caret in
                // vanilla 26.3's ascii.png inside [10, 40, 60, 20] (inner width 52, text_x = 14,
                // text_y = 46): widths i=2, x=6, t=4, y=6, space=4, p=6, i=2, x=6, e=6, l=3, s=6
                // (total 51 <= 52, scrolled to index 34), caret "_" at 14 + 51 + 1 = 66.
                const EXPECTED_FOREGROUND_ORIGINS: [[f32; 2]; 11] = [
                    [14.0, 46.0],
                    [16.0, 46.0],
                    [22.0, 46.0],
                    [26.0, 46.0],
                    [36.0, 46.0],
                    [42.0, 46.0],
                    [44.0, 46.0],
                    [50.0, 46.0],
                    [56.0, 46.0],
                    [59.0, 46.0],
                    [66.0, 46.0],
                ];
                let fg_color = sunrise_client::hud::rgba(224, 224, 224, 255);
                let shadow_color = sunrise_client::hud::rgba(56, 56, 56, 255);
                let field_text_verts: Vec<_> = ours.vertices[n..]
                    .iter()
                    .copied()
                    .filter(|v| v.text == 1)
                    .collect();
                assert_eq!(
                    field_text_verts.len(),
                    EXPECTED_FOREGROUND_ORIGINS.len() * 12
                );
                let mut fg_origins = Vec::new();
                let mut shadow_origins = Vec::new();
                for quad in field_text_verts.as_chunks::<6>().0 {
                    if quad[0].color == fg_color {
                        fg_origins.push(quad[0].pos);
                    } else if quad[0].color == shadow_color {
                        shadow_origins.push(quad[0].pos);
                    } else {
                        panic!("unexpected text quad color: {:#010x}", quad[0].color);
                    }
                }
                assert_eq!(fg_origins, EXPECTED_FOREGROUND_ORIGINS);
                assert_eq!(
                    shadow_origins,
                    EXPECTED_FOREGROUND_ORIGINS.map(|[x, y]| [x + 1.0, y + 1.0])
                );

                let mut hidden = HudBatch::scratch([320, 240]);
                let hidden_at = focus_time + Duration::from_millis(300);
                game.draw_mounted_screen_widgets_at(&mut hidden, hidden_at);
                // The final glyph and its shadow disappear at the phase boundary.
                assert_eq!(
                    hidden.vertices.iter().filter(|v| v.text == 1).count() + 12,
                    ours.vertices.iter().filter(|v| v.text == 1).count(),
                );

                assert!(game.mounted_widget_mouse_clicked([16.0, 50.0], 0, 0, false));
                assert_eq!(game.mounted_focused_text_edit(None).unwrap().cursor, 35);
                assert!(game.mounted_widget_mouse_clicked([36.0, 50.0], 0, 0, false));
                assert_eq!(game.mounted_focused_text_edit(None).unwrap().cursor, 39);
            });
        },
    );
}

/// Mounting at Init & lifecycle: `add_button`, `add_text_field`, `add_widget` assign distinct
/// `ScreenWidgetId`s; `set_initial_focus` focuses the requested widget; `remove` drops removed
/// widgets during init; closing or resizing disposes mounted widgets via `Drop` and clears
/// focus/drag state.
#[test]
#[ignore = "daily"]
fn mounting_lifecycle_focus_remove_resize_and_close_dispose() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::mounting_lifecycle_focus_remove_resize_and_close_dispose",
        "SUNRISE_WIDGETS_LIFECYCLE_CHILD",
        &["window"],
    ) {
        return;
    }

    struct DummyCustom {
        bounds: [f32; 4],
        _drop: DropCounter,
    }
    impl ScreenWidget for DummyCustom {
        fn bounds(&self) -> [f32; 4] {
            self.bounds
        }
    }

    struct MountMod {
        drops: Arc<AtomicUsize>,
        ids: Arc<Mutex<Vec<ScreenWidgetId>>>,
        focus_results: Arc<Mutex<Vec<bool>>>,
        remove_results: Arc<Mutex<Vec<bool>>>,
        init_count: Arc<AtomicUsize>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for MountMod {
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id == "options" {
                self.init_count.fetch_add(1, Ordering::SeqCst);
                let field = event
                    .widgets
                    .add_text_field(TextField::new([10.0, 40.0, 120.0, 20.0], 32));
                assert!(event.widgets.set_initial_focus(field));
                return;
            }
            if host.mod_screen() != Some("mount_mod") {
                return;
            }
            self.init_count.fetch_add(1, Ordering::SeqCst);
            let btn_drop = DropCounter(self.drops.clone());
            let btn_id = event.widgets.add_button(Button::new(
                [10.0, 10.0, 80.0, 20.0],
                ApiText::literal("Btn"),
                move |_state, _host| {
                    let _ = &btn_drop;
                },
            ));
            let tf_drop = DropCounter(self.drops.clone());
            let tf_id = event.widgets.add_text_field(
                TextField::new([10.0, 40.0, 120.0, 20.0], 32).on_change(move |_text, _host| {
                    let _ = &tf_drop;
                }),
            );
            let removed_drop = DropCounter(self.drops.clone());
            let removed_id = event.widgets.add_widget(Box::new(DummyCustom {
                bounds: [10.0, 70.0, 40.0, 20.0],
                _drop: removed_drop,
            }));
            let kept_id = event.widgets.add_widget(Box::new(DummyCustom {
                bounds: [60.0, 70.0, 40.0, 20.0],
                _drop: DropCounter(self.drops.clone()),
            }));
            let inactive_id = event.widgets.add_button(Button::inactive(
                [10.0, 100.0, 80.0, 20.0],
                ApiText::literal("Off"),
            ));

            self.ids
                .lock()
                .unwrap()
                .extend([btn_id, tf_id, removed_id, kept_id, inactive_id]);

            let removed_first = event.widgets.remove(removed_id);
            let removed_again = event.widgets.remove(removed_id);
            self.remove_results
                .lock()
                .unwrap()
                .extend([removed_first, removed_again]);

            let focus_removed = event.widgets.set_initial_focus(removed_id);
            let focus_inactive = event.widgets.set_initial_focus(inactive_id);
            let focus_tf = event.widgets.set_initial_focus(tf_id);
            self.focus_results
                .lock()
                .unwrap()
                .extend([focus_removed, focus_inactive, focus_tf]);
        }
    }

    struct FailedMount(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for FailedMount {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.mod_screen != Some("mount_mod") {
                return;
            }
            let id = event.widgets.add_widget(Box::new(DummyCustom {
                bounds: [10.0, 10.0, 40.0, 20.0],
                _drop: DropCounter(self.0.clone()),
            }));
            assert!(event.widgets.set_initial_focus(id));
            panic!("widget init failed after registering a child");
        }
    }

    let failed_drops = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let ids = Arc::new(Mutex::new(Vec::new()));
    let focus_results = Arc::new(Mutex::new(Vec::new()));
    let remove_results = Arc::new(Mutex::new(Vec::new()));
    let init_count = Arc::new(AtomicUsize::new(0));
    let mods: Arc<[ModEntry]> = vec![
        ModEntry::new("failed_mount", Box::new(FailedMount(failed_drops.clone()))),
        ModEntry::new(
            "mount_mod",
            Box::new(MountMod {
                drops: drops.clone(),
                ids: ids.clone(),
                focus_results: focus_results.clone(),
                remove_results: remove_results.clone(),
                init_count: init_count.clone(),
            }),
        ),
    ]
    .into();

    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);

            assert!(game.mod_screen_open("mount_mod"));
            assert_eq!(init_count.load(Ordering::SeqCst), 1);
            // The failed listener cannot publish its widget or focus, and the healthy listener
            // still mounts. Its failed stage is disposed and diagnosed exactly once.
            assert_eq!(failed_drops.load(Ordering::SeqCst), 1);
            let notices = mod_api::__private::take_hook_panics();
            assert_eq!(notices.len(), 1);
            assert_eq!(notices[0].mod_id, "failed_mount");
            assert_eq!(notices[0].hook, "screen_init_post");

            // All 5 mounted widgets received distinct ScreenWidgetIds.
            let assigned = std::mem::take(&mut *ids.lock().unwrap());
            assert_eq!(assigned.len(), 5);
            for i in 0..assigned.len() {
                for j in (i + 1)..assigned.len() {
                    assert_ne!(assigned[i], assigned[j]);
                }
            }
            assert_eq!(
                std::mem::take(&mut *remove_results.lock().unwrap()),
                vec![true, false]
            );
            assert_eq!(
                std::mem::take(&mut *focus_results.lock().unwrap()),
                vec![false, false, true]
            );
            // The removed custom widget was dropped immediately during init; 3 live drop counters remain.
            assert_eq!(drops.load(Ordering::SeqCst), 1);
            // 4 entries stay mounted (btn, tf, kept custom, inactive btn) and tf (index 1) has focus.
            assert_eq!(game.client.screen_state.0.widgets.len(), 4);
            assert_eq!(game.client.screen_state.0.widgets.focused_index(), Some(1));

            // Click into the text field to establish drag state, then resize the window.
            move_mouse(game, win_id, 30.0, 50.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            assert_eq!(game.client.screen_state.0.widgets.dragging(), Some((1, 0)));

            let gen_before_resize = game.client.screen_state.0.widgets.generation();
            game.handle_window_event(
                &NoLoop,
                win_id,
                WindowEvent::Resized(PhysicalSize::new(800, 600)),
                InputOrigin::User,
            );
            // No render or observe_screen call between resize and fresh input.
            assert_eq!(init_count.load(Ordering::SeqCst), 2);
            assert_eq!(failed_drops.load(Ordering::SeqCst), 1);
            game.screen_chars_typed(Some("r"));
            assert_eq!(game.mounted_focused_text_edit(None).unwrap().value, "r");
            assert_ne!(
                game.client.screen_state.0.widgets.generation(),
                gen_before_resize
            );
            // Prior 3 live drop counters + 1 newly removed drop counter = 5 total drops.
            assert_eq!(drops.load(Ordering::SeqCst), 5);
            // Drag state cleared on rebuild; initial focus re-applied to the newly mounted text field.
            assert_eq!(game.client.screen_state.0.widgets.dragging(), None);
            assert_eq!(game.client.screen_state.0.widgets.focused_index(), Some(1));

            // Closing the screen disposes the remaining 3 widgets and clears focus/drag state.
            assert!(game.close_screen());
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(drops.load(Ordering::SeqCst), 8);
            assert!(game.client.screen_state.0.widgets.is_empty());
            assert_eq!(game.client.screen_state.0.widgets.focused_index(), None);
            assert_eq!(game.client.screen_state.0.widgets.dragging(), None);

            // Options arranges its existing children instead of rebuilding them.
            game.open_options(false);
            let field = game.client.screen_state.0.widgets.focused_id().unwrap();
            assert_eq!(init_count.load(Ordering::SeqCst), 3);
            let mut edit = game.mounted_focused_text_edit(None).unwrap();
            edit.value = "note".to_owned();
            edit.highlight = 1;
            edit.cursor = 3;
            game.mounted_focused_text_edit(Some(&edit)).unwrap();
            game.handle_window_event(
                &NoLoop,
                win_id,
                WindowEvent::Resized(PhysicalSize::new(900, 700)),
                InputOrigin::User,
            );
            assert_eq!(init_count.load(Ordering::SeqCst), 3);
            assert_eq!(game.client.screen_state.0.widgets.focused_id(), Some(field));
            let preserved = game.mounted_focused_text_edit(None).unwrap();
            assert_eq!(preserved.value, "note");
            assert_eq!((preserved.highlight, preserved.cursor), (1, 3));
        });
    });
}

/// Separate input Post & Pre parity + receiving screen:
/// - `MouseButtonPressed.Pre` & `.Post` receive `double_click: bool`.
/// - Button Posts expose the immutable receiver answer and a `TriState` override.
///   Forcing unhandled skips `last_click` recording; forcing handled records it.
/// - A button press transitioning screen A -> screen B (or closing to `Playing`) still fires
///   `MouseButtonPressed.Post` with screen A's `event.screen.id`.
/// - `MouseDragged.Post`, `MouseScrolled.Post`, `CharacterTyped.Post`, `Preedit.Post` fire ONLY
///   when unhandled on the receiving screen (and are non-cancellable notices).
/// - `KeyPressed.Post` and `KeyReleased.Post` fire ONLY when unhandled on the receiving screen,
///   and returning `Flow::Cancel` blocks `mapped_key_event`.
#[test]
#[ignore = "daily"]
fn separate_input_pre_post_parity_and_receiving_screen() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::separate_input_pre_post_parity_and_receiving_screen",
        "SUNRISE_WIDGETS_INPUT_POST_CHILD",
        &["window"],
    ) {
        return;
    }

    struct DragScrollWidget {
        bounds: [f32; 4],
        handle_drag: Arc<AtomicBool>,
        handle_scroll: Arc<AtomicBool>,
        transition_on_drag: Arc<AtomicBool>,
    }

    impl ScreenWidget for DragScrollWidget {
        fn bounds(&self) -> [f32; 4] {
            self.bounds
        }
        fn mouse_clicked(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            _button: i32,
            _modifiers: u32,
            _double_click: bool,
        ) -> bool {
            true
        }
        fn mouse_dragged(
            &mut self,
            host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            _button: i32,
            _delta: [f32; 2],
            _modifiers: u32,
        ) -> bool {
            if self.transition_on_drag.load(Ordering::SeqCst) {
                assert!(host.mod_screen_open("target_mod"));
                host.close_screen();
                return false;
            }
            self.handle_drag.load(Ordering::SeqCst)
        }
        fn mouse_released(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            _button: i32,
            _modifiers: u32,
        ) -> bool {
            true
        }
        fn mouse_scrolled(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            _scroll: [f64; 2],
            _modifiers: u32,
        ) -> bool {
            self.handle_scroll.load(Ordering::SeqCst)
        }
    }

    struct DecliningEditor(Arc<Mutex<Vec<String>>>);
    impl ScreenWidget for DecliningEditor {
        fn bounds(&self) -> [f32; 4] {
            [20.0, 20.0, 80.0, 20.0]
        }
        fn char_typed(&mut self, _: &mut dyn ClientHost, c: char, _: u32) -> bool {
            self.0.lock().unwrap().push(format!("editor_char:{c}"));
            c == 'φ'
        }
        fn mouse_clicked(
            &mut self,
            _: &mut dyn ClientHost,
            _: [f32; 2],
            _: i32,
            _: u32,
            _: bool,
        ) -> bool {
            self.0.lock().unwrap().push("editor_click".into());
            true
        }
    }

    struct InputMod {
        log: Arc<Mutex<Vec<String>>>,
        dialog_mount: AtomicUsize,
        veto_dialog: Arc<AtomicBool>,
        override_press_handled: Arc<Mutex<Option<bool>>>,
        override_release_handled: Arc<Mutex<Option<bool>>>,
        cancel_key_post: Arc<AtomicBool>,
        handle_drag: Arc<AtomicBool>,
        handle_scroll: Arc<AtomicBool>,
        transition_on_drag: Arc<AtomicBool>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for InputMod {
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            match event.screen.id {
                "paused" => {
                    // Button on Paused that transitions to mod screen "target_mod".
                    event.widgets.add_button(Button::new(
                        [20.0, 20.0, 80.0, 20.0],
                        ApiText::literal("To Mod"),
                        |_state, host| {
                            host.mod_screen_open("target_mod");
                        },
                    ));
                }
                "mod" if host.mod_screen() == Some("target_mod") => {
                    // Button on target_mod that closes back to Playing.
                    event.widgets.add_button(Button::new(
                        [20.0, 20.0, 80.0, 20.0],
                        ApiText::literal("Close"),
                        |_state, host| {
                            host.close_screen();
                        },
                    ));
                    event.widgets.add_button(Button::new(
                        [20.0, 100.0, 80.0, 20.0],
                        ApiText::literal("Stay"),
                        |_, _| {},
                    ));
                    // Custom widget for drag/scroll handled vs unhandled checks.
                    event.widgets.add_widget(Box::new(DragScrollWidget {
                        bounds: [20.0, 60.0, 80.0, 20.0],
                        handle_drag: self.handle_drag.clone(),
                        handle_scroll: self.handle_scroll.clone(),
                        transition_on_drag: self.transition_on_drag.clone(),
                    }));
                }
                "dialog" => {
                    let mount = self.dialog_mount.fetch_add(1, Ordering::SeqCst);
                    let log = self.log.clone();
                    let id = event.widgets.add_button(Button::new(
                        [20.0, 20.0, 75.0, 20.0],
                        ApiText::literal("Mounted"),
                        move |_, _| log.lock().unwrap().push(format!("dialog_press:{mount}")),
                    ));
                    event.widgets.set_initial_focus(id);
                }
                "stats" => {
                    let id = event
                        .widgets
                        .add_widget(Box::new(DecliningEditor(self.log.clone())));
                    event.widgets.set_initial_focus(id);
                }
                "link_prompt" => {
                    let a = event
                        .widgets
                        .add_text_field(TextField::new([20.0, 20.0, 80.0, 20.0], 32));
                    event.widgets.add_text_field(
                        TextField::new([20.0, 20.0, 80.0, 20.0], 32).on_change(|text, host| {
                            if text == "switch" {
                                host.mod_screen_open("target_mod");
                            }
                        }),
                    );
                    event.widgets.set_initial_focus(a);
                }
                "chat" => {
                    let field = event
                        .widgets
                        .add_text_field(TextField::new([20.0, 20.0, 80.0, 20.0], 32));
                    event.widgets.set_initial_focus(field);
                }
                _ => {}
            }
        }

        fn screen_mouse_button_pressed_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseButtonPressedPreEvent<'_>,
        ) -> Flow {
            self.log.lock().unwrap().push(format!(
                "press_pre:{}:{}:{}",
                event.screen.id, event.button, event.double_click
            ));
            Flow::Continue
        }

        fn screen_mouse_button_pressed_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseButtonPressedPostEvent<'_>,
        ) {
            self.log.lock().unwrap().push(format!(
                "press_post:{}:{}:{}:{}",
                event.screen.id, event.button, event.double_click, event.was_click_handled
            ));
            if let Some(forced) = *self.override_press_handled.lock().unwrap() {
                *event.result = if forced {
                    mod_api::TriState::True
                } else {
                    mod_api::TriState::False
                };
            }
        }

        fn screen_mouse_button_released_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("release_pre:{}:{}", event.screen.id, event.button));
            Flow::Continue
        }

        fn screen_mouse_button_released_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseButtonReleasedPostEvent<'_>,
        ) {
            self.log.lock().unwrap().push(format!(
                "release_post:{}:{}:{}",
                event.screen.id, event.button, event.was_release_handled
            ));
            if let Some(forced) = *self.override_release_handled.lock().unwrap() {
                *event.result = if forced {
                    mod_api::TriState::True
                } else {
                    mod_api::TriState::False
                };
            }
        }

        fn screen_mouse_dragged_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseDraggedPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("drag_pre:{}", event.screen.id));
            Flow::Continue
        }

        fn screen_mouse_dragged_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseDraggedPostEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("drag_post:{}", event.screen.id));
        }

        fn screen_mouse_scrolled_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseScrolledPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("scroll_pre:{}", event.screen.id));
            Flow::Continue
        }

        fn screen_mouse_scrolled_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseScrolledPostEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("scroll_post:{}", event.screen.id));
        }

        fn screen_key_pressed_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenKeyPressedPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("key_press_pre:{}:{}", event.screen.id, event.key));
            Flow::Continue
        }

        fn screen_key_pressed_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenKeyPressedPostEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("key_press_post:{}:{}", event.screen.id, event.key));
            if self.cancel_key_post.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn screen_key_released_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenKeyReleasedPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("key_rel_pre:{}:{}", event.screen.id, event.key));
            Flow::Continue
        }

        fn screen_key_released_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenKeyReleasedPostEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("key_rel_post:{}:{}", event.screen.id, event.key));
            if self.cancel_key_post.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn screen_char_typed_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenCharTypedPreEvent<'_>,
        ) -> Flow {
            self.log
                .lock()
                .unwrap()
                .push(format!("char_pre:{}:{}", event.screen.id, event.c));
            if event.c == '~'
                && let Some(field) = event.field.as_mut()
            {
                field.value = "switch".into();
            }
            Flow::Continue
        }

        fn screen_char_typed_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenCharTypedPostEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("char_post:{}:{}", event.screen.id, event.c));
        }

        fn screen_preedit_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenPreeditPreEvent<'_>,
        ) -> Flow {
            self.log.lock().unwrap().push(format!(
                "preedit_pre:{}:{}",
                event.screen.id, event.full_text
            ));
            Flow::Continue
        }

        fn screen_preedit_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenPreeditPostEvent<'_>,
        ) {
            self.log.lock().unwrap().push(format!(
                "preedit_post:{}:{}",
                event.screen.id, event.full_text
            ));
        }

        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            if event.current.id == "dialog" && event.new.id == "dialog" {
                self.log
                    .lock()
                    .unwrap()
                    .push("opening:dialog->dialog".into());
                if self.veto_dialog.load(Ordering::SeqCst) {
                    return Some(ScreenOpening::Cancel);
                }
            }
            None
        }

        fn screen_closing(&self, _host: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
            if event.screen.id == "dialog" {
                self.log.lock().unwrap().push("closing:dialog".into());
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let veto_dialog = Arc::new(AtomicBool::new(false));
    let override_press_handled = Arc::new(Mutex::new(None));
    let override_release_handled = Arc::new(Mutex::new(None));
    let cancel_key_post = Arc::new(AtomicBool::new(false));
    let handle_drag = Arc::new(AtomicBool::new(true));
    let handle_scroll = Arc::new(AtomicBool::new(true));
    let transition_on_drag = Arc::new(AtomicBool::new(false));
    let mods: Arc<[ModEntry]> = vec![ModEntry::new(
        "target_mod",
        Box::new(InputMod {
            log: log.clone(),
            dialog_mount: AtomicUsize::new(0),
            veto_dialog: veto_dialog.clone(),
            override_press_handled: override_press_handled.clone(),
            override_release_handled: override_release_handled.clone(),
            cancel_key_post: cancel_key_post.clone(),
            handle_drag: handle_drag.clone(),
            handle_scroll: handle_scroll.clone(),
            transition_on_drag: transition_on_drag.clone(),
        }),
    )]
    .into();

    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);

            // 1) Open Paused and click the button that transitions paused -> mod ("target_mod"):
            //    MouseButtonPressed.Post MUST still carry screen A's id ("paused"), not "mod".
            game.set_screen(Screen::Paused);
            game.observe_screen();
            take_log(&log);
            move_mouse(game, win_id, 40.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("target_mod"));
            assert_eq!(
                take_log(&log),
                vec!["press_pre:paused:0:false", "press_post:paused:0:false:true"]
            );
            // Release now happens on "mod".
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                take_log(&log),
                vec!["release_pre:mod:0", "release_post:mod:0:false"]
            );

            // 2) Force an unhandled result on a handled click suppresses last_click recording,
            //    so an immediate second click does NOT see double_click = true.
            *override_press_handled.lock().unwrap() = Some(false);
            move_mouse(game, win_id, 40.0, 70.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(game.client.screen_state.0.last_click.is_none());
            take_log(&log);
            // Next click immediately after sees double_click = false.
            *override_press_handled.lock().unwrap() = None;
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            let entries = take_log(&log);
            assert_eq!(
                entries,
                vec![
                    "press_pre:mod:0:false",
                    "press_post:mod:0:false:true",
                    "release_pre:mod:0",
                    "release_post:mod:0:true",
                ]
            );
            assert!(game.client.screen_state.0.last_click.is_some());

            // 3) Force a handled result on an unhandled click records last_click, so an
            //    immediate second click receives double_click = true on both Pre and Post.
            game.client.screen_state.0.last_click = None;
            *override_press_handled.lock().unwrap() = Some(true);
            move_mouse(game, win_id, 400.0, 400.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(game.client.screen_state.0.last_click.is_some());
            take_log(&log);
            *override_press_handled.lock().unwrap() = None;
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                take_log(&log),
                vec![
                    "press_pre:mod:0:true",
                    "press_post:mod:0:true:false",
                    "release_pre:mod:0",
                    "release_post:mod:0:false",
                ]
            );

            // 4) A focused button consumes a left drag and release, even
            // outside its bounds. Drag.Post is absent; Release.Post says handled.
            move_mouse(game, win_id, 40.0, 110.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, 115.0, 110.0);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                take_log(&log),
                [
                    "drag_pre:mod",
                    "release_pre:mod:0",
                    "release_post:mod:0:true"
                ]
            );

            // Drag & Scroll Post fire ONLY when unhandled.
            // Handled drag/scroll -> Pre fires, Post does NOT fire.
            handle_drag.store(true, Ordering::SeqCst);
            handle_scroll.store(true, Ordering::SeqCst);
            move_mouse(game, win_id, 40.0, 70.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, 55.0, 70.0);
            send_scroll(game, win_id, 0.0, 1.0);
            assert_eq!(take_log(&log), vec!["drag_pre:mod", "scroll_pre:mod"]);
            send_mouse_button(game, win_id, MouseButton::Left, false);

            //    Unhandled drag/scroll -> Pre AND Post both fire on "mod".
            handle_drag.store(false, Ordering::SeqCst);
            handle_scroll.store(false, Ordering::SeqCst);
            move_mouse(game, win_id, 40.0, 70.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, 50.0, 70.0);
            send_scroll(game, win_id, 0.0, -1.0);
            assert_eq!(
                take_log(&log),
                vec![
                    "drag_pre:mod",
                    "drag_post:mod",
                    "scroll_pre:mod",
                    "scroll_post:mod"
                ]
            );
            send_mouse_button(game, win_id, MouseButton::Left, false);

            // 5) CharacterTyped.Post and Preedit.Post fire when unhandled on "mod".
            take_log(&log);
            game.screen_chars_typed(Some("z"));
            game.ime_preedit("hi", Some((0, 2)));
            assert_eq!(
                take_log(&log),
                vec![
                    "char_pre:mod:z",
                    "char_post:mod:z",
                    "preedit_pre:mod:hi",
                    "preedit_post:mod:hi",
                ]
            );

            // 6) KeyPressed.Post & KeyReleased.Post: returning Flow::Cancel blocks mapped_key_event_from.
            //    Pressing W on Screen::Mod fires KeyPressed.Pre and KeyPressed.Post on "mod" and
            //    never holds a movement key while a screen is open (matching KeyboardHandler.keyPress).
            game.client.keys.clear();
            cancel_key_post.store(true, Ordering::SeqCst);
            take_log(&log);
            send_key_control(game, "key w down");
            assert!(!game.client.keys.raw(&KeyCode::KeyW));
            assert_eq!(
                take_log(&log),
                vec![
                    "key_press_pre:mod:key.keyboard.w",
                    "key_press_post:mod:key.keyboard.w",
                ]
            );
            cancel_key_post.store(false, Ordering::SeqCst);
            send_key_control(game, "key w down");
            assert!(!game.client.keys.raw(&KeyCode::KeyW));
            assert_eq!(
                take_log(&log),
                vec![
                    "key_press_pre:mod:key.keyboard.w",
                    "key_press_post:mod:key.keyboard.w",
                ]
            );
            //    Releasing through the control socket (`key w up`) runs KeyReleased.Pre and
            //    KeyReleased.Post on the receiving screen; cancelling Post blocks mapped_key_event_from
            //    from releasing an already-held key, while continuing lets the release through.
            game.press_device_key(KeyCode::KeyW, sunrise_client::held::Source::Agent);
            assert!(game.client.keys.raw(&KeyCode::KeyW));
            take_log(&log);
            cancel_key_post.store(true, Ordering::SeqCst);
            send_key_control(game, "key w up");
            assert!(game.client.keys.raw(&KeyCode::KeyW));
            assert_eq!(
                take_log(&log),
                vec![
                    "key_rel_pre:mod:key.keyboard.w",
                    "key_rel_post:mod:key.keyboard.w",
                ]
            );
            cancel_key_post.store(false, Ordering::SeqCst);
            send_key_control(game, "key w up");
            assert!(!game.client.keys.raw(&KeyCode::KeyW));
            assert_eq!(
                take_log(&log),
                vec![
                    "key_rel_pre:mod:key.keyboard.w",
                    "key_rel_post:mod:key.keyboard.w",
                ]
            );
            //    `key w tap` runs both press and release phases through Pre and Post.
            take_log(&log);
            send_key_control(game, "key w tap");
            assert!(!game.client.keys.raw(&KeyCode::KeyW));
            assert_eq!(
                take_log(&log),
                vec![
                    "key_press_pre:mod:key.keyboard.w",
                    "key_press_post:mod:key.keyboard.w",
                    "key_rel_pre:mod:key.keyboard.w",
                    "key_rel_post:mod:key.keyboard.w",
                ]
            );

            // 7) Closing screen from button press still fires MouseButtonPressed.Post with "mod"
            //    even though the screen is now Playing.
            game.client.screen_state.0.last_click = None;
            move_mouse(game, win_id, 40.0, 30.0);
            take_log(&log);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(
                take_log(&log),
                vec!["press_pre:mod:0:false", "press_post:mod:0:false:true"]
            );
            send_mouse_button(game, win_id, MouseButton::Left, false);

            // Advancements consumes its binding itself, so Post never sees it and cannot veto it.
            game.set_screen(Screen::Advancements);
            take_log(&log);
            cancel_key_post.store(true, Ordering::SeqCst);
            send_key_control(game, "key l down");
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(
                take_log(&log),
                ["key_press_pre:advancements:key.keyboard.l"]
            );

            game.set_screen(Screen::Advancements);
            game.client
                .settings
                .keys
                .set("key.advancements", "key.keyboard.u");
            take_log(&log);
            send_key_control(game, "key l down");
            assert_eq!(game.client.screen, Screen::Advancements);
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:advancements:key.keyboard.l",
                    "key_press_post:advancements:key.keyboard.l"
                ]
            );
            send_key_control(game, "key u down");
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(
                take_log(&log),
                ["key_press_pre:advancements:key.keyboard.u"]
            );
            game.client
                .settings
                .keys
                .set("key.advancements", "key.keyboard.l");
            cancel_key_post.store(false, Ordering::SeqCst);

            // 8) Advancements blank-space press/release and no-tab scroll report unhandled.
            game.set_screen(Screen::Advancements);
            take_log(&log);
            move_mouse(game, win_id, 5.0, 5.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            send_scroll(game, win_id, 0.0, 1.0);
            assert_eq!(
                take_log(&log),
                vec![
                    "press_pre:advancements:0:false",
                    "press_post:advancements:0:false:false",
                    "release_pre:advancements:0",
                    "release_post:advancements:0:false",
                    "scroll_pre:advancements",
                    "scroll_post:advancements",
                ]
            );
            // Main Options sends Post for an unhandled key and an unhandled character.
            game.open_options(false);
            take_log(&log);
            send_key_control(game, "key z down");
            game.dispatch_window_keyboard(&NoLoop, None, true, false, None, Some("z"))
                .unwrap();
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:options:key.keyboard.z",
                    "key_press_post:options:key.keyboard.z",
                    "char_pre:options:z",
                    "char_post:options:z",
                ]
            );
            move_mouse(game, win_id, 3.0, 3.0);
            send_scroll(game, win_id, 0.0, 1.0);
            assert_eq!(
                take_log(&log),
                ["scroll_pre:options", "scroll_post:options"]
            );

            // There is a real held button, but Options' blank background did not grab a child.
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, 5.0, 6.0);
            assert_eq!(take_log(&log), ["drag_pre:options", "drag_post:options"]);
            send_mouse_button(game, win_id, MouseButton::Left, false);

            // The socket sentinel reports a declined edit, not successful typing. Enter
            // is still a key (Post reports it), never a click on the selected custom child.
            game.set_screen(Screen::Stats);
            take_log(&log);
            assert!(game.type_into_screen("\\b").is_err());
            send_key_control(game, "key enter down");
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:stats:key.keyboard.enter",
                    "key_press_post:stats:key.keyboard.enter"
                ]
            );

            // A custom child needn't capture keyboard shortcuts to receive independent text.
            assert!(!game.mounted_widget_captures_input());
            game.dispatch_window_keyboard(&NoLoop, None, true, false, None, Some("φ"))
                .unwrap();
            assert_eq!(take_log(&log), ["char_pre:stats:φ", "editor_char:φ"]);

            // Two mounted fields at identical bounds still have distinct focus identities.
            game.set_screen(Screen::Chat);
            game.open_screen_overlay("link_prompt", |game| {
                game.client.link_prompt = Some("https://example.com".into())
            });
            assert_eq!(game.client.screen_state.0.widgets.focused_index(), Some(0));
            game.ime_preedit("old", Some((0, 3)));
            assert!(sunrise_client::preedit::composing());
            send_key_control(game, "key tab down");
            assert_eq!(game.client.screen_state.0.widgets.focused_index(), Some(1));
            game.ime_preedit("new", Some((0, 3)));
            game.sync_ime(); // the redraw must not clear B's newly accepted preedit
            assert!(sunrise_client::preedit::composing());
            game.ime_commit("new");
            assert_eq!(game.mounted_focused_text_edit(None).unwrap().value, "new");

            // A non-cancelling rewrite can replace the receiver via on_change. Post is owed to the prompt.
            take_log(&log);
            game.screen_chars_typed(Some("~"));
            assert_eq!(game.mod_screen(), Some("target_mod"));
            assert_eq!(
                take_log(&log),
                ["char_pre:link_prompt:~", "char_post:link_prompt:~"]
            );

            // A drag that replaces its screen but declines the event still posts on the old receiver.
            transition_on_drag.store(true, Ordering::SeqCst);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            move_mouse(game, win_id, 40.0, 70.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, 50.0, 70.0);
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(take_log(&log), ["drag_pre:mod", "drag_post:mod"]);
            send_mouse_button(game, win_id, MouseButton::Left, false);

            // Closing a confirmation exposes the Chat child before any intervening frame.
            game.set_screen(Screen::Chat);
            game.open_screen_overlay("link_prompt", |game| {
                game.client.link_prompt = Some("https://example.com".into())
            });
            let prompt = game.client.screen_state.0.widgets.focused_id();
            game.answer_link_prompt(false);
            assert_ne!(game.client.screen_state.0.widgets.focused_id(), prompt);
            take_log(&log);
            game.screen_chars_typed(Some("c"));
            assert_eq!(game.mounted_focused_text_edit(None).unwrap().value, "c");
            assert_eq!(take_log(&log), ["char_pre:chat:c"]);

            // A held dialog slider consumes drag on the overlay, not on Options under it.
            use sunrise_client::dialog_screen::{Layout, Part};
            let dialog = sunrise_command::snbt::parse_fully(
                r#"{type:"minecraft:notice",title:"Wait",after_action:"wait_for_response",inputs:[{type:"minecraft:number_range",key:"amount",label:"Amount",start:0.0f,end:10.0f,initial:0.0f}]}"#,
            )
            .unwrap();
            game.open_options(false);
            game.show_dialog(&dialog);
            let layout = Layout::new(
                &game.decode_dialog(&dialog, None).unwrap(),
                &game.client.gui.atlas.font,
                game.gui_size(),
            );
            let slider = layout
                .placed(0.0)
                .find_map(|(part, _)| match part {
                    Part::Slider { rect, .. } => Some(rect),
                    _ => None,
                })
                .unwrap();
            let [x, y, width, _] = slider;
            move_mouse(game, win_id, f64::from(x + 4.0), f64::from(y + 6.0));
            send_mouse_button(game, win_id, MouseButton::Left, true);
            take_log(&log);
            move_mouse(game, win_id, f64::from(x + width - 4.0), f64::from(y + 6.0));
            assert_eq!(take_log(&log), ["drag_pre:dialog"]);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            let release = take_log(&log);
            assert_eq!(release.len(), 2, "{release:?}");
            assert_eq!(release[0], "release_pre:dialog:0");
            assert!(
                release[1].starts_with("release_post:dialog:0:"),
                "{release:?}"
            );

            game.sync_mounted_focus(game.mounted_widget_targets()[0]);
            let original = game.client.screen_state.0.widgets.focused_id().unwrap();
            let instance = game.client.screen_state.0.screen_instance;
            let [x, y, _, _] = layout
                .footer
                .iter()
                .find_map(|part| match part {
                    Part::Button { rect, .. } => Some(*rect),
                    _ => None,
                })
                .unwrap();
            // Call the receiving dialog's action without a pointer click moving focus first.
            game.client.cursor = (x + 4.0, y + 6.0);
            veto_dialog.store(true, Ordering::SeqCst);
            assert!(game.dialog_click().unwrap());
            assert!(game.dialog_open());
            assert!(game.dialog_status()["screen"].is_null());
            assert_eq!(game.client.screen_state.0.screen_instance, instance);
            assert_eq!(
                game.client.screen_state.0.widgets.focused_id(),
                Some(original)
            );
            assert_eq!(take_log(&log), ["opening:dialog->dialog"]);

            veto_dialog.store(false, Ordering::SeqCst);
            assert!(game.dialog_click().unwrap());
            assert_eq!(game.dialog_status()["screen"], "waiting_for_response");
            assert_eq!(
                game.client.screen_state.0.screen_instance,
                instance.wrapping_add(1)
            );
            assert_eq!(take_log(&log), ["opening:dialog->dialog", "closing:dialog"]);
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::Enter), true, false, None, None)
                .unwrap();
            assert_eq!(
                take_log(&log),
                ["key_press_pre:dialog:key.keyboard.enter", "dialog_press:1"]
            );

            // Warning Back also proposes a new dialog receiver. Keep the warning's
            // focused widget if that proposal is vetoed.
            game.show_dialog(&dialog);
            let [x, y, _, _] = game.dialog_warning_icon().unwrap();
            game.client.cursor = (x + 1.0, y + 1.0);
            assert!(game.dialog_click().unwrap());
            assert_eq!(game.dialog_status()["screen"], "warning");
            let warning = game.client.screen_state.0.widgets.focused_id().unwrap();
            let instance = game.client.screen_state.0.screen_instance;
            take_log(&log);
            veto_dialog.store(true, Ordering::SeqCst);
            assert!(game.dialog_key(KeyCode::Escape, None).unwrap());
            assert_eq!(game.dialog_status()["screen"], "warning");
            assert_eq!(game.client.screen_state.0.screen_instance, instance);
            assert_eq!(
                game.client.screen_state.0.widgets.focused_id(),
                Some(warning)
            );
            assert_eq!(take_log(&log), ["opening:dialog->dialog"]);
            veto_dialog.store(false, Ordering::SeqCst);
            assert!(game.dialog_key(KeyCode::Escape, None).unwrap());
            assert!(game.dialog_status()["screen"].is_null());
            assert_eq!(
                game.client.screen_state.0.screen_instance,
                instance.wrapping_add(1)
            );
            assert_eq!(take_log(&log), ["opening:dialog->dialog", "closing:dialog"]);
            game.close_dialogs();
            game.set_screen(Screen::Playing);

            // Only a real Yes/No hit consumes the press while the pack prompt covers gameplay.
            use sunrise_client::confirm_screen::Confirm;
            use sunrise_game_data::text::Text;
            use sunrise_protocol::packets::configuration::ResourcePackPush;
            game.handle_pack_push(ResourcePackPush {
                id: 1,
                url: "https://example.invalid/pack.zip".into(),
                hash: String::new(),
                required: false,
                prompt: None,
            });
            move_mouse(game, win_id, 4.0, 4.0);
            take_log(&log);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            assert_eq!(
                take_log(&log),
                [
                    "press_pre:server_pack_prompt:0:false",
                    "press_post:server_pack_prompt:0:false:false"
                ]
            );
            assert!(game.client.server_packs.0.prompt.is_some());
            send_mouse_button(game, win_id, MouseButton::Left, false);
            let (_, (no_x, no_y)) = {
                let layout = Confirm::new(
                    &game.client.gui.atlas.font,
                    game.gui_size(),
                    &Text::translate("multiplayer.texturePrompt.line1", &[]),
                    &Text::translate("multiplayer.texturePrompt.line2", &[]),
                );
                (layout.buttons[0], layout.buttons[1])
            };
            move_mouse(game, win_id, f64::from(no_x + 4.0), f64::from(no_y + 4.0));
            take_log(&log);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            assert!(game.client.server_packs.0.prompt.is_none());
            assert!(game.client.captured);
            assert!(
                !game.mapped_key_down("key.attack"),
                "the prompt's press must not reach the world"
            );
            assert_eq!(
                take_log(&log),
                [
                    "press_pre:server_pack_prompt:0:false",
                    "press_post:server_pack_prompt:0:false:true"
                ]
            );
            send_mouse_button(game, win_id, MouseButton::Left, false);

            // Rebound close skips Post; a rebound slot swap runs even when Post cancels.
            let saved_player = game.player.take();
            let _pipe = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            game.player = saved_player;
            game.client
                .settings
                .keys
                .set("key.inventory", "key.keyboard.j");
            game.client
                .settings
                .keys
                .set("key.hotbar.1", "key.keyboard.k");
            game.open_player_inventory();
            assert_eq!(game.client.screen, Screen::Inventory);
            game.inventory[9] = Some(sunrise_items::items::ItemStack::new("minecraft:stick", 2));
            let batch = game.gui_batch().unwrap();
            let layout = game.built_container(&batch).unwrap();
            let &(_, [x, y, _, _]) = layout.slots.iter().find(|(slot, _)| *slot == 9).unwrap();
            move_mouse(game, win_id, f64::from(x + 5.0), f64::from(y + 5.0));
            cancel_key_post.store(true, Ordering::SeqCst);
            take_log(&log);
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::KeyK), true, false, None, None)
                .unwrap();
            assert!(game.inventory[9].is_none());
            assert_eq!(
                game.inventory[0].as_ref().unwrap().item.as_ref(),
                "minecraft:stick"
            );
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:inventory:key.keyboard.k",
                    "key_press_post:inventory:key.keyboard.k"
                ]
            );
            // The mod browser's search editor declines letter keys. Only the
            // rebound inventory key is handled by the container itself.
            game.panel_items(
                "target_mod",
                vec![mod_api::Stack {
                    item: mod_api::__private::item_key("minecraft:stick").unwrap(),
                    count: 1,
                    components: String::new(),
                }],
            );
            let mut batch = game.gui_batch().unwrap();
            game.draw_mod_browser(&mut batch);
            let search = game.client.panel.search.as_ref().unwrap();
            game.client.cursor = (search.x + 2.0, search.y + 2.0);
            assert!(game.mod_browser_press(false));
            assert!(game.mod_browser_typing());
            cancel_key_post.store(false, Ordering::SeqCst);
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::KeyZ), true, false, None, None)
                .unwrap();
            assert_eq!(game.client.screen, Screen::Inventory);
            assert!(game.mod_browser_typing());
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:inventory:key.keyboard.z",
                    "key_press_post:inventory:key.keyboard.z",
                ]
            );
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::KeyJ), true, false, None, None)
                .unwrap();
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(take_log(&log), ["key_press_pre:inventory:key.keyboard.j"]);

            // K drops one stick but must not trigger sort via its translated Q.
            // Q sorts the remainder into the earlier empty slot without dropping it.
            use sunrise_items::items::ItemStack;
            use sunrise_level::{chunk::Chunk, containers::BlockEntity};
            let pos = (0, 0, 0);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            game.world.set(pos.0, pos.1, pos.2, chest).unwrap();
            let mut stacks = vec![None; 27];
            stacks[1] = Some(ItemStack::new("minecraft:stick", 2));
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(stacks));
            game.client
                .remote
                .as_mut()
                .unwrap()
                .menu
                .set(Some((1, "minecraft:generic_9x3".to_owned())));
            game.container = Some(pos);
            game.client
                .settings
                .keys
                .set("key.hotbar.1", "key.keyboard.1");
            game.client.settings.keys.set("key.drop", "key.keyboard.k");
            game.client.settings.extras.sort = true;
            game.client.settings.extras.sort_key = "key.keyboard.q".into();
            game.open_player_inventory();
            let batch = game.gui_batch().unwrap();
            let layout = game.built_container(&batch).unwrap();
            let &(_, [x, y, _, _]) = layout.slots.iter().find(|(slot, _)| *slot == 301).unwrap();
            move_mouse(game, win_id, f64::from(x + 5.0), f64::from(y + 5.0));
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::KeyK), true, false, None, None)
                .unwrap();
            assert!(game.slot_view(300).is_none());
            assert_eq!(game.slot_view(301).unwrap().count, 1);
            assert_eq!(
                take_log(&log),
                [
                    "key_press_pre:inventory:key.keyboard.k",
                    "key_press_post:inventory:key.keyboard.k"
                ]
            );
            game.dispatch_window_keyboard(&NoLoop, Some(KeyCode::KeyQ), true, false, None, None)
                .unwrap();
            assert_eq!(game.slot_view(300).unwrap().count, 1);
            assert!(game.slot_view(301).is_none());
            assert_eq!(take_log(&log), ["key_press_pre:inventory:key.keyboard.q"]);
        });
    });
}

/// TextField local state, IME & double-click:
/// - Typing, Backspace, Delete, Left/Right/Home/End, Ctrl+A, and Enter update local `EditBox`
///   and fire `on_change` / `on_submit` with no `DrawContext` round-trip.
/// - Focused text field reports `text_field_focused() == true`; `ime_preedit` is taken with no
///   `screen_preedit_post`; `ime_commit` inserts text and fires `on_change`.
/// - Unfocusing (or focusing a non-capturing custom widget) flips `text_field_focused() == false`
///   and routes `ime_preedit` to `screen_preedit_post`.
/// - Double-clicking inside the text field within 250ms selects the word under the cursor; a
///   click outside the window does not set `double_click`.
#[test]
#[ignore = "daily"]
fn text_field_editing_ime_and_double_click_word_selection() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::text_field_editing_ime_and_double_click_word_selection",
        "SUNRISE_WIDGETS_TEXTFIELD_CHILD",
        &["window"],
    ) {
        return;
    }

    struct NonCapturingCustom;
    impl ScreenWidget for NonCapturingCustom {
        fn bounds(&self) -> [f32; 4] {
            [20.0, 60.0, 80.0, 20.0]
        }
        fn mouse_clicked(
            &mut self,
            _host: &mut dyn ClientHost,
            _mouse: [f32; 2],
            _button: i32,
            _modifiers: u32,
            _double_click: bool,
        ) -> bool {
            true
        }
    }

    struct TextFieldMod {
        changes: Arc<Mutex<Vec<String>>>,
        submits: Arc<Mutex<Vec<String>>>,
        preedit_posts: Arc<Mutex<Vec<String>>>,
        pressed_double_clicks: Arc<Mutex<Vec<bool>>>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for TextFieldMod {
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if host.mod_screen() != Some("tf_mod") {
                return;
            }
            let changes = self.changes.clone();
            let submits = self.submits.clone();
            let tf_id = event.widgets.add_text_field(
                TextField::new([20.0, 20.0, 200.0, 20.0], 64)
                    .hint(ApiText::literal("Hint"))
                    .on_change(move |text, _host| {
                        changes.lock().unwrap().push(text.to_owned());
                    })
                    .on_submit(move |text, _host| {
                        submits.lock().unwrap().push(text.to_owned());
                    }),
            );
            event.widgets.add_widget(Box::new(NonCapturingCustom));
            assert!(event.widgets.set_initial_focus(tf_id));
        }

        fn screen_preedit_post(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenPreeditPostEvent<'_>,
        ) {
            self.preedit_posts
                .lock()
                .unwrap()
                .push(format!("{}:{}", event.screen.id, event.full_text));
        }

        fn screen_mouse_button_pressed_pre(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenMouseButtonPressedPreEvent<'_>,
        ) -> Flow {
            self.pressed_double_clicks
                .lock()
                .unwrap()
                .push(event.double_click);
            Flow::Continue
        }
    }

    let changes = Arc::new(Mutex::new(Vec::new()));
    let submits = Arc::new(Mutex::new(Vec::new()));
    let preedit_posts = Arc::new(Mutex::new(Vec::new()));
    let pressed_double_clicks = Arc::new(Mutex::new(Vec::new()));
    let mods: Arc<[ModEntry]> = vec![ModEntry::new(
        "tf_mod",
        Box::new(TextFieldMod {
            changes: changes.clone(),
            submits: submits.clone(),
            preedit_posts: preedit_posts.clone(),
            pressed_double_clicks: pressed_double_clicks.clone(),
        }),
    )]
    .into();

    with_mods(mods, || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            assert!(game.mod_screen_open("tf_mod"));

            // Typing directly into the mounted TextField without any render / DrawContext round-trip.
            for ch in "hello world".chars() {
                assert!(game.mounted_widget_char_typed(ch, 0));
            }
            assert_eq!(
                changes.lock().unwrap().last().map(String::as_str),
                Some("hello world")
            );
            changes.lock().unwrap().clear();

            // Backspace deletes 'd', Home moves to start, Delete removes 'h', Right moves past 'e',
            // typing '!' inserts in place, End moves to end.
            assert!(game.mounted_widget_key_pressed(KeyCode::Backspace, false, 0));
            assert!(game.mounted_widget_key_pressed(KeyCode::Home, false, 0));
            assert!(game.mounted_widget_key_pressed(KeyCode::Delete, false, 0));
            assert!(game.mounted_widget_key_pressed(KeyCode::ArrowRight, false, 0));
            assert!(game.mounted_widget_char_typed('!', 0));
            assert!(game.mounted_widget_key_pressed(KeyCode::End, false, 0));
            assert!(game.mounted_widget_char_typed('?', 0));
            assert_eq!(
                take_log(&changes),
                vec!["hello worl", "ello worl", "e!llo worl", "e!llo worl?"]
            );

            // Ctrl+A selects all, typing replaces the whole selection, Enter fires on_submit.
            let select_mod = if cfg!(target_os = "macos") { 8 } else { 2 };
            assert!(game.mounted_widget_key_pressed(KeyCode::KeyA, false, select_mod));
            for ch in "alpha beta".chars() {
                assert!(game.mounted_widget_char_typed(ch, 0));
            }
            assert!(game.mounted_widget_key_pressed(KeyCode::Enter, false, 0));
            assert_eq!(take_log(&submits), vec!["alpha beta"]);
            changes.lock().unwrap().clear();

            // Focused text field captures IME preedit and commit.
            assert!(game.text_field_focused());
            game.ime_preedit("ka", Some((0, 2)));
            assert!(take_log(&preedit_posts).is_empty());
            game.ime_commit("!");
            assert_eq!(take_log(&changes), vec!["alpha beta!"]);

            // Double-click inside the text field selects the first word ("alpha").
            pressed_double_clicks.lock().unwrap().clear();
            move_mouse(game, win_id, 30.0, 30.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                std::mem::take(&mut *pressed_double_clicks.lock().unwrap()),
                vec![false, true]
            );
            // Vanilla EditBox.selectWord uses getWordPosition(1, pos, stripSpaces=true), which
            // selects "alpha " including the trailing space; replacing it yields "Xbeta!".
            assert!(game.mounted_widget_char_typed('X', 0));
            assert_eq!(take_log(&changes), vec!["Xbeta!"]);

            // Clicking the non-capturing custom widget shifts focus off the text field.
            move_mouse(game, win_id, 40.0, 70.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(!game.text_field_focused());
            game.ime_preedit("ni", Some((0, 2)));
            assert_eq!(take_log(&preedit_posts), vec!["mod:ni"]);

            // A click that hits no child leaves the current focus unchanged.
            let focused = game.client.screen_state.0.widgets.focused_id().unwrap();
            move_mouse(game, win_id, 400.0, 400.0);
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert_eq!(
                game.client.screen_state.0.widgets.focused_id(),
                Some(focused)
            );
            assert!(!game.text_field_focused());
            game.ime_preedit("go", Some((0, 2)));
            assert_eq!(take_log(&preedit_posts), vec!["mod:go"]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn title_init_post_lists_and_removes_the_title_screens_own_widgets() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::title_init_post_lists_and_removes_the_title_screens_own_widgets",
        "SUNRISE_WIDGETS_TITLE_CHILDREN",
        &["window"],
    ) {
        return;
    }
    type Children = Vec<(Option<ApiText>, [f32; 4])>;
    struct RemoveOptionsAndNotice {
        remove: Arc<AtomicBool>,
        seen: Arc<Mutex<Children>>,
        pressed: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for RemoveOptionsAndNotice {
        fn screen_init_post(&self, _: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            if event.screen.id != "title" {
                return;
            }
            let widgets = &mut *event.widgets;
            *self.seen.lock().unwrap() = widgets
                .children()
                .iter()
                .map(|child| (child.label.clone(), child.rect))
                .collect();
            if self.remove.load(Ordering::SeqCst) {
                let options = ApiText::translate("menu.options", vec![]);
                let ids: Vec<_> = widgets
                    .children()
                    .iter()
                    .filter(|child| {
                        child.label.as_ref() == Some(&options)
                            || child.label
                                == Some(ApiText::translate("sunrise.title.not_affiliated", vec![]))
                    })
                    .map(|child| child.id)
                    .collect();
                assert_eq!(ids.len(), 2);
                let rect = widgets
                    .children()
                    .iter()
                    .find(|c| c.id == ids[0])
                    .unwrap()
                    .rect;
                for id in ids {
                    assert!(widgets.remove(id));
                }
                // A replacement where Options... was.
                let pressed = self.pressed.clone();
                widgets.add_button(Button::new(
                    rect,
                    ApiText::literal("Replacement"),
                    move |_, _| pressed.store(true, Ordering::SeqCst),
                ));
            }
        }
    }

    let remove = Arc::new(AtomicBool::new(true));
    let seen = Arc::new(Mutex::new(Vec::new()));
    let pressed = Arc::new(AtomicBool::new(false));
    let mods = vec![ModEntry::new(
        "title_children",
        Box::new(RemoveOptionsAndNotice {
            remove: remove.clone(),
            seen: seen.clone(),
            pressed: pressed.clone(),
        }),
    )];
    with_mods(mods.into(), || {
        with_client_game(|game| {
            let (_event_loop, win_id) = attach_offscreen_window(game);
            let saves = std::env::temp_dir().join("sunrise-title-children-test-no-saves");
            game.client.title.0 = Some(game.new_title(saves));
            game.observe_screen();
            let children = seen.lock().unwrap().clone();
            let key = |label: &Option<ApiText>| label.clone().unwrap();
            let vanilla: Vec<_> = [
                "menu.singleplayer",
                "menu.multiplayer",
                "menu.online",
                "gui.friends.open",
                "options.language",
                "options.accessibility",
                "menu.options",
                "menu.quit",
                "sunrise.title.not_affiliated",
            ]
            .map(|key| ApiText::translate(key, vec![]))
            .into();
            // The shell adds the Mods button after Realms.
            let listed: Vec<_> = children
                .iter()
                .map(|(label, _)| key(label))
                .filter(|label| vanilla.contains(label))
                .collect();
            assert_eq!(listed, vanilla);
            assert_eq!(
                children[..3]
                    .iter()
                    .map(|(l, _)| key(l))
                    .collect::<Vec<_>>(),
                vanilla[..3]
            );
            assert_eq!(
                children[children.len() - 6..]
                    .iter()
                    .map(|(l, _)| key(l))
                    .collect::<Vec<_>>(),
                vanilla[3..]
            );
            // TitleScreen.init's bounds on a 640x480 screen at GUI scale 1.
            let rect = |key: &str| {
                let key = ApiText::translate(key, vec![]);
                children
                    .iter()
                    .find(|(label, _)| label.as_ref() == Some(&key))
                    .unwrap()
                    .1
            };
            let top = rect("menu.singleplayer")[1];
            assert_eq!(rect("menu.singleplayer"), [220.0, top, 200.0, 20.0]);
            assert_eq!(rect("menu.options")[2..], [98.0, 20.0]);
            assert_eq!(rect("menu.quit")[0], 322.0);
            assert_eq!(rect("menu.quit")[1], rect("menu.options")[1]);
            let notice = rect("sunrise.title.not_affiliated");
            assert_eq!(
                (notice[0] + notice[2], notice[1], notice[3]),
                (638.0, 470.0, 10.0)
            );

            let press = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, key: &str| {
                let [x, y, w, h] = rect(key);
                let title = game.client.title.0.as_mut().unwrap();
                title.click(x + w / 2.0, y + h / 2.0, (640.0, 480.0)).0
            };
            use sunrise_client::title::Action;
            assert_eq!(press(game, "menu.options"), Action::None);
            assert_eq!(press(game, "menu.quit"), Action::Quit);
            let drawn = |game: &sunrise_client_gameplay::game::ClientGame<'_>, serial| {
                let mut batch = sunrise_client::hud::HudBatch::new(
                    [640, 480],
                    game.client.settings.gui_scale_inputs(),
                );
                assert_eq!((batch.width, batch.height), (640.0, 480.0));
                let title = game.client.title.0.as_ref().unwrap();
                title.draw(
                    &game.client.gui.atlas,
                    &mut batch,
                    (0.0, 0.0),
                    false,
                    &mut |_, _| {},
                );
                sunrise_client::widgets::end_frame(serial);
                sunrise_client::widgets::last_labelled()
                    .into_iter()
                    .map(|widget| widget.label)
                    .collect::<Vec<_>>()
            };
            let label = |key: &str| sunrise_client::lang::get(key);
            let shown = drawn(game, 1);
            assert!(shown.contains(&label("menu.quit")));
            assert!(!shown.contains(&label("menu.options")));
            assert!(!shown.contains(&label("sunrise.title.not_affiliated")));
            assert_eq!(press(game, "sunrise.title.not_affiliated"), Action::None);

            // The next init starts from the full set again.
            remove.store(false, Ordering::SeqCst);
            game.test_commit_screen_transition();
            assert_eq!(*seen.lock().unwrap(), children);
            let shown = drawn(game, 2);
            assert!(shown.contains(&label("menu.options")));
            assert!(shown.contains(&label("sunrise.title.not_affiliated")));
            assert_eq!(press(game, "sunrise.title.not_affiliated"), Action::Credits);
            assert_eq!(press(game, "menu.options"), Action::Options);

            // The next click uses the rebuilt widgets before the next frame draws them.
            remove.store(true, Ordering::SeqCst);
            game.test_commit_screen_transition();
            let [x, y, w, h] = rect("menu.options");
            move_mouse(game, win_id, f64::from(x + w / 2.0), f64::from(y + h / 2.0));
            send_mouse_button(game, win_id, MouseButton::Left, true);
            send_mouse_button(game, win_id, MouseButton::Left, false);
            assert!(
                pressed.load(Ordering::SeqCst),
                "the replacement took the click"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn config_opens_the_mods_own_screen_and_closing_it_returns_to_the_mods_screen() {
    use sunrise_client::options_ui::{Event, Page};
    if run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_widgets_tests::config_opens_the_mods_own_screen_and_closing_it_returns_to_the_mods_screen",
        "SUNRISE_CONFIG_SCREEN_CHILD",
        &["window"],
    ) {
        return;
    }
    struct ConfigScreen {
        open: Arc<AtomicBool>,
        asked: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for ConfigScreen {
        fn register(
            &self,
            registrar: &mut dyn mod_api::Registrar,
            _: &mod_api::RegistrationContext,
        ) {
            let (open, asked) = (self.open.clone(), self.asked.clone());
            registrar
                .register_config_screen(Arc::new(move |_: &dyn ClientHost| {
                    asked.fetch_add(1, Ordering::SeqCst);
                    open.load(Ordering::SeqCst)
                }))
                .unwrap();
            let refused = registrar.register_config_screen(Arc::new(|_: &dyn ClientHost| true));
            assert_eq!(
                refused,
                Err("mod configured registers a second config screen".into())
            );
        }
    }
    struct NoScreen;
    #[mod_api::__subscriptions]
    impl Hooks for NoScreen {}

    let open = Arc::new(AtomicBool::new(true));
    let asked = Arc::new(AtomicUsize::new(0));
    let mods = vec![
        ModEntry::new(
            "configured",
            Box::new(ConfigScreen {
                open: open.clone(),
                asked: asked.clone(),
            }),
        ),
        ModEntry::new("plain", Box::new(NoScreen)),
    ];
    with_mods(mods.into(), || {
        with_client_game(|game| {
            let (_event_loop, _) = attach_offscreen_window(game);
            assert!(mod_api::__private::has_config_screen("configured"));
            assert!(!mod_api::__private::has_config_screen("plain"));
            game.client.settings.mods.installed = ["configured", "plain"]
                .map(|id| sunrise_client::mod_list::Installed {
                    id: id.into(),
                    title: id.into(),
                    description: String::new(),
                    problem: None,
                    dev: false,
                    native: Some(sunrise_client::mod_list::NativePage {
                        own_screen: mod_api::__private::has_config_screen(id),
                        ..Default::default()
                    }),
                })
                .into();
            let config = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                game.handle_option_events(vec![Event::ModConfigScreen(0)])
                    .unwrap();
            };
            let on_mods = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                assert_eq!(game.client.screen, Screen::Options);
                assert_eq!(game.client.chat_ui.options.page(), Page::Mods);
            };

            // In a world: over the pause menu's Options > Mods.
            game.open_options(false);
            game.client.chat_ui.options.show(Page::Mods);
            config(game);
            assert_eq!(asked.load(Ordering::SeqCst), 1);
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("configured"));
            game.close_mod_screen();
            on_mods(game);
            assert_eq!(game.mod_screen(), None);

            // Declined: the Mods screen stays.
            open.store(false, Ordering::SeqCst);
            config(game);
            assert_eq!(asked.load(Ordering::SeqCst), 2);
            on_mods(game);

            // On the title: closing returns to the Mods screen, not the title.
            open.store(true, Ordering::SeqCst);
            let saves = std::env::temp_dir().join("sunrise-config-screen-test-no-saves");
            game.client.title.0 = Some(game.new_title(saves));
            game.open_options(true);
            game.client.chat_ui.options.show(Page::Mods);
            config(game);
            assert_eq!(game.mod_screen(), Some("configured"));
            game.close_mod_screen();
            on_mods(game);
            assert!(game.client.title.0.is_some());

            // A mod screen opened another way still closes to the title.
            assert!(game.mod_screen_open("configured"));
            game.close_mod_screen();
            assert_eq!(game.client.screen, Screen::Playing);
            assert_eq!(game.mod_screen(), None);
        });
    });
}
