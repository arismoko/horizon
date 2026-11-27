//! A spectated rider backs off by its living vehicle's scale and camera distance (26.3 `Camera.alignWithEntity`).
//! Expected frames are recorded into `.local` by `tools/test_camera_parity.py` (`VanillaCameraOracle`).
use serde_json::Value;
use sunrise_client::camera::Camera;
use sunrise_core::geometry::Vec3;
use sunrise_level::camera_math::CameraType;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client,
};

const RIDER: i32 = 31;
const VEHICLE: i32 = 32;
fn at() -> Vec3 {
    Vec3::new(10.25, 64.0, -3.75)
}

fn bits64(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn bits32(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}

fn fb(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

fn kind_id(game: &sunrise_client_gameplay::game::ClientGame<'_>, kind: &str) -> i32 {
    game.client
        .remote
        .as_ref()
        .unwrap()
        .builtin
        .get("minecraft:entity_type")
        .unwrap()
        .id(kind)
        .unwrap()
}

fn add(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id: i32, kind: &str) {
    let kind = kind_id(game, kind);
    game.handle_packet(C::AddEntity(play::AddEntity {
        id,
        uuid: id as _,
        kind,
        position: [at().x, at().y, at().z],
        velocity: [0.0; 3],
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
    }))
    .unwrap();
}

fn spawn(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id: i32, kind: &str) {
    if kind == "minecraft:oak_boat" {
        // The boat models are not loaded here, so the boat is tracked and listed by hand.
        game.spawn_entity(id, kind, [at().x, at().y, at().z], [0.0; 3]);
        game.boats
            .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                id as u32,
                "oak".into(),
                at(),
                0.0,
            ));
    } else {
        add(game, id, kind);
    }
}

fn attributes(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    id: i32,
    scale: f64,
    distance: f64,
) {
    let snapshot = |name: &str, base: f64| {
        let attribute = game.motion.attributes.id(name).unwrap().registry_id();
        play::AttributeSnapshot {
            attribute,
            base,
            modifiers: vec![],
        }
    };
    let values = vec![
        snapshot("minecraft:scale", scale),
        snapshot("minecraft:camera_distance", distance),
    ];
    game.handle_packet(C::UpdateAttributes(play::UpdateAttributes { id, values }))
        .unwrap();
}

fn pin_pose(game: &mut sunrise_client_gameplay::game::ClientGame<'_>, id: i32) {
    let id = id as u32;
    if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
        (m.position, m.previous) = (at(), at());
        (m.yaw, m.previous_yaw, m.head_yaw, m.previous_head_yaw) = (45.0, 45.0, 45.0, 45.0);
        (m.head_pitch, m.previous_head_pitch) = (22.5, 22.5);
    }
    game.boats.for_each_mut(|b| {
        if b.id == id {
            (b.position, b.previous, b.yaw, b.previous_yaw, b.pitch) =
                (at(), at(), 45.0, 45.0, 22.5);
        }
    });
}

/// The spectated rider's view as set by `Camera.alignWithEntity`.
fn frame(
    game: &sunrise_client_gameplay::game::ClientGame<'_>,
    partial: f32,
    camera_type: CameraType,
) -> (String, String, f32) {
    let e = game
        .spectated_camera_snapshot()
        .expect("the level holds the spectated rider");
    let mut camera = Camera::default();
    camera.tick(Some(e.eye_height), 1.0);
    camera.align_with_entity(&e, partial, camera_type, |_, _| None);
    let q = camera.rotation;
    let v = |a: [f32; 3]| a.map(fb).join(" ");
    let got = format!(
        "{:016x} {:016x} {:016x} {} {} {} {} {} {} {}",
        camera.position[0].to_bits(),
        camera.position[1].to_bits(),
        camera.position[2].to_bits(),
        fb(camera.x_rot),
        fb(camera.y_rot),
        [q.x, q.y, q.z, q.w].map(fb).join(" "),
        v(camera.forwards),
        v(camera.up),
        v(camera.left),
        camera.detached
    );
    (got, format!("{:?}", e.mount), e.eye_height)
}

fn want(r: &Value) -> String {
    r["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Builds the rider and its vehicle as a server's packets do, `attributes_first` or seats first.
fn setup(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    key: &Value,
    attributes_first: bool,
) {
    let rider_kind = if key["rider"] == "zombie" {
        "minecraft:zombie"
    } else {
        "minecraft:oak_boat"
    };
    game.handle_packet(C::RemoveEntities(play::RemoveEntities {
        ids: vec![RIDER, VEHICLE],
    }))
    .unwrap();
    spawn(game, RIDER, rider_kind);
    let vehicle = key["vehicle"].as_array();
    let vehicle_kind = vehicle.map(|v| match v[0].as_str().unwrap() {
        "happy_ghast" => "minecraft:happy_ghast",
        "ghast" => "minecraft:ghast",
        "zombie" => "minecraft:zombie",
        "armor_stand" => "minecraft:armor_stand",
        _ => "minecraft:oak_boat",
    });
    if let Some(kind) = vehicle_kind {
        spawn(game, VEHICLE, kind);
    }
    let sync = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
        if rider_kind == "minecraft:zombie" {
            attributes(
                game,
                RIDER,
                bits64(&key["rider_scale"]),
                bits64(&key["rider_distance"]),
            );
        }
        if let (Some(v), Some(kind)) = (vehicle, vehicle_kind)
            && kind != "minecraft:oak_boat"
            && kind != "minecraft:armor_stand"
        {
            attributes(game, VEHICLE, bits64(&v[1]), bits64(&v[2]));
        }
    };
    let seat = |game: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
        if vehicle.is_some() {
            game.handle_packet(C::SetPassengers(play::SetPassengers {
                vehicle: VEHICLE,
                passengers: vec![RIDER],
            }))
            .unwrap();
        }
        game.handle_packet(C::SetCamera(play::SetCamera { camera_id: RIDER }))
            .unwrap();
    };
    if attributes_first {
        sync(game);
        seat(game);
    } else {
        seat(game);
        sync(game);
    }
    game.refresh_remote_attributes();
    pin_pose(game, RIDER);
}

#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn a_spectated_rider_backs_off_by_its_living_vehicles_camera_distance() {
    run_a_spectated_rider_backs_off_by_its_living_vehicles_camera_distance();
}

pub(crate) fn run_a_spectated_rider_backs_off_by_its_living_vehicles_camera_distance() {
    let path = std::env::var("SUNRISE_CAMERA_MOUNT_WITNESS")
        .expect("run through tools/test_camera_parity.py, which records the witness");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "failed to read SUNRISE_CAMERA_MOUNT_WITNESS={path} (run through tools/test_camera_parity.py): {err}"
        )
    });
    let witness: Value = serde_json::from_str(&raw).unwrap();
    let records = witness["records"].as_array().unwrap();
    let mut failures = Vec::new();
    let mut guards = Vec::new();
    with_client_game(|game| {
        let _server = logged_client(game);
        game.player = Some(
            sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(0.5, 1.0, 0.5)).unwrap(),
        );
        let mut seen = std::collections::BTreeSet::new();
        for first in records {
            let key = serde_json::json!([
                first["rider"],
                first["rider_scale"],
                first["rider_distance"],
                first["vehicle"]
            ]);
            if !seen.insert(key.to_string()) {
                continue;
            }
            for attributes_first in [true, false] {
                setup(game, first, attributes_first);
                for r in records.iter().filter(|r| {
                    r["rider"] == first["rider"]
                        && r["rider_scale"] == first["rider_scale"]
                        && r["rider_distance"] == first["rider_distance"]
                        && r["vehicle"] == first["vehicle"]
                }) {
                    let camera_type = match r["camera_type"].as_u64().unwrap() {
                        0 => CameraType::FirstPerson,
                        1 => CameraType::ThirdPersonBack,
                        _ => CameraType::ThirdPersonFront,
                    };
                    let partial = r["partial"].as_f64().unwrap() as f32;
                    let (got, mount, eye) = frame(game, partial, camera_type);
                    let label = format!(
                        "rider {} scale {} dist {} vehicle {} type {camera_type:?} pt {partial} attributes_first {attributes_first}",
                        r["rider"],
                        bits64(&r["rider_scale"]),
                        bits64(&r["rider_distance"]),
                        r["vehicle"]
                            .as_array()
                            .map_or("none".to_string(), |v| format!(
                                "{} scale {} dist {}",
                                v[0],
                                bits64(&v[1]),
                                bits64(&v[2])
                            )),
                    );
                    if fb(eye) != r["eye"].as_str().unwrap() {
                        guards.push(format!("{label}: eye {} want {}", fb(eye), r["eye"]));
                    }
                    if got != want(r) {
                        failures.push(format!(
                            "{label}: mount {mount}, witness distance {}",
                            bits32(&r["distance"])
                        ));
                    }
                }
            }
        }
        // A dismount, and a vehicle removed from the level, drop the mount term at the next frame.
        let ghast = records
            .iter()
            .find(|r| {
                r["rider"] == "zombie"
                    && r["vehicle"].as_array().is_some_and(|v| {
                        v[0] == "happy_ghast" && bits64(&v[1]) == 1.0 && bits64(&v[2]) == 8.0
                    })
                    && bits64(&r["rider_scale"]) == 1.0
                    && bits64(&r["rider_distance"]) == 4.0
                    && r["camera_type"] == 1
                    && r["partial"] == 0.0
            })
            .unwrap();
        let alone = records
            .iter()
            .find(|r| {
                r["rider"] == "zombie"
                    && r["vehicle"].is_null()
                    && bits64(&r["rider_scale"]) == 1.0
                    && bits64(&r["rider_distance"]) == 4.0
                    && r["camera_type"] == 1
                    && r["partial"] == 0.0
            })
            .unwrap();
        for removal in ["passengers", "vehicle"] {
            setup(game, ghast, true);
            let packet = match removal {
                "passengers" => C::SetPassengers(play::SetPassengers {
                    vehicle: VEHICLE,
                    passengers: vec![],
                }),
                _ => C::RemoveEntities(play::RemoveEntities { ids: vec![VEHICLE] }),
            };
            game.handle_packet(packet).unwrap();
            pin_pose(game, RIDER);
            let (got, mount, _) = frame(game, 0.0, CameraType::ThirdPersonBack);
            if got != want(alone) {
                failures.push(format!(
                    "after removing the vehicle by {removal}: mount {mount}"
                ));
            }
        }
    });
    assert!(
        guards.is_empty(),
        "the rider's eye differs from the witness (a fixture fault, not the finding):\n{}",
        guards.join("\n")
    );
    assert!(
        failures.is_empty(),
        "{} of {} differ from real 26.3:\n{}",
        failures.len(),
        records.len() * 2 + 2,
        failures.join("\n")
    );
}
