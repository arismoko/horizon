#[allow(unused_imports)]
pub(crate) use super::*;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::CancelSecondPress;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::Idle;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::KeyProbe;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::MappingProbe;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::Passer;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::Probe;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::observed;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::run;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::take;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::with_game_of;
#[allow(unused_imports)]
pub(crate) use crate::game::native_hooks::base::screen_open_tests::*;
use mod_api::ScreenOpening;
#[allow(unused_imports)]
pub(crate) use mod_api::{
    ClientHost, Flow, Hooks, InteractionKeyMappingTriggeredEvent, KeyMappingPreEvent, ModEntry,
    ScreenClosingEvent, ScreenOpeningEvent,
};
#[allow(unused_imports)]
pub(crate) use mod_platform::testing::with_mods;
#[allow(unused_imports)]
pub(crate) use std::sync::{Arc, Mutex};
use sunrise_client::hud::HudBatch;
use sunrise_gameplay_common::game::Screen;
use winit::keyboard::KeyCode;

/// A cancelled press of a mapping is dropped while playing, and only then asked; the
/// release of the same key is asked too but never canceled, so nothing stays held.
#[test]
#[ignore = "daily"]
fn a_cancelled_mapping_press_is_dropped_and_its_release_is_not() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new(
        "mapping",
        Box::new(MappingProbe(log.clone())),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            assert!(game.key_mapping_canceled(Some("key.keyboard.space"), true));
            assert!(!game.key_mapping_canceled(Some("key.keyboard.space"), false));
            game.client.screen = Screen::Inventory;
            assert!(!game.key_mapping_canceled(Some("key.keyboard.space"), true));
        });
    });
    assert_eq!(take(&log), ["key.jump:true", "key.jump:false"]);
}

/// A cancelled repeat keeps the hold an accepted press made, while a cancelled first press
/// is never held, nor are its repeats.
#[test]
#[ignore = "daily"]
fn a_cancelled_repeat_keeps_the_hold_and_a_cancelled_first_press_is_never_held() {
    use winit::keyboard::KeyCode;
    let heard = Arc::new(Mutex::new(0));
    let mods = vec![ModEntry::new(
        "second",
        Box::new(CancelSecondPress(heard.clone())),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            let space = KeyCode::Space;
            // The first down is allowed and held; its repeat is cancelled but the hold stays.
            game.client.physical_keys.insert(space);
            assert!(!game.key_press_canceled(space, true, false));
            game.client.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, true));
            assert!(game.client.physical_keys.contains(&space));
            assert!(!game.client.rejected_keys.contains(&space));
        });
    });
    let mods = vec![ModEntry::new(
        "mapping",
        Box::new(MappingProbe(Arc::new(Mutex::new(Vec::new())))),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            let space = KeyCode::Space;
            // A cancelled first press is not held, and neither is its repeat.
            game.client.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, false));
            assert!(!game.client.physical_keys.contains(&space));
            game.client.physical_keys.insert(space);
            assert!(game.key_press_canceled(space, true, true));
            assert!(!game.client.physical_keys.contains(&space));
            // Its release clears the rejection.
            assert!(!game.key_press_canceled(space, false, false));
            assert!(!game.client.rejected_keys.contains(&space));
        });
    });
}

/// Only a mod that listens to `screen_opening` or `screen_closing` makes a screen
/// change a hook call; a mod with neither leaves it a plain assignment, and a mod
/// screen's entry still releases the keys.
#[test]
#[ignore = "daily"]
fn a_mod_with_no_screen_hook_leaves_set_screen_plain() {
    with_game_of(vec![ModEntry::new("idle", Box::new(Idle))].into(), |game| {
        game.insert_device_key(KeyCode::KeyW);
        game.set_screen(Screen::Mod);
        assert_eq!(game.client.screen, Screen::Mod);
        assert!(game.client.keys.is_empty());
    });
}

/// Opening fires once, before the change, and the old screen's closing
/// follows it (`ScreenEvent.Opening`, then `.Closing`): none for the null
/// screen the game plays on, one for each screen it leaves.
#[test]
#[ignore = "daily"]
fn a_new_screen_fires_opening_then_closing_of_the_old_one() {
    run(|game, log, _| {
        game.set_screen(Screen::Inventory);
        assert_eq!(game.client.screen, Screen::Inventory);
        assert_eq!(take(log), ["opening playing->inventory"]);
        game.set_screen(Screen::Playing);
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(
            take(log),
            ["opening inventory->playing", "closing inventory"]
        );
    });
}

/// A title screen asked for through `Title::set_mode` opens once the game
/// fired `screen_opening` (then `screen_closing` for the one it leaves),
/// and `Cancel` keeps the title screen shown.
#[test]
#[ignore = "daily"]
fn a_title_screen_opens_through_screen_opening() {
    use sunrise_client::title::Mode;
    run(|game, log, answer| {
        // No saves: Singleplayer opens Create New World.
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.client.title.0 = Some(game.new_title(saves));
        game.settle_title_mode();
        assert!(game.client.title.0.as_ref().unwrap().gated);
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.client.title.0.as_mut().unwrap().enter_singleplayer();
        assert!(matches!(
            game.client.title.0.as_ref().unwrap().mode,
            Mode::Main
        ));
        game.settle_title_mode();
        assert!(matches!(
            game.client.title.0.as_ref().unwrap().mode,
            Mode::Main
        ));
        assert_eq!(take(log), ["opening title->world_create"]);
        *answer.lock().unwrap() = None;
        game.client.title.0.as_mut().unwrap().enter_singleplayer();
        game.settle_title_mode();
        assert!(matches!(
            game.client.title.0.as_ref().unwrap().mode,
            Mode::Create { .. }
        ));
        assert_eq!(take(log), ["opening title->world_create", "closing title"]);
        game.settle_title_mode();
        assert!(take(log).is_empty());
    });
}

/// An overlay opens through `open_screen_overlay`: Opening fires before it is
/// set, once, and the frame's observer does not announce it again.
#[test]
#[ignore = "daily"]
fn an_overlay_fires_opening_before_it_opens() {
    run(|game, log, _| {
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.client.sign_edit.is_some());
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        game.observe_screen();
        assert!(take(log).is_empty());
    });
}

/// An overlay a mod's `Vanilla` answer replaced is followed by the screen
/// that opened's `screen_init_post`, once, with no second Opening.
#[test]
#[ignore = "daily"]
fn an_overlay_replaced_by_a_vanilla_screen_fires_its_init_post() {
    use mod_api::{ScreenInitPostEvent, ScreenTarget};
    /// Answers the pause screen for every opening; logs Opening and Init.Post.
    struct ToPause(Arc<Mutex<Vec<String>>>);
    #[mod_api::__subscriptions]
    impl Hooks for ToPause {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            self.0
                .lock()
                .unwrap()
                .push(format!("opening {}", event.new.id));
            Some(ScreenOpening::Vanilla(ScreenTarget::Pause))
        }
        fn screen_init_post(&self, _host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0
                .lock()
                .unwrap()
                .push(format!("init {}", event.screen.id));
        }
    }
    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("pause", Box::new(ToPause(log.clone())))].into();
    with_game_of(mods, |game| {
        game.client.title.0 = None;
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.client.sign_edit.is_none());
        assert_eq!(game.client.screen, Screen::Paused);
        assert_eq!(take(&log), ["opening sign_editor", "init paused"]);
    });
}

/// `Cancel` keeps an overlay shut, as it keeps a `Screen` shut.
#[test]
#[ignore = "daily"]
fn cancelling_opening_keeps_an_overlay_shut() {
    run(|game, log, answer| {
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.open_sign_editor((0, 64, 0), true);
        assert!(game.client.sign_edit.is_none());
        assert_eq!(take(log), ["opening playing->sign_editor"]);
        let opened = game.open_screen_overlay("link_prompt", |game| {
            game.client.link_prompt = Some("https://example.com".into())
        });
        assert!(!opened);
        assert!(game.client.link_prompt.is_none());
        assert_eq!(take(log), ["opening playing->link_prompt"]);
    });
}

#[test]
#[ignore = "daily"]
fn cancelling_opening_keeps_the_current_screen() {
    run(|game, log, answer| {
        game.set_screen(Screen::Paused);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.set_screen(Screen::Options);
        assert_eq!(game.client.screen, Screen::Paused);
        assert_eq!(take(log), ["opening paused->options"]);
    });
}

/// The read and the hook see the same live screen, including overlays.
#[test]
#[ignore = "daily"]
fn client_gui_state_matches_screen_init_hook() {
    // Winit permits only one event loop per process, even after it is dropped.
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::native_hooks_client::base::screen_open_tests::client_gui_state_matches_screen_init_hook",
        "SUNRISE_SCREEN_STATE_CHILD",
        &["window"],
    ) {
        return;
    }
    use mod_api::ScreenInitPostEvent;
    struct Observe(Arc<Mutex<Vec<(String, Option<String>)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observe {
        fn screen_opening(
            &self,
            _host: &dyn ClientHost,
            _event: &mut ScreenOpeningEvent<'_>,
        ) -> Option<ScreenOpening> {
            None
        }
        fn screen_init_post(&self, host: &dyn ClientHost, event: &mut ScreenInitPostEvent<'_>) {
            self.0.lock().unwrap().push((
                event.screen.id.to_owned(),
                host.observed_screen_id().map(|id| id.id),
            ));
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![ModEntry::new("observe", Box::new(Observe(seen.clone())))].into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.focused = false;
            assert!(!mod_api::prelude::ScreensClientHost::window_focused(game));
            game.client.focused = true;
            assert!(mod_api::prelude::ScreensClientHost::window_focused(game));
            game.client.title.0 = None;
            game.client.screen = Screen::Playing;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
            game.client.screen = Screen::Inventory;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
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
            {
                let renderer = game.new_renderer(window).unwrap();
                game.set_renderer(renderer);
            }
            game.client.screen = Screen::Playing;
            assert!(mod_api::prelude::ScreensClientHost::observed_screen_id(game).is_none());
            game.set_screen(Screen::Inventory);
            game.observe_screen();
            assert_eq!(
                mod_api::prelude::ScreensClientHost::observed_screen_id(game)
                    .unwrap()
                    .id,
                "inventory"
            );
            game.open_screen_overlay("link_prompt", |game| {
                game.client.link_prompt = Some("https://example.com".into());
            });
            assert_eq!(
                mod_api::prelude::ScreensClientHost::observed_screen_id(game)
                    .unwrap()
                    .id,
                "link_prompt"
            );
            let seen = seen.lock().unwrap();
            assert!(
                seen.iter()
                    .any(|(event, read)| event == "inventory"
                        && read.as_deref() == Some("inventory"))
            );
            assert!(
                seen.iter().any(|(event, read)| event == "link_prompt"
                    && read.as_deref() == Some("link_prompt"))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn no_mods_is_a_plain_assignment() {
    with_mods(Vec::new().into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.set_screen(Screen::Inventory);
            assert_eq!(game.client.screen, Screen::Inventory);
        });
    });
}

/// `ModScreen` opens the mod's screen over the title's screens with no world,
/// and Escape or the controller's Back returns to the title screen.
#[test]
#[ignore = "daily"]
fn replacing_a_title_screen_opens_the_mods_screen_over_the_title() {
    use sunrise_client::title::Mode;
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.player = None;
        game.client.title.0 = Some(game.new_title(saves));
        game.settle_title_mode();
        let back: [fn(&mut sunrise_client_gameplay::game::ClientGame<'_>); 2] = [
            |game| game.key_pressed(KeyCode::Escape, KeyCode::Escape).unwrap(),
            |game| game.pad_back().unwrap(),
        ];
        for back in back {
            *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
            game.client
                .title
                .0
                .as_mut()
                .unwrap()
                .set_mode(Mode::Select {
                    worlds: Vec::new(),
                    selected: None,
                });
            game.settle_title_mode();
            assert_eq!(game.client.screen, Screen::Mod);
            assert_eq!(game.mod_screen(), Some("screens"));
            assert!(matches!(
                game.client.title.0.as_ref().unwrap().mode,
                Mode::Main
            ));
            assert_eq!(take(log), ["opening title->world_select", "closing title"]);
            *answer.lock().unwrap() = None;
            back(game);
            assert_eq!(game.client.screen, Screen::Playing);
            assert!(matches!(
                game.client.title.0.as_ref().unwrap().mode,
                Mode::Main
            ));
            assert_eq!(take(log), ["opening mod->title", "closing mod"]);
        }
    });
}

/// `ModScreen` opens the answering mod's screen in place of the overlay.
#[test]
#[ignore = "daily"]
fn replacing_opening_of_an_overlay_opens_the_mods_screen() {
    run(|game, log, answer| {
        *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
        game.open_book(sunrise_client_gameplay::game::books::BookScreen::View {
            pages: Vec::new(),
            page: 0,
            lectern: None,
        });
        assert!(game.client.book.is_none());
        assert_eq!(game.client.screen, Screen::Mod);
        assert_eq!(game.mod_screen(), Some("screens"));
        assert_eq!(take(log), ["opening playing->book_view"]);
    });
}

/// `Vanilla` opens another vanilla screen where it can: the death screen's
/// way to the title ends on the server list, while the same title target
/// asked for in the world keeps the screen asked for.
#[test]
#[ignore = "daily"]
fn replacing_opening_opens_another_vanilla_screen_where_it_can() {
    use mod_api::ScreenTarget;
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.client.title.0 = None;
        game.set_screen(Screen::Dead);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::Vanilla(ScreenTarget::Multiplayer));
        // Leaving the world: no title screen opens in it.
        game.set_screen(Screen::Playing);
        assert_eq!(game.client.screen, Screen::Playing);
        game.show_title(game.new_title(saves));
        assert_eq!(
            game.client.title.0.as_ref().unwrap().mode.id(),
            "multiplayer"
        );
        // The replacement fires nothing again.
        assert_eq!(
            take(log),
            [
                "opening dead->playing",
                "closing dead",
                "opening playing->title"
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn replacing_opening_opens_the_mods_screen_instead() {
    run(|game, log, answer| {
        game.set_screen(Screen::Paused);
        take(log);
        *answer.lock().unwrap() = Some(ScreenOpening::ModScreen);
        game.set_screen(Screen::Options);
        assert_eq!(game.client.screen, Screen::Mod);
        // The second mod answered: the screen is its own, not the passer's.
        assert_eq!(game.mod_screen(), Some("screens"));
        // One Opening, for the screen asked for: the replacement does not fire it again.
        assert_eq!(take(log), ["opening paused->options", "closing paused"]);
        *answer.lock().unwrap() = None;
        game.close_mod_screen();
        assert_eq!(game.client.screen, Screen::Playing);
        assert_eq!(game.mod_screen(), None);
        assert_eq!(take(log), ["opening mod->playing", "closing mod"]);
    });
}

/// The creative screen keeps its two tab rows clear: one strip above the window
/// and one below it, each as wide as the window and a tab row deep (vanilla's
/// `CreativeModeInventoryScreen` tab buttons, 28 px outside the 195x136 window).
#[test]
#[ignore = "daily"]
fn the_creative_screen_excludes_its_two_tab_rows() {
    with_mods(Vec::new().into(), || {
        crate::game::native_entity_refs::tests::with_client_game(|game| {
            game.client.screen = Screen::Creative;
            let batch = HudBatch::scratch([854, 480]);
            let image = game.container_image(&batch);
            let (x, y) = (image.left as f32, image.top as f32);
            let areas = game.screen_own_exclusion(&batch);
            assert_eq!(areas[0], [x, y - 28.0, 195.0, 28.0]);
            assert_eq!(areas[1], [x, y + 136.0, 195.0, 28.0]);
        });
    });
}

/// The title appearing (a world closed) and going (a world opened) fires
/// `screen_opening`, then `screen_closing` for what it leaves, before the
/// change, so the frame's observer has nothing left to announce. With no
/// world, `Cancel` doesn't keep the title shut.
#[test]
#[ignore = "daily"]
fn the_title_appears_and_goes_through_screen_opening() {
    run(|game, log, answer| {
        let saves = std::env::temp_dir().join("sunrise-title-gate-test-no-saves");
        game.client.title.0 = None;
        *answer.lock().unwrap() = Some(ScreenOpening::Cancel);
        game.show_title(game.new_title(saves));
        assert!(game.client.title.0.is_some());
        assert_eq!(take(log), ["opening playing->title"]);
        game.observe_screen();
        assert!(take(log).is_empty());
        *answer.lock().unwrap() = None;
        game.hide_title();
        assert!(game.client.title.0.is_none());
        assert_eq!(take(log), ["opening title->playing", "closing title"]);
        game.observe_screen();
        assert!(take(log).is_empty());
    });
}
