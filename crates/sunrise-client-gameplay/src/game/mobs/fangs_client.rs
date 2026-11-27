use crate::game::ClientGame;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::fangs_common::fang_attack_pitch;
use sunrise_level::particle_options::{ParticleOptions, ParticleType};
use sunrise_sim::mobs::anim::Poses;

/// One of the twelve `crit` particles `EvokerFangs.tick` spawns on a client
/// when `lifeTicks` hits 14. The box is 0.5 wide (`getBbWidth() * 0.5`).
pub fn fang_crit(random: &mut impl RandomSource, pos: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let half = 0.5 * 0.5;
    let x = pos[0] + (random.next_double() * 2.0 - 1.0) * half;
    let y = pos[1] + 0.05 + random.next_double();
    let z = pos[2] + (random.next_double() * 2.0 - 1.0) * half;
    let xd = (random.next_double() * 2.0 - 1.0) * 0.3;
    let yd = 0.3 + random.next_double() * 0.3;
    let zd = (random.next_double() * 2.0 - 1.0) * 0.3;
    ([x, y + 1.0, z], [xd, yd, zd])
}

fn p_zrot(p: &mut Poses, part: &str, v: f32) {
    p.zrot(part, v);
}

impl ClientGame<'_> {
    /// `EvokerFangsRenderer`: the jaws snapping up and closing.
    pub(in crate::game) fn fangs_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<sunrise_client_render::render_geometry::Vertex>,
    ) {
        let Some(model) = self.client.entity_models.evoker_fangs.as_ref() else {
            return;
        };
        let Some(r) = self
            .client
            .gui
            .atlas
            .sprites
            .get("entity/illager/evoker_fangs")
            .copied()
        else {
            return;
        };
        let partial = alpha as f32;
        for f in &self.common.fangs.list {
            if !f.started || self.hides_as_camera(f.id) {
                continue;
            }
            // getAnimationProgress.
            let i = f.life - 2;
            let bite = if i <= 0 {
                1.0
            } else {
                1.0 - (i as f32 - partial) / 20.0
            };
            if bite == 0.0 {
                continue;
            }
            let mut poses = Poses::new(model);
            let amount = (bite * 2.0).min(1.0);
            let amount = 1.0 - amount * amount * amount;
            let pi = std::f32::consts::PI;
            p_zrot(&mut poses, "upper_jaw", pi - amount * 0.35 * pi);
            p_zrot(&mut poses, "lower_jaw", pi + amount * 0.35 * pi);
            poses.get("base")[1] -= (bite + (bite * 2.7).sin()) * 7.2;
            let pre = if bite > 0.9 { (1.0 - bite) / 0.1 } else { 1.0 };
            let root = poses.get("root");
            root[1] = 24.0 - 20.0 * pre;
            root[6] = pre;
            root[7] = pre;
            root[8] = pre;
            let light = self.common.world.light(
                f.pos[0].floor() as i32,
                f.pos[1].floor() as i32,
                f.pos[2].floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let base =
                glam::Mat4::from_translation((glam::DVec3::from_array(f.pos) - camera).as_vec3())
                    * glam::Mat4::from_rotation_y((90.0 - f.yaw).to_radians())
                    * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                    * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
            sunrise_client::mobs::render::draw(
                model,
                base,
                &poses,
                [r.u0, r.v0, r.u1, r.v1],
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
        }
    }
    /// `EvokerFangs.handleEntityEvent`: only event 4 starts a client attack.
    /// `playLocalSound` at `nextFloat() * 0.2 + 0.85`. The fangs this client
    /// tracks carry no Silent flag, so the sound always plays.
    pub(in crate::game::client_side) fn remote_fangs_event(&mut self, id: u32, event: i8) {
        let sound = self
            .common
            .fangs
            .list
            .iter()
            .position(|f| f.id == id)
            .and_then(|i| {
                self.common.fangs.list.update(i, |f| {
                    if event != 4 {
                        return None;
                    }
                    f.started = true;
                    let pitch = fang_attack_pitch(&mut f.random);
                    Some((f.pos, pitch))
                })
            });
        if let Some((pos, pitch)) = sound {
            self.play_audio_unshared(
                "minecraft:entity.evoker_fangs.attack",
                Some(pos),
                1.0,
                pitch,
            );
        }
    }
    /// `EvokerFangs.tick`: clients only advance animation, never damage or discard.
    /// At life 14, twelve `crit` particles (`ATTACK_TRIGGER_TICKS`).
    pub(in crate::game::client_side) fn tick_remote_fangs(&mut self) {
        let mut bursts = Vec::new();
        self.common.fangs.list.for_each_mut(|f| {
            if f.started {
                f.life -= 1;
                if f.life == 14 {
                    let mut crits = Vec::with_capacity(12);
                    for _ in 0..12 {
                        crits.push(fang_crit(&mut f.random, f.pos));
                    }
                    bursts.push(crits);
                }
            }
        });
        let crit = ParticleOptions::simple(ParticleType::Crit);
        for crits in bursts {
            for (at, velocity) in crits {
                self.client.particles.0.add(&crit, at, velocity);
            }
        }
    }
}
