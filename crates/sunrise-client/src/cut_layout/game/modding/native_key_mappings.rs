#![cfg(test)]
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn opening_and_closing_a_direct_screen_discards_pending_clicks() {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.client.title.0 = None;
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
            game.client
                .mod_keys
                .bind([sunrise_client::keymapping::ModBinding {
                    owner: "test",
                    id: "pending",
                    input: "key.keyboard.k".into(),
                    modifiers: 0,
                    context: sunrise_client::keymapping::MappingContext::InGame,
                }]);
            let pending = game.mod_key_mapping("test", "pending").unwrap();
            game.client.mod_keys.input(
                "key.keyboard.k",
                sunrise_client::keymapping::KeyAction::Press,
                false,
                false,
                0,
            );
            game.open_hud_editor(false);
            assert!(matches!(game.client.screen, crate::game::Screen::HudEditor));
            game.close_hud_editor();
            assert!(!pending.consume_click(game).unwrap());
        });
    }
}
