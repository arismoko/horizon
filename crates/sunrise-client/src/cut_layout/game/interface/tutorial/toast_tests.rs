use sunrise_client_gameplay::game::toast_manager::Toast;
use sunrise_client_gameplay::tutorial::Hint;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::Screen, sunrise_gameplay_common::game::in_own_process,
};

#[test]
#[ignore = "daily"]
fn tutorial_toast_bolds_only_the_rebound_key() {
    in_own_process(
        "cut_layout::game::client_side::tutorial::toast_tests::tutorial_toast_bolds_only_the_rebound_key",
        || {
            with_client_game(|game| {
                game.client
                    .settings
                    .keys
                    .set("key.inventory", "key.keyboard.z");
                game.client
                    .settings
                    .keys
                    .set("key.socialInteractions", "key.keyboard.x");
                for (hint, key) in [(Hint::OpenInventory, 'Z'), (Hint::SocialInteractions, 'X')] {
                    let toast = game.tutorial_toast(hint, None).unwrap();
                    let spans = toast.lines[toast.title_lines..]
                        .iter()
                        .flat_map(|(line, _)| &line.spans);
                    let mut highlighted = String::new();
                    let mut surrounding = String::new();
                    for span in spans {
                        if span.style.bold == Some(true) {
                            assert_ne!(span.style.italic, Some(true));
                            assert_eq!(span.style.color, None);
                            highlighted.push_str(&span.text);
                        } else {
                            surrounding.push_str(&span.text);
                        }
                    }
                    assert_eq!(highlighted, key.to_string());
                    assert!(surrounding.chars().any(char::is_alphabetic));
                }
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn social_toast_is_dismissed_only_by_the_multiplayer_key() {
    in_own_process(
        "cut_layout::game::client_side::tutorial::toast_tests::social_toast_is_dismissed_only_by_the_multiplayer_key",
        || {
            with_client_game(|game| {
                let enqueue = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                    let toast = game
                        .tutorial_toast(Hint::SocialInteractions, Some(0.0))
                        .unwrap();
                    game.client
                        .toast_manager
                        .queued
                        .push_back(Toast::Tutorial(toast));
                    game.client
                        .toast_manager
                        .update(sunrise_client::toasts::ToastClock::Real, 1.0);
                };
                let wanted = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                    let Toast::Tutorial(toast) = &game.client.toast_manager.visible[0].toast else {
                        panic!("expected social toast");
                    };
                    toast.wanted
                };
                game.set_screen(Screen::Playing);
                enqueue(game);
                game.queue_mapping_click("key.socialInteractions");
                game.handle_play_keybinds();
                assert!(game.client.screen == Screen::Playing);
                assert!(wanted(game), "the unavailable key did not open the screen");
                assert_eq!(
                    game.client
                        .titles
                        .overlay
                        .as_ref()
                        .unwrap()
                        .translation_key(),
                    Some("multiplayer.socialInteractions.not_available")
                );

                game.init_remote_for_replay().unwrap();
                game.set_screen(Screen::Paused);
                enqueue(game);
                game.execute_pause_action(15).unwrap();
                assert!(game.client.screen == Screen::Social);
                assert!(wanted(game), "the pause menu must leave the toast alone");
                game.close_social();
                assert!(game.client.screen == Screen::Paused);

                game.set_screen(Screen::Playing);
                game.queue_mapping_click("key.socialInteractions");
                game.handle_play_keybinds();
                assert!(game.client.screen == Screen::Social);
                assert!(!wanted(game), "the multiplayer key must hide the toast");
            });
        },
    );
}
