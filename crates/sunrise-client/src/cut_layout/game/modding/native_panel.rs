#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_client_gameplay::game::client_side::native_panel::tests::frame;

    use sunrise_client_gameplay::game::client_side::native_panel::tests::search_as;

    use sunrise_client_gameplay::game::client_side::native_panel::tests::HEARD;

    use sunrise_client_gameplay::game::client_side::native_panel::tests::Listener;

    use sunrise_client_gameplay::game::client_side::native_panel::tests::heard;

    use sunrise_client_gameplay::game::client_side::native_panel::tests::stack;

    /// `panel_search` in a game: only the owner's own call counts, and the
    /// owner hears its text through the typing event on the first frame that
    /// draws its box, on the first page; never in the call, never from
    /// input before that frame, never on a frame without room for the box,
    /// and never in another mod's box.
    #[test]
    #[ignore = "daily"]
    fn a_queued_search_is_heard_by_its_owner_when_its_box_draws() {
        use sunrise_gameplay_common::game::Screen;
        const WINDOW: [u32; 2] = [854, 480];
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("jei", Box::new(Listener))].into();
            mod_platform::testing::with_mods(mods, || {
                let many = vec![stack("minecraft:stone"); 300];
                game.panel_items("jei", many.clone());
                game.client.screen = Screen::Inventory;
                assert!(game.client.title.0.is_none());
                assert_eq!(frame(game, WINDOW), [], "nothing typed, nothing heard");

                // Another mod naming the owner's id changes nothing.
                search_as(game, "ae2", "iron");
                assert_eq!(game.client.panel.pending, None);
                // The owner's call is queued: the box and the page are as they were.
                game.client.panel.page = 3;
                search_as(game, "jei", "iron");
                assert_eq!(std::mem::take(&mut *HEARD.lock().unwrap()), []);
                assert_eq!(game.client.panel.search.as_ref().unwrap().value, "");

                // Closed, the panel doesn't draw and the text waits; reopened,
                // the owner hears it on the first page.
                game.client.screen = Screen::Playing;
                game.mod_browser_closed();
                assert_eq!(frame(game, WINDOW), []);
                game.client.screen = Screen::Inventory;
                assert_eq!(frame(game, WINDOW), heard("jei", "iron"));
                assert_eq!(game.client.panel.page, 0);
                assert_eq!(game.client.panel.search.as_ref().unwrap().value, "iron");
                assert_eq!(frame(game, WINDOW), [], "heard once");

                // A window too narrow for the panel draws no box: the text waits.
                search_as(game, "jei", "gold");
                assert_eq!(frame(game, [200, 300]), []);
                assert_eq!(game.client.panel.search.as_ref().unwrap().value, "iron");
                assert_eq!(frame(game, WINDOW), heard("jei", "gold"));

                // Another mod takes the panel before a frame: neither hears the
                // text, and the owner coming back doesn't get it either.
                search_as(game, "jei", "coal");
                game.panel_items("ae2", vec![stack("minecraft:dirt")]);
                assert_eq!(frame(game, WINDOW), []);
                game.panel_items("jei", many);
                assert_eq!(frame(game, WINDOW), []);
                assert_eq!(game.client.panel.search.as_ref().unwrap().value, "gold");

                // Input before the frame sends only what the player typed, and
                // that newer edit wins over the queued text.
                search_as(game, "jei", "dirt");
                game.mod_browser_text(Some("s"), false);
                assert_eq!(
                    std::mem::take(&mut *HEARD.lock().unwrap()),
                    heard("jei", "golds")
                );
                assert_eq!(frame(game, WINDOW), []);
                assert_eq!(game.client.panel.search.as_ref().unwrap().value, "golds");
            });
        });
    }
}
