#![cfg(test)]

#[cfg(test)]
mod tests {

    use mod_api::CollisionExclusionsError as Refusal;
    use sunrise_core::position::BlockPos;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;

    /// A portal's opening, 1 wide and 2 tall.
    const OPENING: [[f64; 6]; 1] = [[2.0, 64.0, 5.0, 3.0, 66.0, 6.0]];

    /// 65 boxes with a distinct opening at the 65th slot.
    fn sixty_five_with_opening() -> [[f64; 6]; 65] {
        let mut boxes = [[0.0, 0.0, 0.0, 1.0, 1.0, 1.0]; 65];
        boxes[64] = OPENING[0];
        boxes
    }

    fn guest_boxes(game: &sunrise_gameplay::game::ServerGame<'_>, conn: u32) -> usize {
        let host = game.server.lan.as_ref().unwrap();
        host.hub
            .guest(conn)
            .unwrap()
            .collision_exclusions
            .boxes()
            .len()
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_mod_sets_a_guests_exclusions_and_a_refused_set_keeps_them() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let entity = game.player(12).unwrap().entity();
            assert_eq!(entity.set_collision_exclusions(game, &OPENING), Ok(()));
            assert_eq!(guest_boxes(game, 12), 1);
            assert_eq!(guest_boxes(game, 13), 0, "only that player's");
            let nan = [[0.0, 64.0, 0.0, 1.0, f64::NAN, 1.0]];
            let inverted = [[3.0, 64.0, 5.0, 2.0, 66.0, 6.0]];
            for (boxes, why) in [
                (&nan[..], Refusal::NotFinite),
                (&inverted[..], Refusal::Inverted),
            ] {
                assert_eq!(entity.set_collision_exclusions(game, boxes), Err(why));
                assert_eq!(guest_boxes(game, 12), 1, "{why:?} keeps the opening");
            }
            let sixty_five = sixty_five_with_opening();
            assert_eq!(entity.set_collision_exclusions(game, &sixty_five), Ok(()));
            assert_eq!(guest_boxes(game, 12), 65);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
            assert!(guest.collision_exclusions.spares(BlockPos::new(2, 64, 5)));
            assert!(!guest.collision_exclusions.spares(BlockPos::new(2, 66, 5)));
            assert_eq!(
                entity.set_collision_exclusions(game, &nan),
                Err(Refusal::NotFinite)
            );
            assert_eq!(
                guest_boxes(game, 12),
                65,
                "malformed write keeps the 65 boxes"
            );
            // The guest's body takes them for its turn, and a set made during the turn is the
            // body's at once.
            let in_turn = game.as_guest(12, |game| {
                let before = game
                    .player
                    .as_ref()
                    .unwrap()
                    .collision_exclusions
                    .boxes()
                    .len();
                assert_eq!(entity.set_collision_exclusions(game, &[]), Ok(()));
                let after = game
                    .player
                    .as_ref()
                    .unwrap()
                    .collision_exclusions
                    .boxes()
                    .len();
                (before, after)
            });
            assert_eq!(in_turn, Some((65, 0)));
            assert_eq!(guest_boxes(game, 12), 0);
            // A guest that left is gone with its boxes: its handle names nothing.
            assert_eq!(entity.set_collision_exclusions(game, &OPENING), Ok(()));
            let i = game.guest_index(12).unwrap();
            game.server.lan.as_mut().unwrap().hub.guests.remove(i);
            assert_eq!(
                entity.set_collision_exclusions(game, &OPENING),
                Err(Refusal::Stale)
            );
        });
    }

    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn a_client_sets_its_own_players_and_a_respawn_or_another_level_clears_them() {
        use sunrise_protocol::packets::play::{self, Clientbound as C};
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let (pipe, _server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(pipe, "Steve").unwrap();
            let spawn = |dimension: &str| play::SpawnInfo {
                dimension_type: 0,
                dimension: dimension.into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            };
            game.handle_packet(C::Login(play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: 8,
                simulation_distance: 8,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: spawn("minecraft:overworld"),
                online_mode: false,
                enforces_secure_chat: false,
            }))
            .unwrap();
            let position = |teleport| {
                C::PlayerPosition(play::PlayerPosition {
                    teleport,
                    position: [0.5, 65.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                })
            };
            game.handle_packet(position(1)).unwrap();
            let held = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
                let player = game.player.as_ref().unwrap();
                player.collision_exclusions.boxes().len()
            };
            assert_eq!(game.set_collision_exclusions(&OPENING), Ok(()));
            let nan = [[0.0, 64.0, 0.0, 1.0, f64::NAN, 1.0]];
            let inverted = [[3.0, 64.0, 5.0, 2.0, 66.0, 6.0]];
            assert_eq!(game.set_collision_exclusions(&nan), Err(Refusal::NotFinite));
            assert_eq!(
                game.set_collision_exclusions(&inverted),
                Err(Refusal::Inverted)
            );
            assert_eq!(held(game), 1, "the refused set keeps the opening");
            let sixty_five = sixty_five_with_opening();
            assert_eq!(game.set_collision_exclusions(&sixty_five), Ok(()));
            assert_eq!(held(game), 65);
            let player = game.player.as_ref().unwrap();
            assert!(player.collision_exclusions.spares(BlockPos::new(2, 64, 5)));
            assert!(!player.collision_exclusions.spares(BlockPos::new(2, 66, 5)));
            assert_eq!(game.set_collision_exclusions(&nan), Err(Refusal::NotFinite));
            assert_eq!(held(game), 65, "malformed write keeps the 65 boxes");
            // A death's Respawn, in the same level.
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn("minecraft:overworld"),
                keep: 0,
            }))
            .unwrap();
            game.handle_packet(position(2)).unwrap();
            assert_eq!(held(game), 0, "a respawn clears them");
            // A portal's Respawn into another level, keeping all data.
            assert_eq!(game.set_collision_exclusions(&OPENING), Ok(()));
            game.handle_packet(C::Respawn(play::Respawn {
                spawn: spawn("minecraft:the_nether"),
                keep: 3,
            }))
            .unwrap();
            game.handle_packet(position(3)).unwrap();
            assert_eq!(held(game), 0, "another level clears them");
        });
    }
}
