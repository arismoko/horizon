use super::{entity_culling::at, view_vector};
use std::collections::HashMap;
use std::{f32::consts::PI, sync::Arc};
use sunrise_client::mesher::Vertex;
use sunrise_client::mobs::Part;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::game::boats_common::{
    Boat, HEIGHT, PLAYER_RIDING_OFFSET, Status, WIDTH,
};
use sunrise_sim::mobs::anim::Poses;

#[cfg_attr(test, derive(Clone))]
pub(in crate::game) struct BoatModels {
    pub models: HashMap<String, Part>,
    pub chest_models: HashMap<String, Part>,
    /// `ModelLayers.BOAT_WATER_PATCH`, shared by every boat (not rafts).
    pub water_patch: Option<Part>,
}
impl BoatModels {
    /// Whether this remote boat has a layer to draw.
    pub fn has(&self, wood: &str, chest: bool) -> bool {
        if chest {
            self.chest_models.contains_key(wood)
        } else {
            self.models.contains_key(wood)
        }
    }
    pub fn load(document: Option<&sunrise_client::mobs::EntityModelDocument>) -> Self {
        let mut models = HashMap::new();
        let mut chest_models = HashMap::new();
        if let Some(document) = document {
            for key in document.keys() {
                if let Some(wood) = key
                    .strip_prefix("minecraft:boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    models.insert(wood.to_owned(), part);
                }
                if let Some(wood) = key
                    .strip_prefix("minecraft:chest_boat/")
                    .and_then(|k| k.strip_suffix("#main"))
                    && let Some(part) = document.model(key)
                {
                    chest_models.insert(wood.to_owned(), part);
                }
            }
        }
        Self {
            models,
            chest_models,
            water_patch: document.and_then(|d| d.model("minecraft:boat#water_patch")),
        }
    }
}
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn record_boat_lid_movement(&mut self, i: usize, delta: Vec3) {
        if self.record_driven_boat_lid_movement(i, delta) {
            return;
        }
        let boat = &self.common.boats[i];
        if let Some(lerp) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.lerps.get(&(boat.id as i32)))
            && lerp.steps > 0
        {
            let [x, y, z] = lerp.target;
            let to = Vec3::new(x, y, z).add_vector(delta);
            if self.boat_lerp_target_fits(boat, to) {
                self.client
                    .remote
                    .as_mut()
                    .unwrap()
                    .lerps
                    .get_mut(&(boat.id as i32))
                    .unwrap()
                    .target = to.components();
            }
        }
    }

    /// `AbstractBoat.tick` advances the oars even when this client does not move the hull.
    pub(in crate::game::client_side) fn tick_boat_paddles(b: &mut Boat, audio: &mut super::Audio) {
        // Paddle sounds at the top of each stroke.
        for i in 0..2 {
            if b.paddle_state(i) {
                let p = b.paddles[i] % (2.0 * PI);
                if p <= PI / 4.0 && (b.paddles[i] + PI / 8.0) % (2.0 * PI) >= PI / 4.0 {
                    let sound = match b.status {
                        Status::InWater | Status::UnderWater | Status::UnderFlowingWater => {
                            Some("minecraft:entity.boat.paddle_water")
                        }
                        Status::OnLand => Some("minecraft:entity.boat.paddle_land"),
                        Status::InAir => None,
                    };
                    if let Some(sound) = sound {
                        let v = view_vector(b.yaw, 0.0);
                        let (dx, dz) = if i == 1 { (-v[2], v[0]) } else { (v[2], -v[0]) };
                        let pitch = 0.8 + 0.4 * b.random.next_float();
                        audio.play(
                            sound,
                            Some([b.position.x + dx, b.position.y, b.position.z + dz]),
                            1.0,
                            pitch,
                        );
                    }
                }
                b.paddles[i] += PI / 8.0;
            } else {
                b.paddles[i] = 0.0;
            }
        }
    }
    /// `AbstractBoat.onAboveBubbleColumn`: the client's half after the roll,
    /// the splash sound and particle at the boat.
    pub(in crate::game::client_side) fn boat_bubble_splash(&mut self, i: usize) {
        use sunrise_level::particle_options::{ParticleOptions, ParticleType};
        let p = self.common.boats[i].position;
        let pitch = 0.8 + 0.4 * self.common.world.random.next_float();
        self.play_audio(
            "minecraft:entity.generic.splash",
            Some([p.x, p.y, p.z]),
            1.0,
            pitch,
        );
        let x = p.x + f64::from(self.common.world.random.next_float());
        let z = p.z + f64::from(self.common.world.random.next_float());
        self.client.particles.0.add(
            &ParticleOptions::simple(ParticleType::Splash),
            [x, p.y + 0.7, z],
            [0.0; 3],
        );
    }
    /// AbstractBoatRenderer.
    pub fn boat_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        let a = alpha as f32;
        let range = super::entity_culling::default_range(
            glam::DVec3::new(WIDTH, HEIGHT, WIDTH),
            self.view_scale(),
        );
        for b in &self.common.boats[run] {
            // LevelExtractor: the camera entity is not drawn from its own eyes.
            if self.hides_as_camera(b.id) {
                continue;
            }
            // EntityRenderer.shouldRender.
            let (previous, position) = (at(b.previous), at(b.position));
            if !self.standing_entity_in_view(
                camera, previous, position, alpha, WIDTH, HEIGHT, range, 0.0,
            ) {
                continue;
            }
            let culling = self.entity_cull_decision(b, alpha);
            if culling.hides_entity() {
                continue;
            }
            let entity_start = out.len();
            self.mark_motion(
                entity_start,
                sunrise_gameplay_common::game::MOTION_OBJECT | u64::from(b.id),
            );
            let models = if b.chest {
                &self.client.boat_models.chest_models
            } else {
                &self.client.boat_models.models
            };
            let Some(model) = models.get(&*b.wood) else {
                continue;
            };
            let folder = if b.chest { "chest_boat" } else { "boat" };
            let Some(uv) = self
                .client
                .gui
                .atlas
                .sprites
                .get(&format!("entity/{folder}/{}", b.wood))
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
            else {
                continue;
            };
            let p = glam::DVec3::new(
                b.previous.x + (b.position.x - b.previous.x) * alpha,
                b.previous.y + (b.position.y - b.previous.y) * alpha,
                b.previous.z + (b.position.z - b.previous.z) * alpha,
            ) - camera;
            let yaw = b.previous_yaw + (b.yaw - b.previous_yaw) * a;
            let hurt = b.hurt_time as f32 - a;
            let damage = (b.damage - a).max(0.0);
            let mut base =
                glam::Mat4::from_translation(p.as_vec3() + glam::Vec3::new(0.0, 0.375, 0.0))
                    * glam::Mat4::from_rotation_y((180.0 - yaw).to_radians());
            if hurt > 0.0 {
                base *= glam::Mat4::from_rotation_x(
                    (hurt.sin() * hurt * damage / 10.0 * b.hurt_dir as f32).to_radians(),
                );
            }
            // AbstractBoatRenderer.extractRenderState: bubbleAngle.
            let wobble = b.bubble_angle(a);
            // AbstractBoatRenderer.submit: a boat above water wobbles in a
            // bubble column, about the axis (1, 0, 1).
            if !b.under_water() && wobble.abs() >= 1.0e-5 {
                base *= glam::Mat4::from_axis_angle(
                    glam::Vec3::new(1.0, 0.0, 1.0).normalize(),
                    wobble * (std::f64::consts::PI / 180.0) as f32,
                );
            }
            base *= glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_rotation_y(90f32.to_radians());
            let mut poses = Poses::new(model);
            // AbstractBoatModel.animatePaddle
            for (side, name) in [(0usize, "left_paddle"), (1, "right_paddle")] {
                if !poses.has(name) {
                    continue;
                }
                let t = b.rowing_time(side, a);
                let lerp = |f: f32, lo: f32, hi: f32| lo + f.clamp(0.0, 1.0) * (hi - lo);
                let pose = poses.get(name);
                pose[3] = lerp(((-t).sin() + 1.0) / 2.0, -PI / 3.0, -PI / 12.0);
                pose[4] = lerp(((-t + 1.0).sin() + 1.0) / 2.0, -PI / 4.0, PI / 4.0);
                if side == 1 {
                    pose[4] = PI - pose[4];
                }
            }
            let l = self.common.world.light(
                b.position.x.floor() as i32,
                (b.position.y + 0.3).floor() as i32,
                b.position.z.floor() as i32,
            );
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            sunrise_client::mobs::render::draw(
                model,
                base,
                &poses,
                uv,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            // BoatRenderer.submitTypeAdditions: the water patch hides the water
            // inside the hull unless the boat is under water (RaftRenderer has none).
            if let Some(patch) = self.client.boat_models.water_patch.as_ref()
                && &*b.wood != "bamboo"
                && !matches!(b.status, Status::UnderWater | Status::UnderFlowingWater)
            {
                // Depth only, so its texture is never seen; one texel of the white sprite, the
                // same at every corner (no derivatives, so the base level at any distance), keeps
                // a shader pack's alpha test from throwing the patch away.
                let white = (self.client.gui.atlas.sprites.get("sunrise/white")).map_or(uv, |r| {
                    let (u, v) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
                    [u, v, u, v]
                });
                sunrise_client::mobs::render::draw(
                    patch,
                    base,
                    &Poses::new(patch),
                    white,
                    0xFFFF_FFFF,
                    packed,
                    sunrise_client::gfx::WATER_MASK_OVERLAY,
                    out,
                );
            }
            self.finish_entity_occlusion(culling, b, out, entity_start);
        }
    }

    pub(in crate::game) fn client_boat_can_simulate_movement(&self, id: u32) -> bool {
        self.controls_boat(id)
    }

    pub(in crate::game) fn client_tick_boat_leash(&mut self, _b: &mut Boat) {}

    /// Client-side `AbstractBoat.canAddPassenger` from `Remote::passenger_seats` + local rider.
    pub(in crate::game) fn client_boat_has_room(&self, i: usize) -> bool {
        let b = &self.common.boats[i];
        if b.under_water() {
            return false;
        }
        let Some(remote) = self.client.remote.as_ref() else {
            return usize::from(self.common.riding == Some(b.id)) < b.max_passengers();
        };
        let local_id = remote.entity_id as u32;
        let mut count = 0usize;
        let mut local_seated = false;
        for (pid, _) in remote.passenger_seats(b.id) {
            count += 1;
            if pid == local_id {
                local_seated = true;
            }
        }
        if self.common.riding == Some(b.id) && !local_seated {
            count += 1;
        }
        count < b.max_passengers()
    }

    /// Client-side `player_boat_seat` from `Remote::passenger_seats` + local rider.
    pub(in crate::game) fn client_player_boat_seat(&self, i: usize) -> Vec3 {
        let b = &self.common.boats[i];
        let (index, count) = if let Some(remote) = self.client.remote.as_ref() {
            let local_id = remote.entity_id as u32;
            let mut seats: Vec<_> = remote.passenger_seats(b.id).collect();
            seats.sort_unstable_by_key(|&(_, idx)| idx);
            if let Some(pos) = seats.iter().position(|&(pid, _)| pid == local_id) {
                (pos, seats.len().max(1))
            } else {
                (0, seats.len() + 1)
            }
        } else {
            (0, 1)
        };
        b.seat(index, count, false, PLAYER_RIDING_OFFSET, false)
    }

    pub(in crate::game) fn run_boat_authority_fixture(
        &mut self,
        directory: &std::path::Path,
    ) -> sunrise_game_data::Result<()> {
        use serde::Deserialize;
        use serde_json::json;
        use std::sync::Mutex;

        #[derive(Deserialize)]
        struct Case {
            name: String,
            velocity: [f64; 3],
            spin: f32,
            #[serde(default)]
            owned: Vec<bool>,
            #[serde(default)]
            paddles: Option<PaddleCase>,
            #[serde(default)]
            attack: Option<serde_json::Value>,
        }
        #[derive(Deserialize)]
        struct PaddleCase {
            initial: [f32; 2],
            steps: Vec<PaddleStep>,
        }
        #[derive(Deserialize)]
        struct PaddleStep {
            occupied: bool,
            #[serde(default)]
            owned: bool,
            #[serde(default = "is_player")]
            first_player: bool,
            flags: [bool; 2],
        }
        fn is_player() -> bool {
            true
        }

        let cases: Vec<Case> =
            serde_json::from_str(&std::fs::read_to_string(directory.join("inputs.json"))?)?;
        let total = cases.len();
        let has_attacks = cases.iter().any(|case| case.attack.is_some());
        let mut indexed: Vec<(usize, serde_json::Value)> = Vec::with_capacity(total);
        if has_attacks {
            let attacks = Arc::new(Mutex::new(Vec::new()));
            self.run_component_harness(
                sunrise_gameplay_common::game::integrated_link::ComponentHarness::BoatAuthorityAttacks(
                    directory.to_path_buf(),
                    attacks.clone(),
                ),
            )?;
            indexed.extend(std::mem::take(&mut *attacks.lock().unwrap()));
        }
        for (index, case) in cases.into_iter().enumerate() {
            if case.attack.is_some() {
                continue;
            }
            self.common.boats.clear();
            let mut boat = Boat::new(1, "oak".into(), Vec3::new(0.0, 100.0, 0.0), 0.0);
            let [x, y, z] = case.velocity;
            boat.velocity = Vec3::new(x, y, z);
            boat.delta_rotation = case.spin;
            self.common.boats.push(boat);
            let mut steps = Vec::new();
            if let Some(paddles) = case.paddles {
                self.common.riding = None;
                self.common.boats.update(0, |b| b.paddles = paddles.initial);
                for step in paddles.steps {
                    self.common.boats.update(0, |b| b.paddling = step.flags);
                    self.common.boats.update(0, |b| {
                        b.paddle_controller = step.occupied && step.first_player
                    });
                    self.common.riding = step.owned.then_some(1);
                    if step.owned {
                        self.common.boats.update(0, |b| {
                            b.input = [
                                step.flags[1] && !step.flags[0],
                                step.flags[0] && !step.flags[1],
                                step.flags[0] && step.flags[1],
                                false,
                            ]
                        });
                        let boat = self.tick_boat(self.common.boats[0].clone(), true);
                        self.common.store_boat_back(0, boat);
                    }
                    self.tick_remote_boat_bubbles();
                    let boat = &self.common.boats[0];
                    let rowing = [0.0, 0.25, 0.5, 1.0].map(|a| {
                        [0, 1].map(|side| format!("{:08x}", boat.rowing_time(side, a).to_bits()))
                    });
                    steps.push(json!({
                        "phase": boat.paddles.map(|p| format!("{:08x}", p.to_bits())),
                        "rowing": rowing,
                    }));
                }
                indexed.push((index, json!({"name": case.name, "steps": steps})));
                continue;
            }
            for owned in case.owned {
                self.common.riding = owned.then_some(1);
                if owned {
                    let boat = self.common.boats.remove(0);
                    let boat = self.tick_boat(boat, true);
                    self.common.boats.push(boat);
                }
                self.tick_remote_boat_bubbles();
                let boat = &self.common.boats[0];
                steps.push(json!({
                    "owned": owned,
                    "spin": format!("{:08x}", boat.delta_rotation.to_bits()),
                    "velocity": boat.velocity.components().map(|v| format!("{:016x}", v.to_bits())),
                }));
            }
            indexed.push((index, json!({"name": case.name, "steps": steps})));
        }
        indexed.sort_by_key(|(index, _)| *index);
        let output: Vec<serde_json::Value> = indexed.into_iter().map(|(_, v)| v).collect();
        std::fs::write(directory.join("rust.json"), serde_json::to_vec(&output)?)?;
        Ok(())
    }
}

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;
