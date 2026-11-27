use crate::game::ClientGame;
#[allow(unused_imports)]
use sunrise_client::mesher::Vertex;
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_sim::thrown_trident::{HALF, ThrownTrident};
#[allow(unused_imports)]
use sunrise_sim::{entities::arrows::Pickup, mobs::anim::Poses};
use sunrise_sim::{entities::arrows::back_off, projectiles::ProjectileWorld};

impl ClientGame<'_> {
    /// ThrownTridentRenderer.
    pub(in crate::game) fn trident_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
        run: std::ops::Range<usize>,
    ) {
        // AbstractArrow.shouldRenderAtSqrDistance: ten times the default.
        let range = 10.0
            * super::entity_culling::default_range(
                glam::DVec3::splat(2.0 * HALF),
                self.view_scale(),
            );
        let live = self.common.tridents[run]
            .iter()
            .filter(|t| {
                !self.hides_as_camera(t.id)
                    && self.standing_entity_in_view(
                        camera,
                        super::entity_culling::at(t.previous),
                        super::entity_culling::at(t.position),
                        alpha,
                        2.0 * HALF,
                        2.0 * HALF,
                        range,
                        1.5,
                    )
            })
            .map(|t| {
                let at = [
                    t.previous.x + (t.position.x - t.previous.x) * alpha,
                    t.previous.y + (t.position.y - t.previous.y) * alpha,
                    t.previous.z + (t.position.z - t.previous.z) * alpha,
                ];
                (t, at)
            });
        self.draw_tridents(camera, live, out);
    }
    pub(in crate::game::client_side) fn draw_tridents<'a>(
        &self,
        camera: glam::DVec3,
        tridents: impl Iterator<Item = (&'a ThrownTrident, [f64; 3])>,
        out: &mut Vec<Vertex>,
    ) {
        let Some(model) = &self.client.trident_model else {
            return;
        };
        let Some(uv) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/trident/trident")
            .map(|r| [r.u0, r.v0, r.u1, r.v1])
        else {
            return;
        };
        // The texture's OptiFine emissive overlay, drawn again full bright.
        let glow = self
            .client_res()
            .client_mob_data()
            .emissive
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .overlay("entity/trident/trident")
            .and_then(|e| self.client.gui.atlas.sprites.get(e))
            .map(|r| [r.u0, r.v0, r.u1, r.v1]);
        for (t, at) in tridents {
            let p = glam::DVec3::from_array(at) - camera;
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((t.yaw - 90.0).to_radians())
                * glam::Mat4::from_rotation_z((t.pitch + 90.0).to_radians());
            let l = self.common.world.light(
                t.position.x.floor() as i32,
                t.position.y.floor() as i32,
                t.position.z.floor() as i32,
            );
            if t.fire_ticks > 0 {
                // The flames face the camera, turned round in the front view.
                let fire_yaw = if self.client.camera_type
                    == sunrise_level::camera_math::CameraType::ThirdPersonFront
                {
                    self.common.yaw + 180.0
                } else {
                    self.common.yaw
                };
                sunrise_client::fire_render::flames(
                    at,
                    0.5,
                    0.5,
                    camera,
                    fire_yaw,
                    &self.client.gui.atlas.sprites,
                    l,
                    out,
                );
            }
            let packed = (u32::from(l >> 4) * 16) | ((u32::from(l & 15) * 16) << 8);
            let poses = Poses::new(model);
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
            if let Some(glow) = glow {
                let full = sunrise_client::mobs::render::FULL_BRIGHT;
                sunrise_client::mobs::render::draw(
                    model,
                    base,
                    &poses,
                    glow,
                    0xFFFF_FFFF,
                    full,
                    0,
                    out,
                );
            }
        }
        self.mark_motion(out.len(), 0);
    }
    pub(in crate::game::client_side) fn pickup_trident_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        // ItemPickupParticle: a picked-up trident flying to its collector,
        // drawn with the render state extracted at partial tick 1.
        let taken = self
            .client
            .pickups
            .0
            .live
            .iter()
            .filter_map(|p| match &p.taken {
                super::pickups::Taken::Trident(t) => {
                    Some((&**t, super::pickups::position(p, alpha)))
                }
                _ => None,
            });
        self.draw_tridents(camera, taken, out);
    }
    pub(in crate::game) fn client_in_rain(&self, at: Vec3, top: f64) -> bool {
        let p = BlockPos::new(
            at.x.floor() as i32,
            at.y.floor() as i32,
            at.z.floor() as i32,
        );
        let up = BlockPos::new(p.x, top.floor() as i32, p.z);
        self.raining_at(p) || self.raining_at(up)
    }
    /// Client-side `TridentItem.releaseUsing` for Riptide spin prediction.
    pub(in crate::game) fn client_release_trident(&mut self, ticks: u32) {
        if ticks < 10 {
            return;
        }
        let Some(stack) = self.common.held().cloned() else {
            return;
        };
        if self.common.eye_position().is_none() {
            return;
        }
        let riptide = stack.enchantment_level("minecraft:riptide");
        if riptide == 0 || !self.in_water_or_rain() || self.common.riding.is_some() {
            return;
        }
        if stack.damage >= self.trident_max_damage(&stack).saturating_sub(1) {
            return;
        }
        let spin_stack = self.common.held().cloned();
        let strength = 1.5 + 0.75 * (riptide as f32 - 1.0);
        use sunrise_core::geometry::trig;
        let (y, x) = (
            f64::from(self.common.yaw * trig::DEG_TO_RAD),
            f64::from(self.common.pitch * trig::DEG_TO_RAD),
        );
        let xd = -trig::sin(y) * trig::cos(x);
        let yd = -trig::sin(x);
        let zd = trig::cos(y) * trig::cos(x);
        let scale = strength / f64::from(xd * xd + yd * yd + zd * zd).sqrt() as f32;
        if let Some(p) = self.common.player.as_mut() {
            p.velocity = Vec3::new(
                p.velocity.x + f64::from(xd * scale),
                p.velocity.y + f64::from(yd * scale),
                p.velocity.z + f64::from(zd * scale),
            );
            p.auto_spin_attack_ticks = 20;
            p.auto_spin_attack_dmg = 8.0;
            p.auto_spin_attack_item = spin_stack;
        }
        if self
            .common
            .player
            .as_ref()
            .is_some_and(|p| p.ground.on_ground)
        {
            let _ = self.move_local_player(
                sunrise_world::entity_move::MoverType::SelfMovement,
                Vec3::new(0.0, f64::from(1.1999999f32), 0.0),
                true,
            );
        }
    }
}

/// `ThrownTrident.tick` on a server's client: it flies on by itself
/// between the server's updates (`AbstractArrow.tick`: gravity and drag,
/// a block holds it), and Loyalty pulls it back to its owner; only the
/// server decides what it hits.
pub(in crate::game) fn remote_tick(
    t: &mut ThrownTrident,
    w: &impl ProjectileWorld,
    owner_eye: Option<Vec3>,
) {
    t.previous = t.position;
    t.previous_yaw = t.yaw;
    t.previous_pitch = t.pitch;
    t.shake -= i32::from(t.shake > 0);
    if t.in_ground_time > 4 {
        t.dealt_damage = true;
    }
    if t.loyalty > 0
        && (t.dealt_damage || t.no_physics)
        && let Some(eye) = owner_eye
    {
        t.no_physics = true;
        t.in_ground = false;
        t.loyalty_pull(eye);
    }
    // AbstractArrow.tick releases a changed block only on the server;
    // clients release from the non-SELF move after a piston push.
    if t.in_ground && !t.no_physics {
        t.in_ground_time += 1;
        return;
    }
    t.in_ground_time = 0;
    let m = t.velocity;
    let (x, z) = if t.no_physics {
        (-m.x, -m.z)
    } else {
        (m.x, m.z)
    };
    // Projectile.updateRotation's targets: Mth.atan2 times the folded 180F / (float)PI, the pitch over
    // Vec3.horizontalDistance.
    t.yaw = sunrise_core::geometry::trig::atan2_degrees(x, z);
    t.pitch = sunrise_core::geometry::trig::atan2_degrees(m.y, m.horizontal_distance());
    let to = t.position.add_vector(m);
    if !t.no_physics
        && let Some((at, _, _)) = w.clip(t.position, to)
    {
        // AbstractArrow.onHitBlock's back-off, as the server's tick does it.
        t.position = back_off(at, m);
        t.velocity = Vec3::ZERO;
        t.in_ground = true;
        t.shake = 7;
        return;
    }
    t.position = to;
    // A trident's water inertia (0.99) equals the air drag.
    t.velocity = t.velocity.scale(0.99);
    if !t.no_physics && !t.no_gravity {
        t.velocity.y -= 0.05;
    }
}

use sunrise_gameplay_common::game::weather::WeatherGameExt as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::piston_push::PistonPushGameExt1 as _;
