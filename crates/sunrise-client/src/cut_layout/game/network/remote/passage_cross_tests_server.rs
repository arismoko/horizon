#![cfg(feature = "client")]
use crate::game::remote::passage_cross_tests::scene;
use crate::game::remote::passage_cross_tests::sent;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{PassageCrossError, PassageId};
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S, relative};
use sunrise_protocol::passage_sync::{Cross, decode_cross};

/// The player walks south into the doorway while digging: it comes out 24 blocks east,
/// facing and moving west, its last tick's place carried with it, and the server is told
/// the passage and the feet before. The server's acceptance, all relative and zero, leaves
/// position and velocity as they are, is confirmed with where the player is, aborts the
/// dig, starts the attack strength again and marks the block predictions made so far.
#[test]
#[ignore = "daily"]
fn a_window_crosses_and_keeps_its_velocity_through_the_acceptance() {
    with_client_game(|game| {
        let (mut reader, _server) = scene(game, [0.5, 65.0, 8.25]);
        let player = game.player.as_mut().unwrap();
        player.velocity = Vec3::new(0.0, -0.0784, 0.25);
        game.previous_position = Vec3::new(0.5, 65.0, 8.0);

        assert_eq!(game.cross_passage(PassageId::from_raw(1)), Ok(()));
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(24.75, 65.0, 8.5));
        assert_eq!(player.velocity, Vec3::new(-0.25, -0.0784, 0.0));
        assert_eq!(game.previous_position, Vec3::new(25.0, 65.0, 8.5));
        assert_eq!(game.yaw, 90.0);
        let crossed: Vec<_> = sent(&mut reader)
            .into_iter()
            .filter_map(|p| match p {
                S::CustomPayload(p) if p.channel == "sunrise:passage_cross" => {
                    Some(decode_cross(&p.data).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            crossed,
            [Cross {
                id: 1,
                from: [0.5, 65.0, 8.25]
            }]
        );
        assert_eq!(
            game.cross_passage(PassageId::from_raw(1)),
            Err(PassageCrossError::Pending)
        );

        // Digging a block, with predictions up to sequence 7, when the acceptance comes.
        let r = game.client.remote.as_mut().unwrap();
        r.is_destroying = true;
        r.destroying = Some(([3, 64, 3], 1, 0.5));
        r.sequence = 7;
        game.test_set_hand_strength_ticker(15);
        let all = relative::X
            | relative::Y
            | relative::Z
            | relative::Y_ROT
            | relative::X_ROT
            | relative::DELTA_X
            | relative::DELTA_Y
            | relative::DELTA_Z
            | relative::ROTATE_DELTA;
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 2,
            position: [0.0; 3],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: all,
        }))
        .unwrap();
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, Vec3::new(24.75, 65.0, 8.5));
        assert_eq!(player.velocity, Vec3::new(-0.25, -0.0784, 0.0));
        assert_eq!((game.yaw, game.pitch), (90.0, 0.0));
        let r = game.client.remote.as_ref().unwrap();
        assert!(!r.is_destroying);
        assert_eq!(r.destroying, Some(([3, 64, 3], 1, 0.0)));
        assert_eq!(r.last_teleport_sequence, 7);
        assert_eq!(game.test_hand_strength_ticker(), 0);
        let answers = sent(&mut reader);
        assert!(answers.iter().any(|p| matches!(
            p,
            S::AcceptTeleportation(a) if a.id == 2 && [a.x, a.y, a.z] == [24.75, 65.0, 8.5]
        )));
        assert!(answers.iter().any(|p| matches!(
            p,
            S::PlayerAction(a) if a.action == 2 && a.pos == [3, 64, 3]
        )));
        // Answered, the next crossing is the window's to try: this one is too far now.
        assert_eq!(
            game.cross_passage(PassageId::from_raw(1)),
            Err(PassageCrossError::TooFar { limit: 20 })
        );
    });
}
