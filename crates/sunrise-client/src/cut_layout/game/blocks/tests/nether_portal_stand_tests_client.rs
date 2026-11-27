use sunrise_gameplay_common::game::Screen;

#[test]
#[ignore = "daily"]
fn the_portal_effect_keeps_the_screens_that_allow_it() {
    crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
        // `Screen.isAllowedInPortal` is `isPauseScreen` for these: the pause menu,
        // the options over it and the chat stay open while the effect grows.
        for screen in [Screen::Paused, Screen::Options, Screen::Chat] {
            game.client.screen = screen;
            game.client.client_inside = true;
            assert!(game.client_portal_effect(), "the effect grows");
            assert!(game.client.screen == screen, "{} stayed open", screen.id());
        }
        // A container screen does not: `Gui.setScreen(null)` closes it.
        game.client.screen = Screen::Inventory;
        game.client.client_inside = true;
        assert!(game.client_portal_effect(), "the effect grows");
        assert!(
            game.client.screen == Screen::Playing,
            "the inventory closed in the portal"
        );
        // And no screen at all is nothing to close.
        game.client.client_inside = true;
        assert!(game.client_portal_effect(), "the effect grows");
        assert!(
            game.client.screen == Screen::Playing,
            "playing stays playing"
        );
    });
}
