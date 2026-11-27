use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::Screen;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_protocol::packets::Empty;
#[cfg(test)]
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_sim::motion::PlayerMotion;
use sunrise_sim::sprint::Keys;
use winit::keyboard::KeyCode;

#[test]
#[ignore = "daily"]
fn a_boat_clamps_the_riders_yaw_without_changing_the_previous_yaw() {
    with_client_game(|game| {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        game.connect_local(client, "boat-clamp").unwrap();
        game.client.remote.as_mut().unwrap().first_passenger = false;
        game.boats
            .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 64.0, 2.5), 0.0));
        game.riding = Some(71);
        game.yaw = 200.0;
        game.client.rope_rot_old.0 = 180.0;

        assert!(game.remote_ride(Keys::default()));
        assert_eq!(game.yaw, 255.0);
        assert_eq!(game.client.rope_rot_old.0, 180.0);
        server.shutdown();
    });
}

#[test]
#[ignore = "daily"]
fn a_riders_move_takes_the_look_of_the_tick_and_leaves_it_off_the_ground() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            game.connect_local(client, "rider-tick").unwrap();
            let remote = game.client.remote.as_mut().unwrap();
            remote.logged_in = true;
            remote.loaded_sent = true;
            remote.joined = true;
            remote.first_passenger = true;
            remote.entity_id = 42;
            remote.level_load.finish();
            game.client.screen = Screen::Playing;
            game.client.streaming.insert_remote(
                &mut game.common.world,
                sunrise_level::chunk::Chunk::empty(0, 0),
            );
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            // A floor under the rider and a wall a hair beyond it at +z.
            for x in 6..=11 {
                for z in 6..=11 {
                    game.world.set(x, 4, z, stone).unwrap();
                }
            }
            for y in 5..=7 {
                game.world.set(8, y, 9, stone).unwrap();
            }
            game.boats
                .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 5.0, 2.5), 0.0));
            game.riding = Some(71);

            let mut collided = Vec::new();
            for yaw in [0.0, 180.0] {
                let mut player = PlayerMotion::new(game.motion, Vec3::new(8.5, 5.0, 8.69)).unwrap();
                player.client_side = true;
                // The player's own copy of the look is the opposite one: only this tick's look,
                // synced before the rider moves, turns it to the wall or away from it.
                player.yaw = 180.0 - yaw;
                game.player = Some(player);
                game.yaw = yaw;
                game.clear_device_keys();
                game.insert_device_key(KeyCode::KeyW);
                game.tick_local_player(Vec::new(), 7, None).unwrap();
                let player = game.player.as_ref().unwrap();
                collided.push(player.collision.horizontal);
                // `Entity.rideTick` zeroes the rider's motion and `LivingEntity.travelInAir` adds
                // gravity only after the move, so a move over a floor has no downward step to
                // find it: the rider is off the ground, and the seating and the pose tick that
                // follow do not change what the move left.
                assert!(
                    !player.ground.on_ground,
                    "a rider's own move leaves it off the ground, yaw {yaw}"
                );
            }
            assert_eq!(collided, [true, false]);
            server.shutdown();
        });
    });
}

/// `LocalPlayer.rideTick` -> `Entity.rideTick` -> `LocalPlayer.aiStep`: a mounted client with a
/// Spectator controller, Adventure PlayerInfo, `may_fly = true`, and `flying = false` transitions
/// to `flying = true` and sends `ServerboundPlayerAbilitiesPacket` without altering vehicle control.
#[test]
#[ignore = "daily"]
fn mounted_local_spectator_forces_flight_and_sends_abilities() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let (mut peer, _writer) = super::tests::connect_menu_test(game);
            peer.set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let uuid = 0x1122_3344_5566_7788_99aa_bbcc_ddee_ff00_u128;
            {
                let remote = game.client.remote.as_mut().unwrap();
                remote.logged_in = true;
                remote.loaded_sent = true;
                remote.joined = true;
                remote.first_passenger = false;
                remote.entity_id = 42;
                remote.profile = uuid;
                remote.profile_name = "SpectatorRider".into();
                remote.level_load.finish();
            }
            game.client.screen = Screen::Playing;
            game.boats
                .push(Boat::new(71, "oak".into(), Vec3::new(2.5, 64.0, 2.5), 0.0));
            game.riding = Some(71);

            let mut player = PlayerMotion::new(game.motion, Vec3::new(2.5, 64.0, 2.5)).unwrap();
            player.client_side = true;
            game.player = Some(player);

            game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                actions: 0x01 | 0x04 | 0x08,
                entries: vec![play::PlayerInfo {
                    uuid,
                    name: Some("SpectatorRider".into()),
                    game_mode: Some(2),
                    listed: Some(true),
                    ..play::PlayerInfo::default()
                }],
            }))
            .unwrap();
            game.handle_packet(C::GameEvent(play::GameEvent {
                event: 3,
                param: 3.0,
            }))
            .unwrap();
            game.handle_packet(C::PlayerAbilities(play::PlayerAbilities {
                flags: 0x04,
                flying_speed: 0.05,
                walking_speed: 0.10,
            }))
            .unwrap();
            assert!(game.spectator());
            assert!(!game.own_player_is_spectator());
            assert!(!game.player.as_ref().unwrap().flying);

            game.tick_local_player(Vec::new(), 7, None).unwrap();

            assert!(game.player.as_ref().unwrap().flying);
            assert!(game.remote_abilities().unwrap().flying);
            assert!(!game.client.remote.as_ref().unwrap().first_passenger);

            game.send(S::ClientTickEnd(Empty));
            let mut sent_flying = Vec::new();
            while let Ok((id, bytes)) = peer.recv() {
                match S::decode(id, &bytes) {
                    Ok(S::ClientTickEnd(_)) => break,
                    Ok(S::PlayerAbilities(a)) => sent_flying.push(a.flags & 2 != 0),
                    _ => {}
                }
            }
            assert_eq!(sent_flying, vec![true]);
        });
    });
}

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;
