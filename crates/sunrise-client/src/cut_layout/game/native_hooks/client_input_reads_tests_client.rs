#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {
    use mod_api::MappingError;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::motion::PlayerMotion;
    use winit::{event::MouseButton, keyboard::KeyCode};
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay_common::game::Screen,
    };

    #[test]
    #[ignore = "daily"]
    fn mapping_physical_down_follows_the_binding_and_ignores_the_logical_state() {
        with_client_game(|game| {
            let forward_mapping = game.vanilla_key_mapping("key.forward").unwrap();
            let attack_mapping = game.vanilla_key_mapping("key.attack").unwrap();
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            assert_eq!(
                game.vanilla_key_mapping("key.not_a_mapping"),
                Err(MappingError::Unknown {
                    key: "key.not_a_mapping".into(),
                }),
            );
            assert_eq!(forward_mapping.physical_down(game), Ok(false));
            game.client.physical_keys.insert(KeyCode::KeyW);
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // A write to the logical mapping does not move the physical read.
            game.insert_device_key(KeyCode::KeyW);
            assert_eq!(forward_mapping.is_down(game), Ok(true));
            forward_mapping.set_down(game, false).unwrap();
            assert!(!forward_mapping.is_down(game).unwrap());
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // Rebound, it reads the new key, and the old one no longer counts.
            game.client
                .settings
                .keys
                .set("key.forward", "key.keyboard.up");
            assert_eq!(forward_mapping.physical_down(game), Ok(false));
            game.client.physical_keys.insert(KeyCode::ArrowUp);
            assert_eq!(forward_mapping.physical_down(game), Ok(true));
            // A mapping on a mouse button reads the button, by name too.
            assert_eq!(attack_mapping.physical_down(game), Ok(false));
            game.client.physical_buttons.insert(MouseButton::Left);
            assert_eq!(attack_mapping.physical_down(game), Ok(true));
            assert!(game.is_key_down("key.mouse.left"));
            assert!(!game.is_key_down("key.mouse.right"));
            game.client.physical_buttons.clear();
            assert!(!game.is_key_down("key.mouse.left"));
            // A mapping bound to no key is never down.
            game.client
                .settings
                .keys
                .set("key.attack", "key.keyboard.unknown");
            game.client.physical_buttons.insert(MouseButton::Left);
            assert_eq!(attack_mapping.physical_down(game), Ok(false));

            // Logical state is tracked uniformly for every resolved vanilla mapping,
            // independent of its physical key state.
            game.client.screen = Screen::Playing;
            game.client.title.0 = None;
            let chat = game.vanilla_key_mapping("key.chat").unwrap();
            game.client.physical_keys.insert(KeyCode::KeyT);
            assert_eq!(chat.physical_down(game), Ok(true));
            assert_eq!(chat.is_down(game), Ok(false));
            assert_eq!(chat.set_down(game, true), Ok(()));
            assert_eq!(chat.is_down(game), Ok(true));
            assert_eq!(chat.set_down(game, false), Ok(()));
            assert_eq!(chat.is_down(game), Ok(false));
            assert_eq!(chat.physical_down(game), Ok(true));

            let pick = game.vanilla_key_mapping("key.pickItem").unwrap();
            assert_eq!(pick.is_down(game), Ok(false));
            assert_eq!(pick.set_down(game, true), Ok(()));
            assert_eq!(pick.is_down(game), Ok(true));
            assert_eq!(pick.set_down(game, false), Ok(()));
            assert_eq!(pick.is_down(game), Ok(false));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stale_vanilla_mapping_ref_refuses_foreign_client() {
        with_client_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.client.screen = Screen::Playing;
            game.client.title.0 = None;
            let old = game.vanilla_key_mapping("key.attack").unwrap();
            assert_eq!(old.binding(game), Ok("key.mouse.left"));

            // A new client input store resolves the same fixed vanilla slot with a new identity.
            game.client.mod_keys = sunrise_client::keymapping::ModKeys::default();
            game.client
                .settings
                .keys
                .set("key.attack", "key.keyboard.r");
            assert_eq!(old.binding(game), Err(MappingError::Stale));
            assert_eq!(old.is_down(game), Err(MappingError::Stale));
            assert_eq!(old.physical_down(game), Err(MappingError::Stale));
            assert_eq!(old.translated_key_message(game), Err(MappingError::Stale));
            assert_eq!(old.set_down(game, true), Err(MappingError::Stale));
            assert_eq!(old.click(game), Err(MappingError::Stale));

            let current = game.vanilla_key_mapping("key.attack").unwrap();
            assert_eq!(current.binding(game), Ok("key.keyboard.r"));
            assert_eq!(current.is_down(game), Ok(false));
            assert_eq!(game.pending_mapping_clicks("key.attack"), 0);
            // A live handle can still write with the same player.
            current.set_down(game, true).unwrap();
            current.click(game).unwrap();
            assert_eq!(current.is_down(game), Ok(true));
            assert_eq!(game.pending_mapping_clicks("key.attack"), 1);
        });
    }
}
