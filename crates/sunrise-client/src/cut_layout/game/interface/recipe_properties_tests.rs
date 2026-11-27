//! Connection lifecycle of the authoritative recipe property generation.
#![cfg(test)]
#[cfg(test)]
mod tests {
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use sunrise_protocol::{connection::Pipe, packets::recipes::UpdateRecipes};

    #[test]
    #[ignore = "daily"]
    fn disconnect_discards_properties_before_reconnect() {
        with_client_game(|game| {
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PropertiesA").unwrap();
            game.recipe_tables.server_properties.replace(UpdateRecipes {
                properties: vec![("first:input".into(), vec![7])],
                stonecutter: vec![],
            });
            // Closing a menu is not leaving the connection.
            game.client.recipe_ui.0.reset_screen();
            assert!(
                game.recipe_tables
                    .server_properties
                    .sets
                    .as_ref()
                    .unwrap()
                    .contains_key("first:input")
            );
            game.client_logging_out();
            assert!(game.recipe_tables.server_properties.sets.is_none());
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PropertiesB").unwrap();
            assert!(game.recipe_tables.server_properties.sets.is_none());
            game.recipe_tables
                .server_properties
                .replace(UpdateRecipes::default());
            assert!(
                game.recipe_tables
                    .server_properties
                    .sets
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        });
    }
}
