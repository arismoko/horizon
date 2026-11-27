#[test]
#[ignore = "daily"]
fn client_host_reads_playing_and_key_mappings() {
    use super::super::native_entity_refs::tests::with_client_game;
    use mod_api::prelude::ClientInputClientHost;
    use mod_api::{MappingError, Text};
    use sunrise_gameplay_common::game::Screen;
    use winit::keyboard::KeyCode;

    with_client_game(|game| {
        game.client.screen = Screen::Playing;
        assert!(mod_api::prelude::ClientInputClientHost::playing(game));
        game.client.screen = Screen::Chat;
        assert!(!mod_api::prelude::ClientInputClientHost::playing(game));

        // Vanilla's default bindings: forward is W, attack the left mouse button.
        let forward = ClientInputClientHost::vanilla_key_mapping(game, "key.forward").unwrap();
        assert_eq!(forward.is_down(game), Ok(false));
        assert_eq!(forward.translated_key_message(game), Ok(Text::literal("W")));
        game.insert_device_key(KeyCode::KeyW);
        assert_eq!(forward.is_down(game), Ok(true));
        let attack = ClientInputClientHost::vanilla_key_mapping(game, "key.attack").unwrap();
        assert_eq!(
            attack.translated_key_message(game),
            Ok(Text::translate("key.mouse.left", Vec::new()))
        );
        assert_eq!(
            ClientInputClientHost::vanilla_key_mapping(game, "key.mymod.boost"),
            Err(MappingError::Unknown {
                key: "key.mymod.boost".into(),
            }),
        );
    });
}
