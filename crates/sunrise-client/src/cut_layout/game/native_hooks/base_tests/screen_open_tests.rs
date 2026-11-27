#[allow(unused_imports)]
use crate::game::native_hooks::base::*;
use mod_api::Hand;
use mod_api::InteractionAttempt;
use mod_api::InteractionKey;
use mod_api::ScreenOpening;
#[allow(unused_imports)]
use mod_api::{
    ClientHost, Flow, Hooks, InteractionKeyMappingTriggeredEvent, KeyMappingPreEvent, ModEntry,
    ScreenClosingEvent, ScreenOpeningEvent,
};
#[allow(unused_imports)]
use mod_platform::testing::with_mods;
#[allow(unused_imports)]
use std::sync::{Arc, Mutex};
use sunrise_client::hud::HudBatch;
use sunrise_client_gameplay::game::client_side::screen_build::ObservedScreen;
use sunrise_gameplay_common::game::Screen;

/// A mod that hears `screen_opening` and passes.
pub(crate) struct Passer;
#[mod_api::__subscriptions]
impl Hooks for Passer {
    fn screen_opening(
        &self,
        _host: &dyn ClientHost,
        _event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        None
    }
}
/// A mod with no screen hook.
pub(crate) struct Idle;
#[mod_api::__subscriptions]
impl Hooks for Idle {}

pub(crate) struct KeyProbe(pub(crate) Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for KeyProbe {
    fn interaction_key_mapping_triggered(
        &self,
        _host: &mut dyn ClientHost,
        event: &mut InteractionKeyMappingTriggeredEvent,
    ) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("{:?}:{:?}", event.key, event.attempt));
        match event.key {
            InteractionKey::Attack => {
                event.swing_hand = false;
                Flow::Cancel
            }
            InteractionKey::Use => Flow::Cancel,
            _ => Flow::Continue,
        }
    }
}

/// Each key reaches a listener as its own kind; a cancelled attack that clears
/// `swing_hand` reports no swing, a cancelled use keeps its swing, and an
/// uncancelled pick goes on.
#[test]
#[ignore = "daily"]
fn an_interaction_key_reaches_a_mod_as_its_kind_and_can_be_cancelled() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("keys", Box::new(KeyProbe(log.clone())))].into();
    with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let hand = Hand::Main;
            assert_eq!(
                game.native_interaction(InteractionKey::Attack, hand, InteractionAttempt::Held)
                    .canceled_swing(),
                Some(false)
            );
            assert_eq!(
                game.native_interaction(InteractionKey::Use, hand, InteractionAttempt::Press)
                    .canceled_swing(),
                Some(true)
            );
            assert_eq!(
                game.native_interaction(InteractionKey::PickBlock, hand, InteractionAttempt::Press)
                    .canceled_swing(),
                None
            );
        });
    });
    assert_eq!(take(&log), ["Attack:Held", "Use:Press", "PickBlock:Press"]);
}

pub(crate) struct MappingProbe(pub(crate) Arc<Mutex<Vec<String>>>);
#[mod_api::__subscriptions]
impl Hooks for MappingProbe {
    fn key_mapping_pre(&self, _host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push(format!("{}:{}", event.mapping, event.pressed));
        Flow::Cancel
    }
}

/// Cancels the second press it hears and no other: an allowed down, then a cancelled repeat.
pub(crate) struct CancelSecondPress(pub(crate) Arc<Mutex<usize>>);
#[mod_api::__subscriptions]
impl Hooks for CancelSecondPress {
    fn key_mapping_pre(&self, _host: &mut dyn ClientHost, event: &mut KeyMappingPreEvent) -> Flow {
        let mut heard = self.0.lock().unwrap();
        if event.pressed {
            *heard += 1;
            if *heard == 2 {
                return Flow::Cancel;
            }
        }
        Flow::Continue
    }
}

pub(crate) struct Probe {
    pub(crate) log: Arc<Mutex<Vec<String>>>,
    pub(crate) answer: Arc<Mutex<Option<ScreenOpening>>>,
}
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn screen_opening(
        &self,
        _host: &dyn ClientHost,
        event: &mut ScreenOpeningEvent<'_>,
    ) -> Option<ScreenOpening> {
        self.log
            .lock()
            .unwrap()
            .push(format!("opening {}->{}", event.current.id, event.new.id));
        self.answer.lock().unwrap().clone()
    }
    fn screen_closing(&self, _host: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
        self.log
            .lock()
            .unwrap()
            .push(format!("closing {}", event.screen.id));
    }
}

/// Runs `test` on a client game with one probe mod; the probe's log and answer.
pub(crate) fn run(
    test: impl FnOnce(
        &mut sunrise_client_gameplay::game::ClientGame<'_>,
        &Arc<Mutex<Vec<String>>>,
        &Arc<Mutex<Option<ScreenOpening>>>,
    ),
) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(Mutex::new(None));
    let mods = vec![
        ModEntry::new("passer", Box::new(Passer)),
        ModEntry::new(
            "screens",
            Box::new(Probe {
                log: log.clone(),
                answer: answer.clone(),
            }),
        ),
    ]
    .into();
    with_game_of(mods, |game| test(game, &log, &answer));
}
/// A client game with a player, under `mods`.
pub(crate) fn with_game_of(
    mods: Arc<[ModEntry]>,
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>),
) {
    with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            test(game);
        });
    });
}

pub(crate) fn take(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    std::mem::take(&mut *log.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn the_same_screen_again_fires_nothing() {
    run(|game, log, _| {
        game.set_screen(Screen::Playing);
        assert!(take(log).is_empty());
        game.set_screen(Screen::Inventory);
        take(log);
        game.set_screen(Screen::Inventory);
        assert!(take(log).is_empty());
    });
}

pub(crate) fn observed(id: &'static str, overlay: bool) -> Option<ObservedScreen> {
    Some(ObservedScreen {
        id,
        overlay,
        mod_screen: None,
        menu: None,
        menu_generation: None,
        dialog_key: None,
        title: None,
        rect: [0.0; 4],
        screen_size: [0.0; 2],
        slots: 0,
        mouse: [0.0; 2],
        partial_tick: 0.0,
        regions: Vec::new(),
    })
}

/// An overlay (here a sign editor) is announced as a screen is: Opening
/// when it shows, Opening back to `playing` and Closing when it goes. A
/// `Screen` the observer sees change was announced by `set_screen`, so it
/// is not announced twice.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_opening_then_closing_through_the_observer() {
    run(|game, log, _| {
        game.observe_change(observed("sign_editor", true));
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        game.observe_change(observed("sign_editor", true));
        assert!(take(log).is_empty());
        game.observe_change(None);
        assert_eq!(
            take(log),
            ["opening sign_editor->playing", "closing sign_editor"]
        );
        game.observe_change(observed("inventory", false));
        game.observe_change(None);
        assert!(take(log).is_empty());
        game.observe_change(observed("inventory", false));
        game.observe_change(observed("sign_editor", true));
        assert_eq!(take(log), ["opening playing->sign_editor"]);
    });
}

#[test]
#[ignore = "daily"]
fn title_hook_requests_one_orderly_quit() {
    use mod_api::{ClientStoppingEvent, ScreenKeyPressedPreEvent};
    use std::cell::Cell;
    use sunrise_client_gameplay::game::EventLoopCtl;
    use winit::event_loop::ControlFlow;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    };

    if run_fresh_cases(
        "cut_layout::game::native_hooks::base::screen_open_tests::title_hook_requests_one_orderly_quit",
        "SUNRISE_CLIENT_QUIT_CHILD",
        &["title"],
    ) {
        return;
    }

    struct QuitProbe(Arc<Mutex<Vec<(&'static str, bool)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for QuitProbe {
        fn screen_key_pressed_pre(
            &self,
            host: &dyn ClientHost,
            event: &mut ScreenKeyPressedPreEvent<'_>,
        ) -> Flow {
            if event.screen.id != "title" {
                return Flow::Continue;
            }
            let accepted = host.request_client_quit();
            let duplicate = host.request_client_quit();
            self.0
                .lock()
                .unwrap()
                .extend([("request", accepted), ("request", duplicate)]);
            Flow::Cancel
        }

        fn client_stopping(&self, host: &mut dyn ClientHost, _: &mut ClientStoppingEvent) {
            let accepted = host.request_client_quit();
            self.0.lock().unwrap().push(("stopping", accepted));
        }
    }

    struct QuitLoop(Cell<usize>);
    impl EventLoopCtl for QuitLoop {
        fn exit(&self) {
            self.0.set(self.0.get() + 1);
        }

        fn set_control_flow(&self, _: ControlFlow) {}
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("quit", Box::new(QuitProbe(log.clone())))].into();
    with_mods(mods, || {
        with_client_game(|game| {
            let saves = tempfile::tempdir().unwrap();
            let title = game.new_title(saves.path().to_path_buf());
            *game.test_title_mut() = Some(title);
            let event_loop = QuitLoop(Cell::new(0));
            let batch = HudBatch::scratch([854, 480]);
            let screen = game.title_screen_view(&batch).unwrap();
            let mut canceled = false;
            game.native_screen_key_pressed_pre(&screen, "key.keyboard.q", false, 0, &mut canceled);
            let requests = [("request", true), ("request", false)];
            assert_eq!(*log.lock().unwrap(), requests);
            assert_eq!(event_loop.0.get(), 0);

            game.idle(&event_loop);
            assert_eq!(event_loop.0.get(), 1);
            assert!(game.error.is_none(), "quit failed: {:?}", game.error);
            assert_eq!(*log.lock().unwrap(), requests);

            game.shut_down();
            assert_eq!(
                *log.lock().unwrap(),
                [("request", true), ("request", false), ("stopping", false)]
            );
        });
    });
}

/// `screen_init_post` follows an overlay that opened, after its Opening.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_init_post_after_it_opens() {
    use mod_api::ScreenInitPostEvent;
    /// Hears Opening and Init.Post, in one log.
    struct Ordered(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Ordered {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .lock()
                .unwrap()
                .push(format!("opening {}", event.new.id));
            None
        }
        fn screen_init_post(&self, _host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("init {}", event.screen.id));
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("ordered", Box::new(Ordered(log.clone())))].into();
    with_game_of(mods, |game| {
        game.open_sign_editor((0, 64, 0), true);
        assert_eq!(take(&log), ["opening sign_editor", "init sign_editor"]);
    });
}

/// Audit [85]: `mod_screen_open` opens over `Paused`, `Inventory` (running container close
/// cleanup), `Creative`, the title screen (reporting `"title"`), active overlays (`book_view`,
/// `sign_editor`, `dialog`, `link_prompt` — reporting each overlay's id and clearing
/// `screen_over_playing()` so `screen_hooks_open()` is true), and another mod's `Screen::Mod`
/// (firing `screen_opening`/`screen_closing` and switching `native_mod_screen`); fires
/// `screen_init_post`; is idempotent (`true`) for the same mod; respects `ScreenOpening::Cancel`;
/// and returns `false` when headless (`renderer.is_none()`) or while `remote_loading()`.
#[test]
#[ignore = "daily"]
fn mod_screen_open_replaces_vanilla_screens_overlays_and_mod_screens() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::native_hooks::base::screen_open_tests::mod_screen_open_replaces_vanilla_screens_overlays_and_mod_screens",
        "SUNRISE_MOD_SCREEN_OPEN_CHILD",
        &["window"],
    ) {
        return;
    }
    use mod_api::ScreenInitPostEvent;
    use sunrise_core::nbt::{Compound, Tag};

    struct OpenAuditMod {
        log: Arc<Mutex<Vec<String>>>,
        answer: Arc<Mutex<Option<ScreenOpening>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for OpenAuditMod {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.log
                .lock()
                .unwrap()
                .push(format!("opening {}->{}", event.current.id, event.new.id));
            self.answer.lock().unwrap().clone()
        }
        fn screen_closing(&self, _host: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
            self.log
                .lock()
                .unwrap()
                .push(format!("closing {}", event.screen.id));
        }
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.log.lock().unwrap().push(format!(
                "init {}:{:?}",
                event.screen.id,
                host.mod_screen()
            ));
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(Mutex::new(None));
    let mods: Arc<[ModEntry]> = vec![ModEntry::new(
        "audit",
        Box::new(OpenAuditMod {
            log: log.clone(),
            answer: answer.clone(),
        }),
    )]
    .into();

    with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            // 1) Refused when headless (`renderer.is_none()`).
            assert!(game.test_renderer().is_none());
            assert!(!game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(take(&log).is_empty());

            // Attach an offscreen window + renderer for live screen transitions.
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
                            .with_inner_size(winit::dpi::PhysicalSize::new(320, 240)),
                    )
                    .unwrap(),
            );
            game.options.offscreen = true;
            *game.test_window_mut() = Some(window.clone());
            let renderer = game.new_renderer(window).unwrap();
            game.test_set_renderer(renderer);
            *game.test_title_mut() = None;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.client.focused = true;

            // 2) Opens over `Screen::Paused`, fires opening/closing/init_post.
            game.set_screen(Screen::Paused);
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_a"));
            assert!(game.screen_hooks_open());
            assert_eq!(
                take(&log),
                vec![
                    "opening paused->mod",
                    "closing paused",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            // 4) Idempotent `true` for the same mod with no re-fire.
            assert!(game.mod_screen_open("mod_a"));
            assert!(take(&log).is_empty());

            // 5) Switches from `mod_a` to another mod `mod_b`, firing opening/closing/init_post.
            assert!(game.mod_screen_open("mod_b"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_b"));
            assert_eq!(
                take(&log),
                vec![
                    "opening mod->mod",
                    "closing mod",
                    "init mod:Some(\"mod_b\")",
                ]
            );

            // 6) `ScreenOpening::Cancel` keeps `mod_b` open and returns `false`.
            *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
            assert!(!game.mod_screen_open("mod_c"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_b"));
            assert_eq!(take(&log), vec!["opening mod->mod"]);
            *answer.lock().unwrap() = None;

            // 7) Opens over `Screen::Creative`.
            game.set_screen(Screen::Creative);
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_a"));
            assert_eq!(
                take(&log),
                vec![
                    "opening creative->mod",
                    "closing creative",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            // 8) Opens over `Screen::Inventory` with an open remote chest menu, sending
            //    `ServerboundContainerClosePacket` during close cleanup.
            let (mut reader, _writer) = crate::game::remote::tests::connect_menu_test(game);
            use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 1,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                teleport: 1,
                position: [0.5, 65.0, 0.5],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives: 0,
            }))
            .unwrap();
            game.client.remote.as_mut().unwrap().level_load.finish();
            let root = std::path::PathBuf::from(
                std::env::var_os("SUNRISE_TEST_RESOURCES")
                    .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
            );
            let report = sunrise_protocol::registry::Builtin::find(&root)
                .expect("the prepared resources carry registries.json");
            let builtin = sunrise_protocol::registry::Builtin::load(&report).unwrap();
            let chest_menu = builtin
                .get("minecraft:menu")
                .unwrap()
                .id("minecraft:generic_9x3")
                .unwrap();
            game.handle_packet(C::OpenScreen(play::OpenScreen {
                container: 5,
                menu: chest_menu,
                title: Tag::String("Chest".into()),
            }))
            .unwrap();
            assert_eq!(game.client.screen, Screen::Inventory);
            reader
                .set_read_timeout(Some(std::time::Duration::from_millis(100)))
                .unwrap();
            while reader.recv().is_ok() {}
            take(&log);

            assert!(game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_a"));
            assert!(game.client.remote.as_ref().unwrap().menu.as_ref().is_none());
            let mut closed_container = None;
            while let Ok((id, body)) = reader.recv() {
                if let Ok(S::ContainerClose(close)) = S::decode(id, &body) {
                    closed_container = Some(close.container);
                }
            }
            assert_eq!(closed_container, Some(5));
            assert_eq!(
                take(&log),
                vec![
                    "opening inventory->mod",
                    "closing inventory",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            // 9) Opens over active overlays (`book_view`, `sign_editor`, `dialog`, `link_prompt`):
            //    reports each overlay's id in `opening`/`closing`, clears `screen_over_playing()`,
            //    and leaves `screen_hooks_open() == true`.
            game.set_screen(Screen::Playing);
            game.open_book(sunrise_client_gameplay::game::books::BookScreen::View {
                pages: Vec::new(),
                page: 0,
                lectern: None,
            });
            assert!(game.screen_over_playing());
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert!(game.client.book.is_none());
            assert!(!game.screen_over_playing());
            assert!(game.screen_hooks_open());
            assert_eq!(
                take(&log),
                vec![
                    "opening book_view->mod",
                    "closing book_view",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            game.set_screen(Screen::Playing);
            game.open_sign_editor((0, 64, 0), true);
            assert!(game.screen_over_playing());
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert!(game.client.sign_edit.is_none());
            assert!(!game.screen_over_playing());
            assert!(game.screen_hooks_open());
            assert_eq!(
                take(&log),
                vec![
                    "opening sign_editor->mod",
                    "closing sign_editor",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            game.set_screen(Screen::Playing);
            let mut dialog = Compound::new();
            dialog.insert("type".into(), Tag::String("minecraft:notice".into()));
            dialog.insert("title".into(), Tag::String("Notice".into()));
            game.show_dialog(&Tag::Compound(dialog));
            assert!(game.dialog_open());
            assert!(game.screen_over_playing());
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert!(!game.dialog_open());
            assert!(!game.screen_over_playing());
            assert!(game.screen_hooks_open());
            assert_eq!(
                take(&log),
                vec![
                    "opening dialog->mod",
                    "closing dialog",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            game.set_screen(Screen::Playing);
            game.open_screen_overlay("link_prompt", |game| {
                game.client.link_prompt = Some("https://example.com".into());
            });
            assert!(game.screen_over_playing());
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert!(game.client.link_prompt.is_none());
            assert!(!game.screen_over_playing());
            assert!(game.screen_hooks_open());
            assert_eq!(
                take(&log),
                vec![
                    "opening link_prompt->mod",
                    "closing link_prompt",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            // 10) Opens over the title screen, reporting `"title"`.
            game.set_screen(Screen::Playing);
            game.client.remote = None;
            game.player = None;
            let saves = tempfile::tempdir().unwrap();
            let title = game.new_title(saves.path().to_path_buf());
            *game.test_title_mut() = Some(title);
            take(&log);
            assert!(game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("mod_a"));
            assert_eq!(
                take(&log),
                vec![
                    "opening title->mod",
                    "closing title",
                    "init mod:Some(\"mod_a\")",
                ]
            );

            // 11) Refused while `remote_loading()` (connected with `player.is_none()`).
            game.set_screen(Screen::Playing);
            let (_peer, _w) = crate::game::remote::tests::connect_menu_test(game);
            game.player = None;
            assert!(game.remote_loading());
            take(&log);
            assert!(!game.mod_screen_open("mod_a"));
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(take(&log).is_empty());

            drop(event_loop);
        });
    });
}
