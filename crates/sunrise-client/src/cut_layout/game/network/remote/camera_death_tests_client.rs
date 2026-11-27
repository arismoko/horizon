use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client::level_loading::Reason;
use sunrise_client_gameplay::game::client_side::remote_client::session_state::RemoteDimensionEntry;
use sunrise_core::geometry::Vec3;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C, SpawnInfo},
};
use sunrise_sim::motion::PlayerMotion;

pub(in crate::game) fn run_a_spectated_dying_living_entity_squeezes_the_fov_26_3() {
    let path = std::env::var("SUNRISE_CAMERA_DEATH_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let witness = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_CAMERA_DEATH_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "CameraDeathTest").unwrap();
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
        let creeper = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .builtin
            .get("minecraft:entity_type")
            .unwrap()
            .id("minecraft:creeper")
            .unwrap();
        game.handle_packet(C::AddEntity(play::AddEntity {
            id: 31,
            uuid: 9,
            kind: creeper,
            position: [0.5, 64.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        }))
        .unwrap();
        game.remote_set_camera(31);
        assert!(!game.controls_camera());
        game.client.settings.fov = 70.0;
        let mut wrong = Vec::new();
        for row in witness.lines().filter(|l| !l.starts_with('#')) {
            let [dead, partial, fov, hud] = row.split('\t').collect::<Vec<_>>()[..] else {
                panic!("bad witness row {row}");
            };
            let bits = |s: &str| u32::from_str_radix(s, 16).unwrap();
            let mob = game.mobs.iter_mut().find(|m| m.id == 31).unwrap();
            (mob.health, mob.death_time) = match dead {
                "none" => (20.0, 0),
                n => (0.0, n.parse().unwrap()),
            };
            game.client.view = Default::default();
            for _ in 0..8 {
                game.tick_fov();
            }
            let partial = f32::from_bits(bits(partial));
            let camera = [0.5, 65.62, 0.5];
            let got = (
                game.fov_degrees(partial, camera).to_bits(),
                game.hud_fov_degrees(partial, camera).to_bits(),
            );
            if got != (bits(fov), bits(hud)) {
                wrong.push(format!(
                    "deathTime {dead} partial {partial}: fov {:08x} hud {:08x}, vanilla {fov} {hud}",
                    got.0, got.1
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    });
}
