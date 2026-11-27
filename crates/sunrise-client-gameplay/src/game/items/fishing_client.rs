use super::billboard_axes;
use sunrise_client::{gizmos::Line, mesher::Vertex};
use sunrise_core::{
    geometry::{Vec3, trig},
    random::LegacyRandom,
};
use sunrise_gameplay_common::game::fishing_common::{
    Hook, HookLerp, State, bite_impulse, bounds, resting, synchronized_random,
};
use sunrise_level::random_source;
use sunrise_protocol::entity_data::Value;
use sunrise_sim::pose::Pose;

impl crate::game::ClientGame<'_> {
    /// FishingHookRenderer: camera-facing half-block quads centred on each bobber.
    pub(in crate::game) fn fishing_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let hooks: Vec<glam::DVec3> = self
            .common
            .hook_position(alpha)
            .into_iter()
            .chain(self.remote_hook_ends(alpha).into_iter().map(|(h, _)| h))
            .collect();
        if hooks.is_empty() {
            return;
        }
        let Some(r) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/fishing/fishing_hook")
            .copied()
        else {
            return;
        };
        // FishingHookRenderer.submit: turned by the camera's orientation.
        let [right, up] = billboard_axes(self.camera_orientation());
        let s = 0.25;
        let uv = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
        for p in hooks {
            let light =
                self.common
                    .world
                    .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let corners = [
                p - right * s - up * s,
                p + right * s - up * s,
                p + right * s + up * s,
                p - right * s + up * s,
            ];
            for (v, uv) in corners.iter().zip(uv) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: (*v - camera).as_vec3().to_array(),
                    uv,
                    layer: 0,
                    color: 0xFFFF_FFFF,
                    light: packed,

                    glint_uv: [0.0, 0.0],
                });
            }
        }
    }
    pub(in crate::game::client_side) fn clear_remote_hooks(&mut self) {
        if self.client.remote.is_some() {
            self.common.hook.discard();
        }
        self.client.remote_hooks.clear();
    }
    /// The hooks drawn here, with their owners' ids (0 for this player's own).
    pub(in crate::game::client_side) fn hooks_status(&self) -> Vec<serde_json::Value> {
        let at = |p: Vec3| [p.x, p.y, p.z];
        let own = self.common.hook.iter().map(|h| (h.id, 0, h.position));
        let others = self
            .client
            .remote_hooks
            .iter()
            .map(|h| (h.id, h.owner, h.position));
        own.chain(others)
            .map(|(id, owner, p)| {
                serde_json::json!({"id": id, "kind": "minecraft:fishing_bobber", "owner": owner, "position": at(p)})
            })
            .collect()
    }
    /// The local hook's position and whether a fish is biting (`nibble > 0`).
    #[cfg(feature = "client")]
    pub(in crate::game) fn local_fishing_hook(&self) -> Option<(Vec3, bool)> {
        let hook = self.common.hook.as_ref()?;
        Some((hook.position, hook.nibble > 0))
    }
    /// `FishingHook.onSyncedDataUpdated`: this player's hook is biting
    /// (DATA_BITING), which `status` shows as a nibble.
    pub(in crate::game::client_side) fn remote_hook_data(
        &mut self,
        id: u32,
        values: &[(u8, i32, Value)],
    ) {
        // DATA_HOOKED_ENTITY (the hooked id plus one, 0 for none): a hook on a
        // client follows what it hooked (`FishingHook.onSyncedDataUpdated`).
        let time = self.common.world.game_time;
        let hooked = |value: &Value| match value {
            Value::Int(v) if *v > 0 => State::Hooked(*v as u32 - 1),
            _ => State::Flying,
        };
        if let Some(h) = self
            .common
            .hook
            .as_mut()
            .filter(|h| h.id == id && self.client.remote.is_some())
        {
            for (index, _, value) in values {
                match (index, value) {
                    (9, Value::Bool(biting)) => {
                        h.nibble = i32::from(*biting);
                        if *biting {
                            h.velocity.y = bite_impulse(&mut synchronized_random(h.uuid, time));
                        }
                    }
                    (8, value) => h.state = hooked(value),
                    _ => {}
                }
            }
        } else if let Some(h) = self.client.remote_hooks.iter_mut().find(|h| h.id == id) {
            for (index, _, value) in values {
                match (index, value) {
                    (9, Value::Bool(biting)) => {
                        if *biting {
                            h.velocity.y = bite_impulse(&mut synchronized_random(h.uuid, time));
                        }
                    }
                    (8, value) => h.state = hooked(value),
                    _ => {}
                }
            }
        }
    }
    /// Other players' hooks: the bobber and its line to the owner's hand
    /// (`FishingHookRenderer.getPlayerHandPos` in third person), as
    /// (hook, hand) this frame.
    pub(in crate::game) fn remote_hook_ends(&self, alpha: f64) -> Vec<(glam::DVec3, glam::DVec3)> {
        let lerp = |a: Vec3, b: Vec3| {
            glam::DVec3::new(
                a.x + (b.x - a.x) * alpha,
                a.y + (b.y - a.y) * alpha,
                a.z + (b.z - a.z) * alpha,
            )
        };
        self.client
            .remote_hooks
            .iter()
            .filter_map(|h| {
                if self.hides_as_camera(h.id) {
                    return None;
                }
                let owner = self.common.mobs.iter().find(|m| m.id == h.owner)?;
                let feet = lerp(owner.previous, owner.position);
                let body = mod_api::client::camera::lerp(
                    alpha as f32,
                    owner.previous_body_yaw,
                    owner.body_yaw,
                );
                let angle = f64::from(body * (std::f64::consts::PI / 180.0) as f32);
                let (sin, cos) = (f64::from(trig::sin(angle)), f64::from(trig::cos(angle)));
                // The right arm; a crouching owner's eyes are lower.
                let eye = owner.eye_y() - owner.position.y;
                let hand = feet
                    + glam::DVec3::new(
                        -cos * 0.35 - sin * 0.8,
                        eye - 0.45,
                        -sin * 0.35 + cos * 0.8,
                    );
                Some((lerp(h.previous, h.position), hand))
            })
            .collect()
    }
    /// `Entity.lerpMotion` for a hook a server tracks: the motion its own tick
    /// moves it on with between the tracker's updates.
    pub(in crate::game::client_side) fn remote_hook_motion(&mut self, id: u32, v: Vec3) -> bool {
        if self.client.remote.is_none() {
            return false;
        }
        if let Some(h) = self.common.hook.as_mut().filter(|h| h.id == id) {
            h.velocity = v;
        } else if let Some(h) = self.client.remote_hooks.iter_mut().find(|h| h.id == id) {
            h.velocity = v;
        } else {
            return false;
        }
        true
    }
    pub(in crate::game::client_side) fn remove_remote_hook(&mut self, id: u32) {
        if self.client.remote.is_some() && self.common.hook.as_ref().is_some_and(|h| h.id == id) {
            self.common.hook.discard();
        }
        remove_remote_hook_from(&mut self.client.remote_hooks, id);
    }
    /// `FishingHookRenderer.getPlayerHandPos`: on the near plane in first
    /// person, beside the body otherwise.
    pub(in crate::game) fn rod_hand_position(&self, alpha: f64) -> Option<glam::DVec3> {
        let p = self.common.player.as_ref()?;
        let right_arm = self.main_hand_rod() != self.client.settings.main_hand_left;
        let invert: f32 = if right_arm { 1.0 } else { -1.0 };
        let swing = trig::sin(f64::from(
            self.common.swing.animation(alpha as f32).sqrt() * std::f32::consts::PI,
        ));
        let feet = self.common.previous_position.lerp(p.position, alpha);
        let eye = glam::DVec3::new(feet.x, feet.y + f64::from(p.dimensions.eye_height), feet.z);
        if self.client.camera_type.is_first_person() {
            let fov = self.client.settings.fov as i32 as f32;
            Some(eye + near_plane_hand(self.near_plane(), fov, invert, swing))
        } else {
            let model = self.client.player_model.as_ref()?;
            let body = mod_api::client::camera::lerp(
                alpha as f32,
                model.previous_body_yaw,
                model.body_yaw,
            );
            let angle = f64::from(body * (std::f64::consts::PI / 180.0) as f32);
            let (sin, cos) = (f64::from(trig::sin(angle)), f64::from(trig::cos(angle)));
            let right = f64::from(invert) * 0.35;
            let crouch = if p.pose == Pose::Crouching {
                -0.1875
            } else {
                0.0
            };
            Some(
                eye + glam::DVec3::new(
                    -cos * right - sin * 0.8,
                    crouch - 0.45,
                    -sin * right + cos * 0.8,
                ),
            )
        }
    }
    /// `handleAddEntity` for a fishing bobber: this player's own becomes its
    /// hook (`Player.fishing`), another player's is drawn on its line to them.
    pub(in crate::game::client_side) fn spawn_remote_hook(
        &mut self,
        id: u32,
        uuid: [i32; 4],
        at: Vec3,
        velocity: Vec3,
        owner: i32,
    ) {
        let own = self.client.remote.as_ref().map(|r| r.entity_id);
        if own == Some(owner) {
            let mut hook = Hook::remote(id, at, uuid);
            hook.velocity = velocity;
            self.common.hook.discard();
            self.common.hook.replace(hook);
        } else {
            push_remote_hook(
                &mut self.client.remote_hooks,
                id,
                owner as u32,
                at,
                velocity,
                uuid,
            );
        }
    }
    /// `FishingHook.tick` on a client: the interpolation step towards the
    /// server's last update, then its own movement between the tracker's
    /// updates (every five ticks), floating in water and riding what it
    /// hooked, as the server's does.
    pub(in crate::game::client_side) fn tick_remote_hooks(&mut self) {
        if self.client.remote.is_none() {
            return;
        }
        if let Some(mut h) = self.common.hook.extract() {
            h.previous = h.position;
            h.position = h.lerp.step(h.position);
            let before = h.position;
            let owner = self.common.player.as_ref().map(|p| p.position);
            (h.position, h.velocity, h.state, h.ground) = self.client_hook_step(
                h.id,
                owner,
                true,
                (h.position, h.velocity, h.state, h.ground),
                &mut h.random,
            );
            h.lerp.moved(h.position.subtract(before));
            self.common.hook.put_back(h);
        }
        let mut hooks = std::mem::take(&mut self.client.remote_hooks);
        for h in &mut hooks {
            h.previous = h.position;
            h.position = h.lerp.step(h.position);
            let before = h.position;
            let owner = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == h.owner)
                .map(|m| m.position);
            (h.position, h.velocity, h.state, h.ground) = self.client_hook_step(
                h.id,
                owner,
                false,
                (h.position, h.velocity, h.state, h.ground),
                &mut h.random,
            );
            h.lerp.moved(h.position.subtract(before));
        }
        self.client.remote_hooks = hooks;
    }
    /// FishingHookRenderer: each line, 16 black `lines` segments sagging
    /// from a quarter block above the hook to the rod in its owner's hand,
    /// `width` pixels wide (`Window.getAppropriateLineWidth`).
    pub(in crate::game) fn fishing_line(&self, alpha: f64, size: [u32; 2]) -> Vec<Line> {
        let own = self
            .common
            .hook_position(alpha)
            .zip(self.rod_hand_position(alpha));
        let width = sunrise_client::gfx::outline_width(size[0]);
        let mut lines = Vec::new();
        for (hook, hand) in own.into_iter().chain(self.remote_hook_ends(alpha)) {
            let offset = (hand - (hook + glam::DVec3::new(0.0, 0.25, 0.0))).as_vec3();
            let point = |a: f32| {
                let y = offset.y * (a * a + a) * 0.5 + 0.25;
                hook + glam::Vec3::new(offset.x * a, y, offset.z * a).as_dvec3()
            };
            lines.extend((0..16).map(|i| Line {
                start: point(i as f32 / 16.0).to_array(),
                end: point((i + 1) as f32 / 16.0).to_array(),
                color: 0xFF00_0000,
                width,
            }));
        }
        lines
    }
    /// Client-side bobber collision response without server tables or sound/event writes.
    pub(in crate::game) fn client_move_bobber_response(
        &mut self,
        id: u32,
        position: Vec3,
        ground: &mut sunrise_world::ground_support::GroundState,
        wanted: Vec3,
        moved: Vec3,
        velocity: &mut Vec3,
    ) {
        use sunrise_items::equipment::{BODY, SADDLE};
        let level = self.native_client_level();
        let side = sunrise_gameplay_common::game::native_surface::GameSide {
            dim: self.common.dim,
            level,
            facts: self.common.res.facts(),
            biomes: self.common.res.biomes(),
            behaviours: self.common.res.behaviours(),
            tables: sunrise_gameplay_common::game::native_block_tables::BlockTables::new(
                &self.common.res,
                &self.common.pack,
            ),
            features: match &self.client.remote {
                Some(remote) => &remote.features,
                None => sunrise_gameplay_common::game::tables::default_client_features(),
            },
            items: self.common.res.items(),
            damage_types: &self.common.health_resources.damage,
            player_equipment: [
                self.common.inventory[self.common.selected].as_ref(),
                self.common.offhand.as_ref(),
                self.common.armor[3].as_ref(),
                self.common.armor[2].as_ref(),
                self.common.armor[1].as_ref(),
                self.common.armor[0].as_ref(),
                self.common.equipment.get(BODY),
                self.common.equipment.get(SADDLE),
            ],
            hit_borrows: sunrise_gameplay_common::game::native_surface::HitBorrows {
                living_types: self.common.res.living_type_keys(),
                attributes: &self.common.motion.attributes,
                player_effects: &self.common.potions.effects,
                player_uuid: None,
                player_attachments: &self.common.player_state.attachments,
                entity_attachments: &self.common.entity_attachments,
            },
            tag_items: self
                .client
                .remote
                .as_ref()
                .and_then(|remote| remote.tags.as_ref()),
        };
        let motion = sunrise_sim::motion::BodyMotion {
            world: &self.common.world,
            collision: self.common.collision,
            facts: self.common.res.facts(),
            blocks: Some(&self.common.res.motion().blocks),
            context: &self.common.loop_context,
            side: Some(&side),
        };
        motion.record_move(
            sunrise_sim::motion::BodyCollision {
                footprint: (position, bounds(position)),
                movement: (wanted, moved),
                authoritative: false,
                context: sunrise_sim::motion::BodyMotion::entity_context(id, position),
                subject: None,
            },
            ground,
        );
        let effect = motion.legacy_contact(position, ground);
        let entity = (id, "minecraft:fishing_bobber");
        let response = motion.restitute(
            entity,
            sunrise_sim::motion::BodyMove {
                wanted,
                moved,
                effect,
            },
            f64::from(0.03_f32),
            0.98_f32,
            *velocity,
            0.0,
        );
        *velocity = response.velocity;
        motion.apply_block_speed(position, velocity);
        motion.step_on(entity, (position, bounds(position)), ground, velocity);
    }
}

/// A hook a server tracks to this client (`FishingHook.recreateFromPacket`)
/// that another player cast: drawn where the server puts it, its line
/// running to that player's hand.
pub(in crate::game) struct RemoteHook {
    pub(in crate::game) id: u32,
    pub(in crate::game) uuid: [i32; 4],
    pub(in crate::game) owner: u32,
    pub(in crate::game) owner_fishing: bool,
    pub(in crate::game) position: Vec3,
    pub(in crate::game) previous: Vec3,
    pub(in crate::game) velocity: Vec3,
    pub(in crate::game) state: State,
    pub(in crate::game) ground: sunrise_world::ground_support::GroundState,
    pub(in crate::game) lerp: HookLerp,
    pub(in crate::game) random: LegacyRandom,
}

impl RemoteHook {
    pub(in crate::game) fn id(&self) -> u32 {
        self.id
    }
    /// A server's update of another player's hook.
    pub(in crate::game) fn move_to(&mut self, target: [f64; 3], on_ground: bool) {
        self.ground.on_ground = resting(self.ground.on_ground, self.position, target, on_ground);
        self.lerp.start(target);
    }
}

pub(in crate::game) fn push_remote_hook(
    hooks: &mut Vec<RemoteHook>,
    id: u32,
    owner: u32,
    at: Vec3,
    velocity: Vec3,
    uuid: [i32; 4],
) {
    for h in hooks.iter_mut().filter(|h| h.owner == owner) {
        h.owner_fishing = false;
    }
    hooks.push(RemoteHook {
        id,
        uuid,
        owner,
        owner_fishing: true,
        position: at,
        previous: at,
        velocity,
        state: State::Flying,
        ground: Default::default(),
        lerp: HookLerp::default(),
        random: random_source::entity(),
    });
}

pub(in crate::game) fn remove_remote_hook_from(hooks: &mut Vec<RemoteHook>, id: u32) {
    let owner = hooks.iter().find(|h| h.id == id).map(|h| h.owner);
    hooks.retain(|h| h.id != id);
    if let Some(owner) = owner {
        for h in hooks.iter_mut().filter(|h| h.owner == owner) {
            h.owner_fishing = false;
        }
    }
}

/// `FishingHookRenderer.getPlayerHandPos` in first person, from the eye: the camera's
/// `getNearPlane(fov).getPointOnPlane(invert * 0.525, -0.1)` scaled by 960 / fov, then
/// `Vec3.yRot(swing * 0.5)` and `xRot(-swing * 0.7)`.
pub(in crate::game) fn near_plane_hand(
    plane: sunrise_client::camera::NearPlane,
    fov: f32,
    invert: f32,
    swing: f32,
) -> glam::DVec3 {
    let point = glam::DVec3::from_array(plane.point_on_plane(invert * 0.525, -0.1))
        * (960.0 / f64::from(fov));
    let (s, c) = (
        f64::from(trig::sin(f64::from(swing * 0.5))),
        f64::from(trig::cos(f64::from(swing * 0.5))),
    );
    let point = glam::DVec3::new(
        point.x * c + point.z * s,
        point.y,
        point.z * c - point.x * s,
    );
    let (s, c) = (
        f64::from(trig::sin(f64::from(-swing * 0.7))),
        f64::from(trig::cos(f64::from(-swing * 0.7))),
    );
    glam::DVec3::new(
        point.x,
        point.y * c + point.z * s,
        point.z * c - point.y * s,
    )
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
