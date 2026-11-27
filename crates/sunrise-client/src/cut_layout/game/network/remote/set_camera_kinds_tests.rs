use mod_api::ClientHost;
use sunrise_client::level_loading::Reason;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycle,
};

#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn set_camera_follows_the_real_26_3_clients_camera_for_every_entity_kind() {
    run_set_camera_follows_the_real_26_3_clients_camera_for_every_entity_kind();
}

pub(in crate::game) fn run_set_camera_follows_the_real_26_3_clients_camera_for_every_entity_kind() {
    let path = std::env::var("SUNRISE_SET_CAMERA_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_SET_CAMERA_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    with_client_game(|game| {
        let camera = |game: &sunrise_client_gameplay::game::ClientGame<'_>| {
            (game as &dyn ClientHost).camera_entity()
        };
        game.native_lifecycle = NativeLifecycle::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "SetCameraKinds").unwrap();
        let spawn = SpawnInfo {
            dimension_type: 0,
            dimension: "minecraft:overworld".into(),
            seed: 0,
            game_type: 3,
            previous_game_type: None,
            debug: false,
            flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        };
        game.enter_remote_dimension(&spawn, RemoteDimensionEntry::Login, Reason::Other);
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let local = camera(game).unwrap();
        let mut mismatches = Vec::new();
        for (n, row) in witness.lines().filter(|l| !l.starts_with('#')).enumerate() {
            let (kind, expected) = row.split_once('\t').unwrap();
            let target = 40 + n as i32;
            let kind_id = game
                .client
                .remote
                .as_ref()
                .unwrap()
                .builtin
                .get("minecraft:entity_type")
                .unwrap()
                .id(kind)
                .unwrap();
            game.handle_packet(C::AddEntity(play::AddEntity {
                id: target,
                uuid: 100 + n as u128,
                kind: kind_id,
                position: [10.0, 64.0, 12.0],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            }))
            .unwrap();
            let mut seen = Vec::new();
            for sent in [target, 7, target, 99, target, 99] {
                game.handle_packet(C::SetCamera(play::SetCamera { camera_id: sent }))
                    .unwrap();
                let attached = camera(game).unwrap();
                seen.push(if attached == local {
                    7
                } else {
                    attached.id() as i32
                });
            }
            let seen = seen
                .iter()
                .map(|id| if *id == 7 { 7 } else { target }.to_string());
            let seen = seen.collect::<Vec<_>>().join(",");
            let expected = expected.replace("40", &target.to_string());
            if seen != expected {
                mismatches.push(format!("{kind}: sunrise {seen}, vanilla {expected}"));
            }
        }
        assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    });
}
