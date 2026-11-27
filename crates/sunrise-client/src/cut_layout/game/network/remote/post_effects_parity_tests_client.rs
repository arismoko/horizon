use crate::game::client_side::remote_client::ingress_state::post_effects_parity_tests::json_strings;
use crate::game::client_side::remote_client::ingress_state::post_effects_parity_tests::login_packet;
use crate::game::client_side::remote_client::ingress_state::post_effects_parity_tests::overworld_spawn;
use crate::game::client_side::remote_client::ingress_state::post_effects_parity_tests::post_effects_witness;
use crate::game::client_side::remote_client::ingress_state::post_effects_parity_tests::wait_for_pong;
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use std::time::{Duration, Instant};
use sunrise_client::{multiplayer, net::Session, title::Mode};
use sunrise_level::camera_math::CameraType;
use sunrise_protocol::{
    connection::{Connection, Pipe},
    packets::{
        Empty, configuration as cfg,
        play::{self, Clientbound as C, Serverbound as S},
    },
};

pub(crate) fn post_effects_before_login_disconnects_case(case_name: &str) {
    let witness = post_effects_witness();
    let effects = match case_name {
        "empty" => Vec::new(),
        "nonempty" => vec!["minecraft:invert".to_owned()],
        other => panic!("unknown case {other}"),
    };
    let expected_key = witness["play_before_login"][case_name]["disconnect_key"]
        .as_str()
        .unwrap();
    with_client_game(|game| {
        game.client.title.0 = None;
        game.client.post_effects.clear();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "PostEffectsBeforeLogin")
            .unwrap();
        let (session, mut peer) = Session::play_loop_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        remote.logged_in = false;
        peer.reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();

        peer.send(&C::PostEffects(play::PostEffects { effects }).encode())
            .unwrap();
        peer.send(&C::Ping(cfg::Ping { id: 77 }).encode()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_pong = false;
        while game.client.remote.is_some() && !saw_pong && Instant::now() < deadline {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if matches!(S::decode(id, &body), Ok(S::Pong(p)) if p.id == 77) {
                    saw_pong = true;
                    break;
                }
            }
            std::thread::yield_now();
        }
        assert!(
            game.client.remote.is_none(),
            "play_before_login ({case_name}): post_effects before Login did not disconnect"
        );
        assert!(
            game.client.post_effects.is_empty(),
            "play_before_login ({case_name}): post_effects were applied before Login"
        );
        let Some(Mode::Multiplayer(multiplayer::Screen::Disconnected { reason, .. })) =
            game.client.title.0.as_ref().map(|t| &t.mode)
        else {
            panic!("play_before_login ({case_name}): missing Disconnected screen");
        };
        assert_eq!(
            reason.translation_key(),
            Some(expected_key),
            "play_before_login ({case_name})"
        );
    });
}

pub(crate) fn post_effects_cleared_on_login_respawn_and_reconfigure_body() {
    let witness = post_effects_witness();
    let lifecycle = &witness["play_lifecycle"];

    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "PostEffectsLifecycle").unwrap();
        let (session, mut peer) = Session::play_loop_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        remote.logged_in = false;
        peer.reader
            .set_read_timeout(Some(Duration::from_millis(20)))
            .unwrap();

        // Stale post_effects must clear when initial Login creates the player.
        game.client.post_effects = vec!["minecraft:stale".into()];
        peer.send(&C::Login(login_packet(1)).encode()).unwrap();
        wait_for_pong(game, &mut peer, 1);
        assert_eq!(
            game.client.post_effects,
            json_strings(&lifecycle["after_initial_login"]),
            "after_initial_login"
        );

        let two = json_strings(&lifecycle["after_set_two"]);
        peer.send(
            &C::PostEffects(play::PostEffects {
                effects: two.clone(),
            })
            .encode(),
        )
        .unwrap();
        wait_for_pong(game, &mut peer, 2);
        assert_eq!(game.client.post_effects, two, "after_set_two");

        peer.send(&C::PostEffects(play::PostEffects { effects: vec![] }).encode())
            .unwrap();
        wait_for_pong(game, &mut peer, 3);
        assert_eq!(
            game.client.post_effects,
            json_strings(&lifecycle["after_set_empty"]),
            "after_set_empty"
        );

        // Respawn creates a fresh LocalPlayer with empty post_effects for every keep mask,
        // and clears any spectated entity shader effect.
        game.client.title.0 = None;
        if game.player.is_none() {
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::ZERO,
                )
                .unwrap(),
            );
        }
        let (creeper_kind, enderman_kind, spider_kind) = {
            let entity_types = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap();
            (
                entity_types.id("minecraft:creeper").unwrap(),
                entity_types.id("minecraft:enderman").unwrap(),
                entity_types.id("minecraft:spider").unwrap(),
            )
        };
        let spectate = |peer: &mut Connection, id: i32, kind: i32| {
            peer.send(
                &C::AddEntity(play::AddEntity {
                    id,
                    uuid: id as u128,
                    kind,
                    position: [1.5, 64.0, 1.5],
                    velocity: [0.0; 3],
                    pitch: 0,
                    yaw: 0,
                    head_yaw: 0,
                    data: 0,
                })
                .encode(),
            )
            .unwrap();
            peer.send(&C::SetCamera(play::SetCamera { camera_id: id }).encode())
                .unwrap();
        };

        // F4 toggles only the spectator shader, leaving server post_effects untouched;
        // spectating an enderman while server also requests invert preserves both entries.
        peer.send(
            &C::PostEffects(play::PostEffects {
                effects: vec!["minecraft:invert".into()],
            })
            .encode(),
        )
        .unwrap();
        spectate(&mut peer, 500, creeper_kind);
        wait_for_pong(game, &mut peer, 8);
        assert_eq!(game.client.spectated_post_effect, Some("minecraft:creeper"));
        assert!(game.client.spectated_post_effect_active);
        assert_eq!(
            game.requested_post_effects(),
            vec![
                "minecraft:end_of_frame",
                "minecraft:invert",
                "minecraft:creeper",
            ]
        );
        game.toggle_spectator_post_effect();
        assert!(!game.client.spectated_post_effect_active);
        assert_eq!(
            game.requested_post_effects(),
            vec!["minecraft:end_of_frame", "minecraft:invert"]
        );
        game.toggle_spectator_post_effect();
        assert!(game.client.spectated_post_effect_active);
        spectate(&mut peer, 501, enderman_kind);
        wait_for_pong(game, &mut peer, 9);
        assert_eq!(
            game.requested_post_effects(),
            vec![
                "minecraft:end_of_frame",
                "minecraft:invert",
                "minecraft:invert",
            ]
        );
        // Reusing the frame buffer keeps unchanged String allocations and preserves repeats.
        let mut reused = Vec::new();
        game.sync_requested_post_effects(&mut reused, std::iter::empty());
        let ptrs: Vec<*const u8> = reused.iter().map(|s| s.as_ptr()).collect();
        game.sync_requested_post_effects(&mut reused, std::iter::empty());
        assert_eq!(
            reused,
            vec![
                "minecraft:end_of_frame",
                "minecraft:invert",
                "minecraft:invert",
            ]
        );
        assert_eq!(
            reused.iter().map(|s| s.as_ptr()).collect::<Vec<_>>(),
            ptrs,
            "unchanged effect IDs must reuse their String allocations across frames"
        );

        // Server camera switch vs F5 perspective transitions match vanilla 26.3.
        let cam_witness = &witness["camera_switch_and_perspective"];
        let expect_cam = |game: &sunrise_client_gameplay::game::ClientGame<'_>, prefix: &str| {
            let expected_effect = match cam_witness[format!("{prefix}_effect")].as_str().unwrap() {
                "none" => None,
                other => Some(other),
            };
            let expected_active = cam_witness[format!("{prefix}_active")].as_bool().unwrap();
            assert_eq!(
                game.client.spectated_post_effect, expected_effect,
                "{prefix}"
            );
            assert_eq!(
                game.client.spectated_post_effect_active, expected_active,
                "{prefix}"
            );
        };
        game.apply_camera_type(CameraType::ThirdPersonBack);
        spectate(&mut peer, 502, creeper_kind);
        wait_for_pong(game, &mut peer, 90);
        expect_cam(game, "set_camera_creeper_in_third_person");
        game.apply_camera_type(CameraType::ThirdPersonFront);
        expect_cam(game, "f5_third_back_to_front");
        game.apply_camera_type(CameraType::FirstPerson);
        expect_cam(game, "f5_third_front_to_first");
        game.apply_camera_type(CameraType::ThirdPersonBack);
        expect_cam(game, "f5_first_to_third_back");
        spectate(&mut peer, 503, spider_kind);
        wait_for_pong(game, &mut peer, 91);
        expect_cam(game, "set_camera_spider_in_third_person");
        peer.send(&C::SetCamera(play::SetCamera { camera_id: 1 }).encode())
            .unwrap();
        wait_for_pong(game, &mut peer, 92);
        expect_cam(game, "set_camera_player_in_third_person");
        game.apply_camera_type(CameraType::FirstPerson);

        for keep in 0..=3i8 {
            peer.send(
                &C::PostEffects(play::PostEffects {
                    effects: vec!["minecraft:invert".into()],
                })
                .encode(),
            )
            .unwrap();
            spectate(&mut peer, 510 + i32::from(keep), creeper_kind);
            peer.send(
                &C::Respawn(play::Respawn {
                    spawn: overworld_spawn(),
                    keep,
                })
                .encode(),
            )
            .unwrap();
            wait_for_pong(game, &mut peer, 10 + i32::from(keep));
            let key = format!("keep_{keep}");
            assert_eq!(
                game.client.post_effects,
                json_strings(&lifecycle["after_respawn"][key.as_str()]),
                "after_respawn {key}"
            );
            assert_eq!(
                game.client.spectated_post_effect, None,
                "respawn clears spectator effect"
            );
            assert!(!game.client.spectated_post_effect_active);
        }

        // Repeat Login without clearClientLevel keeps the existing LocalPlayer's post_effects
        // while resetting the spectated camera entity shader.
        peer.send(
            &C::PostEffects(play::PostEffects {
                effects: vec!["minecraft:invert".into()],
            })
            .encode(),
        )
        .unwrap();
        spectate(&mut peer, 520, creeper_kind);
        wait_for_pong(game, &mut peer, 19);
        assert_eq!(game.client.spectated_post_effect, Some("minecraft:creeper"));
        peer.send(&C::Login(login_packet(2)).encode()).unwrap();
        wait_for_pong(game, &mut peer, 20);
        assert_eq!(
            game.client.post_effects,
            json_strings(&lifecycle["after_repeat_login_without_clear"]),
            "after_repeat_login_without_clear"
        );
        assert_eq!(
            game.client.spectated_post_effect, None,
            "repeat login clears spectator effect"
        );
        assert!(!game.client.spectated_post_effect_active);

        // Reconfiguration runs clearClientLevel, so the next Login starts with empty post_effects
        // and clears the spectated entity shader immediately.
        spectate(&mut peer, 521, creeper_kind);
        wait_for_pong(game, &mut peer, 21);
        assert_eq!(game.client.spectated_post_effect, Some("minecraft:creeper"));
        peer.send(&C::StartConfiguration(Empty).encode()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut acked = false;
        while Instant::now() < deadline && !acked {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if matches!(S::decode(id, &body), Ok(S::ConfigurationAcknowledged(_))) {
                    acked = true;
                    break;
                }
            }
            std::thread::yield_now();
        }
        assert!(acked, "never received ConfigurationAcknowledged");
        assert_eq!(
            game.client.spectated_post_effect, None,
            "reconfigure clears spectator effect"
        );
        assert!(!game.client.spectated_post_effect_active);

        peer.send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut finished = false;
        while Instant::now() < deadline && !finished {
            game.poll_remote().unwrap();
            while let Ok((id, body)) = peer.recv() {
                if matches!(
                    cfg::Serverbound::decode(id, &body),
                    Ok(cfg::Serverbound::FinishConfiguration(_))
                ) {
                    finished = true;
                    break;
                }
            }
            std::thread::yield_now();
        }
        assert!(finished, "never received FinishConfiguration");

        peer.send(&C::Login(login_packet(3)).encode()).unwrap();
        wait_for_pong(game, &mut peer, 30);
        assert_eq!(
            game.client.post_effects,
            json_strings(&lifecycle["after_reconfigure_then_login"]),
            "after_reconfigure_then_login"
        );

        // Disconnecting also clears the spectated entity shader.
        spectate(&mut peer, 522, creeper_kind);
        wait_for_pong(game, &mut peer, 31);
        assert_eq!(game.client.spectated_post_effect, Some("minecraft:creeper"));
        game.leave_server(None).unwrap();
        assert_eq!(
            game.client.spectated_post_effect, None,
            "disconnect clears spectator effect"
        );
        assert!(!game.client.spectated_post_effect_active);
    });
}
