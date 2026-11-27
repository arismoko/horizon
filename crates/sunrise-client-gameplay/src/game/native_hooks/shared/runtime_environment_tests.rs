#[test]
#[ignore = "daily"]
fn client_host_reads_current_pause_state() {
    use super::super::native_entity_refs::tests::with_client_game;

    with_client_game(|game| {
        game.pause = false;
        assert!(!mod_api::prelude::ClientViewClientHost::is_paused(game));
        game.pause = true;
        assert!(mod_api::prelude::ClientViewClientHost::is_paused(game));
    });
}

#[test]
fn observed_overlay_prefers_first_open_screen() {
    use crate::game::client_side::native_hooks_client::observed_overlay_id;

    assert_eq!(
        observed_overlay_id(&[(false, "sign"), (false, "book")]),
        None
    );
    assert_eq!(
        observed_overlay_id(&[(false, "sign"), (true, "book")]),
        Some("book")
    );
    assert_eq!(
        observed_overlay_id(&[(true, "sign"), (true, "book")]),
        Some("sign")
    );
}

/// A mod's own screen refuses only when the client window is unavailable (no
/// renderer, acting for a guest, neither a player nor a title screen, or
/// loading); when available, opening the same mod's already-visible screen is a
/// no-op (`Some(true)`), and opening over playing, another vanilla screen, an
/// overlay (`owner: None`), or another mod's screen proceeds (`None`).
#[test]
fn a_mod_screen_opens_when_client_window_is_available() {
    use crate::game::client_side::native_hooks_client::mod_screen_opens;
    use sunrise_gameplay_common::game::native_hooks::Screen;
    assert_eq!(mod_screen_opens(Screen::Playing, false, None, "a"), None);
    for other in [
        Screen::Paused,
        Screen::Inventory,
        Screen::Creative,
        Screen::Chat,
        Screen::Options,
    ] {
        assert_eq!(mod_screen_opens(other, false, None, "a"), None);
    }
    assert_eq!(
        mod_screen_opens(Screen::Mod, false, Some("a"), "a"),
        Some(true)
    );
    assert_eq!(mod_screen_opens(Screen::Mod, false, Some("b"), "a"), None);
    assert_eq!(mod_screen_opens(Screen::Mod, false, None, "a"), None);
    for screen in [Screen::Playing, Screen::Paused, Screen::Mod] {
        assert_eq!(mod_screen_opens(screen, true, Some("a"), "a"), Some(false));
    }
}

/// A mod container opens a slot grid or a furnace's menu, by its slot
/// count; no menu with rules of its own (a brewing stand's, a crafter's).
#[test]
fn mod_menus_are_slot_grids_and_furnaces() {
    use sunrise_gameplay_common::game::native_hooks::mod_menu;
    assert_eq!(mod_menu("minecraft:hopper"), Some(("minecraft:hopper", 5)));
    assert_eq!(mod_menu("minecraft:generic_9x3").map(|m| m.1), Some(27));
    assert_eq!(mod_menu("minecraft:furnace").map(|m| m.1), Some(3));
    assert_eq!(mod_menu("minecraft:brewing_stand"), None);
    assert_eq!(mod_menu("hopper"), None);
}

#[test]
fn block_containers_cannot_bind_an_owned_menu_type() {
    let menu = |id: &str, limits| {
        (
            id.into(),
            "test".into(),
            1,
            0,
            vec![],
            vec![],
            None,
            None,
            true,
            limits,
        )
    };
    sunrise_level::containers::with_mod_menu_types(
        [
            menu("test:block", None),
            menu("test:transient", Some(vec![1])),
        ],
        || {
            assert_eq!(super::mod_menu("test:block"), Some(("test:block", 1)));
            assert_eq!(super::mod_menu("test:transient"), None);
        },
    );
}

/// The config folder is `config/` beside `options.txt`, made when
/// missing.
#[test]
fn config_dir_is_config_in_the_game_folder() {
    use sunrise_gameplay_common::game::native_hooks::config_dir;
    let root = std::env::temp_dir().join(format!("sunrise-config-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    assert!(!root.join("config").exists());
    let dir = config_dir(&root).unwrap();
    assert_eq!(dir, root.join("config"));
    assert!(dir.is_dir());
    assert_eq!(config_dir(&root).unwrap(), dir);
    let _ = std::fs::remove_dir_all(&root);
}

/// Vanilla's F3 draws a column in each top corner (`extractLines`, left
/// and right aligned), and its charts along the bottom.
#[test]
fn the_debug_screen_takes_both_top_corners_and_its_charts_corners() {
    use crate::game::client_side::native_hooks_client::debug_takes;
    use sunrise_client::extras::Corner;
    for corner in Corner::ALL {
        assert!(!debug_takes(corner, false, [true, true]), "{corner:?}");
    }
    assert!(debug_takes(Corner::TopLeft, true, [false, false]));
    assert!(debug_takes(Corner::TopRight, true, [false, false]));
    assert!(!debug_takes(Corner::BottomLeft, true, [false, false]));
    assert!(!debug_takes(Corner::BottomRight, true, [false, false]));
    assert!(debug_takes(Corner::BottomLeft, true, [true, false]));
    assert!(!debug_takes(Corner::BottomRight, true, [true, false]));
    assert!(debug_takes(Corner::BottomRight, true, [false, true]));
}
