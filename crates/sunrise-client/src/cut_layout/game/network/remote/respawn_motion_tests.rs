//! A portal's `ClientboundRespawnPacket` keeps the entity data (`KEEP_ENTITY_DATA`): `handleRespawn`
//! gives the new `LocalPlayer` the old one's motion, and the portal's teleport adds its own (the
//! position packet's `DELTA_*` relatives). A player standing in a Nether portal so arrives still falling
//! by a tick's gravity and lands on its first move; a new player without it reported `on_ground`
//! false on that move and Grim flagged it (GroundSpoof). A death's packet (`resetPos`) keeps none.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::packets::play::{self, Clientbound as C};

#[test]
#[ignore = "daily"]
fn a_portals_respawn_keeps_the_players_motion_and_a_deaths_does_not() {
    with_client_game(|game| {
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
        // The portal's teleport keeps the motion (TeleportTransition's DELTA relatives).
        use play::relative as rel;
        let position = |teleport, relatives| {
            C::PlayerPosition(play::PlayerPosition {
                teleport,
                position: [0.5, 65.0, 0.5],
                delta: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                relatives,
            })
        };
        let kept = rel::DELTA_X | rel::DELTA_Y | rel::DELTA_Z;
        game.handle_packet(position(1, 0)).unwrap();
        let standing = Vec3::new(0.0, -0.0784, 0.0);
        game.player.as_mut().unwrap().velocity = standing;
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:the_nether"),
            keep: 3,
        }))
        .unwrap();
        assert!(game.player.is_none());
        game.handle_packet(position(2, kept)).unwrap();
        assert_eq!(game.player.as_ref().unwrap().velocity, standing);
        // A death's Respawn keeps no data: the new player starts still.
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        game.handle_packet(position(3, kept)).unwrap();
        assert_eq!(game.player.as_ref().unwrap().velocity, Vec3::ZERO);
    });
}
