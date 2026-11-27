use crate::game::ClientGame;
use sunrise_client::actions::Action;
use sunrise_client::mesher::Vertex;
use sunrise_sim::{
    mobs::leash::{self, Holder},
    pose::Pose,
};

/// `Vec3.yRot(angle)`.
/// `LocalPlayer.getRopeHoldPosition`'s `Mth.lerp(partialTick * 0.5F, getYRot(),
/// yRotO)` and the same for the pitch, in radians.
pub(in crate::game) fn rope_hold_angles(
    now: (f32, f32),
    old: (f32, f32),
    alpha: f32,
) -> (f64, f64) {
    let t = alpha * 0.5;
    let lerp = |a: f32, b: f32| a + t * (b - a);
    (
        f64::from(lerp(now.0, old.0) * (std::f32::consts::PI / 180.0)),
        f64::from(lerp(now.1, old.1) * (std::f32::consts::PI / 180.0)),
    )
}
fn rot_y(v: glam::DVec3, angle: f64) -> glam::DVec3 {
    let (s, c) = (angle.sin(), angle.cos());
    glam::DVec3::new(v.x * c + v.z * s, v.y, v.z * c - v.x * s)
}
/// `Entity.getRopeHoldPosition` of a boat: 0.7 of its eye height (0.5625) up.
fn boat_hold() -> glam::DVec3 {
    glam::DVec3::new(0.0, f64::from(0.5625_f32) * 0.7, 0.0)
}
fn pack_light(light: u8) -> u32 {
    (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8)
}
fn abgr(r: f32, g: f32, b: f32) -> u32 {
    let c = |v: f32| (v * 255.0) as u32 & 255;
    0xFF00_0000 | c(b) << 16 | c(g) << 8 | c(r)
}
/// One leashed entity's rope, as `EntityRenderer.extractRenderState` reads it.
struct Rope {
    holder: Holder,
    /// `getPosition(partialTicks)`.
    here: glam::DVec3,
    /// `getPreciseBodyRotation` in radians.
    body: f64,
    /// `getLeashOffset`.
    offset: [f64; 3],
    /// `getQuadLeashOffsets`, when the entity supports a quad lead.
    quad: Option<[[f64; 3]; 4]>,
}

impl ClientGame<'_> {
    /// `LeashFeatureRenderer` for every lead and `LeashKnotRenderer` for the knots.
    pub(in crate::game::client_side) fn leash_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let atlas = &self.client.gui.atlas;
        let white = atlas.font.white();
        let uv = [
            [white.u0, white.v1],
            [white.u1, white.v1],
            [white.u1, white.v0],
            [white.u0, white.v0],
        ];
        // A Polytone pack's `entity/lead.png`: each of the 24 segments, from the leashed end,
        // takes its row of the texture, unshaded (measured on the real mod).
        let lead = atlas.sprites.get("entity/lead").copied();
        let lerp3 = |a: sunrise_core::geometry::Vec3, b: sunrise_core::geometry::Vec3| {
            glam::DVec3::new(
                a.x + (b.x - a.x) * alpha,
                a.y + (b.y - a.y) * alpha,
                a.z + (b.z - a.z) * alpha,
            )
        };
        let mut knots: Vec<[i32; 3]> = Vec::new();
        // EntityRenderer.extractRenderState: a leashed mob or boat (an
        // `AbstractBoat` is `Leashable` too) with where it hangs.
        let body_of = |previous: f32, current: f32| {
            f64::from(previous + sunrise_sim::mobs::living::wrap(current - previous) * alpha as f32)
                .to_radians()
        };
        let mut ropes: Vec<Rope> = Vec::new();
        for m in &self.common.mobs {
            if let Some(holder) = m.extra.leash {
                // A mob a mod's pre hook canceled draws no lead (it is part of the mob's own
                // render), but the fence knot it hangs from is an entity of its own.
                if self.render_canceled(m.id) || self.hides_as_camera(m.id) {
                    if let Holder::Knot(p) = holder
                        && !knots.contains(&p)
                    {
                        knots.push(p);
                    }
                    continue;
                }
                ropes.push(Rope {
                    holder,
                    here: lerp3(m.previous, m.position),
                    body: body_of(m.previous_body_yaw, m.body_yaw),
                    offset: leash::leash_offset(m),
                    quad: leash::supports_quad_leash(m.species())
                        .then(|| leash::quad_leash_offsets(m)),
                });
            }
        }
        for b in &self.common.boats {
            if let Some(holder) = b.leash {
                if self.hides_as_camera(b.id) {
                    if let Holder::Knot(p) = holder
                        && !knots.contains(&p)
                    {
                        knots.push(p);
                    }
                    continue;
                }
                ropes.push(Rope {
                    holder,
                    here: lerp3(b.previous, b.position),
                    // Entity.getPreciseBodyRotation: the boat's yaw.
                    body: body_of(b.previous_yaw, b.yaw),
                    offset: leash::boat_leash_offset(),
                    quad: Some(leash::boat_quad_leash_offsets()),
                });
            }
        }
        for rope in ropes {
            let Rope {
                holder,
                here,
                body,
                offset,
                quad,
            } = rope;
            // One lead from the leash offset to the holder's rope hold
            // position, or, when the holder is a happy ghast and the leashee
            // supports a quad lead, four from `getQuadLeashOffsets` to
            // `getQuadLeashHolderOffsets`.
            let mut ends: Vec<(glam::DVec3, glam::DVec3, bool)> = Vec::new();
            let quad = match (holder, quad) {
                (Holder::Mob(id), Some(offsets)) => self
                    .common
                    .mobs
                    .iter()
                    .find(|o| o.id == id)
                    .and_then(|o| leash::quad_holder_offsets(o).map(|h| (o, h, offsets))),
                _ => None,
            };
            if let Some((roper, holder_points, own)) = quad {
                let roper_body = body_of(roper.previous_body_yaw, roper.body_yaw);
                let holder_pos = lerp3(roper.previous, roper.position);
                for (mine, theirs) in own.iter().zip(holder_points) {
                    let start = here + rot_y(glam::DVec3::from_array(*mine), -body);
                    let end = holder_pos + rot_y(glam::DVec3::from_array(theirs), -roper_body);
                    ends.push((start, end, false));
                }
            } else {
                let offset = rot_y(glam::DVec3::from_array(offset), -body);
                let start = here + offset;
                let end = match holder {
                    Holder::Knot(p) => {
                        if !knots.contains(&p) {
                            knots.push(p);
                        }
                        glam::DVec3::new(
                            f64::from(p[0]) + 0.5,
                            f64::from(p[1]) + 0.375 + 0.2,
                            f64::from(p[2]) + 0.5,
                        )
                    }
                    Holder::Mob(id) => {
                        let Some(o) = self.common.mobs.iter().find(|o| o.id == id) else {
                            continue;
                        };
                        lerp3(o.previous, o.position)
                            + glam::DVec3::new(0.0, o.eye_height() * 0.7, 0.0)
                    }
                    Holder::Boat(id) => {
                        let Some(o) = self.common.boats.iter().find(|o| o.id == id) else {
                            continue;
                        };
                        lerp3(o.previous, o.position) + boat_hold()
                    }
                    Holder::Player => {
                        let Some(p) = self.common.player.as_ref() else {
                            continue;
                        };
                        self.player_rope_hold(p, alpha)
                    }
                };
                ends.push((start, end, true));
            }
            let light = |p: glam::DVec3| {
                self.common
                    .world
                    .light(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32)
            };
            // Block and sky light averaged between the two ends.
            let (a, b) = (light(ends[0].0), light(ends[0].1));
            let packed = pack_light((((a >> 4) + (b >> 4)) / 2) << 4 | (((a & 15) + (b & 15)) / 2));
            for (start, end, slack) in ends {
                let d = end - start;
                let k = 1.0 / (d.x * d.x + d.z * d.z).sqrt().max(1e-6) * 0.05 / 2.0;
                let (dx_off, dz_off) = (d.z * k, d.x * k);
                let point = |step: i32, fudge: f64| {
                    let progress = f64::from(step) / 24.0;
                    // LeashFeatureRenderer.addVertexPair: a quad lead is straight.
                    let y = if !slack {
                        d.y * progress
                    } else if d.y > 0.0 {
                        d.y * progress * progress
                    } else {
                        d.y - d.y * (1.0 - progress) * (1.0 - progress)
                    };
                    let base = start + glam::DVec3::new(d.x * progress, y, d.z * progress);
                    (
                        base + glam::DVec3::new(-dx_off, fudge, dz_off),
                        base + glam::DVec3::new(dx_off, 0.05 - fudge, -dz_off),
                    )
                };
                for (fudge, backwards) in [(0.05, false), (0.0, true)] {
                    for step in 0..24 {
                        let shade = |s: i32| {
                            let dark = s % 2 == i32::from(backwards);
                            let f = if dark { 0.7 } else { 1.0 };
                            abgr(0.5 * f, 0.4 * f, 0.3 * f)
                        };
                        let (a0, b0) = point(step, fudge);
                        let (a1, b1) = point(step + 1, fudge);
                        let (color, rows) = match lead {
                            Some(r) => {
                                let v = |s: i32| r.v0 + (r.v1 - r.v0) * s as f32 / 24.0;
                                let (top, bottom) = (v(step), v(step + 1));
                                let near =
                                    [[r.u0, top], [r.u1, top], [r.u1, bottom], [r.u0, bottom]];
                                let far =
                                    [[r.u0, bottom], [r.u1, bottom], [r.u1, top], [r.u0, top]];
                                (0xFFFF_FFFF, [near, far])
                            }
                            None => (shade(step), [uv; 2]),
                        };
                        // Both faces: the leash is not culled.
                        for (quad, uv) in [[a0, b0, b1, a1], [a1, b1, b0, a0]].into_iter().zip(rows)
                        {
                            for (v, uv) in quad.iter().zip(uv) {
                                out.push(Vertex {
                                    normal: [0; 4],
                                    pos: (*v - camera).as_vec3().to_array(),
                                    uv,
                                    layer: 0,
                                    color,
                                    light: packed,

                                    glint_uv: [0.0, 0.0],
                                });
                            }
                        }
                    }
                }
            }
        }
        // The knots: a 6 by 8 by 6 pixel block on the fence post.
        let Some(model) = self.client.entity_models.leash_knot.as_ref() else {
            return;
        };
        let Some(r) = atlas.sprites.get("entity/lead_knot/lead_knot").copied() else {
            return;
        };
        for p in knots {
            if self
                .client
                .remote_knots
                .iter()
                .any(|(&id, &pos)| pos == p && self.hides_as_camera(id))
            {
                continue;
            }
            let state = leash::knot_state(p);
            let at =
                glam::DVec3::new(state.position.x, state.position.y, state.position.z) - camera;
            let base = glam::Mat4::from_translation(at.as_vec3())
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0));
            let poses = sunrise_sim::mobs::anim::Poses::new(model);
            let light = pack_light(self.common.world.light(p[0], p[1], p[2]));
            sunrise_client::mobs::render::draw(
                model,
                base,
                &poses,
                [r.u0, r.v0, r.u1, r.v1],
                0xFFFF_FFFF,
                light,
                0,
                out,
            );
        }
    }
    /// `Player.getRopeHoldPosition` (and `LocalPlayer`'s in first person).
    pub(in crate::game::client_side) fn player_rope_hold(
        &self,
        p: &sunrise_sim::motion::PlayerMotion,
        alpha: f64,
    ) -> glam::DVec3 {
        // Player.getRopeHoldPosition: `0.22 * (RIGHT ? -1 : 1)`.
        let hand_dir = if self.client.settings.main_hand_left {
            1.0
        } else {
            -1.0
        };
        if self.client.camera_type.is_first_person() {
            let (yaw, pitch) = rope_hold_angles(
                (self.common.yaw, self.common.pitch),
                self.client.rope_rot_old,
                alpha as f32,
            );
            let v = glam::DVec3::new(0.39 * hand_dir, -0.6, 0.3);
            // Vec3.xRot(-pitch) then yRot(-yaw).
            let (s, c) = ((-pitch).sin(), (-pitch).cos());
            let v = glam::DVec3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s);
            // Entity.getEyePosition(partialTick).
            let feet = self.common.previous_position.lerp(p.position, alpha);
            let eye = glam::DVec3::new(feet.x, feet.y + f64::from(p.dimensions.eye_height), feet.z);
            return rot_y(v, -yaw) + eye;
        }
        // Player.getRopeHoldPosition: the view's pitch eased toward `xRotO`,
        // the body's yaw between `yBodyRotO` and `yBodyRot`, from
        // `getPosition(partialTick)`.
        let (_, x_rot) = rope_hold_angles(
            (self.common.yaw, self.common.pitch),
            self.client.rope_rot_old,
            alpha as f32,
        );
        let t = alpha as f32;
        let y_rot = self.client.player_model.as_ref().map_or(
            f64::from(self.common.yaw * (std::f32::consts::PI / 180.0)),
            |m| {
                f64::from(
                    (m.previous_body_yaw + t * (m.body_yaw - m.previous_body_yaw))
                        * (std::f32::consts::PI / 180.0),
                )
            },
        );
        let feet = self.common.previous_position.lerp(p.position, alpha);
        let pos = glam::DVec3::new(feet.x, feet.y, feet.z);
        let x_rotated = |v: glam::DVec3| {
            // Vec3.xRot(-xRot).
            let (s, c) = ((-x_rot).sin(), (-x_rot).cos());
            glam::DVec3::new(v.x, v.y * c + v.z * s, v.z * c - v.y * s)
        };
        if p.fall_flying || p.auto_spin_attack_ticks > 0 {
            // getViewVector(partialTick) against getDeltaMovement, flat.
            let look = sunrise_world::view_direction::view_vector(
                self.client.rope_rot_old.1 + t * (self.common.pitch - self.client.rope_rot_old.1),
                self.client.rope_rot_old.0 + t * (self.common.yaw - self.client.rope_rot_old.0),
            );
            let movement = p.velocity;
            let speed_len = movement.x * movement.x + movement.z * movement.z;
            let look_len = look.x * look.x + look.z * look.z;
            let z_rot = if speed_len > 0.0 && look_len > 0.0 {
                let dot =
                    (movement.x * look.x + movement.z * look.z) / (speed_len * look_len).sqrt();
                let sign = movement.x * look.z - movement.z * look.x;
                // Math.signum: 0 for 0.
                let signum = if sign > 0.0 {
                    1.0
                } else if sign < 0.0 {
                    -1.0
                } else {
                    0.0
                };
                f64::from((signum * sunrise_core::jmath::acos(dot)) as f32)
            } else {
                0.0
            };
            // Vec3(xOff, -0.11, 0.85).zRot(-zRot).xRot(-xRot).yRot(-yRot).
            let v = glam::DVec3::new(0.22 * hand_dir, -0.11, 0.85);
            let (s, c) = ((-z_rot).sin(), (-z_rot).cos());
            let v = glam::DVec3::new(v.x * c + v.y * s, v.y * c - v.x * s, v.z);
            return pos + rot_y(x_rotated(v), -y_rot);
        }
        // LivingEntity.isVisuallySwimming.
        if p.pose == Pose::Swimming || (!p.fall_flying && p.pose == Pose::FallFlying) {
            let v = glam::DVec3::new(0.22 * hand_dir, 0.2, -0.15);
            return pos + rot_y(x_rotated(v), -y_rot);
        }
        let crouching = self.held_mapping(Action::Sneak);
        let v = glam::DVec3::new(
            0.22 * hand_dir,
            f64::from(p.dimensions.height) - 1.0,
            if crouching { -0.2 } else { 0.07 },
        );
        pos + rot_y(v, -y_rot)
    }
}
