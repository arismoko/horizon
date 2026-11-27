#![cfg(test)]
#[cfg(test)]
pub(crate) mod customize_chat_overlay_tests {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_client_gameplay::game::native_hooks::customize_chat_overlay::customize_chat_overlay_tests::Lift;
    use sunrise_client_gameplay::game::native_hooks::customize_chat_overlay::customize_chat_overlay_tests::Seen;
    use sunrise_client_gameplay::game::native_hooks::customize_chat_overlay::customize_chat_overlay_tests::chat_top;
    use mod_api::ModEntry;
    use mod_platform::testing::with_mods;
    use std::sync::Arc;
    use sunrise_client::hud::HudBatch;
    use sunrise_game_data::text::Text;

    /// `CustomizeGuiOverlayEvent.Chat`: the mod gets vanilla's place (0, GUI height - 40) and the
    /// GUI size, focused or not, and the chat is drawn where it moved it. With the chat hidden
    /// (F1, chat closed) nothing is asked.
    #[test]
    #[ignore = "daily"]
    fn a_client_mod_moves_the_chat() {
        let message = Text::literal("hello from the chat test");
        let mut plain = None;
        with_client_game(|game| {
            game.client_system_message(message.clone());
            plain = chat_top(game).0;
        });
        let plain = plain.expect("the message is drawn");
        let seen = Seen::default();
        let mods: mod_api::Mods =
            vec![ModEntry::new("lift", Box::new(Lift(Arc::clone(&seen))))].into();
        with_mods(mods, || {
            with_client_game(|game| {
                game.client_system_message(message.clone());
                let (moved, scale) = chat_top(game);
                let moved = moved.expect("the message is drawn");
                assert!(
                    (plain - moved - 100.0).abs() < 0.5,
                    "lifted by 100 GUI pixels: {plain} -> {moved} at scale {scale}"
                );
                let (w, h) = {
                    let b = HudBatch::scratch([1280, 720]);
                    (b.width as i32, b.height as i32)
                };
                assert_eq!(*seen.lock().unwrap(), [(w, h, false, 0, h - 40)]);
                // Hidden: F1 with the chat closed asks nothing.
                game.client.chat_ui.hide_gui = true;
                chat_top(game);
                assert_eq!(seen.lock().unwrap().len(), 1);
                // Focused, the chat shows under F1 and is asked again.
                game.client.screen = sunrise_gameplay_common::game::Screen::Chat;
                chat_top(game);
                let seen = seen.lock().unwrap();
                assert_eq!(seen.len(), 2);
                assert!(seen[1].2, "focused");
            });
        });
    }
}
