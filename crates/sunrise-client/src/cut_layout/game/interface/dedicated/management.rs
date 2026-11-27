#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use serde_json::json;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn management_online_skips_fake_players_by_name_and_id() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            game.fake_player(level, 100, "robot").unwrap();
            assert_eq!(game.management_online(&json!({"name": "robot"})), None);
            assert_eq!(game.management_online(&json!({"id": "64"})), None);
            assert_eq!(
                game.management_online(&json!({"name": "GUEST12"})),
                Some((12, "127.0.0.1".into()))
            );
            // Id takes precedence, with no fallback from a fake id to a real name.
            assert_eq!(
                game.management_online(&json!({"id": "c", "name": "robot"})),
                Some((12, "127.0.0.1".into()))
            );
            assert_eq!(
                game.management_online(&json!({"id": "64", "name": "guest12"})),
                None
            );
        });
    }
}
