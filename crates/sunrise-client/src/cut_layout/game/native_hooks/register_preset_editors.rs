#![cfg(test)]
#[cfg(feature = "client")]
use std::sync::Arc;
#[cfg(feature = "client")]
use sunrise_client::hud::HudBatch;
#[cfg(all(test, feature = "client"))]
pub(crate) mod tests {
    use crate::game::native_hooks::register_preset_editors::*;
    use mod_api::{
        DrawContext, Hooks, Identifier, ModEntry, PresetEditor, PresetEditorDraft,
        RegisterPresetEditorsEvent, ScreenCharTypedPostEvent, ScreenCharTypedPreEvent,
        ScreenKeyPressedPostEvent, ScreenKeyPressedPreEvent, ScreenKeyReleasedPreEvent,
        ScreenMouseButtonPressedPostEvent, ScreenMouseButtonPressedPreEvent,
        ScreenMouseButtonReleasedPostEvent, ScreenOpening, ScreenOpeningEvent,
        ScreenRenderPostEvent, TriState,
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_client::title::Mode;
    use sunrise_client_gameplay::game::WindowKeyInput;
    use sunrise_gameplay_common::game::Screen;
    use sunrise_sim::world_options::WorldOptions;
    use winit::{
        event::{DeviceId, ElementState, MouseButton, WindowEvent},
        keyboard::{KeyCode, PhysicalKey},
    };

    #[derive(Debug, PartialEq, Eq)]
    enum MouseStep {
        Pre(i32, bool),
        Editor(i32, bool),
        // Original screen, GLFW button, double click, handled, editor still open.
        Post(&'static str, i32, bool, bool, bool),
        Release(&'static str, i32, bool),
    }

    #[derive(Default)]
    struct MouseProbe {
        pub events: Vec<MouseStep>,
        cancel_press: bool,
        press_result: TriState,
    }

    struct IslandsEditor {
        pub mouse: Arc<Mutex<MouseProbe>>,
        clicks: Arc<AtomicUsize>,
        keys: Arc<AtomicUsize>,
        typed: Arc<Mutex<String>>,
    }

    impl PresetEditor for IslandsEditor {
        fn title(&self) -> Option<String> {
            Some("Islands Editor".into())
        }

        fn render(
            &mut self,
            draw: &mut dyn DrawContext,
            rect: [f32; 4],
            _draft: &mut PresetEditorDraft<'_>,
        ) {
            draw.fill([rect[0] + 10.0, rect[1] + 24.0, 40.0, 16.0], 0xFF336699);
        }

        fn mouse_clicked(
            &mut self,
            _x: f32,
            _y: f32,
            button: i32,
            double_click: bool,
            draft: &mut PresetEditorDraft<'_>,
        ) -> bool {
            self.mouse
                .lock()
                .unwrap()
                .events
                .push(MouseStep::Editor(button, double_click));
            if button != 0 {
                return button == 1;
            }
            self.clicks.fetch_add(1, Ordering::Relaxed);
            let mut dimensions = draft.dimensions().clone();
            if let Some(sunrise_core::nbt::Tag::Compound(overworld)) =
                dimensions.get_mut(&"minecraft:overworld".into())
            {
                overworld.insert(
                    "type".into(),
                    sunrise_core::nbt::Tag::String("minecraft:the_nether".into()),
                );
            }
            assert!(draft.set_dimensions(dimensions));
            true
        }

        fn key_pressed(
            &mut self,
            key_code: u32,
            _scan_code: u32,
            modifiers: u32,
            _draft: &mut PresetEditorDraft<'_>,
        ) -> bool {
            // Vanilla 26.3 InputConstants uses SDL scancodes, not the old GLFW key IDs.
            const TAB: u32 = 43;
            const LEFT_SHIFT: u32 = 225;
            const RIGHT_SHIFT: u32 = 229;
            if key_code == TAB && modifiers == 1 {
                self.keys.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            matches!(key_code, LEFT_SHIFT | RIGHT_SHIFT)
        }

        fn char_typed(
            &mut self,
            ch: char,
            _modifiers: u32,
            _draft: &mut PresetEditorDraft<'_>,
        ) -> bool {
            if ch == 'x' {
                return false;
            }
            self.typed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(ch);
            true
        }
    }

    struct EditorMod {
        pub mouse: Arc<Mutex<MouseProbe>>,
        clicks: Arc<AtomicUsize>,
        keys: Arc<AtomicUsize>,
        typed: Arc<Mutex<String>>,
        char_post: Arc<Mutex<String>>,
        key_post: Arc<Mutex<Vec<String>>>,
        releases: Arc<Mutex<Vec<(String, String, u32)>>>,
        create_openings: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for EditorMod {
        fn screen_mouse_button_pressed_pre(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenMouseButtonPressedPreEvent<'_>,
        ) -> mod_api::Flow {
            let mut mouse = self.mouse.lock().unwrap();
            mouse
                .events
                .push(MouseStep::Pre(event.button, event.double_click));
            if mouse.cancel_press {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }

        fn screen_mouse_button_pressed_post(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut ScreenMouseButtonPressedPostEvent<'_>,
        ) {
            let mut mouse = self.mouse.lock().unwrap();
            mouse.events.push(MouseStep::Post(
                event.screen.id,
                event.button,
                event.double_click,
                event.was_click_handled,
                host.preset_editor_dimensions().is_some(),
            ));
            *event.result = mouse.press_result;
        }

        fn screen_mouse_button_released_post(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenMouseButtonReleasedPostEvent<'_>,
        ) {
            self.mouse.lock().unwrap().events.push(MouseStep::Release(
                event.screen.id,
                event.button,
                event.was_release_handled,
            ));
        }

        fn screen_char_typed_pre(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenCharTypedPreEvent<'_>,
        ) -> mod_api::Flow {
            if event.screen.id == "world_preset_editor" && event.c == '!' {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
        fn screen_char_typed_post(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenCharTypedPostEvent<'_>,
        ) {
            if event.screen.id == "world_preset_editor" {
                self.char_post.lock().unwrap().push(event.c);
            }
        }
        fn screen_key_pressed_post(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenKeyPressedPostEvent<'_>,
        ) -> mod_api::Flow {
            if event.screen.id == "world_preset_editor" {
                self.key_post.lock().unwrap().push(event.key.to_owned());
            }
            mod_api::Flow::Continue
        }

        fn screen_opening(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            if event.current.id == "world_preset_editor" && event.new.id == "world_create" {
                self.create_openings.fetch_add(1, Ordering::Relaxed);
            }
            None
        }

        fn screen_key_released_pre(
            &self,
            _: &dyn mod_api::ClientHost,
            event: &mut ScreenKeyReleasedPreEvent<'_>,
        ) -> mod_api::Flow {
            self.releases.lock().unwrap().push((
                event.screen.id.to_owned(),
                event.key.to_owned(),
                event.modifiers,
            ));
            mod_api::Flow::Continue
        }

        fn register_preset_editors(
            &self,
            _: &mod_api::RegistrationContext,
            event: &mut RegisterPresetEditorsEvent<'_>,
        ) {
            let mouse = Arc::clone(&self.mouse);
            let clicks = Arc::clone(&self.clicks);
            let keys = Arc::clone(&self.keys);
            let typed = Arc::clone(&self.typed);
            event.register(Identifier::parse("example:islands").unwrap(), move |ctx| {
                assert_eq!(ctx.preset(), &Identifier::parse("example:islands").unwrap());
                assert_eq!(ctx.seed(), "1");
                Ok(Box::new(IslandsEditor {
                    mouse: Arc::clone(&mouse),
                    clicks: Arc::clone(&clicks),
                    keys: Arc::clone(&keys),
                    typed: Arc::clone(&typed),
                }))
            });
        }
        fn screen_render_post(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut ScreenRenderPostEvent<'_>,
        ) {
            if event.screen.id == "world_preset_editor" && host.preset_editor_dimensions().is_some()
            {
                event.draw.fill([20.0, 20.0, 50.0, 20.0], 0xFF00FF00);
            }
        }
        fn screen_key_pressed_pre(
            &self,
            host: &dyn mod_api::ClientHost,
            event: &mut ScreenKeyPressedPreEvent<'_>,
        ) -> mod_api::Flow {
            if event.key != "key.keyboard.space" || event.screen.id != "world_preset_editor" {
                return mod_api::Flow::Continue;
            }
            let mut dimensions = host.preset_editor_dimensions().unwrap();
            let sunrise_core::nbt::Tag::Compound(overworld) =
                dimensions.get_mut(&"minecraft:overworld".into()).unwrap()
            else {
                unreachable!()
            };
            overworld.insert(
                "type".into(),
                sunrise_core::nbt::Tag::String("minecraft:overworld_caves".into()),
            );
            assert!(host.set_preset_editor_dimensions(dimensions));
            mod_api::Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn failed_preset_editor_paint_leaves_stock_frame_and_post() {
        struct PaintFailure;
        impl PresetEditor for PaintFailure {
            fn render(
                &mut self,
                draw: &mut dyn DrawContext,
                _rect: [f32; 4],
                _draft: &mut PresetEditorDraft<'_>,
            ) {
                draw.with_scissor([8.0, 20.0, 32.0, 24.0], &mut |draw| {
                    draw.fill([12.0, 24.0, 10.0, 10.0], 0xFFFF00FF);
                    panic!("preset editor failed after painting");
                })
                .unwrap();
            }
        }

        #[mod_api::__subscriptions]
        impl Hooks for PaintFailure {
            fn register_preset_editors(
                &self,
                _: &mod_api::RegistrationContext,
                event: &mut RegisterPresetEditorsEvent<'_>,
            ) {
                event.register(Identifier::parse("minecraft:flat").unwrap(), |_| {
                    Ok(Box::new(PaintFailure))
                });
            }
        }

        struct PostPainter;
        #[mod_api::__subscriptions]
        impl Hooks for PostPainter {
            fn screen_render_post(
                &self,
                _: &dyn mod_api::ClientHost,
                event: &mut ScreenRenderPostEvent<'_>,
            ) {
                if event.screen.id == "world_preset_editor" {
                    event.draw.fill([56.0, 24.0, 10.0, 10.0], 0xFF00FF00);
                }
            }
        }

        use mod_api::__private::game::RegistrationContextNew;
        use sunrise_client::title::preset_editors::PresetEditorFactories;
        use {
            crate::game::server_side::native_entity_refs::tests::with_client_game,
            sunrise_gameplay_common::game::in_own_process,
        };

        // The real-game resource fixture also initializes process-wide engine tables.
        const TEST: &str = "cut_layout::game::native_hooks::register_preset_editors::tests::failed_preset_editor_paint_leaves_stock_frame_and_post";
        in_own_process(TEST, || {
            let mods = vec![
                ModEntry::new("editor_paint_failure", Box::new(PaintFailure)),
                ModEntry::new("editor_post_painter", Box::new(PostPainter)),
            ];
            mod_platform::testing::with_mods(mods.into(), || {
                with_client_game(|game| {
                    let saves = tempfile::tempdir().unwrap();
                    sunrise_level::logging::init(saves.path(), "Client thread");
                    let mut editors = Vec::new();
                    sunrise_gameplay_common::game::native_hooks::native_register_preset_editors(
                        &mod_api::RegistrationContext::new(),
                        &mut editors,
                    );
                    let mut title = game.new_title(saves.path().join("editor-saves"));
                    title.set_preset_editor_factories(PresetEditorFactories::bind(editors));
                    title.set_preset_data_source(game.pack.data_changes.clone());
                    title.set_mode(Mode::Create {
                        name: "Test".into(),
                        seed: "1".into(),
                        creative: false,
                        focus: 0,
                        options: WorldOptions {
                            world_type: sunrise_sim::save::WorldPreset::Flat,
                            tab: 1,
                            ..Default::default()
                        },
                    });
                    title.focused_button = Some(26);
                    title.confirm_key_size((320.0, 240.0));
                    assert_eq!(title.mode.id(), "world_preset_editor");

                    let size = [640, 480];
                    let mut stock = HudBatch::new(size, game.client.settings.gui_scale_inputs());
                    title.draw(
                        &game.client.gui.atlas,
                        &mut stock,
                        game.client.cursor,
                        false,
                        &mut |_, _| {},
                    );
                    assert!(!stock.vertices.is_empty());
                    *game.test_title_mut() = Some(title);
                    game.client.screen = Screen::Playing;
                    let frame = game.hud(size);
                    let (prefix, later) = frame.vertices.split_at(stock.vertices.len());
                    assert!(
                        prefix
                            .iter()
                            .zip(&stock.vertices)
                            .all(|(a, b)| a.pos == b.pos && a.uv == b.uv && a.color == b.color),
                        "stock screen paint changed"
                    );
                    assert!(frame.vertices.iter().all(|v| v.color != 0xFFFF00FF));
                    assert!(
                        later
                            .iter()
                            .any(|v| v.color == 0xFF00FF00 && v.pos == [56.0, 24.0]),
                        "Render.Post did not paint outside the failed editor's clip"
                    );
                    // The title frame drains the report queue into the mandatory toast path.
                    let [
                        sunrise_client_gameplay::game::toast_manager::VisibleToast {
                            toast:
                                sunrise_client_gameplay::game::toast_manager::Toast::Diagnostic {
                                    toast,
                                    hide_with_f1: false,
                                },
                            ..
                        },
                    ] = game.client.toast_manager.visible.as_slice()
                    else {
                        panic!("the preset editor failure should produce one mandatory toast");
                    };
                    assert_eq!(
                        toast.message.as_ref().unwrap().component(),
                        serde_json::json!({
                            "translate": "sunrise.mods.hook_panic.description",
                            "with": [{"text": "editor_paint_failure"}, {"text": "render"}],
                        })
                    );
                    assert!(sunrise_level::logging::recent_lines().iter().any(|line| {
                        line.contains("mod editor_paint_failure panicked in hook render")
                            && line.contains("preset editor failed after painting")
                    }));
                });
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn preset_editor_test_mod_draws_and_edits_through_real_client_hooks() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::register_preset_editors::tests::preset_editor_test_mod_draws_and_edits_through_real_client_hooks",
            exercise_preset_editor,
        );
    }

    fn exercise_preset_editor() {
        struct NoopLoop;
        impl sunrise_client_gameplay::game::event_loop::EventLoopCtl for NoopLoop {
            fn exit(&self) {}
            fn set_control_flow(&self, _: winit::event_loop::ControlFlow) {}
        }

        let mouse = Arc::new(Mutex::new(MouseProbe::default()));
        let clicks = Arc::new(AtomicUsize::new(0));
        let keys = Arc::new(AtomicUsize::new(0));
        let typed = Arc::new(Mutex::new(String::new()));
        let char_post = Arc::new(Mutex::new(String::new()));
        let key_post = Arc::new(Mutex::new(Vec::new()));
        let releases = Arc::new(Mutex::new(Vec::new()));
        let create_openings = Arc::new(AtomicUsize::new(0));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "example",
                Box::new(EditorMod {
                    mouse: Arc::clone(&mouse),
                    clicks: Arc::clone(&clicks),
                    keys: Arc::clone(&keys),
                    typed: Arc::clone(&typed),
                    char_post: Arc::clone(&char_post),
                    key_post: Arc::clone(&key_post),
                    releases: Arc::clone(&releases),
                    create_openings: Arc::clone(&create_openings),
                }),
            )]
            .into(),
            || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let _window =
                        sunrise_client_gameplay::game::native_hooks::particle_count_tests::frame_scene(game);
                    use mod_api::__private::game::RegistrationContextNew;
                    let mut editors = Vec::new();
                    sunrise_gameplay_common::game::native_hooks::native_register_preset_editors(
                        &mod_api::RegistrationContext::new(),
                        &mut editors,
                    );
                    assert_eq!(editors.len(), 1);
                    let saves = tempfile::tempdir().unwrap();
                    let mut title = game.new_title(saves.path().join("editor-saves"));
                    // Keep the data preset available when returning to Create World reloads its
                    // available presets, so the fixture reopens a freshly constructed editor.
                    let data = tempfile::tempdir().unwrap();
                    let preset = data.path().join("data/example/worldgen/world_preset");
                    std::fs::create_dir_all(&preset).unwrap();
                    std::fs::write(
                        preset.join("islands.json"),
                        r#"{"dimensions":{"minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}}}"#,
                    ).unwrap();
                    let tags = data
                        .path()
                        .join("data/minecraft/tags/worldgen/world_preset");
                    std::fs::create_dir_all(&tags).unwrap();
                    std::fs::write(
                        tags.join("normal.json"),
                        r#"{"values":["example:islands"]}"#,
                    )
                    .unwrap();
                    title.set_preset_data_source(
                        sunrise_assets::resource_source::ResourceSource::directory(
                            data.path().to_owned(),
                        ),
                    );
                    title.set_preset_editor_factories(
                        sunrise_client::title::preset_editors::PresetEditorFactories::bind(editors),
                    );
                    title.set_mode(Mode::Create {
                        name: "Test".into(),
                        seed: "1".into(),
                        creative: false,
                        focus: 0,
                        options: WorldOptions {
                            custom_preset: Some("example:islands".into()),
                            tab: 1,
                            ..Default::default()
                        },
                    });
                    title.focused_button = Some(26);
                    title.confirm_key_size((320.0, 240.0));
                    assert_eq!(title.mode.id(), "world_preset_editor");
                    assert_eq!(
                        title.preset_editor_session().unwrap().title(),
                        Some("Islands Editor")
                    );
                    *game.test_title_mut() = Some(title);
                    game.client.screen = Screen::Playing;
                    game.settle_title_mode();
                    let mut batch =
                        HudBatch::new([640, 480], game.client.settings.gui_scale_inputs());
                    game.test_title().unwrap().draw(
                        &game.client.gui.atlas,
                        &mut batch,
                        (0.0, 0.0),
                        false,
                        &mut |phase, batch| game.draw_screen_phase(batch, phase),
                    );
                    let stock_vertices = batch.vertices.len();
                    game.draw_preset_editor(&mut batch);
                    let after_editor_vertices = batch.vertices.len();
                    assert!(after_editor_vertices > stock_vertices);
                    let view = game.title_screen_render_view(&batch).unwrap();
                    assert_eq!(view.id, "world_preset_editor");
                    game.native_screen_render_post(&mut batch, &view);
                    assert!(batch.vertices.len() > after_editor_vertices);
                    assert!(game.preset_editor_mouse_clicked(0, false));
                    // The window's keyboard gate must swallow the IME's composing key (e.g.
                    // Enter accepting the candidate) before it can reach the footer.
                    game.handle_window_event_inner(
                        &NoopLoop,
                        winit::event::WindowEvent::Ime(winit::event::Ime::Preedit(
                            "e".into(),
                            Some((0, 1)),
                        )),
                        None,
                        None,
                    );
                    let window_press =
                        |game: &mut sunrise_client_gameplay::game::ClientGame<'_>, key| {
                            // winit's KeyEvent contains private platform data. Send the key to the
                            // same dispatcher that receives those events, without OS shortcuts.
                            game.dispatch_window_key(
                                &NoopLoop,
                                WindowKeyInput {
                                    physical: PhysicalKey::Code(key),
                                    state: ElementState::Pressed,
                                    repeat: false,
                                    text: None,
                                    shortcut: None,
                                },
                                |_| false,
                            );
                        };
                    window_press(game, KeyCode::Escape);
                    game.settle_title_mode();
                    assert!(game.preset_editor_title().is_some());
                    game.handle_window_event_inner(
                        &NoopLoop,
                        winit::event::WindowEvent::Ime(winit::event::Ime::Commit("é!x".into())),
                        None,
                        None,
                    );
                    assert_eq!(typed.lock().unwrap().as_str(), "é");
                    assert!(!sunrise_client::preedit::composing());
                    sunrise_client::preedit::reset();
                    // The socket passes a held modifier into the editor, then releases both keys.
                    let (reply, _) = std::sync::mpsc::channel();
                    for input in [
                        "key shift down",
                        "key tab tap",
                        "key shift up",
                        "key shift tap",
                    ] {
                        game.control_command(input, &reply, &NoopLoop).unwrap();
                    }
                    assert!(!game.has_shift_down());
                    assert!(!game.client.physical_keys.contains(&KeyCode::Tab));
                    assert!(!game.client.physical_keys.contains(&KeyCode::ShiftLeft));
                    game.control_command("type Ωx", &reply, &NoopLoop).unwrap();
                    // Outside composition the same window dispatcher reaches the editor;
                    // Q is declined by both the editor and its widgets.
                    window_press(game, KeyCode::KeyQ);
                    assert_eq!(*key_post.lock().unwrap(), ["key.keyboard.q"]);
                    assert_eq!(keys.load(Ordering::Relaxed), 1);
                    assert_eq!(clicks.load(Ordering::Relaxed), 1);
                    assert_eq!(char_post.lock().unwrap().as_str(), "xx");
                    assert_eq!(typed.lock().unwrap().as_str(), "éΩ");
                    let after_click = game.preset_editor_dimensions().unwrap();
                    assert_eq!(
                        after_click
                            .get(&"minecraft:overworld".into())
                            .unwrap()
                            .get("type"),
                        Some(&sunrise_core::nbt::Tag::String(
                            "minecraft:the_nether".into()
                        ))
                    );
                    game.dispatch_screen_key(
                        &NoopLoop,
                        KeyCode::Space,
                        true,
                        false,
                        None,
                        sunrise_client::held::Source::Keyboard,
                    )
                    .unwrap();
                    game.screen_chars_typed(Some("z"));
                    assert_eq!(typed.lock().unwrap().as_str(), "éΩz");
                    assert_eq!(*key_post.lock().unwrap(), ["key.keyboard.q"]);
                    let dimensions = game.preset_editor_dimensions().unwrap();
                    assert_eq!(
                        dimensions
                            .get(&"minecraft:overworld".into())
                            .unwrap()
                            .get("type"),
                        Some(&sunrise_core::nbt::Tag::String(
                            "minecraft:overworld_caves".into()
                        ))
                    );
                    game.client.screen = Screen::Options;
                    assert!(game.preset_editor_dimensions().is_none());
                    assert!(!game.set_preset_editor_dimensions(dimensions));
                    game.client.screen = Screen::Playing;

                    // Releases target the screen that is open when they arrive, not the editor
                    // that saw the presses. No world is loaded on either screen.
                    releases.lock().unwrap().clear();
                    for line in ["key shift down", "key q down"] {
                        game.control_command(line, &reply, &NoopLoop).unwrap();
                    }
                    assert!(game.has_shift_down());
                    assert!(game.client.physical_keys.contains(&KeyCode::ShiftLeft));
                    assert!(game.client.physical_keys.contains(&KeyCode::KeyQ));
                    game.control_command("key escape tap", &reply, &NoopLoop)
                        .unwrap();
                    assert_eq!(game.test_title().unwrap().mode.id(), "world_create");
                    for line in ["key shift up", "key q up"] {
                        game.control_command(line, &reply, &NoopLoop).unwrap();
                    }
                    assert!(!game.has_shift_down());
                    assert!(!game.client.physical_keys.contains(&KeyCode::ShiftLeft));
                    assert!(!game.client.physical_keys.contains(&KeyCode::KeyQ));
                    assert_eq!(create_openings.load(Ordering::Relaxed), 1);
                    assert_eq!(
                        releases
                            .lock()
                            .unwrap()
                            .iter()
                            .map(|(screen, key, modifiers)| (
                                screen.as_str(),
                                key.as_str(),
                                *modifiers
                            ))
                            .collect::<Vec<_>>(),
                        [
                            ("world_create", "key.keyboard.escape", 1),
                            ("world_create", "key.keyboard.left.shift", 0),
                            ("world_create", "key.keyboard.q", 0),
                        ]
                    );

                    use MouseStep::{Editor, Post, Pre, Release};
                    const PRESET: &str = "world_preset_editor";
                    let open_editor = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                        let size = game.gui_size();
                        let title = game.test_title_mut().as_mut().unwrap();
                        title.focused_button = Some(26);
                        title.confirm_key_size(size);
                        game.settle_title_mode();
                        assert_eq!(game.test_title().unwrap().mode.id(), PRESET);
                    };
                    let button_event =
                        |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                         state,
                         button,
                         millis| {
                            game.handle_window_event_inner(
                                &NoopLoop,
                                WindowEvent::MouseInput {
                                    device_id: DeviceId::dummy(),
                                    state,
                                    button,
                                },
                                None,
                                millis,
                            );
                            std::mem::take(&mut mouse.lock().unwrap().events)
                        };
                    let press = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                 button,
                                 millis| {
                        button_event(game, ElementState::Pressed, button, millis)
                    };
                    let editor_press =
                        |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                         button,
                         millis,
                         code,
                         double,
                         handled| {
                            assert_eq!(
                                press(game, button, millis),
                                [
                                    Pre(code, double),
                                    Editor(code, double),
                                    Post(PRESET, code, double, handled, true)
                                ]
                            );
                        };
                    let left = MouseButton::Left;
                    let right = MouseButton::Right;
                    let other = MouseButton::Other(300);
                    mouse.lock().unwrap().events.clear(); // The direct edit above is not a window press.
                    open_editor(game);
                    game.client.cursor = (20.0, 30.0);

                    // Post sees the original answer even when it forces unhandled. The next
                    // press is not a double-click: the first press was not recorded.
                    mouse.lock().unwrap().press_result = TriState::False;
                    editor_press(game, left, Some(1000), 0, false, true);
                    mouse.lock().unwrap().press_result = TriState::Default;
                    editor_press(game, left, Some(1010), 0, false, true);
                    game.handle_window_event_inner(
                        &NoopLoop,
                        WindowEvent::Resized(winit::dpi::PhysicalSize::new(360, 260)),
                        None,
                        None,
                    );
                    game.build_screen();
                    editor_press(game, left, Some(1259), 0, true, true);
                    editor_press(game, left, Some(1509), 0, false, true);
                    editor_press(game, right, Some(1510), 1, false, true);
                    editor_press(game, other, Some(1511), 300, false, false);
                    // A synthetic press passes false and does not enter the real press history.
                    editor_press(game, left, None, 0, false, true);
                    editor_press(game, right, Some(1512), 1, true, true);
                    mouse.lock().unwrap().press_result = TriState::True;
                    editor_press(game, other, Some(1513), 300, false, false);
                    mouse.lock().unwrap().press_result = TriState::Default;
                    editor_press(game, other, Some(1514), 300, true, false);

                    // Clicking Done would set the Tab starting point to Done. When Pre
                    // cancels the press, the screen stays open and the first Tab lands on
                    // Done instead of moving from it to Cancel.
                    game.pad_clear_focus();
                    sunrise_client::widgets::end_frame(1);
                    let size = {
                        let renderer = game.test_renderer().unwrap();
                        [renderer.config.width, renderer.config.height]
                    };
                    let _ = game.hud_layers(size);
                    sunrise_client::widgets::end_frame(2);
                    let (w, h) = game.gui_size();
                    let done = (w / 2.0 - 80.0, h - 18.0);
                    let target = game
                        .pad_targets()
                        .into_iter()
                        .find(|target| {
                            let [x, y, width, height] = target.rect;
                            (x..x + width).contains(&done.0) && (y..y + height).contains(&done.1)
                        })
                        .expect("the rendered Done button is a focus target");
                    game.client.cursor = done;
                    mouse.lock().unwrap().cancel_press = true;
                    assert_eq!(press(game, left, Some(2000)), [Pre(0, false)]);
                    assert_eq!(game.test_title().unwrap().mode.id(), PRESET);
                    mouse.lock().unwrap().cancel_press = false;
                    assert_eq!(game.key_focus_key(&NoopLoop, KeyCode::Tab), Some(false));
                    assert_eq!(
                        game.test_pad().menu.focus.map(|t| t.rect),
                        Some(target.rect)
                    );
                    game.pad_clear_focus();
                    game.client.cursor = (20.0, 30.0);
                    editor_press(game, left, Some(2001), 0, true, true);

                    // Done and Cancel are taken by the footer, not the editor callback. The
                    // release resolves its own receiver on the screen they have just opened.
                    // Start the footer sequence outside the preceding press's double-click window.
                    game.client.cursor = done;
                    assert_eq!(
                        press(game, left, Some(2251)),
                        [Pre(0, false), Post(PRESET, 0, false, true, false)]
                    );
                    assert_eq!(game.test_title().unwrap().mode.id(), "world_create");
                    assert_eq!(
                        button_event(game, ElementState::Released, left, None),
                        [Release("world_create", 0, false)]
                    );
                    open_editor(game);
                    game.client.cursor = (20.0, 30.0);
                    // Same hook ID and button as Done, but a different editor instance.
                    editor_press(game, left, Some(2260), 0, false, true);
                    game.client.cursor = (w / 2.0 + 80.0, h - 18.0);
                    assert_eq!(
                        press(game, left, Some(2270)),
                        [Pre(0, true), Post(PRESET, 0, true, true, false)]
                    );
                    assert_eq!(game.test_title().unwrap().mode.id(), "world_create");
                });
            },
        );
    }
}
