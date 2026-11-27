//! A player's melee attack (`Player.attack`): the attack strength, critical and sweep attacks, and
//! what a hit on a target that is not a mob does.
use super::*;

impl crate::game::ServerGame<'_> {
    /// `Player.getAttackStrengthScale(0.5)`.
    pub(in crate::game) fn attack_strength_scale(&self) -> f32 {
        self.attack_strength_scale_at(0.5)
            .expect("finite partial tick")
    }

    /// `Player.getAttackStrengthScale(partial)` for the player in place.
    /// Returns `None` for a non-finite partial tick; finite partials are not restricted.
    pub(in crate::game) fn attack_strength_scale_at(&self, partial: f32) -> Option<f32> {
        if !partial.is_finite() {
            return None;
        }
        let delay = sunrise_sim::attack_strength::delay(self.main_hand_attack_speed());
        Some(((self.server.attack_ticks as f32 + partial) / delay).clamp(0.0, 1.0))
    }

    /// Sets the player in place's `attackStrengthTicker` to the fewest ticks for which
    /// `getAttackStrengthScale(0.5)` is 1.0 with the main hand's attack speed.
    pub(in crate::game::server_side) fn charge_attack_fully(&mut self) {
        let delay = sunrise_sim::attack_strength::delay(self.main_hand_attack_speed());
        // The cast saturates: with no attack speed the delay is infinite and no ticker reaches it.
        self.server.attack_ticks = (delay - 0.5).ceil() as i32;
    }

    /// `Player.isSweepAttack`: a charged, grounded sword hit below the movement
    /// limit, which `native_sweep_attack` may force or stop for the hit on `target`.
    pub(in crate::game) fn sweep_attack_position(
        &mut self,
        scale: f32,
        crit: bool,
        target: &str,
    ) -> Option<[f64; 3]> {
        // Player.getSpeed: the movement speed attribute as it is now (the
        // field is only refreshed by the local movement step, never for a
        // server's player).
        let p = self.common.player.as_mut()?;
        if let Ok(speed) = p.attribute("minecraft:movement_speed") {
            p.speed = speed as f32;
        }
        if !self.sweep_decision(scale, crit, target) {
            return None;
        }
        let p = self.common.player.as_ref()?;
        // Mth.sin / Mth.cos of yRot * (PI / 180), from its sine table.
        use sunrise_core::geometry::trig;
        let yaw = f64::from(self.common.yaw * trig::DEG_TO_RAD);
        Some([
            p.position.x - f64::from(trig::sin(yaw)),
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z + f64::from(trig::cos(yaw)),
        ])
    }

    /// `critical_hit` around the box `area` of entity `id` (`magic`: the
    /// enchanted hit instead of the crit).
    pub(in crate::game) fn area_critical_hit(&mut self, id: i32, area: Aabb, magic: bool) {
        let (feet, size) = (area.bottom_center(), area.size());
        self.critical_hit(
            self.server.lan_acting,
            id,
            [feet.x, feet.y, feet.z],
            size.x,
            size.y,
            magic,
        );
    }

    /// Player.attack with the attack-strength charge, crits and knockback.
    pub fn attack(&mut self) -> bool {
        if self.common.spectator() {
            return false;
        }
        if let Some(hit) = self.server_crosshair_entity_hit() {
            if self.common.falling.iter().any(|f| f.id as i32 == hit.id) {
                return false;
            }
            if let Some(tnt_id) = self
                .common
                .tnts
                .iter()
                .find(|t| t.id as i32 == hit.id)
                .map(|t| t.id)
            {
                self.strike_general_row(tnt_id, "minecraft:tnt");
                return true;
            }
        }
        // A painting nearer than the targeted block and mob breaks instead.
        if let Some((i, d)) = self.painting_hit()
            && self.mob_hit().is_none_or(|m| d < m.1)
            && self.before_block(d)
        {
            // `BlockAttachedEntity.hurtServer` refuses first through `isInvulnerableToBase`.
            if self
                .native_player_hit_invulnerable(self.common.paintings[i].id, "minecraft:painting")
            {
                self.server.attack_ticks = 0;
            } else {
                self.break_painting(i, true);
            }
            return true;
        }
        // A ghast's fireball or a wind charge in front: turned back.
        if self.attack_redirectable() {
            return true;
        }
        if self.attack_frame()
            || self.attack_armor_stand()
            || self.attack_cushion()
            || self.attack_boat()
            || self.attack_cart()
            || self.attack_dragon()
            || self.attack_general_row()
        {
            return true;
        }
        let Some((index, distance, _)) = self.mob_hit() else {
            return false;
        };
        if !self.before_block(distance) || !self.held_range_allows(distance) {
            return false;
        }
        self.attack_mob(index);
        // Minecraft.startAttack swings after the attack.
        self.swing_arm();
        true
    }
}

impl crate::game::ServerGame<'_> {
    /// Whether the ray picked a general row and consumed the click.
    pub(in crate::game::server_side) fn attack_general_row(&mut self) -> bool {
        let Some((eye, direction)) = self.aim() else {
            return false;
        };
        let catalog = sunrise_gameplay_common::game::general::general_type_catalog();
        let Some((id, kind, distance)) = self
            .common
            .resident_entities
            .general
            .rows
            .iter()
            .filter(|row| catalog.pickable(row.type_slot))
            .filter_map(|row| {
                let hit = sunrise_gameplay_common::game::ray_box(
                    eye,
                    direction,
                    self.common.entity_reach,
                    sunrise_gameplay_common::game::entity::Entity::bounds(row),
                )?;
                Some((row.id, catalog.key(row.type_slot), hit))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
        else {
            return false;
        };
        if !self.before_block(distance)
            || !self.held_range_allows(distance)
            || self.mob_hit().is_some_and(|(_, d, _)| d <= distance)
        {
            return false;
        }
        self.strike_general_row(id, kind);
        true
    }

    /// `Player.attack` on a named general row. A plain `Entity` declines `hurtServer`,
    /// so the blow spends the charge and plays the no-damage sound, and hurts nothing.
    pub(in crate::game) fn strike_general_row(&mut self, id: u32, kind: &str) {
        let ((_, scale, _, _), admitted) = self.melee_damage_with_admission(kind, false, Some(id));
        // Player.onAttack: the blow starts the attack strength again, hurt or not.
        self.server.attack_ticks = 0;
        if admitted {
            if scale > 0.9 && self.common.player.as_ref().is_some_and(|p| p.sprinting) {
                self.server_play_at_feet("minecraft:entity.player.attack.knockback");
            }
            self.sweep_decision(scale, false, kind);
            self.server_play_at_feet("minecraft:entity.player.attack.nodamage");
        }
        self.post_piercing_attack();
        self.swing_arm();
    }
}

impl crate::game::ServerGame<'_> {
    /// `Player.attack` on a mob: the damage, knockback, sweep, particles and
    /// sounds.
    pub fn attack_mob(&mut self, index: usize) {
        let attack = self.attack_of(self.common.player.as_ref().map_or(0, |p| p.entity_id));
        let target_kind = self.common.mobs[index].kind.id.clone();
        let target_kind = format!("minecraft:{}", target_kind.trim_start_matches("minecraft:"));
        let ((amount, scale, crit, magic), admitted) =
            self.melee_damage_with_admission(&target_kind, true, Some(self.common.mobs[index].id));
        // Player.onAttack: the hit starts the attack strength again, hurt or not.
        self.server.attack_ticks = 0;
        if !admitted {
            // Player.attack skips the hurt and its whole tail when the hit has neither
            // base nor enchantment damage (`baseDamage > 0.0F || magicBoost > 0.0F`):
            // no hurt, no `causeExtraKnockback`, no `setLastHurtMob`, no exhaustion.
            return;
        }
        let sweep = self.sweep_attack_position(scale, crit, &target_kind);
        let knockback_attack =
            scale > 0.9 && self.common.player.as_ref().is_some_and(|p| p.sprinting);
        let player = self.common.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if knockback_attack && let Some(at) = from {
            self.server_play_sound("minecraft:entity.player.attack.knockback", at, 1.0, 1.0);
        }
        let old_health = self.common.mobs[index].health;
        let source = match from {
            Some(f) => self.attack_source(f),
            None => sunrise_sim::mobs::Damage::of(sunrise_sim::mobs::DamageKind::Generic),
        };
        let (id, join) = (self.common.mobs[index].id, self.common.mobs[index].join);
        let (landed, _) = self.hurt_by_player(index, amount, source);
        let Some(index) = self
            .common
            .mob_index
            .find(&self.common.mobs, id)
            .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
        else {
            if landed {
                self.remember_player_target(Some(id));
            }
            return;
        };
        if landed {
            let victim = self.mob_entity(&self.common.mobs[index]);
            self.player_hit(
                &victim,
                sunrise_gameplay_common::game::creatures::damage_type(source.kind, true),
                None,
                amount,
            );
        }
        if let Some(p) = self.common.player.as_mut() {
            if p.attack_log.len() >= 64 {
                p.attack_log.remove(0);
            }
            p.attack_log.push(sunrise_sim::health::AttackRecord {
                game_time: p.health.game_time(),
                target: target_kind.clone(),
                id: self.common.mobs[index].id,
                killed: landed && !self.common.mobs[index].alive(),
                amount,
                scale,
                crit,
                landed,
                dealt: old_health - self.common.mobs[index].health,
            });
        }
        if landed {
            let (knockback, _) = self.extra_knockback(&target_kind, id, scale, None);
            if knockback > 0.0 {
                let (xd, zd) = self.extra_knockback_direction();
                let aim = self.attacker_aim(source.attacker);
                let plan =
                    self.prepare_knockback_credit(self.common.mobs[index].id as i32, None, &attack);
                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                self.common.mobs[index].attacker_aim = aim;
                let level = crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &attack,
                    &mut credit,
                    None
                );
                self.common.mobs[index].knockback_full(
                    level,
                    f64::from(knockback),
                    xd,
                    zd,
                    &source,
                    amount,
                    true,
                );
                self.common.mobs[index].attacker_aim = None;
                self.common.stop_sprint_after_extra_knockback();
            }
            let mob = &self.common.mobs[index];
            let actual = old_health - mob.health; // Player.damageStatsAndHearts
            // Player.attack: crit emitter and damage indicator hearts.
            let (feet, width, height) = (
                [mob.position.x, mob.position.y, mob.position.z],
                mob.kind.width,
                mob.kind.height,
            );
            self.award_custom_stat("damage_dealt", (actual * 10.0).round() as i32);
            if crit {
                self.critical_hit(
                    self.server.lan_acting,
                    self.common.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    false,
                );
            }
            // Player.attackVisualEffects: magicCrit when the enchantments added damage.
            if magic {
                self.critical_hit(
                    self.server.lan_acting,
                    self.common.mobs[index].id as i32,
                    feet,
                    width,
                    height,
                    true,
                );
            }
            if actual > 2.0 {
                // The players a server serves are sent them too.
                self.server_send_particles(
                    sunrise_sim::lan::server_effects::ServerQueuedParticle {
                        dimension: self.common.dim.id(),
                        particle: sunrise_sim::lan::server_effects::ServerParticle::simple(
                            sunrise_level::particle_options::ParticleType::DamageIndicator,
                        ),
                        at: [feet[0], feet[1] + height * 0.5, feet[2]],
                        spread: [0.1, 0.0, 0.1],
                        speed: [0.2; 3],
                        count: (actual * 0.5) as u32 as i32,
                        randomization: 0,
                        except: None,
                    },
                );
            }
            let sound = if crit {
                "minecraft:entity.player.attack.crit"
            } else if scale > 0.9 {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            // Player.playServerSideSound: at the player. A critical hit a mod
            // let sweep has both sounds (attackVisualEffects).
            if let Some(at) = sweep {
                let target = (
                    self.common.mobs[index].bounds(),
                    mace::Spared::Mob(self.common.mobs[index].id),
                );
                self.sweep_attack(at, from, Some(target), scale);
            }
            if (crit || sweep.is_none())
                && let Some(at) = from
            {
                self.server_play_sound(sound, at, 1.0, 1.0);
            }
            self.remember_player_target(Some(id));
            // The hurt or death sound the mob raised (getHurtSound / getDeathSound,
            // its voice pitch from its own random). A sweep's own hurt/death
            // listeners may have changed the entity list.
            let Some(index) = self
                .common
                .mob_index
                .find(&self.common.mobs, id)
                .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
            else {
                return;
            };
            let queued = std::mem::take(&mut self.common.mobs[index].outbox);
            let mut infested = Vec::new();
            for event in queued {
                match event {
                    sunrise_sim::mobs::MobEvent::Sound(s, at, volume, pitch) => {
                        self.server_play_sound(&format!("minecraft:{s}"), at, volume, pitch);
                    }
                    sunrise_sim::mobs::MobEvent::Infested { at, velocities } => {
                        infested.push((at, velocities))
                    }
                    other => {
                        // Left for the end of the tick.
                        self.common.mobs[index].outbox.push(other);
                    }
                }
            }
            for (at, velocities) in infested {
                self.spawn_infested(at, velocities);
            }
            if let Some(i) = self.common.mob_index.find(&self.common.mobs, id)
                && self.common.mobs[i].join == join
                && !self.common.mobs[i].removed
            {
                self.weapon_hit(i, source);
            }
            // Player.attack: causeFoodExhaustion(0.1F) only when the hit hurt
            // (inside `if (wasHurt)`), after damageStatsAndHearts.
            if let Some(p) = self.common.player.as_mut()
                && !self.common.creative
            {
                p.cause_food_exhaustion(0.1);
            }
        } else {
            if let Some(at) = from {
                self.server_play_sound("minecraft:entity.player.attack.nodamage", at, 1.0, 1.0);
            }
        }
    }
    /// `Player.attack`'s damage against `target_kind` (with the attack
    /// strength and whether it is a critical hit): the weapon and effects
    /// scaled by the attack strength, a mace's smash, the critical hit, then
    /// enchantments.
    pub(in crate::game::server_side) fn melee_damage(
        &mut self,
        target_kind: &str,
        living: bool,
        victim: Option<u32>,
    ) -> (f32, f32, bool, bool) {
        self.melee_damage_with_admission(target_kind, living, victim)
            .0
    }
    /// Player.attack admits its hurt/tail before the item's damage bonus. Keep
    /// that predicate with the one damage evaluation, including its RNG draws.
    pub(in crate::game::server_side) fn melee_damage_with_admission(
        &mut self,
        target_kind: &str,
        living: bool,
        victim: Option<u32>,
    ) -> ((f32, f32, bool, bool), bool) {
        let hand_damage = self.attack_damage();
        // Riptide's spin replaces the attack damage attribute.
        let spin = self
            .common
            .player
            .as_ref()
            .map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 { spin } else { hand_damage };
        let scale = self.attack_strength_scale();
        // getEnchantedDamage(target, damage, source) - damage, taken before the
        // attack strength scales the base damage.
        let enchant_bonus = self.enchanted_damage(target_kind, damage, victim) - damage;
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        // Player.attack decides the critical hit only for a hit with base or
        // enchantment damage: checked before the mace's smash bonus is added.
        let damaging = amount > 0.0 || enchant_bonus * scale > 0.0;
        // Item.getAttackDamageBonus (a mace's smash), before the critical hit.
        amount += self.smash_damage_bonus(target_kind, victim);
        let vanilla_crit = living && self.common.critical_attack(scale);
        let mut crit = vanilla_crit;
        let mut multiplier = if vanilla_crit { 1.5 } else { 1.0 };
        if living && damaging {
            sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_critical_hit(
                self,
                target_kind,
                vanilla_crit,
                &mut crit,
                &mut multiplier,
            );
        }
        if crit {
            amount *= multiplier;
        }
        // Enchantment damage (sharpness, smite, bane) scaled by the attack strength.
        amount += enchant_bonus * scale;
        ((amount, scale, crit, enchant_bonus * scale > 0.0), damaging)
    }
}

impl crate::game::ServerGame<'_> {
    /// `Player.attack` after a target that is not a mob took the hit:
    /// the sprint knockback sound, a living target's critical hit (`living`:
    /// its entity id; `crit`: whether `melee_damage` decided the hit critical,
    /// mods' `critical_hit` included), the `sweep` decided before the hit
    /// (`sweep_before_hit`) around the target's box `area` and the strong or weak sound.
    pub(in crate::game) fn attack_effects(
        &mut self,
        area: Aabb,
        sweep: Option<[f64; 3]>,
        living: Option<i32>,
        crit: bool,
        spared: mace::Spared,
        magic: Option<i32>,
    ) {
        let scale = self.attack_strength_scale();
        let full = scale > 0.9;
        let player = self.common.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if full
            && player.is_some_and(|p| p.sprinting)
            && let Some(at) = from
        {
            self.server_play_sound("minecraft:entity.player.attack.knockback", at, 1.0, 1.0);
        }
        let crit = living.is_some() && crit;
        if let Some(at) = sweep {
            self.sweep_attack(at, from, Some((area, spared)), scale);
        }
        if crit {
            if let Some(at) = from {
                self.server_play_sound("minecraft:entity.player.attack.crit", at, 1.0, 1.0);
            }
            if let Some(id) = living {
                self.area_critical_hit(id, area, false);
            }
        } else if sweep.is_none() {
            let sound = if full {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            if let Some(at) = from {
                self.server_play_sound(sound, at, 1.0, 1.0);
            }
        }
        // Player.magicCrit: the enchantments added damage.
        if let Some(id) = magic {
            self.area_critical_hit(id, area, true);
        }
    }
    /// `Player.doSweepAttack` after a successful hit, in vanilla's order: the
    /// sweep sound, the victims around `target` (its box, and the mob that
    /// was hit, when there is one) and then the arc.
    pub(in crate::game) fn sweep_attack(
        &mut self,
        at: [f64; 3],
        from: Option<[f64; 3]>,
        target: Option<(Aabb, mace::Spared)>,
        scale: f32,
    ) {
        if let Some(at) = from {
            self.server_play_sound("minecraft:entity.player.attack.sweep", at, 1.0, 1.0);
        }
        if let Some((area, mob)) = target {
            self.sweep_damage(area, mob, scale);
        }
        // Player.sweepAttack's `sendParticles(SWEEP_ATTACK, ..., 0, dx, 0, dz, 0)`:
        // the offset is the one from the player to the particle.
        let origin = from.unwrap_or(at);
        let spread = [at[0] - origin[0], 0.0, at[2] - origin[2]];
        self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
            dimension: self.common.dim.id(),
            particle: sunrise_sim::lan::server_effects::ServerParticle::simple(
                sunrise_level::particle_options::ParticleType::SweepAttack,
            ),
            at,
            spread,
            speed: [0.0; 3],
            count: 0,
            randomization: 0,
            except: None,
        });
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;
