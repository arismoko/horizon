//! #1999: closing the options screens opened from the title grabs the pointer
//! only when no title is shown (`MouseHandler.grabMouse` runs on
//! `setScreen(null)`, and `OptionsScreen.onClose` from `TitleScreen` returns to
//! the title). The fixture's game has no window, so `capture(true)`'s trace is
//! that the real pointer takes over from an agent's: `agent_pointer` clears.
use sunrise_gameplay_common::game::Screen;

/// The title's Options page, closed: back under the title, which is still
/// shown, with the pointer left free.
#[test]
#[ignore = "daily"]
fn closing_options_opened_from_the_title_leaves_the_pointer_free() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        let saves = tempfile::tempdir().unwrap();
        game.client.title.0 = Some(game.new_title(saves.path().to_path_buf()));
        game.client.focused = true;
        game.open_options(true);
        game.client.agent_pointer = true;
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(game.client.title.0.is_some());
        assert!(game.client.agent_pointer, "the title is shown: no grab");
    });
}

/// The startup handoff entered a world behind its open options: closing them
/// returns to that world and grabs the pointer, as before.
#[test]
#[ignore = "daily"]
fn closing_title_options_into_a_world_entered_behind_them_grabs() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.title.0 = None;
        game.client.focused = true;
        game.open_options(true);
        game.client.agent_pointer = true;
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Playing);
        assert!(
            !game.client.agent_pointer,
            "a world is shown: the pointer is grabbed"
        );
    });
}

/// The pause menu's options, closed: back to the pause menu, a screen over
/// the world that keeps the pointer free, as before.
#[test]
#[ignore = "daily"]
fn closing_pause_menu_options_leaves_the_pointer_free() {
    crate::game::native_entity_refs::tests::with_client_game(|game| {
        game.client.title.0 = None;
        game.client.focused = true;
        game.open_options(false);
        game.client.agent_pointer = true;
        game.leave_options();
        assert_eq!(game.client.screen, Screen::Paused);
        assert!(
            game.client.agent_pointer,
            "the pause menu keeps the pointer free"
        );
    });
}
