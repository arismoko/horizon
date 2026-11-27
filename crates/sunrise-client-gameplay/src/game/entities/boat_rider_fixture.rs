//! Headless passenger-order regression through the real remote client paths.
use super::Keys;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use sunrise_client::held::Source;
use sunrise_core::geometry::Vec3;
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_protocol::{connection::Pipe, packets::play::Serverbound};
use sunrise_sim::motion::PlayerMotion;
use winit::keyboard::KeyCode;

#[derive(Deserialize)]
struct Case {
    name: String,
    first: Vec<bool>,
    velocity: [f64; 3],
    spin: f32,
    keys: [bool; 4],
    event: i8,
}
impl crate::game::ClientGame<'_> {
    pub(in crate::game::client_side) fn run_boat_rider_fixture(
        &mut self,
        directory: &Path,
    ) -> Result<()> {
        let cases: Vec<Case> =
            serde_json::from_str(&std::fs::read_to_string(directory.join("inputs.json"))?)?;
        let (client, peer) = Pipe::pair();
        self.connect_local(client, "BoatSeatFixture")?;
        self.client.remote.as_mut().unwrap().entity_id = 1;
        self.common.player = Some(PlayerMotion::new(
            self.common.motion,
            Vec3::new(0.0, 100.0, 0.0),
        )?);
        self.spawn_entity(2, "minecraft:marker", [0.0, 100.0, 0.0], [0.0; 3]);
        self.spawn_entity(3, "minecraft:oak_boat", [0.0, 100.0, 0.0], [0.0; 3]);

        let mut output = Vec::new();
        for case in cases {
            self.common.boats.clear();
            let mut b = Boat::new(3, "oak".into(), Vec3::new(0.0, 100.0, 0.0), 0.0);
            let [x, y, z] = case.velocity;
            b.velocity = Vec3::new(x, y, z);
            b.delta_rotation = case.spin;
            for key in self.client.keys.release_source(Source::Agent) {
                self.release_device_key(&key, Source::Agent);
            }
            for (key, held) in [KeyCode::KeyA, KeyCode::KeyD, KeyCode::KeyW, KeyCode::KeyS]
                .into_iter()
                .zip(case.keys)
            {
                if held {
                    self.press_device_key(key, Source::Agent);
                }
            }
            self.common.boats.push(b);
            let mut steps = Vec::new();
            for first in case.first {
                self.remote_set_passengers(3, if first { vec![1, 2] } else { vec![2, 1] });
                let mut accepts_server_moves = false;
                self.tick_remote_objects(
                    |_, p| {
                        accepts_server_moves = true;
                        p
                    },
                    |_, yaw| yaw,
                    |_| false,
                );
                self.remote_ride(Keys {
                    left: case.keys[0],
                    right: case.keys[1],
                    forward: case.keys[2],
                    backward: case.keys[3],
                    ..Keys::default()
                });
                let packets = self.remote_vehicle_packets();
                if case.event != 0 {
                    self.remote_boat_event(3, case.event);
                }
                let boat = &self.common.boats[0];
                steps.push(json!({
                    "controlled": self.client.remote.as_ref().unwrap().first_passenger,
                    "accepts_server_moves": accepts_server_moves,
                    "hands_busy": self.hands_busy(),
                    "moves": packets.iter().filter(|p| matches!(p, Serverbound::MoveVehicle(_))).count(),
                    "paddles": packets.iter().filter(|p| matches!(p, Serverbound::PaddleBoat(_))).count(),
                    "spin": format!("{:08x}", boat.delta_rotation.to_bits()),
                    "velocity": boat.velocity.components().map(|v| format!("{:016x}", v.to_bits())),
                }));
            }
            output.push(json!({"name": case.name, "steps": steps}));
        }
        peer.shutdown();
        self.client.remote = None;
        std::fs::write(directory.join("rust.json"), serde_json::to_vec(&output)?)?;
        Ok(())
    }
}
