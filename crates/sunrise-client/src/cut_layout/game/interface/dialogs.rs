#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
#[cfg(any())]
mod warning_tests {
    use sunrise_protocol::connection::Pipe;
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_client_gameplay::game::client_side::integrated_client::Integrated,
        sunrise_gameplay::game::server_side::integrated::Serving,
        sunrise_gameplay_common::game::in_own_process,
    };

    #[test]
    #[ignore = "daily"]
    fn warning_disconnect_label_follows_server_ownership() {
        in_own_process(
            "cut_layout::game::client_side::dialogs::warning_tests::warning_disconnect_label_follows_server_ownership",
            || {
                with_client_game(|game| {
                    let (client, _server) = Pipe::pair();
                    game.connect_local(client, "DialogWarning").unwrap();

                    let (_, _, [disconnect, back]) = game.warning_texts();
                    assert_eq!(disconnect.translation_key(), Some("menu.disconnect"));
                    assert_eq!(back.translation_key(), Some("gui.back"));

                    // The same client connection backs singleplayer, even after publishing to LAN.
                    for port in [0, 25565] {
                        let serving = Serving::fixture(port);
                        game.client.integrated = Some(Integrated::fixture_for(&serving));
                        let (_, _, [disconnect, back]) = game.warning_texts();
                        assert_eq!(disconnect.translation_key(), Some("menu.returnToMenu"));
                        assert_eq!(back.translation_key(), Some("gui.back"));
                    }
                });
            },
        );
    }
}
