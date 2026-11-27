//! Lightning (`ServerLevel.tickThunder`, `LightningBolt`,
//! `LightningBoltRenderer`): strikes on the highest motion-blocking block or
//! a living entity under open sky nearby, skeleton trap horses, fire,
//! `thunderHit` (damage and burning, charged creepers, zombified piglins,
//! witches, mooshroom colours, turtles, burnt items), lightning rods, the sky
//! flash, the sounds and the branching additive bolt.
use crate::game::ClientGame;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::lightning_common::Bolt;

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `ClientLevel.addEntity` for a server's lightning bolt: on a client it
    /// only flashes, sounds and draws (`LightningBolt.tick` off the server).
    pub fn spawn_remote_bolt(&mut self, id: u32, at: [f64; 3]) {
        self.common.bolts.retain(|b| b.id != id);
        let (bolt, _) = Bolt::new(id, at, true);
        self.client.lightning_seen[0] += 1;
        self.common.bolts.push(bolt);
    }
    /// Client-side `LightningBolt.tick`: thunder/impact sounds and sky flash.
    pub fn client_tick_bolts(&mut self) {
        let _save_lease = sunrise_sim::join_seq::visibility::PassLease::enter();
        let ids: Vec<u32> = self.common.bolts.iter().map(|b| b.id).collect();
        for id in ids {
            let Some(i) = self.common.bolts.iter().position(|b| b.id == id) else {
                continue;
            };
            let (at, life, silent) = {
                let b = &self.common.bolts[i];
                (b.at, b.life, b.silent)
            };
            if life == 2 && !silent {
                let (thunder_pitch, impact_pitch) = self.common.bolts.update(i, |b| {
                    (
                        0.8 + b.random.next_float() * 0.2,
                        0.5 + b.random.next_float() * 0.2,
                    )
                });
                self.play_audio(
                    "minecraft:entity.lightning_bolt.thunder",
                    Some(at),
                    10000.0,
                    thunder_pitch,
                );
                self.play_audio(
                    "minecraft:entity.lightning_bolt.impact",
                    Some(at),
                    2.0,
                    impact_pitch,
                );
            }
            let Some(i) = self.common.bolts.iter().position(|b| b.id == id) else {
                continue;
            };
            enum Step {
                Discard,
                Reflash,
                Active,
                Idle,
            }
            let step = self.common.bolts.update(i, |b| {
                b.life -= 1;
                if b.life < 0 {
                    if b.flashes == 0 {
                        return Step::Discard;
                    }
                    if b.life < -b.random.next_int_bound(10).unwrap_or(0) {
                        b.flashes -= 1;
                        b.life = 1;
                        b.seed = b.random.next_long();
                        return Step::Reflash;
                    }
                }
                if b.life >= 0 {
                    Step::Active
                } else {
                    Step::Idle
                }
            });
            if matches!(step, Step::Discard) {
                self.common.bolts.retain(|b| b.id != id);
                continue;
            }
            if matches!(step, Step::Reflash | Step::Active) {
                if self.common.flash == 0 {
                    self.client.lightning_seen[1] += 1;
                }
                self.common.flash = 2;
            }
        }
    }
    /// `LightningBolt.shouldRenderAtSqrDistance`: a bolt is drawn within 64
    /// blocks times the view scale of the camera, and past that only its sky
    /// flash and thunder remain. Simulation-order render inputs; seeded
    /// geometry lives in the renderer.
    pub fn bolt_instances(&self, camera: glam::DVec3) -> Vec<sunrise_client::gfx::lightning::Bolt> {
        let range = 64.0 * self.view_scale();
        self.common
            .bolts
            .iter()
            .filter(|b| {
                !self.hides_as_camera(b.id) && {
                    let at = glam::DVec3::new(b.at[0], b.at[1], b.at[2]);
                    at.distance_squared(camera) < range * range
                }
            })
            .map(|b| sunrise_client::gfx::lightning::Bolt {
                at: b.at,
                seed: b.seed,
            })
            .collect()
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_sim::mobs::{Anim, Brain, Mob, MobType, Species};

    /// `Pig.thunderHit`: striking a pig with lightning converts it to a zombified
    /// piglin while preserving age (#7510).
    #[test]
    fn a_baby_pig_hit_by_lightning_becomes_a_baby_zombified_piglin() {
        let make_pig = |baby: bool| {
            let kind = Arc::new(MobType {
                type_key: Default::default(),
                id: "minecraft:pig".into(),
                species: Species::Pig,
                width: 0.9,
                height: 0.9,
                eye_height: 0.85,
                category: "creature".into(),
                max_health: 10.0,
                speed: 0.25,
                flying_speed: 0.02,
                attack_damage: 0.0,
                attack_knockback: 0.0,
                follow_range: 16.0,
                step_height: 0.6,
                armor: 0.0,
                armor_toughness: 0.0,
                knockback_resistance: 0.0,
                jump_strength: 0.42,
                attributes: Default::default(),
                item_stack_limits: Default::default(),
                attribute_bounds: Default::default(),
                gravity: 0.08,
                safe_fall_distance: 3.0,
                fall_damage_multiplier: 1.0,
                water_efficiency: 0.0,
                water_slowdown: 0.8,
                air_speed: 0.02,
                baby_scale: 0.5,
                head_rot_speed: 10.0,
                max_head_x_rot: 40.0,
                max_head_y_rot: 75.0,
                ambient_interval: 80,
                flags: Default::default(),
                brain: Brain::Passive,
                anim: Anim::Quadruped,
                sounds: ["", "", ""],
                sound_overrides: Default::default(),
                food: Default::default(),
                tempt: Default::default(),
                passenger_attachments: Vec::new(),
            });
            let mut pig = Mob::new(1, kind, Vec3::ZERO, 45.0);
            if baby {
                pig.extra.age = -24000;
            }
            pig
        };

        let adult_pig = make_pig(false);
        assert!(!adult_pig.is_baby());
        let baby_pig = make_pig(true);
        assert!(baby_pig.is_baby());
        let mut kind = (*adult_pig.kind).clone();
        kind.species = Species::ZombifiedPiglin;
        kind.id = "minecraft:zombified_piglin".into();
        let params = sunrise_sim::mobs::conversion::Params {
            keep_equipment: false,
            preserve_can_pick_up_loot: true,
        };
        for (pig, baby) in [(&adult_pig, false), (&baby_pig, true)] {
            let mut zombified = Mob::new(2, Arc::new(kind.clone()), Vec3::ZERO, 0.0);
            let mut pig = pig.clone();
            sunrise_sim::mobs::conversion::single(&pig, &mut zombified, params);
            sunrise_sim::mobs::conversion::pig_thunder_hit(&mut pig, &mut zombified);
            assert_eq!(zombified.is_baby(), baby);
            assert_eq!(zombified.yaw, 45.0);
            assert!(zombified.persistent);
        }
    }
}

// A conversion keeps the old mob's state; lives here for the lightning's.
