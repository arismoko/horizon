#![cfg(test)]
#[cfg(test)]
pub(crate) mod fake_travel_tests {
    use sunrise_gameplay_common::game::Dim;
    use sunrise_gameplay_common::game::commands_common::Target;

    #[test]
    #[ignore = "daily"]
    fn a_fake_player_is_not_teleported_to_another_dimension() {
        super::super::super::native_entity_refs::tests::with_game(|game| {
            sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
            let host = game.server.lan.as_ref().unwrap();
            let conn = |uuid| {
                host.hub
                    .guests
                    .iter()
                    .find(|g| g.uuid == uuid)
                    .unwrap()
                    .conn
            };
            let (fake, real) = (conn(2), conn(1));
            let to_nether = |game: &mut sunrise_gameplay::game::ServerGame<'_>, conn| {
                game.as_guest(conn, |g| {
                    g.teleport_across(
                        Target::Player,
                        Dim::Nether,
                        [0.0, 64.0, 0.0],
                        None,
                        false,
                        sunrise_gameplay::game::server_side::vehicle_teleport::TeleportMotion::Stop,
                    )
                })
            };
            // Taken (true), so no same-level teleport stands in for it, and nothing queued.
            assert!(matches!(to_nether(game, fake), Some(Ok(true))));
            assert!(game.server.level_teleports.is_empty());
            // A real player's goes on the queue as before.
            assert!(matches!(to_nether(game, real), Some(Ok(true))));
            assert_eq!(game.server.level_teleports.len(), 1);
        });
    }

    /// A mod's `teleport_to_level`: a player to another level goes on the queue; to their own
    /// level, nothing is queued; an entity no level holds is refused.
    #[test]
    #[ignore = "daily"]
    fn a_mod_sends_a_player_to_another_level_and_not_a_missing_entity() {
        super::super::super::native_entity_refs::tests::with_game(|game| {
            sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
            let conn = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guests
                .iter()
                .find(|g| g.uuid == 1)
                .unwrap()
                .conn;
            let sent = |game: &mut sunrise_gameplay::game::ServerGame<'_>, level| {
                game.as_guest(conn, |g| {
                    g.teleport_to_level(Target::Player, level, [0.0, 64.0, 0.0], None)
                })
            };
            assert_eq!(sent(game, Dim::Nether), Some(true));
            assert_eq!(game.server.level_teleports.len(), 1);
            // Their own level: the same-level teleport, nothing queued.
            let home = game.target_level(Target::Guest(conn));
            assert_eq!(sent(game, home), Some(true));
            assert_eq!(game.server.level_teleports.len(), 1);
            // No level holds this item.
            assert!(!game.teleport_to_level(Target::Item(9_999), Dim::Nether, [0.0; 3], None));
        });
    }
}
