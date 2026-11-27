use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_level::camera_math::CameraType;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;

pub(in crate::game) fn run_local_player_and_camera_entity_draw_like_the_real_client() {
    let path = std::env::var("SUNRISE_CAMERA_DRAW_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_CAMERA_DRAW_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    with_client_game(|game| {
        game.native_lifecycle = Default::default();
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraDrawTest").unwrap();
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
        game.enter_remote_dimension(
            &spawn,
            RemoteDimensionEntry::Login,
            sunrise_client::level_loading::Reason::Other,
        );
        let remote = game.client.remote.as_mut().unwrap();
        remote.logged_in = true;
        remote.entity_id = 7;
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        let villager = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:villager")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: villager,
            position: [10.0, 64.0, 12.0],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        let mut actual = Vec::new();
        for row in witness.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = row.split(' ').collect();
            game.client.remote.as_mut().unwrap().camera = (f[0] == "villager").then_some(31);
            game.client.camera_type = match f[1] {
                "0" => CameraType::FirstPerson,
                "1" => CameraType::ThirdPersonBack,
                _ => CameraType::ThirdPersonFront,
            };
            game.sleeping = (f[2] == "1").then_some([0, 64, 0]);
            let mob = game.mobs.iter_mut().find(|m| m.id == 31).unwrap();
            mob.extra.ints[1] = i32::from(f[3] == "1");
            actual.push(format!(
                "{} {} {} {} {} {}",
                f[0],
                f[1],
                f[2],
                f[3],
                u8::from(game.player_model_shown()),
                u8::from(!game.hides_as_camera(31)),
            ));
        }
        let expected: Vec<String> = witness
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(String::from)
            .collect();
        assert_eq!(actual, expected);
    });
}
