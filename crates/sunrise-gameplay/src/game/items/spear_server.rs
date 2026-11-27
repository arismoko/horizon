use super::view_vector;
use sunrise_core::{
    geometry::{Vec3, traverse_blocks},
    position::BlockPos,
};
use sunrise_gameplay_common::game::spear::OWN;
use sunrise_gameplay_common::game::spear::Target;
use sunrise_sim::spear_sim::hits_in_reach;
use {
    sunrise_items::items::ItemStack, sunrise_level::collisions::SceneCollisions,
    sunrise_sim::first_person_sim::Hand, sunrise_sim::mobs::Damage,
    sunrise_sim::spear_sim::Kinetic, sunrise_sim::spear_sim::REACH,
};

impl crate::game::ServerGame<'_> {
    /// `Level.clip` with COLLIDER shapes and no fluids: where the segment
    /// first meets a block, if it does (tall grass lets a stab through).
    pub(in crate::game::server_side) fn clip_collider(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        let mut scene = SceneCollisions {
            world: &self.common.world,
            binding: self.common.collision,
            subject: None,
        };
        traverse_blocks(from, to, 512, |p: BlockPos| {
            let state = self.common.world.block(p.x, p.y, p.z);
            if self.common.res.facts().is_air(state) {
                return None;
            }
            let shape = self
                .common
                .collision
                .collision_shape(&mut scene, state, [p.x, p.y, p.z])
                .ok()?;
            shape.clip(from, to, p).map(|hit| hit.location)
        })
        .ok()
        .flatten()
    }
    /// `Player.damageStatsAndHearts`: the damage dealt statistic, and the
    /// damage indicator particles over a target that lost more than 2 health
    /// (`feet` and `height`: the target's).
    pub(in crate::game::server_side) fn damage_stats_and_hearts(
        &mut self,
        actual: f32,
        feet: [f64; 3],
        height: f64,
    ) {
        self.award_custom_stat("damage_dealt", (actual * 10.0).round() as i32);
        if actual > 2.0 {
            // The players a server serves are sent them too (the window of the server thread).
            self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
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
            });
        }
    }
    /// `PiercingWeapon.attack` of the spear in the main hand, for a left
    /// click: every mob along the view is stabbed, then `postPiercingAttack`
    /// (Lunge), the hit sound and the swing. Gives the sound of the jab
    /// (`makeSound`), which everyone but the user hears from the server.
    pub(in crate::game) fn jab(&mut self) -> Option<&'static str> {
        let kinetic = self.common.held().and_then(|s| Kinetic::of(&s.item))?;
        let eye = self.common.eye_position()?;
        let eye = Vec3::new(eye[0], eye[1], eye[2]);
        let look = view_vector(self.common.yaw, self.common.pitch);
        let look = Vec3::new(look[0], look[1], look[2]);
        // The attack damage attribute.
        let damage = self.attack_damage();
        let movement = self.known_player_movement();
        let mut last_mob = None;
        let mut on_players = Vec::new();
        for (contact, target) in self
            .stab_targets(eye, look, movement)
            .into_iter()
            .enumerate()
        {
            match target {
                Target::Mob(i) => {
                    if self.stab_mob(i, damage, true, true, false) {
                        last_mob = Some(contact);
                    }
                }
                Target::Player(k) => {
                    on_players.extend(self.queue_player_stab(k, contact, damage, true, true, false))
                }
            }
        }
        let hit = last_mob.is_some();
        // Player.onAttack, then postPiercingAttack.
        self.server.attack_ticks = 0;
        self.post_piercing_attack();
        if hit {
            self.server_play_at_feet(kinetic.hit_sound);
        }
        self.keep_player_stabs(kinetic, false, last_mob, on_players);
        // swingAndResetAttackStrength (ServerPlayer): a swing that starts
        // resets the attack strength.
        let swings = self.common.swings_started;
        self.swing_hand(Hand::Main, kinetic.attack_animation());
        if self.common.swings_started != swings {
            self.reset_attack_strength();
        }
        Some(kinetic.attack_sound)
    }
    pub(in crate::game) fn server_play_at_feet(&mut self, sound: &str) {
        if let Some(p) = &self.common.player {
            let at = [p.position.x, p.position.y, p.position.z];
            self.server_play_sound(sound, at, 1.0, 1.0);
        }
    }
    /// What follows a charge tick that affected an entity:
    /// `Level.broadcastEntityEvent(player, 2)`, felt by the player too, and
    /// SPEAR_MOBS_TRIGGER with the living entities stabbed in this use.
    pub(in crate::game::server_side) fn spear_hit_feedback(&mut self) {
        if self.common.player.is_none() {
            return;
        }
        let actor = self.server.lan_acting;
        if self.server.audio_capture_active {
            self.server.lan_kinetic_hits.push(actor);
        }
        let who = actor.map_or(OWN, |conn| conn as i32);
        self.common.kinetic_hit_feedback(who);
        let stabbed = self
            .server
            .stabbed
            .keys()
            .filter(|(who, _)| *who == actor)
            .count();
        self.trigger_spear_mobs(stabbed);
    }
    /// `KineticWeapon.damageEntities`: one tick of the spear in use, `ticks`
    /// ticks into the use.
    pub(in crate::game::server_side) fn spear_tick(&mut self, stack: &ItemStack, ticks: u32) {
        let Some(kinetic) = Kinetic::of(&stack.item) else {
            return;
        };
        let used = ticks as i32 - kinetic.delay_ticks;
        let (Some(eye), Some(_)) = (
            self.common.eye_position(),
            self.common.player.as_ref().map(|p| p.position),
        ) else {
            return;
        };
        if used < 0 || self.common.spectator() {
            return;
        }
        let eye = Vec3::new(eye[0], eye[1], eye[2]);
        let look = view_vector(self.common.yaw, self.common.pitch);
        let look = Vec3::new(look[0], look[1], look[2]);
        let movement = self.known_player_movement();
        let attacker_speed = look.dot(movement.scale(20.0));
        let now = self.common.world.game_time;
        let actor = self.server.lan_acting;
        let mut affected = false;
        let mut last_mob = None;
        let mut on_players = Vec::new();
        for (contact, target) in self
            .stab_targets(eye, look, movement)
            .into_iter()
            .enumerate()
        {
            let (entity, target_motion) = match target {
                Target::Mob(i) => (self.common.mobs[i].id, self.common.mob_speed(i)),
                Target::Player(k) => {
                    let p = &self.server.stab_players[k];
                    (p.entity_id as u32, p.movement.scale(20.0))
                }
            };
            let key = (actor, entity);
            let recent = self
                .server
                .stabbed
                .get(&key)
                .is_some_and(|&t| now - t < i64::from(kinetic.contact_cooldown_ticks));
            if recent {
                continue;
            }
            self.server.stabbed.insert(key, now);
            let target_speed = look.dot(target_motion);
            let relative = (attacker_speed - target_speed).max(0.0);
            // An entity factor of 1 for a player.
            let dismounts = kinetic.dismount.test(used, attacker_speed, relative, 1.0);
            let pushes = kinetic.knockback.test(used, attacker_speed, relative, 1.0);
            let damages = kinetic.damage.test(used, attacker_speed, relative, 1.0);
            if dismounts || pushes || damages {
                // The attack damage attribute's base value (1) and the speed.
                let amount = 1.0 + (relative * f64::from(kinetic.damage_multiplier)).floor() as f32;
                match target {
                    Target::Mob(i) => {
                        if self.stab_mob(i, amount, damages, pushes, dismounts) {
                            affected = true;
                            last_mob = Some(contact);
                        }
                    }
                    Target::Player(k) => on_players.extend(
                        self.queue_player_stab(k, contact, amount, damages, pushes, dismounts),
                    ),
                }
            }
        }
        if affected {
            self.spear_hit_feedback();
        }
        self.keep_player_stabs(kinetic, true, last_mob, on_players);
    }
    /// `Player.stabAttack` on the mob at `index`: whether it was affected.
    pub(in crate::game::server_side) fn stab_mob(
        &mut self,
        index: usize,
        base_damage: f32,
        deals_damage: bool,
        deals_knockback: bool,
        dismounts: bool,
    ) -> bool {
        let kind = format!(
            "minecraft:{}",
            self.common.mobs[index]
                .kind
                .id
                .trim_start_matches("minecraft:")
        );
        let id = self.common.mobs[index].id;
        let Some(p) = self.common.player.as_ref() else {
            return false;
        };
        let attack = self.attack_of(p.entity_id);
        let from = [p.position.x, p.position.y, p.position.z];
        // The enchantments' share, scaled by the attack strength unless the
        // weapon is in use.
        let magic_boost = self.enchant_damage_bonus(&kind, Some(id));
        let (base_damage, magic_boost) = if self.common.using.is_none() {
            let scale = self.attack_strength_scale();
            (
                base_damage * (0.2 + scale * scale * 0.8),
                magic_boost * scale,
            )
        } else {
            (base_damage, magic_boost)
        };
        let total = if deals_damage {
            base_damage + magic_boost
        } else {
            0.0
        };
        let (old_health, join) = (self.common.mobs[index].health, self.common.mobs[index].join);
        let (hurt, _) = if deals_damage {
            self.hurt_by_player(index, total, Damage::spear(from))
        } else {
            (false, false)
        };
        let Some(index) = self
            .common
            .mob_index
            .find(&self.common.mobs, id)
            .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)
        else {
            if hurt {
                self.remember_player_target(Some(id));
            }
            return hurt;
        };
        // causeExtraKnockback: a push of 0.4, then the weapon's Knockback.
        if deals_knockback {
            let (xd, zd) = self.extra_knockback_direction();
            let source = Damage::spear(from);
            let aim = self.attacker_aim(source.attacker);
            let plan = self.prepare_knockback_credit(id as i32, None, &attack);
            let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
            self.common.mobs[index].attacker_aim = aim;
            let level = crate::game::server_side::entity_server::damage_level_of!(
                self,
                &attack,
                &mut credit,
                None
            );
            self.common.mobs[index].knockback_full(level, 0.4, xd, zd, &source, total, false);
            self.common.mobs[index].attacker_aim = None;
            let (strength, _) = self.extra_knockback(&kind, id, 0.0, Some("minecraft:spear"));
            if strength > 0.0 {
                let plan = self.prepare_knockback_credit(id as i32, None, &attack);
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
                    f64::from(strength),
                    xd,
                    zd,
                    &source,
                    total,
                    true,
                );
                self.common.mobs[index].attacker_aim = None;
            }
            // ...and the attacker stops sprinting.
            self.common.stop_sprint_after_extra_knockback();
        }
        // `Player.stabAttack`: a type in `cannot_be_dismounted_by_item_usage` stays seated.
        let dismounted = dismounts
            && self.common.mobs[index].extra.mount.is_some()
            && !self.items().entity_type_tags.has_bit(
                self.common.mobs[index].kind.flags.entity_tag_slot,
                sunrise_items::items::EntityTypeTags::CANNOT_BE_DISMOUNTED_BY_ITEM_USAGE,
            );
        if dismounted {
            self.dismount_mob(id);
        }
        if !hurt && !deals_knockback && !dismounted {
            return false;
        }
        let mob = &self.common.mobs[index];
        let actual = old_health - mob.health;
        let (feet, width, height) = (
            [mob.position.x, mob.position.y, mob.position.z],
            mob.kind.width,
            mob.kind.height,
        );
        // attackVisualEffects: no attack sound, the enchantments' magic crit.
        if magic_boost > 0.0 {
            self.critical_hit(self.server.lan_acting, id as i32, feet, width, height, true);
        }
        self.remember_player_target(Some(id));
        // itemAttackInteraction: hurtEnemy (statistic, wear), and the
        // post-attack enchantments when the target was hurt.
        if let Some(stack) = self.common.held().cloned() {
            self.award_stat("used", &stack.item, 1);
            if hurt {
                self.post_player_melee(id, &kind, "minecraft:spear", Some(&stack));
            }
            self.weapon_wear(&stack);
        }
        // damageStatsAndHearts.
        self.damage_stats_and_hearts(actual, feet, height);
        if let Some(p) = self.common.player.as_mut()
            && !self.common.creative
        {
            p.cause_food_exhaustion(0.1);
        }
        true
    }
    /// `ProjectileUtil.getHitEntitiesAlong` with the spear's `AttackRange`:
    /// the mobs and the other players along the view, nearest first.
    pub(in crate::game::server_side) fn stab_targets(
        &self,
        eye: Vec3,
        look: Vec3,
        movement: Vec3,
    ) -> Vec<Target> {
        // AttackRange.effectiveMinRange and effectiveMaxRange for a player.
        let (min, max) = if self.common.creative {
            (REACH.min_creative, REACH.max_creative)
        } else {
            (REACH.min, REACH.max)
        };
        let reach = (min, max, REACH.hitbox_margin);
        let clip = |from, to| self.clip_collider(from, to);
        let mobs = self.common.mobs.len();
        let mut boxes = self.common.hittable_mob_boxes();
        boxes.extend(
            (self.server.stab_players.iter().enumerate()).map(|(k, p)| (mobs + k, p.bounds)),
        );
        hits_in_reach(&boxes, eye, look, movement, reach, clip)
            .into_iter()
            .map(|(i, _)| {
                if i < mobs {
                    Target::Mob(i)
                } else {
                    Target::Player(i - mobs)
                }
            })
            .collect()
    }
    /// `Item.use` of a kinetic weapon: `startUsingItem` with a new
    /// `recentKineticEnemies`, and `KineticWeapon.makeSound` at the user.
    pub(in crate::game::server_side) fn use_spear(&mut self, stack: &ItemStack) -> bool {
        let Some(kinetic) = Kinetic::of(&stack.item) else {
            return false;
        };
        if self.start_using_item() {
            let actor = self.server.lan_acting;
            self.server.stabbed.retain(|(who, _), _| *who != actor);
            if let Some(p) = &self.common.player {
                let at = [p.position.x, p.position.y, p.position.z];
                self.server_play_sound(kinetic.sound, at, 1.0, 1.0);
            }
        }
        true
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;
