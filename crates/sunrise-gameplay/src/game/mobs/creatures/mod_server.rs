use crate::game::ServerGame;

pub(in crate::game) struct LandedPlayerArrow {
    victim: u32,
    attack: Attack,
    arrow: Projectile,
}
use super::piston_push::Lid;
#[allow(unused_imports)]
use super::{Dim, InteractTarget, view_vector};
use crate::game::server_side::native_projectiles::{
    Impact, PendingImpact, ProjectileHit, projectile_impact_event,
};
use mod_api::{EquipmentSlot, LootDrops, SoundSource};
use std::{borrow::Cow, sync::Arc};
use sunrise_core::{
    geometry::{Aabb, Direction, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use sunrise_gameplay_common::game::api_values::{TryIntoApi, TryIntoEngine};
use sunrise_gameplay_common::game::combat_common::PlayerCredit;
use sunrise_gameplay_common::game::creatures::DyingDrops;
use sunrise_gameplay_common::game::creatures::MobOutputBatch;
use sunrise_gameplay_common::game::creatures::NativeSound;
use sunrise_gameplay_common::game::creatures::NormalActionFrame;
use sunrise_gameplay_common::game::creatures::NormalMobActions;
use sunrise_gameplay_common::game::creatures::OrbOrder;
use sunrise_gameplay_common::game::creatures::ProjectileHitter;
use sunrise_gameplay_common::game::creatures::SpawnReason;
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_gameplay_common::game::creatures::arrow_miss;
use sunrise_gameplay_common::game::creatures::bounced_place;
use sunrise_gameplay_common::game::creatures::breeding_credit;
use sunrise_gameplay_common::game::creatures::bucketable;
use sunrise_gameplay_common::game::creatures::death_source::death_hurt_source;
use sunrise_gameplay_common::game::creatures::dye_mix;
use sunrise_gameplay_common::game::creatures::from_loot;
use sunrise_gameplay_common::game::creatures::killed_by_mob;
use sunrise_gameplay_common::game::creatures::mob_effect_instance;
use sunrise_gameplay_common::game::creatures::projectile_targets::{
    NonMobHit, ProjectileHitInfo, TargetQuery, is_pickable_projectile,
};
use sunrise_gameplay_common::game::creatures::seen;
use sunrise_gameplay_common::game::creatures::to_loot;
use sunrise_gameplay_common::game::creatures::*;
use sunrise_gameplay_common::game::dragon_common::SPECIAL;
use sunrise_gameplay_common::game::item_abilities::ability;
use sunrise_gameplay_common::game::level_sounds_common::LevelSound;
use sunrise_gameplay_common::projectile_world::ProjWorld;
use sunrise_sim::health::source_damage_type;
use {
    sunrise_game_data::Result, sunrise_items::items::ItemStack,
    sunrise_level::fast_hash::FxHashMap, sunrise_level::particle_options::ParticleType,
    sunrise_sim::entities::arrows::EntityHit, sunrise_sim::health::Attack,
    sunrise_sim::health::HealthEvent, sunrise_sim::health::HurtSource,
    sunrise_sim::join_seq::JoinSeq, sunrise_sim::mobs::Damage, sunrise_sim::mobs::DamageKind,
    sunrise_sim::mobs::Mob, sunrise_sim::mobs::MobEvent, sunrise_sim::mobs::MobWorld,
    sunrise_sim::mobs::PlayerView, sunrise_sim::mobs::Seen, sunrise_sim::mobs::Species,
    sunrise_sim::mobs::Who, sunrise_sim::mobs::ai::eating_sound_is_bound,
    sunrise_sim::mobs::ai::eating_sound_with_variants,
    sunrise_sim::mobs::ai::feed_animal_with_variants, sunrise_sim::mobs::ai::takes_food,
    sunrise_sim::mobs::entity_sounds::full_id, sunrise_sim::mobs::finalize::Group,
    sunrise_sim::mobs::spawning, sunrise_sim::projectiles::Hit,
    sunrise_sim::projectiles::Projectile, sunrise_sim::projectiles::ProjectileKind,
};

#[path = "borrowed_tick_server.rs"]
pub(in crate::game) mod borrowed_tick;

#[path = "breeding_server.rs"]
pub(in crate::game) mod breeding;

#[path = "player_loot_entity_server.rs"]
pub(in crate::game) mod player_loot_entity;

#[path = "projectile_targets_server.rs"]
pub(in crate::game) mod projectile_targets;

#[path = "pufferfish_player_server.rs"]
pub(in crate::game) mod pufferfish_player;

#[path = "world_view_server.rs"]
pub(in crate::game) mod world_view;

#[path = "death_weapon_server.rs"]
pub(in crate::game) mod death_weapon;

#[path = "knockback_server.rs"]
pub(in crate::game) mod knockback;

pub(in crate::game) mod combat_adaptations;
pub(in crate::game) mod conversion_identity;
pub(in crate::game) mod death_tail;
pub(in crate::game) mod destroy_rules;
pub(in crate::game) mod hoglin_target;
#[path = "../../creatures/mob_conversion.rs"]
pub(in crate::game) mod mob_conversion;
pub(in crate::game) mod residency;
pub(in crate::game) mod section_steps;

/// What the server's projectile loop keeps to hand its list back for a projectile told alone,
/// made only in a tick where a mod named a projectile type: each slot's projectile by id, the
/// ids of the list's rows, where those are once a hook changed the list, and the projectile
/// whose own Post is still to tell.
pub(in crate::game) struct ProjectileHandBack {
    pub(in crate::game) slots: Vec<u32>,
    pub(in crate::game) rows: Vec<u32>,
    pub(in crate::game) places: Option<sunrise_level::fast_hash::FxHashMap<u32, usize>>,
    pub(in crate::game) told: Option<(u32, &'static str)>,
}

impl ProjectileHandBack {
    pub(in crate::game) fn new(projectiles: &[Projectile]) -> Box<Self> {
        let slots: Vec<u32> = projectiles.iter().map(|p| p.id).collect();
        Box::new(Self {
            rows: slots.clone(),
            slots,
            places: None,
            told: None,
        })
    }

    /// Where slot `slot`'s projectile is in the list now, if it is still there.
    pub(in crate::game) fn place(&self, slot: usize) -> Option<usize> {
        super::native_ticks::slot_place(&self.places, &self.slots, slot)
    }

    /// After a hook told with the list handed back changed it (it wrote, added or removed a
    /// projectile, or the tick's spawns went first on the hand-back): the loop's state of each
    /// row follows it by id. A new row is kept, and an impact whose projectile a hook took out
    /// is dropped with it.
    pub(in crate::game) fn follow(
        &mut self,
        projectiles: &[Projectile],
        keep: &mut Vec<bool>,
        impacts: &mut [Impact],
    ) {
        let places = super::native_ticks::places_of(projectiles.iter().map(|p| p.id));
        for impact in impacts {
            impact.index = places.get(&impact.before.id).copied();
        }
        let kept: sunrise_level::fast_hash::FxHashMap<u32, bool> = self
            .rows
            .iter()
            .copied()
            .zip(keep.iter().copied())
            .collect();
        self.rows.clear();
        self.rows.extend(projectiles.iter().map(|p| p.id));
        keep.clear();
        keep.extend(
            self.rows
                .iter()
                .map(|id| kept.get(id).copied().unwrap_or(true)),
        );
        self.places = Some(places);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod peaceful_fixture;

#[cfg(test)]
#[test]
#[ignore = "daily"]
fn a_hand_egg_checks_and_spawns_the_type_named_by_its_component() {
    use sunrise_core::nbt::Tag;
    use sunrise_level::chunk::block_entity::BlockEntity;
    use sunrise_sim::mobs::spawner::{SpawnDataExt, SpawnerExt};
    use {
        sunrise_gameplay_common::game::execution_role::ExecutionRole,
        sunrise_gameplay_common::game::server_side::commands::peaceful_tests,
    };
    peaceful_fixture::with_types(ExecutionRole::IntegratedServer, |game| {
        game.set_mode_id(0);
        game.server.forced_hit = Some(sunrise_gameplay_common::game::Hit {
            pos: [4, 64, 4],
            face: [0, 1, 0],
            point: [4.5, 65.0, 4.5],
            inside: false,
        });
        let egg = peaceful_tests::trader_egg();
        game.common.inventory[game.common.selected] = Some(egg.clone());
        assert!(game.use_spawn_egg(&egg).unwrap());
        assert!(game.server.use_failed);
        assert!(game.mobs.is_empty());
        assert_eq!(game.held().unwrap().count, 1);

        game.server.use_failed = false;
        game.set_difficulty_forced(2);
        assert!(game.use_spawn_egg(&egg).unwrap());
        assert!(!game.server.use_failed);
        assert!(game.held().is_none());
        assert_eq!(game.mobs.len(), 1);
        assert_eq!(game.mobs[0].kind.id, peaceful_tests::TRADER);

        let mut egg = egg;
        egg.data_mut().set_other(
            "minecraft:entity_data",
            Some(Tag::compound([
                ("id", Tag::String(peaceful_tests::TRADER.into())),
                ("NoAI", Tag::Byte(1)),
            ])),
        );
        let key = (4, 64, 4);
        for op in [false, true] {
            game.server.lan_acting_permission = Some(if op { 2 } else { 0 });
            game.world
                .block_entities
                .insert(key, BlockEntity::Spawner(Box::default()));
            game.common.inventory[game.common.selected] = Some(egg.clone());
            assert!(game.use_spawn_egg(&egg).unwrap());
            assert!(game.held().is_none());
            let BlockEntity::Spawner(spawner) = &game.world.block_entities[&key] else {
                unreachable!()
            };
            let data = spawner.next_data().unwrap();
            assert_eq!(data.id().as_deref(), Some(peaceful_tests::TRADER));
            assert_eq!(data.entity.get(&"NoAI".into()), op.then_some(&Tag::Byte(1)));
        }
    });
}

impl crate::game::ServerGame<'_> {
    /// Adds a projectile, giving it an id from the entity counter
    /// (`Entity.id`, the same counter as the rest) when it has none. Its id.
    pub fn add_projectile(&mut self, mut p: sunrise_sim::projectiles::Projectile) -> u32 {
        if p.id == 0 {
            self.server.next_entity += 1;
            p.id = self.server.next_entity;
        }
        // The UUID its own random made, unless it already has one (a client
        // copy's from the spawn packet).
        if let Some(uuid) = p.uuid.take() {
            self.common.persist.uuids.get_or_insert_with(p.id, || uuid);
        }
        // Projectile.setOwner: the acting player's or the mob's UUID, which
        // its save keeps even once the owner is gone.
        if p.owner_uuid.is_none() {
            p.owner_uuid = match p.owner {
                Some(Who::Player) => Some(self.acting_player_uuid()),
                Some(Who::Mob(id)) => Some(self.uuid_for(id)),
                None => None,
            };
        }
        let id = p.id;
        if let Some(a) = p.arrow.as_mut()
            && !p.stuck
        {
            a.spawn_pos = Some(p.position);
            a.spawn_velocity = Some(p.velocity);
            a.spawn_rot = Some((p.pitch, p.yaw));
        }
        self.common.creatures.projectiles.push(p);
        id
    }
    pub fn add_fresh_projectile(&mut self, mut projectile: Projectile) -> Option<u32> {
        if projectile.owner == Some(Who::Player) && projectile.guest.is_none() {
            projectile.guest = self.server.lan_acting;
        }
        let id = self.add_projectile(projectile);
        let row = sunrise_gameplay_common::game::entity::EntityHandle::Projectile(
            self.common.creatures.projectiles.len() - 1,
        );
        self.announce_fresh_join(row, id).then_some(id)
    }

    /// Dispatch one arrow entity hit onto LAN guest `victim_conn` without aliasing `Who::Player`.
    pub(in crate::game::server_side) fn arrow_hit_guest_victim(
        &mut self,
        arrow: &mut Projectile,
        victim_conn: u32,
        amount: f32,
    ) -> Result<EntityHit> {
        let refused = arrow.owner == Some(Who::Player)
            && !self.can_arrow_harm_player(
                Who::Player,
                arrow.guest,
                Who::Mob(
                    self.server
                        .lan
                        .as_ref()
                        .and_then(|h| h.hub.guest(victim_conn))
                        .map_or(0, |g| g.entity_id as u32),
                ),
            );
        let shooter = if arrow.owner == Some(Who::Player) {
            self.player_attacker(arrow.guest)
        } else {
            None
        };
        let effect_attacker = shooter
            .as_ref()
            .map_or(self.own_entity_id() as u32, |s| s.0);
        let result = self.guest_hit_by(victim_conn, |g| {
            g.server.combat.0.refuse_player_hit = refused;
            g.server.combat.0.projectile_attacker = shooter;
            let saved_effect_attacker = g
                .server
                .combat
                .0
                .enchantment_projectile_attacker
                .replace(effect_attacker as i32);
            let res = g.arrow_hit_one_entity(arrow, Who::Player, amount, None);
            g.server.combat.0.refuse_player_hit = false;
            g.server.combat.0.projectile_attacker = None;
            g.server.combat.0.enchantment_projectile_attacker = saved_effect_attacker;
            res
        });
        result.unwrap_or(Ok(EntityHit::Refused))
    }
    /// Dispatch one arrow entity hit onto the host player with guest-shooter attribution.
    pub(in crate::game::server_side) fn arrow_hit_host_victim(
        &mut self,
        arrow: &mut Projectile,
        amount: f32,
        deferred: Option<&mut Option<LandedPlayerArrow>>,
    ) -> Result<EntityHit> {
        let shooter = arrow
            .guest
            .filter(|_| arrow.owner == Some(Who::Player))
            .and_then(|c| self.player_attacker(Some(c)));
        let effect_attacker = shooter
            .as_ref()
            .map(|s| s.0)
            .or_else(|| (arrow.owner == Some(Who::Player)).then(|| self.own_entity_id() as u32));
        let prev_attacker = self.server.combat.0.projectile_attacker.take();
        let prev_effect = self.server.combat.0.enchantment_projectile_attacker.take();
        self.server.combat.0.projectile_attacker = shooter;
        self.server.combat.0.enchantment_projectile_attacker = effect_attacker.map(|id| id as i32);
        let outcome = self.arrow_hit_one_entity(arrow, Who::Player, amount, deferred);
        self.server.combat.0.projectile_attacker = prev_attacker;
        self.server.combat.0.enchantment_projectile_attacker = prev_effect;
        outcome
    }
    /// Apply one arrow entity hit (`AbstractArrow.onHitEntity`), returning the hit outcome.
    pub(in crate::game::server_side) fn arrow_hit_one_entity(
        &mut self,
        arrow: &mut Projectile,
        who: Who,
        amount: f32,
        deferred: Option<&mut Option<LandedPlayerArrow>>,
    ) -> Result<EntityHit> {
        self.server.combat.0.projectile_owner = match arrow.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.server.combat.0.projectile_direct = match who {
            Who::Player => Some(self.target_name(super::commands::Target::Projectile(arrow.id))),
            Who::Mob(_) => None,
        };
        if let Who::Mob(id) = who
            && self.is_special_target(id)
        {
            return Ok(self.special_arrow_hit(arrow, id, amount));
        }
        if let Who::Mob(id) = who {
            let was_stand = self.common.armor_stands.iter().any(|a| a.id == id);
            let ignite = arrow
                .arrow
                .as_ref()
                .is_some_and(|a| a.on_fire())
                .then_some(5.0);
            let info = ProjectileHitInfo::from_projectile(
                arrow,
                self.projectile_damage_type(arrow),
                amount,
                ignite,
            );
            match self.projectile_hurts_non_mob(&info, id) {
                NonMobHit::Hurt => {
                    if was_stand {
                        self.shot_post_attack(arrow, who, "minecraft:arrow");
                        self.arrow_post_hurt_effects(arrow, who);
                    }
                    return Ok(EntityHit::Hurt);
                }
                NonMobHit::Refused => return Ok(EntityHit::Refused),
                NonMobHit::NotObject => {}
            }
        }
        let source_kind = self.common.projectile_damage_kind(arrow);
        let attacker_species = match arrow.owner {
            Some(Who::Mob(id)) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species()),
            _ => None,
        };
        let source = Damage {
            kind: source_kind,
            attacker: arrow.owner,
            from: Some([arrow.previous.x, arrow.previous.y, arrow.previous.z]),
            attacker_species,
            direct: Some(arrow.entity_type()),
        };
        let aim = self.attacker_aim(source.attacker);
        let player_fire = self.common.player.as_ref().map(|pl| pl.fire_ticks);
        let mob_fire = match who {
            Who::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| (m.join, m.fire_ticks)),
            Who::Player => None,
        };
        let same_target = |mob: &Mob, id| {
            mob.id == id && !mob.removed && mob_fire.is_some_and(|(join, _)| mob.join == join)
        };
        let mut dodged = false;
        if arrow.arrow.as_ref().is_some_and(|a| a.on_fire()) {
            self.handle_mob_events(vec![MobEvent::Ignite(who, 5)])?;
        }
        let hurt = match who {
            Who::Player => {
                let hurt = self.arrow_hits_player(amount, arrow);
                if hurt && arrow.arrow.as_ref().is_some_and(|a| (a.pierce as i8) <= 0) {
                    self.server.combat.0.stuck.add_arrow();
                }
                hurt
            }
            Who::Mob(id) => {
                let hurt = self.damage_projectile_with_weapon(arrow, id, amount, source, aim);
                // Enderman.projectileReceivesSideEffectsOnHit(false) is always false, even when
                // damage was refused by invulnerability or boat/passenger constraints.
                dodged = !hurt
                    && self
                        .common
                        .mobs
                        .iter()
                        .find(|m| same_target(m, id))
                        .is_some_and(|m| m.species() == Species::Enderman);
                if arrow.owner == Some(Who::Player) {
                    if hurt && let Some(m) = self.common.mobs.iter().find(|m| same_target(m, id)) {
                        let victim = self.mob_entity(m);
                        let direct = sunrise_sim::advancements::EntityView {
                            kind: if arrow.is_spectral_arrow() {
                                "minecraft:spectral_arrow"
                            } else {
                                "minecraft:arrow"
                            }
                            .into(),
                            position: [arrow.position.x, arrow.position.y, arrow.position.z],
                            ..Default::default()
                        };
                        self.player_hit(&victim, "minecraft:arrow", Some(direct), amount);
                    }
                    if hurt && let Some(m) = self.common.mobs.iter().find(|m| same_target(m, id)) {
                        let dead = !m.alive();
                        let victim = self.mob_entity(m);
                        let piercing = arrow.arrow.as_ref().is_some_and(|a| (a.pierce as i8) > 0);
                        if piercing {
                            if dead && let Some(a) = arrow.arrow.as_mut() {
                                a.pierced_killed.push(victim);
                            }
                            let victims: Vec<_> = arrow
                                .arrow
                                .as_ref()
                                .map(|a| a.pierced_killed.clone())
                                .unwrap_or_default();
                            let weapon = arrow
                                .arrow
                                .as_ref()
                                .and_then(|a| a.fired_from_weapon.clone());
                            self.trigger_killed_by_arrow(&victims, weapon.as_ref());
                        } else if dead {
                            let weapon = arrow
                                .arrow
                                .as_ref()
                                .and_then(|a| a.fired_from_weapon.clone());
                            self.trigger_killed_by_arrow(&[victim], weapon.as_ref());
                        }
                    }
                }
                hurt
            }
        };
        let target_present = match who {
            Who::Player => self.common.player.is_some(),
            Who::Mob(id) => self.common.mobs.iter().any(|m| same_target(m, id)),
        };
        if target_present {
            self.burning_arrow_primes(arrow, who);
        }
        let horizontal_len =
            (arrow.velocity.x * arrow.velocity.x + arrow.velocity.z * arrow.velocity.z).sqrt();
        let resistance = match who {
            Who::Player => self
                .common
                .player
                .as_mut()
                .and_then(|pl| pl.attribute("minecraft:knockback_resistance").ok())
                .unwrap_or(0.0),
            Who::Mob(id) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or(0.0, |m| m.knockback_resistance()),
        };
        let knockback = if !hurt || !target_present {
            0.0
        } else if let Some(weapon) = arrow
            .arrow
            .as_ref()
            .and_then(|a| a.fired_from_weapon.as_ref())
        {
            let (victim, kind) = match who {
                Who::Player => (self.own_entity_id() as u32, "minecraft:player".to_owned()),
                Who::Mob(id) => (
                    id,
                    self.common
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map_or_else(String::new, |m| m.kind.id.clone()),
                ),
            };
            let attack = self.projectile_attack(arrow);
            let source = sunrise_sim::health::mob_source(&source, &attack);
            let ctx = attack.enchantment_context(source, &kind);
            self.evaluate_weapon_value(weapon, "minecraft:knockback", Some(victim), ctx, 0.0)
        } else {
            arrow.bonus.punch as f32
        };
        // AbstractArrow.doKnockback: normalize horizontal velocity (`len >= 1e-5f`), scale, check > 0.
        let push = f64::from(knockback) * 0.6 * (1.0 - resistance).max(0.0);
        if hurt && push > 0.0 && horizontal_len >= f64::from(1.0e-5_f32) {
            let mx = (arrow.velocity.x / horizontal_len) * push;
            let mz = (arrow.velocity.z / horizontal_len) * push;
            if mx * mx + mz * mz > 0.0 {
                self.handle_mob_events(vec![MobEvent::Push {
                    target: who,
                    velocity: [mx, 0.1, mz],
                }])?;
            }
        }
        if hurt {
            if target_present {
                if let Some(deferred) = deferred {
                    *deferred = Some(LandedPlayerArrow {
                        victim: self.own_entity_id() as u32,
                        attack: self.current_attack(),
                        arrow: arrow.clone(),
                    });
                } else {
                    self.shot_post_attack(arrow, who, "minecraft:arrow");
                    self.arrow_post_hurt_effects(arrow, who);
                }
                if who == Who::Player {
                    self.arrow_hit_sound_to_shooter(arrow);
                }
            }
            return Ok(EntityHit::Hurt);
        }
        if !dodged {
            match who {
                Who::Player => {
                    if let (Some(pl), Some(ticks)) = (self.common.player.as_mut(), player_fire) {
                        pl.set_remaining_fire_ticks(ticks);
                    }
                }
                Who::Mob(id) => {
                    if let Some((join, ticks)) = mob_fire
                        && let Some(m) = self
                            .common
                            .mobs
                            .iter_mut()
                            .find(|m| m.id == id && m.join == join && !m.removed)
                    {
                        m.fire_ticks = ticks;
                    }
                }
            }
        }
        Ok(arrow_miss(dodged))
    }
    /// Animal.spawnChildFromBreeding
    pub(in crate::game) fn deliver_breed(&mut self, event: MobEvent, at_tick_boundary: bool) {
        let MobEvent::Breed {
            parent,
            partner,
            equine,
            parent_variant,
            wolf_voice,
            experience,
        } = event
        else {
            unreachable!("only the breed event is delivered here");
        };
        let child = self.breed(
            parent,
            partner,
            equine,
            parent_variant,
            wolf_voice,
            experience,
            at_tick_boundary,
        );
        if at_tick_boundary {
            for (id, other) in [(parent, partner), (partner, parent)] {
                if let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    sunrise_sim::mobs::animal_brain::forget_breed_target(mob, other);
                }
            }
        }
        self.native_entity_breed(parent, partner, child);
    }

    pub fn breed(
        &mut self,
        parent: u32,
        partner: u32,
        equine: Option<Box<sunrise_sim::mobs::equine::Offspring>>,
        parent_variant: Option<bool>,
        wolf_voice: Option<
            sunrise_sim::lan::data::RegistryRef<sunrise_sim::mobs::variants::WolfSoundVariant>,
        >,
        experience: sunrise_sim::mobs::BreedExperience,
        at_tick_boundary: bool,
    ) -> Option<u32> {
        let p = self.common.mobs.iter().find(|m| m.id == parent).cloned()?;
        let q = self.common.mobs.iter().find(|m| m.id == partner).cloned();
        if p.species() == Species::Sniffer {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild, false);
            self.lay_sniffer_egg(&p);
            return None;
        }
        if matches!(p.species(), Species::Turtle | Species::Frog) {
            self.finalize_breeding(&p, partner, None, OrbOrder::BeforeChild, false);
            self.award_breeding_experience(parent, [p.position.x, p.position.y, p.position.z]);
            return None;
        }
        let coin = if sunrise_sim::mobs::draws_parent_variant(p.species()) {
            match parent_variant {
                Some(choice) => choice,
                None => {
                    let parent = self.common.mobs.iter_mut().find(|m| m.id == parent)?;
                    parent.random.next_boolean()
                }
            }
        } else {
            false
        };
        let rabbit_variant = self.common.bred_rabbit_variant(parent, &p, q.as_ref());
        let axolotl = if p.species() == Species::Axolotl {
            let other = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            self.common
                .mobs
                .iter_mut()
                .find(|m| m.id == parent)
                .map(|m| {
                    if m.random.next_int_bound(1200).unwrap_or(1) == 0 {
                        let _ = m.random.next_int_bound(1);
                        4
                    } else if m.random.next_boolean() {
                        p.extra.variant
                    } else {
                        other
                    }
                })
        } else {
            None
        };
        let mooshroom = if p.species() == Species::Mooshroom {
            let mate = q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
            let own = p.extra.variant;
            self.common
                .mobs
                .iter_mut()
                .find(|m| m.id == parent)
                .map(|m| {
                    if own == mate && m.random.next_int_bound(1024).unwrap_or(1) == 0 {
                        if own == 1 { 0 } else { 1 }
                    } else if m.random.next_boolean() {
                        own
                    } else {
                        mate
                    }
                })
        } else {
            None
        };
        let kind = match (p.species(), q.as_ref().map(|q| q.species())) {
            (Species::Horse, Some(Species::Donkey)) | (Species::Donkey, Some(Species::Horse)) => {
                "minecraft:mule".to_owned()
            }
            _ => p.kind.id.clone(),
        };
        let mut child = self.create_mob_reason(
            &kind,
            [p.position.x, p.position.y, p.position.z],
            true,
            SpawnSite::Raw,
            Some(0.0),
        );
        if let Some(child) = child.as_mut() {
            'offspring: {
                spawning::bred_offspring(child);
                if p.species() == Species::Fox {
                    child.extra.set_main_hand(None);
                }
                if p.species() == Species::Goat {
                    child.extra.lost_horns = [false; 2];
                    sunrise_sim::mobs::finalize::goat_init_memories(
                        child,
                        &mut self.common.world.random,
                    );
                    let pick_parent = self.world.random.next_boolean();
                    let parent_screams = if pick_parent {
                        p.extra.screaming
                    } else {
                        q.as_ref().unwrap_or(&p).extra.screaming
                    };
                    child.extra.screaming =
                        parent_screams || self.world.random.next_double() < 0.02;
                }
                if let Some(o) = equine {
                    sunrise_sim::mobs::equine::apply_offspring(child, &o);
                    break 'offspring;
                }
                if sunrise_sim::mobs::camel::is_camel(p.species()) {
                    break 'offspring;
                }
                if p.species() == Species::Cat {
                    let other = q.as_ref().unwrap_or(&p);
                    child.extra.cat_variant = if coin {
                        p.extra.cat_variant
                    } else {
                        other.extra.cat_variant
                    };
                    if p.extra.tame {
                        child.extra.tame = true;
                        child.extra.owner = p.extra.owner;
                        let mixed = dye_mix(
                            &self.common.loop_context,
                            server_world_recipes!(self),
                            &mut self.common.world.random,
                            p.extra.color,
                            other.extra.color,
                        );
                        child.extra.color = mixed;
                    }
                    break 'offspring;
                }
                if p.species() == Species::Wolf {
                    let other = q.as_ref().unwrap_or(&p);
                    child.extra.wolf_variant = if coin {
                        p.extra.wolf_variant
                    } else {
                        other.extra.wolf_variant
                    };
                    if p.extra.tame {
                        sunrise_sim::mobs::tame::set_tame(child, true);
                        child.extra.owner = p.extra.owner;
                        let mixed = dye_mix(
                            &self.common.loop_context,
                            server_world_recipes!(self),
                            &mut self.common.world.random,
                            p.extra.color,
                            other.extra.color,
                        );
                        child.extra.color = mixed;
                    }
                    child.extra.wolf_sound_variant =
                        wolf_voice.or_else(|| self.bred_wolf_voice(parent));
                    break 'offspring;
                }
                if p.species() == Species::Panda {
                    sunrise_sim::mobs::panda::inherit(child, &p, q.as_ref().unwrap_or(&p));
                    break 'offspring;
                }
                if let Some(variant) = axolotl {
                    child.extra.variant = variant;
                    child.persistent = true;
                    break 'offspring;
                }
                if p.species() == Species::Nautilus && p.extra.tame {
                    child.extra.tame = true;
                    child.extra.owner = p.extra.owner;
                    child.persistent = true;
                }
                match p.species() {
                    Species::Pig => {
                        let other = q.as_ref().unwrap_or(&p);
                        child.extra.pig_variant = if coin {
                            p.extra.pig_variant
                        } else {
                            other.extra.pig_variant
                        };
                    }
                    Species::Cow => {
                        let other = q.as_ref().unwrap_or(&p);
                        child.extra.cow_variant = if coin {
                            p.extra.cow_variant
                        } else {
                            other.extra.cow_variant
                        };
                    }
                    Species::Chicken => {
                        let other = q.as_ref().unwrap_or(&p);
                        child.extra.chicken_variant = if coin {
                            p.extra.chicken_variant
                        } else {
                            other.extra.chicken_variant
                        };
                    }
                    _ => {
                        child.extra.variant = p.extra.variant;
                        if sunrise_sim::mobs::draws_parent_variant(p.species())
                            && p.species() != Species::Cat
                            && !coin
                        {
                            child.extra.variant =
                                q.as_ref().map_or(p.extra.variant, |q| q.extra.variant);
                        }
                    }
                }
                if let Some(variant) = rabbit_variant {
                    child.extra.variant = variant;
                }
                if let Some(variant) = mooshroom {
                    child.extra.variant = variant;
                }
                if p.species() == Species::Sheep {
                    let other = q.as_ref().unwrap_or(&p);
                    let mixed = dye_mix(
                        &self.common.loop_context,
                        server_world_recipes!(self),
                        &mut self.common.world.random,
                        p.extra.color,
                        other.extra.color,
                    );
                    child.extra.color = mixed;
                }
            }
        }
        let listened = mod_api::__private::listened(mod_api::HookId::BabyEntitySpawn);
        let accepted = if listened {
            let feeders = [parent, partner].map(|id| self.server.love_cause.get(&id).copied());
            let player = feeders
                .into_iter()
                .flatten()
                .find_map(|conn| match conn {
                    Some(conn) if self.server.lan_acting != Some(conn) => {
                        let guest = self.server.lan.as_ref()?.hub.guest(conn)?;
                        guest
                            .placed_in(self.common.dim.id())
                            .then_some((guest.uuid, guest.entity_id))
                    }
                    conn if conn == self.server.lan_acting && self.common.player.is_some() => {
                        Some((self.native_acting_id(), self.own_entity_id()))
                    }
                    _ => None,
                })
                .map_or((None, None), |(uuid, id)| (Some(uuid), Some(id)));
            let constraint = if p.species() == Species::Fox {
                super::native_pending_children::ChildConstraint::BreedingFox
            } else {
                super::native_pending_children::ChildConstraint::BreedingAgeable
            };
            match self.breed_with_listeners(parent, partner, player, child, constraint) {
                super::pending_births::BreedOutcome::Accepted(baby) => baby,
                super::pending_births::BreedOutcome::Cancelled => {
                    breeding_credit(&mut self.server.love_cause, parent, partner);
                    for id in [parent, partner] {
                        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                            sunrise_sim::mobs::finalize::set_age(m, 6000, None);
                            m.extra.in_love = 0;
                        }
                    }
                    return None;
                }
            }
        } else {
            child.map(|child| self.stash_breeding_child(child))
        };
        let (mut pending, action) = accepted?;
        let child = &mut pending.mob;
        let live_parent = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == parent)
            .cloned()
            .unwrap_or(p);
        let fox = live_parent.species() == Species::Fox;
        let order = if sunrise_sim::mobs::orb_after_child(live_parent.species()) {
            OrbOrder::AfterChild
        } else {
            OrbOrder::BeforeChild
        };
        if fox {
            let parent_cause = self
                .server
                .love_cause
                .get(&parent)
                .copied()
                .and_then(|conn| self.killer_uuid(conn));
            let partner_cause = self
                .server
                .love_cause
                .get(&partner)
                .copied()
                .and_then(|conn| self.killer_uuid(conn));
            sunrise_sim::mobs::fox::trust_breeders(
                child,
                parent_cause.map(sunrise_sim::lan::playerdata::uuid_ints),
                partner_cause.map(sunrise_sim::lan::playerdata::uuid_ints),
            );
            self.finalize_breeding(
                &live_parent,
                partner,
                Some(child),
                order,
                listened || at_tick_boundary,
            );
        }
        sunrise_sim::mobs::finalize::set_baby(child, true);
        sunrise_sim::mobs::living::teleports::snap_to(child, live_parent.position);
        child.yaw = 0.0;
        child.previous_yaw = 0.0;
        child.body_yaw = 0.0;
        child.previous_body_yaw = 0.0;
        child.head_yaw = 0.0;
        child.previous_head_yaw = 0.0;
        child.head_pitch = 0.0;
        child.previous_head_pitch = 0.0;
        if !fox {
            self.finalize_breeding(
                &live_parent,
                partner,
                Some(child),
                order,
                listened || at_tick_boundary,
            );
        }
        let at = [
            live_parent.position.x,
            live_parent.position.y,
            live_parent.position.z,
        ];
        if order == OrbOrder::BeforeChild {
            self.breeding_orb(parent, at, experience);
        }
        let child = self.issue_pending_child(pending, action);
        let joined = self.insert_spawned_mob(child);
        if order == OrbOrder::AfterChild {
            self.mob_event_particles(parent, 18);
            self.breeding_orb(parent, at, experience);
        }
        joined
    }
    pub(in crate::game) fn complete_mob_hurt_with_weapon(
        &mut self,
        mut index: usize,
        mut hurt: bool,
        weapon: Option<&ItemStack>,
    ) -> Option<(usize, bool)> {
        use sunrise_sim::{health::HitLend, mobs::MobHurtAction};

        let victim = self.common.mobs.get(index)?;
        let (id, join) = (victim.id, victim.join);
        while self
            .server
            .paused_mob_hurt
            .0
            .as_ref()
            .is_some_and(|hit| hit.victim == id && hit.join == join)
        {
            let mut paused = *self
                .server
                .paused_mob_hurt
                .0
                .take()
                .expect("matching paused hit");
            // Nested hits from this callback are independent of the suspended hit.
            self.common.mobs[index].release_hurt_pause();
            match paused.action {
                MobHurtAction::Wake => self.wake_villager(id),
                MobHurtAction::AwardResistance { player, tenths } => {
                    self.award_custom_stat_for(player, "damage_dealt_resisted", tenths);
                }
            }
            // Owner callbacks can remove the victim or replace its id with another body.
            index = self
                .common
                .mob_index
                .find(&self.common.mobs, id)
                .filter(|&i| self.common.mobs[i].join == join && !self.common.mobs[i].removed)?;
            let attack = std::mem::take(&mut paused.attack);
            let plan = self.prepare_knockback_credit(id as i32, paused.attacker(), &attack);
            let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
            let kind = self.common.mobs[index].kind.id.clone();
            let mut breach = super::mace::breach_modifier(
                weapon,
                &kind,
                paused.source_kind(),
                server_world_items!(self),
                &server_world_stations!(self).enchantments,
            );
            // The paused hit carries its setup's decision: an attacker id only if it was lent then.
            let lent_index = paused
                .lent_attacker
                .and_then(|id| self.common.mob_index.find(&self.common.mobs, id as u32));
            let (mob, lend) = if let Some(other) = lent_index.filter(|&i| i != index) {
                let [mob, attacker] = self
                    .common
                    .mobs
                    .get_disjoint_mut([index, other])
                    .expect("distinct hit and attacker");
                (mob, HitLend::mob(attacker))
            } else {
                (
                    &mut self.common.mobs[index],
                    paused.lent_attacker.and_then(|id| {
                        HitLend::attacking_player(
                            self.common.player.as_mut(),
                            id,
                            &self.common.potions.effects,
                            acting_equipment!(self.common),
                        )
                    }),
                )
            };
            hurt = mob.resume_hurt(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &attack,
                    &mut credit,
                    lend
                ),
                paused,
                weapon.map(|_| &mut breach as &mut _),
            );
        }
        Some((index, hurt))
    }
    /// [`Self::convert_to`] for a conversion its mob made during its own tick, before the rest
    /// of that tick moved it: `at_conversion` is the mob as it was then, and the new mob takes
    /// its place and motion from it (`conversion::motion_at`), the rest from the original as
    /// the events before this one left it.
    pub(in crate::game) fn convert_to_moved(
        &mut self,
        from: &Mob,
        kind: &str,
        params: sunrise_sim::mobs::conversion::Params,
        pre_checked: bool,
        at_conversion: Option<&Mob>,
        after: impl FnOnce(&mut Self, &mut Mob),
    ) -> Option<u32> {
        // Tick conversions can have been removed from the tick list before
        // their queued event is handled. Restore them until Post has returned.
        match self.common.mobs.iter_mut().find(|m| m.id == from.id) {
            Some(old) => old.removed = false,
            None => {
                let mut old = from.clone();
                old.removed = false;
                self.common.mobs.push(old);
            }
        }
        let t = self.common.res.mob_types().get(kind).cloned()?;
        let mut cancel = false;
        let from_kind = (mod_api::__private::listened(mod_api::HookId::LivingConversionPre)
            || mod_api::__private::listened(mod_api::HookId::LivingConversionPost))
        .then(|| sunrise_gameplay_common::game::native_hooks::mob_kind(from));
        if !pre_checked && let Some(from_kind) = &from_kind {
            self.native_living_conversion_pre(from.id, from_kind, kind, &mut cancel);
        }
        if cancel {
            return None;
        }
        let mut from = self.common.mobs.iter().find(|m| m.id == from.id)?.clone();
        if let Some(at) = at_conversion {
            sunrise_sim::mobs::conversion::motion_at(&mut from, at);
        }
        self.server.next_entity += 1;
        let mut mob = Mob::new(self.server.next_entity, t, from.position, from.yaw);
        sunrise_sim::mobs::conversion::single(&from, &mut mob, params);
        self.transfer_conversion_identity(from.id, mob.id);
        if matches!(from.species(), Species::Zombie | Species::Husk) {
            let at = [from.position.x, from.position.y, from.position.z];
            let special = self.common.local_difficulty(at).special();
            sunrise_sim::mobs::finalize::zombie_attributes(
                &mut mob,
                special,
                sunrise_sim::mobs::spawn_rules::Reason::Conversion,
            );
        }
        let mut nauseous = false;
        match (from.species(), kind) {
            (Species::Tadpole, "minecraft:frog") | (Species::Villager, "minecraft:witch") => {
                if !self.finalize_with(&mut mob, SpawnReason::Conversion, None) {
                    self.forget_entity(mob.id);
                    return None;
                }
                mob.persistent = true;
            }
            (Species::Pig, "minecraft:zombified_piglin") => {
                let old = self.common.mobs.iter_mut().find(|m| m.id == from.id)?;
                sunrise_sim::mobs::conversion::pig_thunder_hit(old, &mut mob);
            }
            (Species::Hoglin, "minecraft:zoglin")
            | (Species::Piglin | Species::PiglinBrute, "minecraft:zombified_piglin") => {
                nauseous = true;
            }
            _ => {}
        }
        after(self, &mut mob);
        let id = mob.id;
        self.common.mobs.push(mob);
        // ConversionType.SINGLE: `to.startRiding(vehicle, false, false)`.
        self.seat_mob(id);
        self.copy_conversion_attachments(from.id, id, kind);
        self.admit_new_mobs(self.common.mobs.len() - 1);
        if nauseous {
            // `Hoglin.finishConversion` / `AbstractPiglin.finishConversion` call the hooked
            // `LivingEntity.addEffect` on the converted mob, now that it is resident.
            self.add_mob_effect(
                id,
                sunrise_level::effects::EffectInstance::new("nausea", 200, 0),
                None,
            );
        }
        let transformed = self.common.mobs.iter().find(|m| m.id == id)?;
        let transformed_kind = from_kind.as_ref().map(|_| transformed.kind.id.clone());
        // Only move passengers once the new mob's join has succeeded.
        for rider in self.common.mobs.iter_mut() {
            if rider.extra.mount == Some(from.id) {
                rider.extra.mount = Some(id);
            }
        }
        if let (Some(from_kind), Some(transformed_kind)) = (from_kind, transformed_kind) {
            self.native_living_conversion_post(from.id, &from_kind, id, &transformed_kind);
        }
        if let Some(old) = self.common.mobs.iter_mut().find(|m| m.id == from.id) {
            old.removed = true;
        }
        Some(id)
    }
    pub(in crate::game) fn create_mob_reason(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
        fixed_yaw: Option<f32>,
    ) -> Option<Mob> {
        let kind = if kind.contains(':') {
            kind.to_owned()
        } else {
            format!("minecraft:{kind}")
        };
        let t = self.common.res.mob_types().get(&kind).cloned()?;
        self.server.next_entity += 1;
        // Yaw belongs to the vanilla creation path, independently of whether
        // the fork's SpawnSite allows finalizeSpawn (and its hook).
        let yaw = match (fixed_yaw, site) {
            (Some(yaw), _) => yaw,
            (None, SpawnSite::Finalizing(SpawnReason::Command)) => 0.0,
            (None, SpawnSite::Finalizing(SpawnReason::SpawnItemUse | SpawnReason::Dispenser)) => {
                sunrise_level::camera_math::wrap_degrees(
                    self.common.world.random.next_float() * 360.0,
                )
            }
            _ => self.common.world.random.next_float() * 360.0,
        };
        // Entity.random first supplies the UUID, then LivingEntity's initial
        // yaw; species constructor draws continue from that same stream.
        let (own, uuid) = sunrise_sim::mobs::living_random();
        let mut mob = Mob::with_random(
            self.server.next_entity,
            t,
            Vec3::new(at[0], at[1], at[2]),
            yaw,
            own,
        );
        // `EntityType.create` constructs the mob before `finalizeSpawn`.
        self.common.persist.uuids.insert(mob.id, uuid);
        self.initialize_unpublished_mob(mob.id, &kind);
        self.flush_unjoined_mob_parts(&mut mob);
        mob.persistent = persistent;
        match site {
            SpawnSite::Raw => return Some(mob),
            SpawnSite::Finalizing(reason) => {
                if !self.finalize_with(&mut mob, reason, None) {
                    return self.discard_unjoined_mob(mob.id);
                }
            }
            // A plain spawn finalizes as a summoned one (vanilla's /summon
            // and egg path): its few reason-gated arms behave the same.
            SpawnSite::Plain => {
                let difficulty =
                    self.common
                        .local_difficulty([mob.position.x, mob.position.y, mob.position.z]);
                self.finalize_plain(
                    &mut mob,
                    &difficulty,
                    sunrise_sim::mobs::spawn_rules::Reason::MobSummoned,
                );
            }
        }
        Some(mob)
    }
    /// `LivingEntity.dropFromGiftLootTable`: the gift table from its random
    /// sequence with the mob as `this` (a chicken's egg depends on its
    /// variant); the loot data of the game's resources decides, and without it
    /// the items' table engine (no entity) rolls it. Whether anything dropped.
    pub(in crate::game::server_side) fn drop_gift_loot(
        &mut self,
        mob: u32,
        table: &'static str,
        at: [f64; 3],
    ) -> bool {
        use sunrise_sim::mobs::drops;
        use sunrise_world::loot::EntityLootParams;
        let loot = self.server.server_loot.0.get();
        let this = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == mob)
            .map(|m| drops::loot_entity_with_variants(m, None, Some(self.world_variants())));
        let stacks = match (loot, this) {
            (Some(loot), Some(this)) if loot.has_table(table) => {
                let params = EntityLootParams {
                    this,
                    ..EntityLootParams::default()
                };
                let seed = self.common.level.seed;
                match loot.random_sequence(table) {
                    Some(id) => {
                        let random = self.server.rand_sequences.get(id, seed);
                        loot.roll_entity(table, random, &params)
                            .iter()
                            .filter(|s| !s.is_empty())
                            .map(from_loot)
                            .collect()
                    }
                    None => Vec::new(),
                }
            }
            _ => self.common.res.items().loot_table(
                table,
                false,
                None,
                &mut self.common.world.random,
            ),
        };
        let dropped = !stacks.is_empty();
        for d in stacks {
            // The ItemEntity constructor's motion, from the entity's own
            // random (not the level's).
            self.spawn_item_own(d, at, 10);
        }
        dropped
    }
    /// `MobEffectUtil.addEffectToPlayersAround` for an elder guardian:
    /// mining fatigue III for five minutes unless a stronger or longer one
    /// is on, with the curse sound and the elder's apparition.
    pub(in crate::game::server_side) fn elder_curse(&mut self, at: [f64; 3], silent: bool) {
        let mut cursed = Vec::new();
        self.for_each_player(|g| {
            if g.elder_curse_player(at, silent) {
                cursed.push(g.server.lan_acting);
            }
        });
        // A guest's client shows the apparition and plays the curse itself
        // (ClientboundGameEventPacket.GUARDIAN_ELDER_EFFECT), the sound only
        // for 1: `isSilent() ? 0.0F : 1.0F`.
        if let Some(host) = self.server.lan.as_ref() {
            for conn in cursed.into_iter().flatten() {
                host.hub.send_to(
                    conn,
                    &sunrise_protocol::packets::play::Clientbound::GameEvent(
                        sunrise_protocol::packets::play::GameEvent {
                            event: sunrise_sim::lan::hub::game_event::GUARDIAN_ELDER_EFFECT,
                            param: sunrise_sim::mobs::guardian::elder_curse_param(silent),
                        },
                    ),
                );
            }
        }
    }
    /// The curse on the player in place; whether it took.
    pub(in crate::game::server_side) fn elder_curse_player(
        &mut self,
        at: [f64; 3],
        silent: bool,
    ) -> bool {
        if self.common.creative || self.common.spectator() {
            return false;
        }
        let Some(p) = self.common.player.as_ref() else {
            return false;
        };
        let d = p.position.subtract(Vec3::new(at[0], at[1], at[2]));
        if d.x * d.x + d.y * d.y + d.z * d.z >= 2500.0 || p.health.dead_or_dying() {
            return false;
        }
        // The player's effects are kept by their bare name. A curse as strong
        // that won't end within the next minute stays (endsWithin(1199)).
        let active = self.common.potions.effects.get("mining_fatigue");
        if active.is_some_and(|e| e.amplifier >= 2 && !e.ends_within(1199)) {
            return false;
        }
        self.give_player_effect("minecraft:mining_fatigue", 6000, 2);
        let _ = silent;
        true
    }
    /// Whether the chunk's own status is entity-ticking (block coordinates), whatever the
    /// simulation distance: the chunks whose entities are on `ServerLevel`'s entity tick list
    /// (`Visibility.fromFullChunkStatus`), so they check despawning.
    pub fn entity_tick_listed_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.server.tickets.0.levels();
        move |x, z| levels.loaded_entity_ticking(x >> 4, z >> 4)
    }
    /// `DistanceManager.inEntityTickingRange` in an entity-ticking chunk
    /// (block coordinates), from every player's tickets (`update_chunk_tickets`).
    pub fn entity_ticking_area(&self) -> impl Fn(i32, i32) -> bool + use<> {
        let levels = self.server.tickets.0.levels();
        move |x, z| levels.entity_ticking_at(x, z)
    }
    /// Complete `AbstractArrow.stepMoveAndHit` for any non-trident arrow entity hit batch.
    pub(in crate::game::server_side) fn finish_arrow_step(
        &mut self,
        mut arrow: Projectile,
        first_who: Who,
        first_at: Vec3,
        mut deferred: Option<&mut Option<LandedPlayerArrow>>,
    ) -> Result<()> {
        let step = arrow.arrow.as_mut().and_then(|a| a.pierce_step.take());
        let (step_end, step_block, remaining) = match step {
            Some(s) => (Some(s.end), s.block, s.remaining_hits),
            None => (None, None, Vec::new()),
        };
        let mut batch: Vec<(Who, Vec3)> = std::iter::once((first_who, first_at))
            .chain(remaining)
            .collect();
        let hears = self.native_hears_impacts();
        let mut save_lease = None;
        loop {
            let mut breeze_deflected = false;
            for (hit_who, hit_at) in batch {
                if let Who::Mob(id) = hit_who
                    && self
                        .common
                        .mobs
                        .iter()
                        .any(|m| m.id == id && m.alive() && m.species() == Species::Breeze)
                {
                    if arrow.last_deflected_by != Some(id) {
                        super::wind::reverse(&mut arrow);
                        arrow.last_deflected_by = Some(id);
                        let sound = LevelSound::new(
                            "minecraft:entity.breeze.deflect",
                            [hit_at.x, hit_at.y, hit_at.z],
                            1.0,
                            1.0,
                        )
                        .in_source(SoundSource::Hostile);
                        self.play_bound_sound(sound, id, None);
                    }
                    breeze_deflected = true;
                    break;
                }
                if let Who::Mob(id) = hit_who {
                    if save_lease.is_none() && self.common.is_crystal_id(id) {
                        save_lease = self.common.creatures.projectiles.visibility_lease();
                    }
                    self.redirect_projectile(id, arrow.owner, arrow.owner_uuid, arrow.guest, None);
                }
                // AbstractArrow.onHitEntity: modifyDamage + ceil before pierce cap, crit RNG after.
                let victim_kind = self.entity_kind_for_who(hit_who);
                let base = self.shot_damage_base(&arrow, victim_kind.as_deref(), hit_who);
                let rounded = arrow.arrow_damage_rounded(base);
                if !sunrise_sim::entities::arrows::record_pierce_hit(&mut arrow, hit_who) {
                    self.projectile_landed_event(
                        arrow.id,
                        arrow.owner,
                        [hit_at.x, hit_at.y, hit_at.z],
                    );
                    break;
                }
                let amount = arrow.arrow_apply_crit(rounded) + arrow.bonus.damage;
                // AbstractArrow.onHitEntity: a living owner's setLastHurtMob after the crit roll.
                self.remember_projectile_target(&arrow, hit_who);
                let outcome = if let Who::Mob(id) = hit_who
                    && let Some(victim_conn) = self.lan_guest_conn_by_id(id)
                {
                    self.arrow_hit_guest_victim(&mut arrow, victim_conn, amount)?
                } else if hit_who == Who::Player {
                    self.arrow_hit_host_victim(&mut arrow, amount, deferred.as_deref_mut())?
                } else if let Some(shooter_conn) =
                    arrow.guest.filter(|&c| self.server.lan_acting != Some(c))
                {
                    let res = self.as_guest(shooter_conn, |g| {
                        g.arrow_hit_one_entity(&mut arrow, hit_who, amount, None)
                    });
                    if res.is_some()
                        && self
                            .server
                            .lan
                            .as_ref()
                            .and_then(|host| host.hub.guest(shooter_conn))
                            .is_some_and(|guest| guest.participates_online())
                        && let Who::Mob(id) = hit_who
                        && self.common.mobs.iter().any(|m| m.id == id)
                        && let Some(host) = self.server.lan.as_mut()
                    {
                        host.mob_players.insert(id, shooter_conn);
                    }
                    match res {
                        Some(r) => r?,
                        None => self.arrow_hit_one_entity(&mut arrow, hit_who, amount, None)?,
                    }
                } else {
                    self.arrow_hit_one_entity(&mut arrow, hit_who, amount, None)?
                };
                sunrise_sim::entities::arrows::apply_pierce_hit(&mut arrow, outcome);
                self.flush_arrow_events(&mut arrow);
                if let Who::Mob(id) = hit_who {
                    self.settle_mob_hurt(id);
                }
                self.settle_hurt_teleports()?;
                self.projectile_landed_event(arrow.id, arrow.owner, [hit_at.x, hit_at.y, hit_at.z]);
                if arrow.arrow.as_ref().is_none_or(|a| a.removed) {
                    break;
                }
            }
            if breeze_deflected
                || step_end.is_none()
                || arrow
                    .arrow
                    .as_ref()
                    .is_none_or(|a| a.removed || a.no_physics || (a.pierce as i8) <= 0)
            {
                break;
            }
            let end = step_end.unwrap();
            let mob_shot = !matches!(arrow.owner, Some(Who::Player));
            let pvp = !mob_shot && super::lan_pvp::meets_players(&arrow.kind);
            let shooter = arrow.guest.filter(|_| !mob_shot).and_then(|c| {
                self.server
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guest(c))
                    .map(|g| Who::Mob(g.entity_id as u32))
            });
            if shooter.is_some() {
                arrow.owner = shooter;
            }
            let query = TargetQuery::for_projectile(&arrow, mob_shot || pvp);
            let mut scratch = std::mem::take(&mut self.common.creatures.arrow_hit_scratch);
            let (next_hit, next_rest) = {
                let rain = self.arrow_rain();
                let scene = self.arrow_scene(&rain);
                let live = self.live_proj_world(query, &[]);
                sunrise_sim::entities::arrows::step_pierce_next_live(
                    &mut arrow,
                    &scene,
                    &live,
                    end,
                    step_block,
                    hears,
                    &mut scratch,
                )
            };
            self.common.creatures.arrow_hit_scratch = scratch;
            if shooter.is_some() {
                arrow.owner = Some(Who::Player);
            }
            self.flush_arrow_events(&mut arrow);
            if hears && next_hit != Hit::None {
                let saved = arrow.clone();
                let target = self.native_impact_target(next_hit);
                self.common.creatures.projectiles.push(arrow);
                let canceled = self.native_projectile_impact_canceled(&saved, next_hit);
                let Some(idx) = self.common.creatures.projectiles.locate(saved.join) else {
                    return Ok(());
                };
                arrow = self.common.creatures.projectiles.remove(idx);
                if canceled
                    || self.projectile_removal_pending(arrow.id)
                    || arrow != saved
                    || target.is_none()
                    || target != self.native_impact_target(next_hit)
                {
                    break;
                }
            }
            match next_hit {
                Hit::Entity { who: nw, at: na } => {
                    batch = std::iter::once((nw, na)).chain(next_rest).collect();
                }
                Hit::Block { pos, at: ba, face } => {
                    if hears {
                        {
                            let rain = self.arrow_rain();
                            sunrise_sim::entities::arrows::apply_block_contact(
                                &mut arrow,
                                &self.arrow_scene(&rain),
                                BlockPos::new(pos[0], pos[1], pos[2]),
                            );
                        }
                        self.flush_arrow_events(&mut arrow);
                    }
                    if face != [0; 3] {
                        self.projectile_block_impact(&ProjectileHitter::of(&arrow), pos, ba, face)?;
                        self.projectile_breaks_block(&arrow, pos);
                        self.target_block_hit(&ProjectileHitter::of(&arrow), pos, ba, face);
                    }
                    self.projectile_block_landed_event(arrow.id, arrow.owner, pos);
                    break;
                }
                Hit::None => break,
            }
        }
        let keep = {
            let rain = self.arrow_rain();
            sunrise_sim::entities::arrows::finish_pierce_tick(&mut arrow, &self.arrow_scene(&rain))
        };
        self.flush_arrow_events(&mut arrow);
        if keep {
            self.sweep_projectile_inside(&mut arrow, None);
            let (aid, from, to) = (arrow.id, arrow.previous, arrow.position);
            self.common.reinsert_continuing_projectile(arrow);
            self.entity_section_step(aid, from.components(), to.components());
        } else if save_lease.is_some()
            && self
                .common
                .creatures
                .projectiles
                .locate(arrow.join)
                .is_none()
        {
            self.common
                .creatures
                .projectiles
                .retire_extracted(arrow.id, arrow.join);
        }
        Ok(())
    }
    pub(in crate::game) fn flush_mob_death_protection(&mut self, id: u32) {
        let Some(index) = self.common.mob_index.find(&self.common.mobs, id) else {
            return;
        };
        let mob = &mut self.common.mobs[index];
        let Some(mut hit) = mob.take_protected_hurt() else {
            return;
        };
        let join = mob.join;
        let same_mob = |game: &mut Self| {
            game.common
                .mob_index
                .find(&game.common.mobs, id)
                .filter(|&i| game.common.mobs[i].join == join && !game.common.mobs[i].removed)
        };
        self.apply_mob_death_protection_effects(id, &hit.stack, &hit.effects);
        let Some(i) = same_mob(self) else {
            return;
        };
        let at = self.common.mobs[i].position.components();
        self.lan_entity_event(id as i32, 35, at);
        let variants = world_variants!(self);
        hit.before_post(&mut self.common.mobs[i], Some(variants));
        if let Some(post) = hit.post.take() {
            let kind = self.common.mobs[i].kind.clone();
            self.finish_damage_post(id as i32, &kind.id, post);
        }
        if let Some(i) = same_mob(self) {
            hit.after_post(&mut self.common.mobs[i]);
        }
    }
    /// Settle a hit while its damage owner still has the level in place.
    pub(in crate::game::server_side) fn finish_mob_damage_at(&mut self, index: usize) {
        let Some((index, _)) = self.complete_mob_hurt(index, false) else {
            return;
        };
        self.settle_villager_hurt(index);
        let Some(mob) = self.common.mobs.get_mut(index) else {
            return;
        };
        if mob.has_protected_hurt() {
            let id = mob.id;
            self.flush_mob_death_protection(id);
            return;
        }
        let Some(hit) = mob.damage_completion.take() else {
            return;
        };
        let id = mob.id;
        // Keep the hit apart from the body: the death hooks may remove or revive it.
        let kind = hit.post.as_ref().map(|_| mob.kind.clone());
        if self.server.died.insert(id) {
            let death = mob.clone();
            self.mob_die(&death);
        }
        if let Some((post, kind)) = hit.post.zip(kind) {
            self.finish_damage_post(id as i32, &kind.id, post);
        }
    }
    pub(in crate::game) fn handle_mob_events_pending(
        &mut self,
        actions: &mut NormalMobActions,
    ) -> Result<()> {
        loop {
            let frame = actions.nested.last_mut().unwrap_or(&mut actions.root);
            let Some(e) = frame.events.pop_front() else {
                if actions.nested.pop().is_some() {
                    continue;
                }
                return Ok(());
            };
            let actor = frame.actor;
            let broke = &mut frame.broke;
            match e {
                MobEvent::ForPlayer { conn, event } => {
                    let mut event = *event;
                    // Projectiles are spawned in the world rather than run
                    // as a guest, but must retain the selected guest target.
                    if let Some(conn) = conn
                        && let MobEvent::Projectile(p) = &mut event
                        && let ProjectileKind::ShulkerBullet { target, .. } = &mut p.kind
                        && *target == Some(Who::Player)
                    {
                        *target = self.guest_identity(conn).map(|(_, id)| Who::Mob(id));
                        self.handle_mob_events(vec![event])?;
                    } else if let Some(conn) = conn {
                        if let Some(mob) = actor {
                            self.lan_guest_mob_events(&mut vec![(conn, mob, event)]);
                        } else {
                            // No nested block-break actor is in scope. A
                            // missing guest must not redirect to another player.
                            if let Some(result) =
                                self.as_guest(conn, |game| game.handle_mob_events(vec![event]))
                            {
                                result?;
                            }
                        }
                    } else {
                        self.handle_mob_events(vec![event])?;
                    }
                }
                MobEvent::BubbleColumnParticles { pos } => {
                    self.bubble_column_particles(BlockPos::new(pos[0], pos[1], pos[2]))
                }
                MobEvent::LandingDust {
                    state,
                    at,
                    count,
                    speed,
                } => self.server_landing_dust(state, Vec3::new(at[0], at[1], at[2]), count, speed),
                MobEvent::SquidCycle { mob, at } => self.lan_entity_event(mob as i32, 19, at),
                MobEvent::RabbitHop { mob, at } => self.lan_entity_event(mob as i32, 1, at),
                MobEvent::Drown { mob, at, bubbles } => self.mob_drown(mob, at, bubbles),
                MobEvent::PowderSnowParticle { at, velocity } => {
                    self.snowflake_particle(at, velocity);
                }
                MobEvent::SunProtectionWear { mob } => self.wear_sun_protection(mob),
                MobEvent::ArmorHurt {
                    mob,
                    amount,
                    source,
                } => self.hurt_mob_armor(mob, amount, &source),
                MobEvent::GliderWear { mob, slot } => self.wear_mob_glider(mob, slot),
                MobEvent::EntityEvent { mob, event, at } => {
                    // Ravager.roar (event 69 ends it): with the raw mob_griefing rule (not
                    // canEntityGrief), ROAR_TARGET_WITH_GRIEFING also takes the armor stands.
                    if event == 69
                        && self
                            .common
                            .ui
                            .rules
                            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
                        && let Some(area) = self
                            .common
                            .mobs
                            .iter()
                            .find(|m| m.id == mob && m.species() == Species::Ravager)
                            .map(|m| m.bounds().inflate(Vec3::new(4.0, 4.0, 4.0)))
                    {
                        self.roar_pushes_armor_stands(area, at);
                    }
                    self.lan_entity_event(mob as i32, event, at)
                }
                MobEvent::LevelParticles {
                    kind,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    if let Some(ty) = sunrise_level::particle_options::ParticleType::from_name(kind)
                    {
                        self.server_level_particles(
                            &sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                            at,
                            spread,
                            speed,
                            count,
                        );
                    }
                }
                MobEvent::ItemParticles {
                    item,
                    at,
                    spread,
                    speed,
                    count,
                } => {
                    self.server_level_particles(
                        &sunrise_sim::lan::server_effects::ServerParticle::item(
                            sunrise_items::items::ItemStack::new(item, 1),
                        ),
                        at,
                        spread,
                        speed,
                        count,
                    );
                }
                MobEvent::Sound(name, at, volume, pitch) => {
                    self.server_play_sound(&full_id(name), at, volume, pitch)
                }
                MobEvent::ProfessionWorkSound(name, at, volume, pitch) => {
                    self.server_play_sound(&name, at, volume, pitch)
                }
                MobEvent::LocalSound(..) => {}
                MobEvent::BoundSound {
                    mob,
                    sound,
                    source,
                    at,
                    volume,
                    pitch,
                } => {
                    let name = full_id(sound);
                    let sound = LevelSound::new(&name, at, volume, pitch).in_source(source);
                    self.play_bound_sound(sound, mob, None);
                }
                MobEvent::LevelEventSound {
                    sound,
                    at,
                    volume,
                    spread,
                } => {
                    // WitherBoss.customServerAiStep: the spawn roar is global
                    // level event 1023 at the block position.
                    if sound == "entity.wither.spawn" {
                        let pos = at.map(|v| v.floor() as i32);
                        self.global_level_event(1023, pos, 0);
                    } else {
                        // The client's own level random: nothing drawn from the server's.
                        let mut r = sunrise_core::random::LegacyRandom::new(
                            self.common.world.game_time.wrapping_mul(341_873_128_712),
                        );
                        let pitch = (r.next_float() - r.next_float()) * spread + 1.0;
                        self.server_play_sound(&full_id(sound), at, volume, pitch)
                    }
                }
                MobEvent::SoundId(name, at, volume, pitch) => {
                    self.server_play_sound(&name, at, volume, pitch)
                }
                MobEvent::ConsumeSound(sound, at) => {
                    self.queue_consume_sound(&sound, at, SoundSource::Neutral);
                }
                MobEvent::GameEvent {
                    event, at, source, ..
                } if event == "minecraft:flap"
                    && self.common.mobs.iter().any(|m| {
                        m.id == source && matches!(m.species(), Species::Chicken | Species::Parrot)
                    }) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                // A mob's own `gameEvent(..)` calls: Armadillo.rollUp/rollOut's ENTITY_ACTION,
                // PRIME_FUSE, EAT, a fox's DRINK, the shulker's lid, an evoker's vexes
                // (ENTITY_PLACE), TELEPORT, EQUIP/UNEQUIP, updateFallFlying's ELYTRA_GLIDE.
                MobEvent::GameEvent {
                    event, at, source, ..
                } if matches!(
                    event,
                    "minecraft:entity_action"
                        | "minecraft:prime_fuse"
                        | "minecraft:eat"
                        | "minecraft:drink"
                        | "minecraft:container_open"
                        | "minecraft:container_close"
                        | "minecraft:entity_place"
                        | "minecraft:teleport"
                        | "minecraft:equip"
                        | "minecraft:unequip"
                        | "minecraft:elytra_glide"
                ) =>
                {
                    let ctx = self.who_context(Some(Who::Mob(source)), None);
                    self.post_game_event(event, at, ctx);
                }
                MobEvent::GameEvent {
                    event: event @ ("minecraft:block_place" | "minecraft:block_destroy"),
                    at,
                    source,
                    state,
                } => {
                    let ctx = self.who_context(Some(Who::Mob(source)), Some(state));
                    self.post_game_event(event, at, ctx);
                }
                // `FarmlandBlock.turnToBaseBlock` and `TurtleEggBlock.decreaseEggs`:
                // the landing's own block change posts its game event, from the
                // mob that landed (`Context.of(entity, state)`) unless the block
                // destroyed itself under it (`Context.of(state)`).
                MobEvent::LandingGameEvent {
                    event,
                    at,
                    source,
                    state,
                } => {
                    let who = source.map(Who::Mob);
                    self.post_block_game_event(event, at, who, Some(state));
                }
                // The sculk layer tracks steps and bee/bat flaps itself
                // (`entity_motion_events`, `tick_mob_sculk_events`). Birds
                // above use their movement's flyDist/nextFlap threshold.
                MobEvent::GameEvent { .. } => {}
                MobEvent::WeaponAttack(hit) => {
                    self.server.combat.0.hit_landed = self.mob_weapon_attack(hit);
                }
                MobEvent::AttackPlayer(amount, from, attacker) => {
                    self.server.combat.0.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                    if self.server.combat.0.hit_landed
                        && let Some(attacker) = attacker
                    {
                        self.remember_mob_target(
                            attacker,
                            self.common.player.as_ref().map(|p| p.entity_id as u32),
                        );
                    }
                }
                // Bee.doHurtTarget: a sting that hurt leaves the stinger in the
                // body, then the poison; the bee's own stinger, calm-down and
                // sound follow only then too.
                MobEvent::Sting {
                    target,
                    amount,
                    poison_ticks,
                    source,
                } => self.sting(target, amount, poison_ticks, source),
                MobEvent::AttackPlayerWithEffect {
                    attacker,
                    amount,
                    from,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    self.server.combat.0.hit_landed =
                        self.mob_hits_player(amount, from, "minecraft:mob_attack");
                    if self.server.combat.0.hit_landed {
                        self.remember_mob_target(
                            attacker,
                            self.common.player.as_ref().map(|p| p.entity_id as u32),
                        );
                        self.give_player_effect(effect, ticks, amplifier);
                    }
                }
                MobEvent::DamagePlayer {
                    amount,
                    from,
                    kind,
                    wielder: _,
                } => {
                    let hurt = self.mob_hits_player(amount, from, kind);
                    if kind == "minecraft:spear" {
                        self.server.combat.0.hit_landed = hurt;
                    }
                }
                MobEvent::ElderCurse { at, silent } => self.elder_curse(at, silent),
                MobEvent::SetVelocity { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.common.player.as_mut() {
                                p.velocity = v;
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = v;
                            }
                        }
                    }
                }
                MobEvent::RemoveKilled { target } => {
                    // The bite that killed it ran `LivingEntity.die` (loot, such
                    // as a magma cube's froglight, and experience) before
                    // `eatEntity` removes it: a removed mob never reaches the
                    // next tick's deaths, so it dies here first.
                    if let Some(m) = self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == target && !m.alive() && !m.removed)
                        .cloned()
                    {
                        if self.server.died.insert(target) {
                            self.mob_die(&m);
                        }
                        self.remove_mob_now(target, false);
                    }
                }
                MobEvent::Push { target, velocity } => {
                    let v = Vec3::new(velocity[0], velocity[1], velocity[2]);
                    match target {
                        Who::Player => {
                            if let Some(p) = self.common.player.as_mut() {
                                p.velocity = p.velocity.add_vector(v);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                                m.velocity = m.velocity.add_vector(v);
                            }
                        }
                    }
                }
                MobEvent::PushJoined { id, join, velocity } => {
                    if let Some(mob) = self.mob_object_mut(id).filter(|mob| mob.join == join) {
                        mob.velocity = mob.velocity.add_vector(Vec3::new(
                            velocity[0],
                            velocity[1],
                            velocity[2],
                        ));
                        mob.needs_sync = true;
                    }
                }
                MobEvent::Knockback {
                    target,
                    power,
                    x,
                    z,
                    source,
                    apply_blocking,
                } => {
                    self.mob_event_knockback(target, power, [x, z], source, apply_blocking);
                }
                MobEvent::LidPush {
                    shulker,
                    area,
                    delta,
                } => self.server_shulker_lid_push(shulker, area, delta),
                MobEvent::Wololo { mob } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        // DyeColor.RED.
                        m.extra.color = 14;
                    }
                }
                MobEvent::Fangs {
                    at,
                    yaw,
                    delay,
                    owner,
                } => self.add_fangs(at, yaw, delay, owner),
                MobEvent::Shoot(..) => {}
                MobEvent::ExplosionSource {
                    mob,
                    charged,
                    name,
                    in_water,
                    kind,
                } => {
                    self.server.combat.0.explosion_source = Some(mob);
                    self.server.combat.0.explosion_in_water = in_water;
                    // The creeper causes the blast's damage, but it is out of
                    // the mob list already: its name comes with the event.
                    self.server.combat.0.exploded.push((mob, name));
                    self.server.combat.0.exploded_kinds.push((mob, kind));
                    if charged {
                        self.server.combat.0.charged_exploders.push(mob);
                    }
                }
                MobEvent::LingeringCloud {
                    at,
                    effects,
                    hidden,
                    ambient,
                    color,
                } => {
                    self.spawn_creeper_cloud(at, effects, hidden, ambient, color);
                }
                MobEvent::Explode(at, radius, species, silent) => {
                    // The exploding mob causes the damage (Creeper.explodeCreeper).
                    let noted = self.server.combat.0.explosion_source.is_some();
                    let source_mob = self.server.combat.0.explosion_source.or_else(|| {
                        self.common
                            .mobs
                            .iter()
                            .filter(|m| {
                                (m.position.x - at[0]).powi(2) + (m.position.z - at[2]).powi(2)
                                    < 1.0
                            })
                            .map(|m| m.id)
                            .next()
                    });
                    self.server.combat.0.explosion_source = source_mob;
                    self.server.combat.0.explosion_source_named = noted;
                    let portal_proof = species == Species::SulfurCube
                        && source_mob.is_some_and(|id| {
                            self.server
                                .portals
                                .entities
                                .get(&id)
                                .is_some_and(|p| p.cooldown > 0)
                        });
                    self.server.combat.0.explosion_cause = match species {
                        Species::Creeper => Some("minecraft:creeper"),
                        Species::Wither => Some("minecraft:wither"),
                        Species::SulfurCube => Some("minecraft:sulfur_cube"),
                        _ => None,
                    }
                    .map(|kind| sunrise_sim::advancements::EntityView {
                        kind: kind.into(),
                        position: at,
                        ..Default::default()
                    });
                    // Only a creeper or a wither empties the hives it blows up.
                    // The creeper is already gone from the list, so the event
                    // carries its kind. A sulfur cube's blast is TNT's.
                    let mut source =
                        super::explosion::ExplosionSource::default().with_silence(silent);
                    if let Some(id) = source_mob {
                        source = source.with_damage_direct(
                            id,
                            None,
                            sunrise_sim::health::DirectEntity::Living,
                        );
                    }
                    if let Some(id) = source_mob
                        && matches!(species, Species::Creeper | Species::Wither)
                    {
                        let kind = match species {
                            Species::Creeper => "minecraft:creeper",
                            _ => "minecraft:wither",
                        };
                        source =
                            source.with_entity(id, kind, sunrise_sim::health::DirectEntity::Living);
                    }
                    let result = match species {
                        Species::Creeper | Species::Wither => self.with_hive_emptying_blast(|g| {
                            g.explode(source_mob, at, radius, source)
                        }),
                        Species::SulfurCube => self.explode_sulfur_cube(
                            source_mob,
                            at,
                            radius,
                            source.with_portal_proof(portal_proof),
                        ),
                        _ => self.explode(source_mob, at, radius, source),
                    };
                    self.server.combat.0.explosion_source = None;
                    self.server.combat.0.explosion_source_named = false;
                    self.server.combat.0.explosion_in_water = false;
                    self.server.combat.0.explosion_cause = None;
                    result?
                }
                MobEvent::AttackMob {
                    target,
                    amount,
                    source,
                    remember_target,
                } => {
                    self.server.combat.0.hit_landed = self.mob_attacks_mob(target, amount, source);
                    if remember_target
                        && self.server.combat.0.hit_landed
                        && let Some(Who::Mob(attacker)) = source.attacker
                    {
                        self.remember_mob_target(attacker, Some(target));
                    }
                }
                MobEvent::DamageMob {
                    target,
                    amount,
                    source,
                } => {
                    self.mob_hits_mob(target, amount, source);
                }
                MobEvent::ContactDamageMob {
                    target,
                    amount,
                    kind,
                    attacker,
                } => {
                    let mut source = Damage::of(Self::damage_kind(
                        &self.common.health_resources.damage,
                        kind,
                    ));
                    if let Some((cube, from)) = attacker {
                        source.attacker = Some(Who::Mob(cube));
                        source.from = Some(from);
                    }
                    self.mob_hits_mob(target, amount, source);
                }
                MobEvent::AttackMobWithEffect {
                    target,
                    amount,
                    source,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    // CaveSpider, Husk and WitherSkeleton.doHurtTarget: the
                    // effect sits inside the branch where the hit landed, and
                    // `LivingEntity.addEffect` takes a mob as well as a player.
                    let attacker = source.attacker;
                    self.server.combat.0.hit_landed = self.mob_attacks_mob(target, amount, source);
                    if self.server.combat.0.hit_landed {
                        if let Some(Who::Mob(attacker)) = attacker {
                            self.remember_mob_target(attacker, Some(target));
                        }
                        if let Some(def) = sunrise_level::brewing::effect(effect) {
                            let effect_source = match attacker {
                                Some(Who::Mob(id)) => self.entity(id),
                                Some(Who::Player) => self
                                    .common
                                    .player
                                    .as_ref()
                                    .and_then(|p| self.entity(p.entity_id as u32)),
                                None => None,
                            };
                            self.add_mob_effect(
                                target,
                                sunrise_level::effects::EffectInstance::from_def(
                                    def, ticks, amplifier,
                                ),
                                effect_source,
                            );
                        }
                    }
                }
                MobEvent::CubeHurtPlayer { cube, amount, from } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.hurt_player_from(
                        amount,
                        Some(from),
                        "minecraft:mob_attack",
                        KnownCause::Entity(cube),
                        false,
                    ) && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                    }
                }
                MobEvent::CubeHurtMob {
                    cube,
                    target,
                    amount,
                    source,
                } => {
                    // AbstractCubeMob.dealDamage: the attack sound (and its
                    // two pitch draws, made even for a silent cube, whose
                    // Entity.playSound is then skipped) only follows a hit
                    // that landed.
                    if self.mob_hits_mob(target, amount, source)
                        && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == cube)
                    {
                        let pitch = (m.random.next_float() - m.random.next_float()) * 0.2 + 1.0;
                        let at = m.center();
                        if !m.silent {
                            self.handle_mob_events(vec![MobEvent::Sound(
                                "entity.slime.attack",
                                at,
                                1.0,
                                pitch,
                            )])?;
                        }
                        self.mob_post_attack(
                            target,
                            source.attacker,
                            None,
                            Some(sunrise_sim::health::damage_type(source.kind, false)),
                        );
                    }
                }
                MobEvent::SonicBoom {
                    target,
                    attacker,
                    from,
                    direction,
                } => {
                    // SonicBoom: the push only follows a hit that landed, scaled
                    // by the target's KNOCKBACK_RESISTANCE.
                    let (hurt, resistance) = match target {
                        Who::Player => {
                            let hurt = self.mob_hits_player(10.0, from, "minecraft:sonic_boom");
                            let resistance = self.common.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            });
                            (hurt, resistance)
                        }
                        Who::Mob(id) => {
                            let source = Damage {
                                kind: DamageKind::SonicBoom,
                                attacker: Some(Who::Mob(attacker)),
                                from: Some(from),
                                attacker_species: Some(Species::Warden),
                                direct: None,
                            };
                            let hurt = self.mob_hits_mob(id, 10.0, source);
                            let resistance = self
                                .common
                                .mobs
                                .iter()
                                .find(|m| m.id == id)
                                .map_or(0.0, Mob::knockback_resistance);
                            (hurt, resistance)
                        }
                    };
                    if hurt {
                        let horizontal = 2.5 * (1.0 - resistance);
                        let vertical = 0.5 * (1.0 - resistance);
                        let velocity = [
                            direction[0] * horizontal,
                            direction[1] * vertical,
                            direction[2] * horizontal,
                        ];
                        self.handle_mob_events(vec![MobEvent::Push { target, velocity }])?;
                    }
                }
                MobEvent::FinishAdmiring { mob } => self.finish_admiring(mob),
                MobEvent::Hiss { mob } => {
                    if let Some(m) = self.common.mobs.iter().find(|m| m.id == mob) {
                        let at = m.center();
                        let sound = sunrise_sim::mobs::ai::cat_sound(
                            m,
                            Some(self.world_variants()),
                            "hiss",
                        )
                        .map(|s| full_id(s).into_owned());
                        if let Some(sound) = sound {
                            self.server_play_sound(&sound, at, 1.0, 1.0);
                        }
                    }
                }
                MobEvent::StabMemory {
                    attacker,
                    target,
                    contact,
                    attempted_damage,
                    otherwise_affected,
                } => {
                    if (otherwise_affected || (attempted_damage && self.server.combat.0.hit_landed))
                        && self.common.latest_stab_contact(attacker, contact)
                    {
                        let target = match target {
                            Who::Player => self.common.player.as_ref().map(|p| p.entity_id as u32),
                            Who::Mob(id) => Some(id),
                        };
                        self.remember_mob_target(attacker, target);
                    }
                }
                MobEvent::Tamed { .. }
                | MobEvent::Buck { .. }
                | MobEvent::StabDismount { .. }
                | MobEvent::EjectPassengers { .. } => self.mount_event(&e),
                MobEvent::IronGolemHit {
                    target,
                    amount,
                    source,
                } => {
                    // IronGolem.doHurtTarget: the toss only follows a hit that
                    // landed, scaled by the target's KNOCKBACK_RESISTANCE.
                    let hurt = match target {
                        Who::Player => {
                            let from = source.from.unwrap_or_default();
                            self.mob_hits_player(amount, from, "minecraft:mob_attack")
                        }
                        Who::Mob(id) => self.mob_hits_mob(id, amount, source),
                    };
                    if self.server.fake_action.0.acting() {
                        self.server.combat.0.hit_landed = hurt;
                    }
                    if hurt {
                        let resistance = match target {
                            Who::Player => self.common.player.as_mut().map_or(0.0, |p| {
                                p.attribute("minecraft:knockback_resistance").unwrap_or(0.0)
                            }),
                            Who::Mob(id) => self
                                .common
                                .mobs
                                .iter()
                                .find(|m| m.id == id)
                                .map_or(0.0, Mob::knockback_resistance),
                        };
                        let up = f64::from(0.4f32) * (1.0 - resistance).max(0.0);
                        match target {
                            Who::Player => {
                                if let Some(p) = self.common.player.as_mut() {
                                    p.velocity.y += up;
                                }
                            }
                            Who::Mob(id) => {
                                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                                    m.velocity.y += up;
                                }
                            }
                        }
                        if let Who::Mob(id) = target {
                            self.mob_post_attack(
                                id,
                                source.attacker,
                                None,
                                Some("minecraft:mob_attack"),
                            );
                        }
                    }
                }
                MobEvent::Bonemeal { mob, pos } => {
                    self.villager_bonemeal(mob, BlockPos::new(pos[0], pos[1], pos[2]));
                }
                MobEvent::RingBell { bell } => {
                    let pos = BlockPos::new(bell[0], bell[1], bell[2]);
                    let rang = self
                        .with_level(|b, level, ticks| b.bell_attempt_to_ring(level, ticks, pos));
                    if rang {
                        let _ = self.server_process_level_events();
                    }
                }
                MobEvent::ChestOpen { chest, open, mob } => self.golem_chest_open(chest, open, mob),
                MobEvent::ChestSlot { chest, slot, stack } => {
                    self.golem_chest_slot(chest, slot, stack)
                }
                MobEvent::BecomeStatue { mob } => self.golem_become_statue(mob),
                MobEvent::EnterHive { mob, hive } => self.bee_enter_hive(mob, hive),
                MobEvent::ParrotToShoulder { mob } => self.parrot_to_shoulder(mob),
                MobEvent::Infested { at, velocities } => self.spawn_infested(at, velocities),
                MobEvent::Gift { target, item } => self.receive_gift(target, item),
                MobEvent::HurtSelf { mob, kind, amount } => {
                    let plan = self.prepare_knockback_credit(
                        mob as i32,
                        None,
                        self.server.combat.0.attack(),
                    );
                    let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                    if let Some(index) = self.common.mob_index.find(&self.common.mobs, mob) {
                        self.common.mobs[index].damage(
                            crate::game::server_side::entity_server::damage_level_of!(
                                self,
                                self.server.combat.0.attack(),
                                &mut credit,
                                None
                            ),
                            amount,
                            sunrise_sim::mobs::Damage::of(kind),
                        );
                        self.finish_mob_damage_at(index);
                    }
                }
                MobEvent::SetTarget { mob, target } => {
                    let player = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.server.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                }
                MobEvent::PiglinAlertTarget(alert) => self.piglin_alert_target(alert),
                MobEvent::HoglinAttackTarget {
                    mob,
                    target,
                    player,
                } => self.hoglin_attack_target(mob, target, player),
                MobEvent::HuntHoglin { mob, target, ticks } => {
                    let mut acquire = false;
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.ints[0] = ticks;
                        acquire = m.ai.target.is_none();
                    }
                    if acquire {
                        let player = self
                            .server
                            .lan
                            .as_ref()
                            .and_then(|host| host.mob_players.get(&mob).copied())
                            .or(self.server.lan_acting);
                        self.set_event_ai_target(mob, Some(Who::Mob(target)), player);
                    }
                }
                MobEvent::Projectile(p) => self.launch_mob_projectile(p),
                MobEvent::BowShot {
                    mob,
                    weapon,
                    mut arrow,
                } => {
                    self.mob_bow_ammo(mob, &weapon, &mut arrow);
                    if let Some(state) = arrow.arrow.as_mut() {
                        state.fired_from_weapon = Some(weapon);
                        arrow.bonus.damage = 0.0;
                    }
                    self.launch_mob_projectile(arrow);
                }
                MobEvent::CrossbowShot { ammo, mut arrow } => {
                    if &*ammo.item == "minecraft:firework_rocket" {
                        // `CrossbowItem.createProjectile`: a rocket, from 0.15 under the eyes.
                        let mut at = arrow.position;
                        at.y -= f64::from(0.15f32) - 0.1;
                        self.launch_angled_rocket(at, &ammo, arrow.owner, |_| arrow.velocity);
                    } else {
                        Self::arrow_from_ammo(&ammo, &mut arrow);
                        self.launch_mob_projectile(arrow);
                    }
                }
                MobEvent::CrossbowLoad {
                    mob,
                    weapon,
                    at,
                    pitch,
                } => {
                    if self.mob_crossbow_ammo(mob, &weapon) {
                        self.server_play_sound(
                            "minecraft:item.crossbow.loading_end",
                            at,
                            1.0,
                            pitch,
                        );
                    }
                }
                MobEvent::Spawn(mut m) => {
                    self.server.next_entity += 1;
                    m.id = self.server.next_entity;
                    self.common.mobs.push(*m);
                    self.admit_new_mobs(self.common.mobs.len() - 1);
                }
                MobEvent::SpawnKind { kind, at, setup } => {
                    if let Some(id) = self.spawn_mob(
                        &kind,
                        at,
                        setup.persistent,
                        if setup.skip_finalize {
                            SpawnSite::Raw
                        } else {
                            SpawnSite::Finalizing(
                                sunrise_sim::mobs::spawn_rules::Reason::MobSummoned,
                            )
                        },
                    ) {
                        self.common.apply_setup(id, &setup);
                    }
                }
                MobEvent::Convert {
                    from,
                    kind,
                    params,
                    after,
                    before_travel,
                } => self.convert_event(&from, kind, params, after, before_travel),
                MobEvent::ThrowItem {
                    mob,
                    stack,
                    at,
                    motion,
                    pickup_delay,
                } => {
                    // `thrownItem.setThrower(the mob)`.
                    self.spawn_item(stack, at, motion, pickup_delay);
                    let thrower = self.mob_uuid(mob);
                    if let Some(thrown) = self.just_spawned_item() {
                        thrown.thrower = Some(thrower);
                    }
                }
                MobEvent::DropStack { mob, stack, at } => {
                    self.native_entity_drop_item(mob, &stack.item, stack.count as i32, at);
                    self.spawn_item_own(stack, at, 10);
                }
                MobEvent::DropInventory { mob, at } => {
                    let stacks = self
                        .common
                        .mobs
                        .iter_mut()
                        .find(|m| m.id == mob)
                        .map(|m| std::mem::take(&mut m.extra.inventory))
                        .unwrap_or_default();
                    for stack in stacks.into_iter().filter(|stack| stack.count > 0) {
                        self.native_entity_drop_item(mob, &stack.item, stack.count as i32, at);
                        self.spawn_item_own(stack, at, 10);
                    }
                }
                MobEvent::DropItem {
                    mob,
                    item,
                    count,
                    at,
                } => {
                    self.native_entity_drop_item(mob, &item, count as i32, at);
                    // The ItemEntity constructor's motion, from the entity's
                    // own random (not the level's), so spawnAtLocation drops
                    // (a fox's berries, a panda's held food, a goat's horn)
                    // land apart.
                    self.spawn_item_own(
                        ItemStack {
                            item,
                            count,
                            damage: 0,
                            extra: None,
                        },
                        at,
                        10,
                    );
                }
                MobEvent::DropGoatHorn {
                    mob,
                    screaming,
                    at,
                    motion,
                } => {
                    let instrument =
                        sunrise_sim::mobs::goat::create_horn(self.mob_uuid(mob), screaming);
                    let mut horn = ItemStack::new("minecraft:goat_horn", 1);
                    horn.data_mut().set_other(
                        "minecraft:instrument",
                        Some(sunrise_core::nbt::Tag::String(instrument.into())),
                    );
                    // Goat.dropHorn's ItemEntity constructor leaves pickupDelay at
                    // 0 (no setDefaultPickUpDelay).
                    self.spawn_item(horn, at, motion, 0);
                }
                MobEvent::DropGift { table, at } => {
                    let stacks = self.common.res.items().loot_table(
                        table,
                        false,
                        None,
                        &mut self.common.world.random,
                    );
                    for stack in stacks {
                        // The ItemEntity constructor's motion, from the
                        // entity's own random (not the level's).
                        self.spawn_item_own(stack, at, 10);
                    }
                }
                // `Level.setBlockAndUpdate`: neighbour and shape updates, so a crop
                // above trampled farmland breaks and drops.
                MobEvent::SetBlock { pos, state } => self.level_set(pos, state)?,
                MobEvent::EndermanPlaceBlock {
                    mob,
                    pos,
                    carried,
                    occupants,
                } => {
                    self.enderman_place_block(mob, pos, carried, &occupants)?;
                }
                MobEvent::PlaceBlock { mob, pos, state } => {
                    self.mob_place_block(mob, pos, state)?
                }
                MobEvent::CropRaided {
                    pos,
                    state,
                    old,
                    mob,
                } => {
                    self.level_set_with_flags(
                        pos,
                        state,
                        sunrise_world::level::flags::UPDATE_CLIENTS,
                    )?;
                    if let Some(old) = old {
                        self.post_block_game_event(
                            "minecraft:block_change",
                            pos,
                            Some(Who::Mob(mob)),
                            None,
                        );
                        self.server_level_event_at(2001, pos, old.0 as i32);
                    }
                }
                MobEvent::CrackTurtleEgg { pos } => self.crack_turtle_egg(pos)?,
                MobEvent::BedOccupied { pos, occupied } => self.set_bed_occupied(pos, occupied),
                MobEvent::SetDoorOpen { pos, open } => {
                    let pos = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_door_open(level, ticks, pos, open));
                    self.server_process_level_events()?;
                }
                MobEvent::TakeBlock { mob, pos, state } => {
                    // The block must still be the one the enderman picked.
                    if self.common.world.block(pos[0], pos[1], pos[2]) == state
                        && self.mob_may_destroy(mob, pos)
                    {
                        let air = self.common.world.air;
                        self.level_set(pos, air)?;
                    } else if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.carried = None;
                    }
                }
                MobEvent::IfBroke {
                    mob,
                    effects,
                    otherwise,
                } => {
                    let follow = if broke.contains(&mob) {
                        effects
                    } else {
                        otherwise
                    };
                    actions.nested.push(NormalActionFrame {
                        events: follow.into(),
                        broke: Vec::new(),
                        actor: Some(mob),
                    });
                }
                MobEvent::Jump { mob } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.jumping = true;
                    }
                }
                MobEvent::BreakDoor { mob, pos } => {
                    if !self.mob_may_destroy(mob, pos) {
                        continue;
                    }
                    // BreakDoorGoal.tick: removeBlock, then level events 1021
                    // and 2001 with the state that is there now.
                    let kind = self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(sunrise_gameplay_common::game::native_hooks::mob_kind)
                        .unwrap_or_default();
                    let air = self.common.world.air;
                    let at = BlockPos::new(pos[0], pos[1], pos[2]);
                    self.with_level(|b, level, ticks| b.set_block(level, ticks, at, air, 3));
                    self.server_process_level_events()?;
                    self.server_level_event_at(1021, pos, 0);
                    let left = self.common.world.block(pos[0], pos[1], pos[2]);
                    self.server_level_event_at(2001, pos, left.0 as i32);
                    self.native_entity_break_door(mob, &kind, pos);
                }
                MobEvent::DestroyProgress { mob, pos, progress } => {
                    self.server_destroy_block_progress(mob as i32, pos, progress);
                }
                MobEvent::LevelEvent { id, pos, data } => self.server_level_event_at(id, pos, data),
                MobEvent::SetBlockName {
                    mob,
                    pos,
                    name,
                    effect,
                    flags,
                } => {
                    if effect {
                        // EatBlockGoal passes the block's default state constant
                        // (Blocks.GRASS_BLOCK.defaultBlockState()), not the live state
                        // (which could be snowy).
                        let old = self.common.world.block(pos[0], pos[1], pos[2]);
                        let state = self
                            .common
                            .pack
                            .assets
                            .registry
                            .default_state(&self.common.res.facts().get(old).name)
                            .unwrap_or(old);
                        self.server_level_event_at(2001, pos, state.0 as i32);
                    }
                    if let Some(state) = self.common.pack.assets.registry.default_state(name) {
                        let old_name = self
                            .common
                            .res
                            .facts()
                            .get(self.common.world.block(pos[0], pos[1], pos[2]))
                            .name
                            .clone();
                        let mut cancel = false;
                        if let Some(mob) = mob
                            && let Some(kind) = self
                                .common
                                .mobs
                                .iter()
                                .find(|m| m.id == mob)
                                .map(|m| m.kind.id.clone())
                        {
                            let from = self.common.world.block(pos[0], pos[1], pos[2]);
                            self.native_entity_change_block_pre(
                                crate::game::native_hooks::fire::EntityChangeBlockPreCall {
                                    player: None,
                                    entity: Some(mob),
                                    kind: &kind,
                                    pos,
                                    from,
                                    to: state,
                                    cancel: &mut cancel,
                                },
                            );
                        }
                        if !cancel {
                            self.level_set_with_flags(pos, state, flags)?;
                            if let Some(mob) = mob
                                && let Some(m) = self.common.mobs.iter().find(|m| m.id == mob)
                            {
                                let kind = m.kind.id.clone();
                                self.native_entity_change_block(mob, &kind, pos, &old_name, name);
                            }
                        }
                    }
                }
                MobEvent::Infest {
                    pos,
                    infested,
                    poof,
                } => {
                    let registry = &self.common.pack.assets.registry;
                    let old = self.common.world.block(pos[0], pos[1], pos[2]);
                    if let Some(mut state) = registry.state(old).cloned() {
                        let (from, to) = if infested {
                            ("minecraft:", "minecraft:infested_")
                        } else {
                            ("minecraft:infested_", "minecraft:")
                        };
                        state.name = state.name.replacen(from, to, 1);
                        if let Ok(id) = registry.resolve(&state) {
                            self.level_set(pos, id)?;
                        }
                    }
                    if let Some(mob) = poof {
                        self.mob_poof(mob);
                    }
                }
                MobEvent::BreakBlock {
                    pos,
                    drop,
                    by,
                    breaker,
                } => {
                    if let Some(mob) = by {
                        if !self.mob_may_destroy(mob, pos) {
                            continue;
                        }
                        broke.push(mob);
                    }
                    let old = self.common.world.block(pos[0], pos[1], pos[2]);
                    // Block.spawnDestroyParticles: fire breaks leave nothing.
                    if !self.common.res.facts().get(old).has_class("BaseFireBlock") {
                        self.server_level_event_at(2001, pos, old.0 as i32);
                    }
                    if drop {
                        let entity = by.or(breaker);
                        let key = (pos[0], pos[1], pos[2]);
                        // Level.destroyBlock binds its original subject before the roll.
                        // A writable loot listener can replace it before BlockDrops runs.
                        let data = if self.native_authoritative()
                            && mod_api::__private::listened(mod_api::HookId::BlockDrops)
                        {
                            match self.common.world.block_entities.get(&key) {
                                Some(row)
                                    if mod_api::__private::listened(mod_api::HookId::LootDrops) =>
                                {
                                    self.saved_block_entity_data(
                                        key,
                                        row,
                                        &self.common.res.facts().get(old).name,
                                    )
                                }
                                Some(_) => None,
                                None => sunrise_gameplay_common::game::spill_common::saved_entity(&self.common.world, pos)
                                    .and_then(
                                        sunrise_gameplay_common::game::spill_common::packed_block_entity_payload,
                                    ),
                            }
                        } else {
                            None
                        };
                        let block_entity =
                            sunrise_gameplay_common::game::spill_common::BlockDropEntity::at(
                                &self.common.world,
                                key,
                                data.as_ref(),
                            );
                        let mut drops = self.roll_block_loot(pos, old, entity, Default::default());
                        self.native_block_loot(pos, old, None, None, None, &mut drops);
                        let dropped =
                            self.entity_block_dropped(pos, old, entity, block_entity, drops);
                        let canceled = dropped.canceled;
                        self.spawn_block_drops(pos, dropped);
                        // InfestedBlock.spawnAfterBreak runs after the drops,
                        // unless BlockDropsEvent canceled it.
                        if !canceled
                            && self
                                .common
                                .res
                                .facts()
                                .get(old)
                                .name
                                .starts_with("minecraft:infested_")
                            && self
                                .common
                                .ui
                                .rules
                                .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
                        {
                            let at = [
                                f64::from(pos[0]) + 0.5,
                                f64::from(pos[1]),
                                f64::from(pos[2]) + 0.5,
                            ];
                            if let Some(id) =
                                self.spawn_mob("minecraft:silverfish", at, false, SpawnSite::Plain)
                            {
                                self.mob_poof(id);
                            }
                        }
                    }
                    // Level.destroyBlock: setBlock(fluidState.createLegacyBlock(), 3),
                    // so water stays water and a waterlogged block leaves its water.
                    let b = self.common.res.behaviours();
                    let left = b.legacy_block(b.fluid(old));
                    if left != old {
                        self.level_set(pos, left)?;
                    }
                }
                MobEvent::Particles { kind, at, count } => {
                    // Mob particle bursts (hearts, smoke, splashes) around a point.
                    if let Some(ty) = sunrise_level::particle_options::ParticleType::from_name(kind)
                    {
                        if ty == sunrise_level::particle_options::ParticleType::SonicBoom {
                            self.lan_sonic_boom(
                                &sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                                at,
                            );
                        }
                        // Count 0: one particle exactly there, at rest.
                        if count == 0
                            && ty != sunrise_level::particle_options::ParticleType::SonicBoom
                        {
                            self.server_particle_at(
                                sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                                at,
                                [0.0; 3],
                            );
                        }
                        for _ in 0..count {
                            let r = &mut self.common.world.random;
                            let p = [
                                at[0] + (f64::from(r.next_float()) - 0.5),
                                at[1] + (f64::from(r.next_float()) - 0.5) * 0.5,
                                at[2] + (f64::from(r.next_float()) - 0.5),
                            ];
                            let v = [
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                                r.next_gaussian() * 0.02,
                            ];
                            self.server_particle_at(
                                sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                                p,
                                v,
                            );
                        }
                    }
                }
                MobEvent::AroundSelf {
                    mob, event, feet, ..
                } => {
                    self.lan_entity_event(mob as i32, event, feet);
                }
                MobEvent::ParticleAt { kind, at, velocity } => {
                    if let Some(ty) = sunrise_level::particle_options::ParticleType::from_name(kind)
                    {
                        self.server_particle_at(
                            sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                            at,
                            velocity,
                        );
                    }
                }
                MobEvent::ParticleColored {
                    at,
                    color: [r, g, b],
                } => {
                    self.server_particle_at(
                        sunrise_sim::lan::server_effects::ServerParticle::colored(
                            sunrise_level::particle_options::ParticleType::EntityEffect,
                            [r, g, b, 1.0],
                        ),
                        at,
                        [0.0; 3],
                    );
                }
                MobEvent::Experience { at, amount } => self.add_orb(at, amount),
                MobEvent::AxolotlSupportPlayer { mob } => {
                    let previous = self
                        .common
                        .potions
                        .effects
                        .get("regeneration")
                        .map_or(0, |e| e.duration);
                    if (0..=2399).contains(&previous) {
                        self.common.potions.effects.add(
                            sunrise_level::effects::EffectInstance::new(
                                "regeneration",
                                (previous + 100).min(2400),
                                0,
                            ),
                        );
                    }
                    self.common
                        .potions
                        .effects
                        .list
                        .retain(|e| e.id != "mining_fatigue");
                    let source = self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == mob)
                        .map(|m| self.mob_entity(m));
                    self.trigger_effects_changed_by(source);
                }
                MobEvent::DolphinTreasure { mob } => {
                    // ServerLevel.findNearestMapStructure(#dolphin_located, 50).
                    let Some(at) = self.common.mobs.iter().find(|m| m.id == mob).map(|m| {
                        [m.position.x, m.position.y, m.position.z].map(|v| v.floor() as i32)
                    }) else {
                        continue;
                    };
                    let names = self.structure_tag("minecraft:dolphin_located");
                    let found = self
                        .common
                        .end
                        .locator()
                        .filter(|_| !names.is_empty())
                        .and_then(|t| t.locate(&names, at, 50));
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        m.extra.home = found.as_ref().map(|(_, p)| *p);
                    }
                    // DolphinSwimToTreasureGoal.start: entity event 38 once it has a treasure.
                    if found.is_some() {
                        self.mob_event_particles(mob, 38);
                    }
                }
                MobEvent::EffectPlayer {
                    effect,
                    ticks,
                    amplifier,
                    ambient,
                } => {
                    self.give_mob_effect(effect, ticks, amplifier, ambient);
                }
                MobEvent::EffectMob {
                    target,
                    effect,
                    ticks,
                    amplifier,
                } => {
                    if let Some(def) = sunrise_level::brewing::effect(effect) {
                        self.add_mob_effect(
                            target,
                            sunrise_level::effects::EffectInstance::from_def(def, ticks, amplifier),
                            None,
                        );
                    }
                }
                MobEvent::Ignite(target, seconds) => self.common.ignite_entity(target, seconds)?,
                MobEvent::ZombieIgnite(target, seconds) if self.server.combat.0.hit_landed => {
                    self.common.ignite_entity(target, seconds)?
                }
                MobEvent::ZombieIgnite(..) => {}
                event @ MobEvent::Breed { .. } => self.deliver_breed(event, false),
                MobEvent::LockGaze { mob, partner } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_sim::mobs::animal_brain::lock_gaze(m, partner);
                    }
                }
                MobEvent::BreedTarget { mob, partner } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_sim::mobs::animal_brain::set_breed_partner(m, partner);
                    }
                }
                MobEvent::Retreat { mob, from, at } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_sim::mobs::piglin::receive_retreat(m, from, at);
                    }
                }
                MobEvent::StartRiding { rider, vehicle } => {
                    if sunrise_sim::mobs::riders::start_riding(
                        &mut self.common.mobs,
                        rider,
                        vehicle,
                    ) {
                        self.seat_mob(rider);
                        self.break_leash(rider, true, false);
                        // sendEventAndTriggers: the server players already on it.
                        self.note_started_riding(vehicle);
                    }
                }
                // The borrowed tick has already applied these fish events,
                // including settling each sting before the next event.
                MobEvent::StartFollowing { .. }
                | MobEvent::SchoolSize { .. }
                | MobEvent::PufferfishSting { .. } => {}
                MobEvent::LeashBreak { mob, drop, sound } => {
                    self.tick_break_leash(mob, drop, sound)
                }
                MobEvent::NotifyLeashHolder { holder } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == holder) {
                        sunrise_sim::mobs::happy_ghast::notify_leash_holder(m);
                    }
                }
                MobEvent::Vibration {
                    from,
                    to,
                    ticks,
                    source,
                } => self.mob_vibration_particle(source, from, to, ticks),
                MobEvent::DigParticles { .. } => {}
                MobEvent::BlockBreakEffect { pos, block } => {
                    // Turtle.aiStep broadcasts LevelEvent 2001, not the
                    // particles themselves: the event carries the particles and
                    // the sand's break sound, and reaches the clients the same
                    // way a broken block's does.
                    let state = self.common.world.block(block[0], block[1], block[2]);
                    self.server_level_event_at(2001, pos, state.0 as i32);
                }
                MobEvent::Darkness { at, radius } => self.apply_darkness(at, radius),
                MobEvent::CreakingTearDown { center, spread } => {
                    self.creaking_tear_down(center, spread)
                }
                MobEvent::CureZombieVillager { mob } => self.cure_zombie_villager(mob),
                MobEvent::Reinforcements { mob, target } => self.call_reinforcements(mob, target),
                MobEvent::VillagerMeet { mob, other } => self.villager_meet(mob, other),
                MobEvent::VillagerTrade { mob, player } => self.villager_traded_by(mob, player),
                MobEvent::VillagerOffers { mob } => {
                    if let Some(i) = self.common.mobs.iter().position(|m| m.id == mob) {
                        self.ensure_offers(i);
                    }
                }
                MobEvent::VillagerBirth {
                    parent,
                    partner,
                    child,
                } => self.villager_birth(parent, partner, child),
                MobEvent::WorkAtComposter { mob, pos } => self.villager_work_at_composter(mob, pos),
                // The borrowed tick routes these directly to the open trading screen.
                MobEvent::VillagerRestockStep(_) => {}
                MobEvent::GolemCheck { mob, needed } => self.villager_golem_check(mob, needed),
                MobEvent::Firework { at, color, flight } => {
                    self.villager_firework(at, color, flight)
                }
                MobEvent::ThrowGift { table, from, to } => self.throw_gift(&table, from, to),
                MobEvent::PatrolTarget { mob, target } => {
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        sunrise_sim::mobs::raider::set_patrol_target(m, target);
                    }
                }
                MobEvent::RaiderTarget {
                    mob,
                    target,
                    aggressive,
                } => {
                    let player = self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|host| host.mob_players.get(&mob).copied())
                        .or(self.server.lan_acting);
                    self.set_event_ai_target(mob, Some(target), player);
                    if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == mob) {
                        m.ai.aggressive |= aggressive;
                    }
                }
                MobEvent::PandaSneezed { mob } => self.panda_sneezed(mob),
                MobEvent::FallToPassengers {
                    mob,
                    distance,
                    modifier,
                } => self.fall_to_passengers(mob, distance, modifier),
                MobEvent::ShedScute { mob, at, pitch } => {
                    // The mob already checked `shouldDropLoot` (a baby, `mob_drops`).
                    let table = "minecraft:gameplay/armadillo_shed";
                    let drops = self.sequence_loot(table, table, None);
                    for d in drops {
                        self.spawn_at_location(Vec3::new(at[0], at[1], at[2]), 0.0, d);
                    }
                    self.server_play_sound("minecraft:entity.armadillo.scute_drop", at, 1.0, pitch);
                    let ctx = self.who_context(Some(Who::Mob(mob)), None);
                    self.post_game_event("minecraft:entity_place", at, ctx);
                }
                MobEvent::LayEgg { mob, at, pitch } => {
                    // Chicken.aiStep: `dropFromGiftLootTable` says whether
                    // anything dropped; only then the sound and the game event.
                    if self.drop_gift_loot(mob, "minecraft:gameplay/chicken_lay", at) {
                        self.server_play_sound("minecraft:entity.chicken.egg", at, 1.0, pitch);
                        let ctx = self.who_context(Some(Who::Mob(mob)), None);
                        self.post_game_event("minecraft:entity_place", at, ctx);
                    }
                }
                MobEvent::GiftLoot { mob, table, at } => {
                    // Turtle.ageBoundaryReached and Panda.afterSneeze: only
                    // with mob loot on.
                    if matches!(
                        table,
                        "minecraft:gameplay/turtle_grow" | "minecraft:gameplay/panda_sneeze"
                    ) && !self
                        .common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::MOB_DROPS)
                    {
                        continue;
                    }
                    self.drop_gift_loot(mob, table, at);
                }
            }
        }
    }
    pub fn hurt_player_by(&mut self, damage_type: &str, amount: f32) {
        let mut canceled = false;
        let attack = self.current_attack();
        if self.common.player.is_some()
            && let Some(id) = self.common.health_resources.damage.id(damage_type)
            && let Ok(o) =
                self.hurt_player_equipment(id, amount, None, false, 0, false, &attack, None)
        {
            canceled = o.canceled;
        }
        self.server.combat.0.forget_noted(canceled);
        self.finish_guest_damage();
    }
    /// `hurtServer` on the player. `from` is the source's position (none: no
    /// direction), `known`: the causing mob when it is given (`/damage`),
    /// else the melee attacker is the mob nearest to `from`.
    pub fn hurt_player_from(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        damage_type: &str,
        known: KnownCause,
        by_creative_player: bool,
    ) -> bool {
        self.hurt_player_from_inner(amount, from, damage_type, known, by_creative_player, true)
    }

    fn hurt_player_from_inner(
        &mut self,
        amount: f32,
        from: Option<[f64; 3]>,
        damage_type: &str,
        known: KnownCause,
        by_creative_player: bool,
        post_attack: bool,
    ) -> bool {
        let rocket = std::mem::take(&mut self.server.combat.0.rocket_hit);
        let pierce = std::mem::take(&mut self.server.combat.0.piercing_hit);
        let melee = self.server.combat.0.melee_mob.take();
        let projectile = rocket
            || world_stations!(self)
                .enchantments
                .damage_tags
                .get("minecraft:is_projectile")
                .is_some_and(|t| t.contains(damage_type));
        let blocking = self.player_blocking(from, damage_type, pierce);
        let raised = blocking.is_some();
        let owner = self.server.combat.0.projectile_owner.take();
        let current = self.server.combat.0.attack();
        let player_melee = known == KnownCause::Infer
            && melee.is_none()
            && current.projectile.is_none()
            && current.direct.is_none()
            && current
                .enchantment_attacker
                .as_ref()
                .is_some_and(|actor| actor.kind.as_ref() == "minecraft:player");
        let attacker = match known {
            KnownCause::Entity(id) => Some(id),
            KnownCause::Attack | KnownCause::None => None,
            KnownCause::Infer if melee.is_some() => melee,
            KnownCause::Infer if player_melee => None,
            KnownCause::Infer => from
                .filter(|_| !projectile && damage_type != "minecraft:player_attack")
                .and_then(|from| {
                    self.common
                        .mobs
                        .iter()
                        .filter(|m| m.alive())
                        .min_by(|a, b| {
                            let d = |m: &sunrise_sim::mobs::Mob| {
                                (m.position.x - from[0]).powi(2) + (m.position.z - from[2]).powi(2)
                            };
                            d(a).total_cmp(&d(b))
                        })
                        .map(|m| m.id)
                }),
        };
        let mut attack = match attacker.or(owner) {
            Some(id) => self.attack_of(id as i32),
            None => self.current_attack(),
        };
        attack.projectile = current.projectile;
        attack.projectile_kind = current.projectile_kind;
        attack.projectile_type = current.projectile_type;
        attack.enchantment_direct = current.enchantment_direct.clone();
        attack.direct = if player_melee {
            Some(attack.attacker)
        } else {
            current.direct
        };
        attack.direct_kind = current.direct_kind;
        attack.direct_type.clone_from(&current.direct_type);
        attack.position = from;
        attack.projectile_weapon = current.projectile_weapon.clone();
        if let Some(a) = attacker.or(owner) {
            self.note_attacker(damage_type, a);
        }
        if let Some((id, name)) = self.server.combat.0.projectile_attacker.take() {
            self.note_named_attacker(damage_type, id, name);
        }
        if let Some(name) = self.server.combat.0.projectile_direct.take() {
            self.note_direct_entity(damage_type, name);
        }
        let living_cause = attacker.or(owner).is_some();
        let owner = owner.filter(|_| projectile);
        let causing = match known {
            KnownCause::Infer => attacker.or(owner).map_or(attack.attacker, |id| id as i32),
            KnownCause::Attack => attack.attacker,
            KnownCause::Entity(id) => id as i32,
            KnownCause::None => 0,
        };
        let mut outcome = sunrise_sim::health::BlockOutcome::default();
        let mut default_push: Option<([f64; 2], HurtSource<'_>)> = None;
        let events_before = self.common.player.as_ref().map_or(0, |p| p.events.len());
        let damage_outcome = if let Some(id) = self.common.health_resources.damage.id(damage_type) {
            let weapon =
                self.player_armor_weapon(&attack, attack.player_source(damage_type, causing));
            self.hurt_player_equipment(
                id,
                amount,
                blocking,
                living_cause,
                causing,
                by_creative_player,
                &attack,
                weapon,
            )
        } else {
            Ok(Default::default())
        };
        if let Ok(o) = damage_outcome
            && let Some(player) = self.common.player.as_mut()
            && {
                outcome = o;
                o.hurt
            }
            && o.knockback
        {
            let (xd, zd) = from.map_or((0.0, 0.0), |from| {
                (from[0] - player.position.x, from[2] - player.position.z)
            });
            if o.blocked <= 0.0 {
                player.health.indicate_damage(xd, zd, player.yaw);
            }
            let source = HurtSource {
                damage_type,
                causing,
                direct: attack.projectile_direct(damage_type),
                position: None,
                direct_type: None,
            };
            default_push = Some(([xd, zd], source));
        }
        if let Some(p) = self.common.player.as_ref()
            && p.events.get(events_before..).is_some_and(|new| {
                new.iter().any(|e| {
                    matches!(
                        e,
                        sunrise_sim::motion::MotionEvent::Health(HealthEvent::Phase(
                            "thorns-sound"
                        ))
                    )
                })
            })
        {
            let at = [p.position.x, p.position.y, p.position.z];
            self.server_play_sound("minecraft:enchant.thorns.hit", at, 1.0, 1.0);
        }
        self.server.combat.0.forget_noted(outcome.canceled);
        if mod_api::__private::listened(mod_api::HookId::LivingKnockback)
            && self.common.player.as_ref().is_some_and(|player| {
                player.events[events_before..].iter().any(|event| {
                    matches!(
                        event,
                        sunrise_sim::motion::MotionEvent::Health(HealthEvent::Criterion { .. })
                    )
                })
            })
        {
            self.commit_pending_hurt_memories(damage_type, Some(&attack));
        }
        if outcome.hurt || outcome.sound {
            let source = attacker
                .or(owner)
                .and_then(|id| self.common.mob_index.get(&self.common.mobs, id))
                .map(|m| self.mob_entity(m));
            self.trigger_entity_hurt_player(&sunrise_sim::advancements::DamageView {
                kind: damage_type.to_owned(),
                direct: if rocket {
                    Some(sunrise_sim::advancements::EntityView {
                        kind: "minecraft:firework_rocket".to_owned(),
                        position: from.unwrap_or_default(),
                        ..Default::default()
                    })
                } else {
                    source.clone().filter(|_| !projectile)
                },
                is_direct: !projectile,
                source,
                original: amount,
                actual: outcome.dealt,
                blocked: outcome.blocked > 0.0,
            });
        }
        if raised {
            let source = HurtSource {
                damage_type,
                causing,
                direct: attack.projectile_direct(damage_type),
                position: None,
                direct_type: None,
            };
            self.after_block(outcome, (attacker, projectile), source, &attack);
        }
        if let Some(([xd, zd], source)) = default_push {
            self.knock_back_player(f64::from(0.4_f32), xd, zd, source, &attack, amount);
            if let Some(p) = self.common.player.as_mut()
                && let Some(r) = p.hurt_log.last_mut()
            {
                r.knockback = Some([p.velocity.x, p.velocity.y, p.velocity.z]);
            }
        }
        self.finish_guest_damage();
        if post_attack
            && outcome.hurt
            && attack.projectile.is_none()
            && attacker.or(owner).is_some()
            && damage_type != "minecraft:thorns"
        {
            let source = attack.player_source(damage_type, causing);
            self.post_attack_from_hand(
                self.own_entity_id() as u32,
                "minecraft:player",
                &attack,
                source,
            );
        }
        outcome.hurt
    }
    /// Right click on a mob (Mob.interact / mobInteract), on the mob under the
    /// crosshair or on the one a guest's interact packet named: returns true
    /// when consumed.
    pub fn interact_mob(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        let Some(mut index) = self.mob_interaction_target(target) else {
            return Ok(false);
        };
        // CamelHusk.interact: any click on it keeps it (`setPersistenceRequired`).
        sunrise_sim::mobs::camel::keep_on_interact(&mut self.common.mobs[index]);
        // PlayerInteractTrigger: the name tag as it was before it named the mob.
        let used = self.common.held().cloned();
        if self.name_mob(index) {
            if let Some(stack) = &used {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            return Ok(true);
        }
        // Shears ask the mob's entity parts first, then fire `player_shear_entity`.
        if self.held_shears(ability::SHEARS_HARVEST) {
            let id = self.common.mobs[index].id;
            if let Some(consumed) = self.player_shear(index) {
                return Ok(consumed);
            }
            // The shear hook may have moved the list: the mob by its id.
            match self.common.mobs.iter().position(|m| m.id == id) {
                Some(now) => index = now,
                None => return Ok(true),
            }
        }
        if self.spawn_egg_offspring(index) {
            self.swing_arm();
            return Ok(true);
        }
        {
            let item = self
                .common
                .held()
                .map(|h| h.item.to_string())
                .unwrap_or_default();
            let used = self.common.held().cloned();
            if let Some(context) = self.interact_leash_with_item_context(index, &item) {
                let used = match context {
                    super::leads::LeashInteraction::WithItem => used,
                    super::leads::LeashInteraction::WithoutItem => {
                        Some(ItemStack::new("minecraft:air", 0))
                    }
                };
                if let Some(stack) = &used {
                    let target = self.mob_entity(&self.common.mobs[index]);
                    self.trigger_item_entity(
                        "minecraft:player_interacted_with_entity",
                        stack,
                        &target,
                    );
                }
                self.swing_arm();
                return Ok(true);
            }
        }
        let held = self.common.held().cloned();
        let item = held
            .as_ref()
            .map(|h| h.item.to_string())
            .unwrap_or_default();
        let shears = self.held_shears(ability::SHEARS_HARVEST);
        // Bucketable.bucketMobPickup: a water bucket scoops up a fish, a
        // tadpole or an axolotl, with the data its bucket carries.
        if item == "minecraft:water_bucket"
            && self.common.mobs[index].alive()
            && let Some((bucket, sound)) = bucketable(self.common.mobs[index].species())
        {
            let mut filled = ItemStack::new(bucket, 1);
            filled.extra = Some(Box::new(sunrise_sim::entity_nbt::bucket_components(
                &self.common.mobs[index],
            )));
            let center = self.common.mobs[index].center();
            self.server_play_sound(sound, center, 1.0, 1.0);
            self.trigger_item("minecraft:filled_bucket", &filled);
            // PlayerInteractTrigger: the water bucket as it was, and the mob picked up.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            let species = bucket.trim_end_matches("_bucket").to_owned();
            let id = self.common.mobs[index].id as i32;
            // Leashable.dropLeash, then discard.
            self.break_leash(id as u32, true, false);
            self.remove_mob_now(id as u32, false);
            if self.common.creative {
                if let Some(rest) = self.add_stack_to_inventory(filled) {
                    self.spawn_item(
                        rest,
                        self.common.eye_position().unwrap_or([0.0; 3]),
                        [0.0; 3],
                        40,
                    );
                }
            } else {
                *self.common.held_mut() = Some(filled);
            }
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_bucket_entity(player, &name, id, &species);
            }
            self.swing_arm();
            return Ok(true);
        }
        // AgeableMob.mobInteract: a golden dandelion on a baby freezes its
        // growth, or lets it grow again.
        if self.common.mobs[index].alive() && self.age_lock_takes(&self.common.mobs[index], &item) {
            let entity_id = self.common.mobs[index].id;
            self.set_age_locked(index);
            if let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            ) {
                self.native_player_toggle_entity_age_lock(
                    player,
                    &name,
                    entity_id,
                    self.common.mobs[index].extra.age_locked,
                );
            }
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        let creative = self.common.creative;
        // The hand's slot (Player.interactOn passes the hand).
        let selected = self.common.held_slot();
        // Panda.mobInteract: bamboo, or a click on a panda lying on its back.
        if self.common.mobs[index].species() == Species::Panda && self.common.mobs[index].alive() {
            let taken = self.interact_panda(index, &item);
            if taken {
                self.swing_arm();
            }
            return Ok(taken);
        }
        if self.interact_zombie_villager(index, &item) {
            return Ok(true);
        }
        if self.interact_trader(index, &item)? {
            self.swing_arm();
            return Ok(true);
        }
        if !self.common.mobs[index].kind.food.contains(&item)
            && self.server_interact_sheep_dye(index, self.common.hand)
        {
            // PlayerInteractTrigger: the dye as it was before it coloured the sheep.
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_allay(index, held.as_ref()) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_sulfur_cube(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_armadillo(index, &item) {
            if let Some(stack) = &held {
                let target = self.mob_entity(&self.common.mobs[index]);
                self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
            }
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_copper_golem(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        if self.interact_shear_mob(index, shears) {
            self.swing_arm();
            return Ok(true);
        }
        if let Some(sheared) = self.shear_saddle(index) {
            if sheared {
                self.swing_arm();
            }
            return Ok(sheared);
        }
        // AbstractHorse.mobInteract: feed, equip or ride.
        if self.interact_mount(index, &item) {
            self.swing_arm();
            return Ok(true);
        }
        // PiglinAi.mobInteract: hand over barter currency to admire.
        if self.common.mobs[index].alive()
            && held.as_ref().is_some_and(|h| self.give_piglin(index, h))
        {
            self.swing_arm();
            if !creative {
                self.consume_player_slot(selected, 1);
            }
            return Ok(true);
        }
        // Creeper.mobInteract: any item in #creeper_igniters, damaged when it
        // is damageable and used up otherwise.
        let creeper_igniter = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:creeper_igniters")
            .is_some_and(|t| t.contains(&*item));
        let fish = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:fishes")
            .is_some_and(|t| t.contains(&*item));
        let damageable = self
            .common
            .res
            .items()
            .get(&item)
            .is_some_and(|i| i.max_damage > 0);
        // Wolf.mobInteract: the owner's #wolf_collar_dyes item only ever dyes the
        // collar; one of the colour the collar already has does nothing.
        let wolf_collar_dye = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:wolf_collar_dyes")
            .is_some_and(|t| t.contains(&*item));
        let parrot_poisonous = self
            .common
            .res
            .items()
            .item_tags
            .get("minecraft:parrot_poisonous_food")
            .is_some_and(|t| t.contains(&*item));
        let player_at = self.common.eye_position();
        let player_feet = self
            .common
            .player
            .as_ref()
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let acting = self.acting_player_uuid();
        let (acting_id, acting_entity_id) = (self.native_acting_id(), self.own_entity_id());
        let sneaking = self.server_acting_sneak();
        let held_damage = held.as_ref().map_or(0, |h| h.damage as i32);
        let held_extra = held.as_ref().and_then(|h| h.extra.clone());
        let feeder = sunrise_sim::health::Attack {
            player_uuid: self.player_uuid_of(self.server.lan_acting),
            ..Default::default()
        };
        let credit_plan = self.prepare_knockback_credit(
            self.common.mobs[index].id as i32,
            Some(Who::Player),
            &feeder,
        );
        let removes_armor = self.common.mobs[index].species() == Species::Wolf
            && self.held_shears(ability::SHEARS_REMOVE_ARMOR);
        let m = &mut self.common.mobs[index];
        if !m.alive() {
            return Ok(false);
        }
        let species = m.species();
        let center = m.center();
        // Wolf armor the owner takes off with shears or leaves a dropped stack of.
        let mut armor_drop: Option<ItemStack> = None;
        let mut equip_event: Option<bool> = None;
        let mut consume = false;
        let mut give: Option<&str> = None;
        let mut sound: Option<(&str, f32, f32)> = None;
        // Sounds played as vanilla plays them: (event, volume, pitch) at the feet.
        let mut eat: Option<(Cow<'static, str>, f32, f32)> = None;
        let mut shear = None;
        let feet = [m.position.x, m.position.y, m.position.z];
        let mut handled = true;
        let mut ignited_creeper = false;
        let mut clear_wolf_target = false;
        let mut drops: Vec<(String, u32)> = Vec::new();
        // The shearing table rolled, for the mods.
        let mut drops_table = String::new();
        let mob = (
            m.id,
            sunrise_gameplay_common::game::native_hooks::mob_kind(m),
        );
        // `Sheep.shear` adds a spread on top of the ItemEntity's own motion.
        let mut wool = false;
        let is_food = m.kind.food.contains(&item);
        let baby_turtle = species == Species::Turtle && m.is_baby();
        // MushroomCow.getEffectsFromItemStack: the item's block supplies them.
        let stew_effects = (species == Species::Mooshroom && m.extra.variant == 1 && !m.is_baby())
            .then(|| world_items!(self).suspicious_stew_effects(&item))
            .flatten();
        let mut give_stew: Option<Vec<(Arc<str>, i32)>> = None;
        // `Some(true)` for the four spell particles, `Some(false)` for the two
        // smoke puffs of a cow that already holds effects.
        let mut stew_particles: Option<bool> = None;
        // `Ocelot.mobInteract`: whether the fish won its trust.
        let mut ocelot_trust: Option<bool> = None;
        match species {
            Species::Cow | Species::Mooshroom if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                sound = Some(("entity.cow.milk", 1.0, 1.0));
            }
            // Goat.mobInteract: a bucket on an adult gives milk.
            Species::Goat if item == "minecraft:bucket" && !m.is_baby() => {
                give = Some("minecraft:milk_bucket");
                consume = true;
                let milk = if m.extra.screaming {
                    "entity.goat.screaming.milk"
                } else {
                    "entity.goat.milk"
                };
                sound = Some((milk, 1.0, 1.0));
            }
            // MushroomCow.mobInteract: a bowl gives back what a flower left as
            // a suspicious stew, or plain stew when it holds no effects.
            Species::Mooshroom if item == "minecraft:bowl" && !m.is_baby() => {
                consume = true;
                match m.extra.stew_effects.take() {
                    Some(effects) => {
                        give = Some("minecraft:suspicious_stew");
                        give_stew = Some(effects);
                        // `playSound(.., 1.0F, 1.0F)` at the feet.
                        eat = Some((Cow::Borrowed("entity.mooshroom.suspicious_milk"), 1.0, 1.0));
                    }
                    None => {
                        give = Some("minecraft:mushroom_stew");
                        sound = Some(("entity.mooshroom.milk", 1.0, 1.0));
                    }
                }
            }
            Species::Mooshroom if shears && !m.is_baby() => {
                let player = Some(self.native_acting_id());
                self.shear_mooshroom(index, player, held.as_ref().expect("held shears"));
                let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // MushroomCow.mobInteract: a brown adult takes the block's effects
            // unless it holds some already, when it only puffs smoke.
            Species::Mooshroom if let Some(effects) = stew_effects => {
                if m.extra.stew_effects.is_some() {
                    stew_particles = Some(false);
                } else {
                    m.extra.stew_effects = Some(effects.to_vec());
                    consume = true;
                    eat = Some((Cow::Borrowed("entity.mooshroom.eat"), 2.0, 1.0));
                    stew_particles = Some(true);
                }
            }
            Species::Sheep if shears && !m.extra.sheared && !m.is_baby() => {
                shear = Some((m.id, m.silent));
                m.extra.sheared = true;
                let color = sunrise_level::dyes::DYE_NAMES[m.extra.color as usize & 15];
                drops_table = format!("minecraft:shearing/sheep/{color}");
                let stacks = world_items!(self).loot_table(
                    &drops_table,
                    false,
                    None,
                    &mut self.common.world.random,
                );
                drops.extend(stacks.into_iter().map(|s| (s.item.to_string(), s.count)));
                wool = true;
                let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
                self.post_game_event("minecraft:shear", feet, ctx);
                self.damage_held_tool();
            }
            // Entity.interact: shears take a tamed wolf's armor off for its owner.
            Species::Wolf
                if removes_armor
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_some()
                    && !sneaking =>
            {
                armor_drop = m.equipment_stacks()[6].clone();
                m.extra.set_body_stack(None);
                eat = Some((Cow::Borrowed("item.armor.unequip_wolf"), 1.0, 1.0));
                equip_event = Some(false);
                self.damage_held_tool();
            }
            // Wolf.mobInteract: heal, dye the collar or sit/stand when tame;
            // a bone tames a wild one a third of the time.
            Species::Wolf if m.extra.tame && is_food && m.health < m.attrs.max_health => {
                let heal = self
                    .common
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(2.0, |f| f.0 as f32 * 2.0);
                // TamableAnimal.feed(.., 2.0F, 2.0F): heals twice the nutrition,
                // then playEatingSound, which wolves leave empty.
                m.heal(
                    &self.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(self),
                    heal,
                );
                consume = true;
            }
            // Wolf.mobInteract: the owner puts wolf armor on a tamed adult.
            Species::Wolf
                if item == "minecraft:wolf_armor"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.body_id().is_none()
                    && !m.is_baby() =>
            {
                let mut body = ItemStack::new(item.as_str(), 1);
                body.damage = held_damage.max(0) as u32;
                body.extra = held_extra;
                m.extra.set_body_stack(Some(body));
                consume = true;
                eat = Some((Cow::Borrowed("item.armor.equip_wolf"), 1.0, 1.0));
                equip_event = Some(true);
            }
            // Wolf.mobInteract: an armadillo scute repairs a sitting wolf's
            // armor by an eighth of its durability.
            Species::Wolf
                if item == "minecraft:armadillo_scute"
                    && m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.extra.sitting
                    && m.extra.body_id().is_some()
                    && m.extra.body_damage() > 0 =>
            {
                m.extra.set_body_damage(
                    m.extra.body_damage()
                        - (sunrise_sim::mobs::WOLF_ARMOR_DURABILITY as f32 * 0.125) as i32,
                );
                consume = true;
                eat = Some((Cow::Borrowed("item.wolf_armor.repair"), 1.0, 1.0));
            }
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && sunrise_level::dyes::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = sunrise_level::dyes::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                consume = true;
            }
            // Wolf.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Wolf
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && !wolf_collar_dye
                    && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                m.ai.jumping = false;
                m.nav_stop();
                clear_wolf_target = true;
                self.common.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Wolf
                if !m.extra.tame
                    && item == "minecraft:bone"
                    && !sunrise_sim::mobs::tame::angry(m) =>
            {
                consume = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.common.mobs[index];
                if tamed {
                    sunrise_sim::mobs::tame::set_tame(m, true);
                    m.extra.owner = Some(acting);
                    m.persistent = true;
                    m.nav_stop();
                    clear_wolf_target = true;
                    m.extra.sitting = true;
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.common.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.common.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.common.mobs[index];
                self.common.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Cat.mobInteract: dye, heal or sit when tame; fish may tame a wild one.
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && item.ends_with("_dye")
                    && sunrise_level::dyes::DYE_NAMES
                        .iter()
                        .position(|d| {
                            item.trim_start_matches("minecraft:")
                                .trim_end_matches("_dye")
                                == *d
                        })
                        .is_some_and(|c| c as u8 != m.extra.color) =>
            {
                let name = item
                    .trim_start_matches("minecraft:")
                    .trim_end_matches("_dye");
                if let Some(c) = sunrise_level::dyes::DYE_NAMES
                    .iter()
                    .position(|d| *d == name)
                {
                    m.extra.color = c as u8;
                }
                m.persistent = true;
                consume = true;
            }
            Species::Cat
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && is_food
                    && m.health < m.attrs.max_health =>
            {
                let heal = self
                    .common
                    .res
                    .items()
                    .get(&item)
                    .and_then(|i| i.food.as_ref())
                    .map_or(1.0, |f| f.0 as f32);
                // TamableAnimal.feed.
                m.heal(
                    &self.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(self),
                    heal,
                );
                consume = true;
                let variants = world_variants!(self);
                eat = eating_sound_with_variants(m, &mut self.common.world.random, Some(variants));
            }
            // Cat.mobInteract: Animal.mobInteract first breeds an adult or
            // grows a baby with the food; only when it did not does the owner
            // make the pet sit or stand.
            Species::Cat
                if m.extra.tame && m.extra.owner == Some(acting) && !(is_food && takes_food(m)) =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.common.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // Parrot.mobInteract: the owner makes a tame parrot on the ground
            // sit or stand (poisonous food is handled apart).
            Species::Parrot
                if m.extra.tame
                    && m.extra.owner == Some(acting)
                    && m.ground.on_ground
                    && !parrot_poisonous =>
            {
                m.extra.sitting = !m.extra.sitting;
                self.common.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            Species::Cat if !m.extra.tame && is_food => {
                consume = true;
                m.persistent = true;
                let rolled = m.random.next_int_bound(3).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.common.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                    m.extra.sitting = true;
                }
                let variants = world_variants!(self);
                eat = eating_sound_with_variants(m, &mut self.common.world.random, Some(variants));
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.common.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.common.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
                let m = &self.common.mobs[index];
                self.common.world.modified.insert((
                    m.position.x.floor() as i32 >> 4,
                    m.position.z.floor() as i32 >> 4,
                ));
            }
            // IronGolem.mobInteract: an iron ingot repairs 25 health.
            Species::IronGolem
                if item == "minecraft:iron_ingot" && m.health < m.attrs.max_health =>
            {
                m.heal(
                    &self.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(self),
                    25.0,
                );
                consume = true;
                eat = Some((
                    Cow::Borrowed("entity.iron_golem.repair"),
                    1.0,
                    m.golem_repair_pitch(),
                ));
            }
            Species::Creeper if creeper_igniter => {
                m.extra.flag = true;
                ignited_creeper = true;
                // FIRECHARGE_USE for a fire charge, else FLINTANDSTEEL_USE,
                // at the creeper's feet with a pitch from its own random.
                let event = if item == "minecraft:fire_charge" {
                    "item.firecharge.use"
                } else {
                    "item.flintandsteel.use"
                };
                eat = Some((Cow::Borrowed(event), 1.0, m.random.next_float() * 0.4 + 0.8));
                if damageable {
                    self.damage_held_tool();
                } else {
                    consume = true;
                }
            }
            // Parrot.mobInteract: #parrot_poisonous_food (a cookie) poisons the
            // parrot for 45 s and then hurts it with a player attack of
            // Float.MAX_VALUE, which an invulnerable one shrugs off.
            Species::Parrot
                if self
                    .common
                    .res
                    .items()
                    .item_tags
                    .get("minecraft:parrot_poisonous_food")
                    .is_some_and(|t| t.contains(&*item)) =>
            {
                consume = true;
                let id = m.id;
                let join = m.join;
                self.add_mob_effect(
                    id,
                    sunrise_level::effects::EffectInstance::new("poison", 900, 0),
                    None,
                );
                if let Some(index) = self.common.mob_index.find(&self.common.mobs, id)
                    && self.common.mobs[index].join == join
                    && !self.common.mobs[index].removed
                {
                    let hurt = self.common.mobs[index].damage(
                        crate::game::server_side::entity_server::damage_level_of!(
                            self,
                            &sunrise_sim::health::NO_ATTACK,
                            &mut credit_plan.lend(&mut self.server.last_hurt_by_player),
                            None
                        ),
                        f32::MAX,
                        sunrise_sim::mobs::Damage::player(player_at.unwrap_or(center)),
                    );
                    if let Some((index, hurt)) = self.complete_mob_hurt(index, hurt) {
                        // The feeder gets the kill, as for a normal player attack.
                        if hurt {
                            self.remember_mob_player_credit(id);
                        }
                        self.finish_mob_damage_at(index);
                    }
                }
            }
            // Parrot.mobInteract: parrot food may tame a wild one (one time in
            // ten); a parrot never takes it as Animal food (isFood is false).
            Species::Parrot if !m.extra.tame && is_food => {
                consume = true;
                if !m.silent {
                    let pitch = 1.0 + (m.random.next_float() - m.random.next_float()) * 0.2;
                    eat = Some((Cow::Borrowed("entity.parrot.eat"), 1.0, pitch));
                }
                let rolled = m.random.next_int_bound(10).unwrap_or(1) == 0;
                let tamed =
                    rolled && !self.animal_tame_cancelled(index, acting_id, acting_entity_id);
                let m = &mut self.common.mobs[index];
                if tamed {
                    m.extra.tame = true;
                    m.extra.owner = Some(acting);
                }
                // TamableAnimal.spawnTamingParticles (entity event 7 or 6).
                self.mob_event_particles(self.common.mobs[index].id, if tamed { 7 } else { 6 });
                if tamed {
                    let view = self.mob_entity(&self.common.mobs[index]);
                    self.trigger_entity("minecraft:tame_animal", "entity", &view);
                    self.pet_tamed(index);
                }
            }
            // Dolphin.mobInteract: a fish makes a baby grow by a tenth and an
            // adult remember it (`gotFish`), to swim to a treasure.
            Species::Dolphin if fish => {
                eat = Some((Cow::Borrowed("entity.dolphin.eat"), 1.0, 1.0));
                if sunrise_sim::mobs::ai::can_age_up(m) {
                    let secs = ((-m.extra.age / 20) as f32 * 0.1) as i32;
                    sunrise_sim::mobs::ai::age_up(m, secs, true);
                } else {
                    m.extra.flag = true;
                }
                consume = true;
            }
            // Tadpole.feed: frog food ages it by a tenth of the time left to
            // grow into a frog (it grows on its next tick once past 24000).
            Species::Tadpole if is_food && !m.extra.age_locked => {
                consume = true;
                let left = 24_000 - m.extra.age;
                m.extra.age += ((left / 20) as f32 * 0.1) as i32 * 20;
                // One happy villager particle at rest about the body.
                self.common.particles_around_mob(
                    self.common.mobs[index].id,
                    "happy_villager",
                    sunrise_level::particle_options::Burst::HORSE_EATING,
                );
            }
            // Bee.mobInteract: a wither rose or an open eyeblossom is eaten for
            // its `getBeeInteractionEffect` instead of breeding the bee.
            Species::Bee
                if is_food
                    && matches!(
                        &*item,
                        "minecraft:wither_rose" | "minecraft:open_eyeblossom"
                    ) =>
            {
                let id = m.id;
                let effect = if &*item == "minecraft:wither_rose" {
                    sunrise_level::effects::EffectInstance::new("wither", 40, 0)
                } else {
                    sunrise_level::effects::EffectInstance::new("poison", 25, 0)
                };
                self.add_mob_effect(id, effect, None);
                consume = true;
            }
            // Ocelot.mobInteract: raw fish from a hand within 3 blocks, while it comes for the
            // food (`temptGoal.isRunning`), is used up; one time in three it trusts the player
            // (entity event 41, hearts), otherwise smoke (event 40). Anything else is
            // `Animal.mobInteract`.
            Species::Ocelot
                if is_food
                    && !m.extra.trusting
                    && m.ai.goals.is_running("OcelotTemptGoal")
                    && player_feet.is_some_and(|p| {
                        let d = [
                            p[0] - m.position.x,
                            p[1] - m.position.y,
                            p[2] - m.position.z,
                        ];
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 9.0
                    }) =>
            {
                consume = true;
                let trusts = m.random.next_int_bound(3).unwrap_or(1) == 0;
                if trusts {
                    m.extra.trusting = true;
                }
                ocelot_trust = Some(trusts);
            }
            // HappyGhast.canFallInLove: never, so only a ghastling takes food.
            _ if is_food
                && m.kind.flags.animal
                && species != Species::Parrot
                && (species != Species::HappyGhast || m.is_baby()) =>
            {
                // Animal.mobInteract: breed adults, speed up babies, then
                // playEatingSound (silent unless the species overrides it).
                let variants = world_variants!(self);
                match feed_animal_with_variants(
                    m,
                    &mut self.common.world.random,
                    Some(acting),
                    Some(variants),
                ) {
                    Some(sound) => {
                        consume = true;
                        eat = sound;
                        // Axolotl.usePlayerItem: a tropical fish bucket is
                        // emptied into a water bucket in its place.
                        if species == Species::Axolotl && &*item == "minecraft:tropical_fish_bucket"
                        {
                            give = Some("minecraft:water_bucket");
                        }
                        // Animal.setInLove: entity event 18 (a baby that
                        // only grew is not in love).
                        if self.common.mobs[index].extra.in_love > 0 {
                            let id = self.common.mobs[index].id;
                            self.server.love_cause.insert(id, self.server.lan_acting);
                            let kind = self.common.mobs[index].kind.id.clone();
                            self.native_entity_enter_love_mode(
                                id,
                                &kind,
                                self.server.lan_acting_uuid,
                            );
                            self.mob_event_particles(id, 18);
                        }
                    }
                    // Animal.mobInteract still consumes the client click when
                    // an adult cannot breed, without swinging or taking food.
                    None => return Ok(true),
                }
            }
            _ => handled = false,
        }
        if !handled {
            return Ok(false);
        }
        if clear_wolf_target {
            self.set_event_ai_target(mob.0, None, self.server.lan_acting);
        }
        if ignited_creeper {
            self.native_creeper_ignite(
                self.common.mobs[index].id,
                sunrise_sim::lan::playerdata::ints_uuid(acting),
                &item,
            );
        }
        if let Some(trusts) = ocelot_trust {
            // `spawnTrustingParticles` on the clients: entity event 41 (hearts) or 40 (smoke).
            self.mob_event_particles(self.common.mobs[index].id, if trusts { 41 } else { 40 });
        }
        if let Some(fresh) = stew_particles {
            // `SpellParticleOption.create(EFFECT, -1, 1)` is the plain white
            // effect particle; the smoke is the already-fed answer.
            let ty = if fresh {
                ParticleType::Effect
            } else {
                ParticleType::Smoke
            };
            let at = [feet[0], feet[1] + 0.5, feet[2]];
            let (spread, count) = ([0.5, 0.0, 0.5], if fresh { 4 } else { 2 });
            // shortcut: scalar speed models DEFAULT randomization; 26.3 sends
            // ALTERNATIVE_WITH_SPEED with a per-axis maximum speed.
            self.server_send_particles(sunrise_sim::lan::server_effects::ServerQueuedParticle {
                dimension: self.common.dim.id(),
                particle: sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                at,
                spread,
                speed: [0.2; 3],
                count,
                randomization: 0,
                except: None,
            });
        }
        // PlayerInteractTrigger: the item used on the mob, once the interaction took.
        if let Some(stack) = &held {
            let target = self.mob_entity(&self.common.mobs[index]);
            self.trigger_item_entity("minecraft:player_interacted_with_entity", stack, &target);
        }
        // Turtle.ageBoundaryReached: food that finishes a baby's growth.
        if baby_turtle && !self.common.mobs[index].is_baby() {
            self.handle_mob_events(vec![MobEvent::GiftLoot {
                mob: self.mobs[index].id,
                table: "minecraft:gameplay/turtle_grow",
                at: feet,
            }])?;
        }
        self.swing_arm();
        match sound {
            // Sheep.shear: level.playSound(null, this, SHEEP_SHEAR, PLAYERS, 1, 1).
            // DyeItem.interactLivingEntity: level.playSound(player, sheep,
            // DYE_USE, PLAYERS, 1, 1), whose player is left out because their
            // client played it; Sunrise's own client plays it from the packet,
            // so every player gets it.
            Some((s @ ("entity.sheep.shear" | "item.dye.use"), volume, pitch)) => {
                let name = full_id(s);
                let id = self.common.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Players);
                self.play_bound_sound(sound, id, None);
            }
            Some((s, volume, pitch)) => {
                self.server_play_sound(&full_id(s), feet, volume, pitch);
            }
            None => {}
        }
        if let Some((id, silent)) = shear
            && !silent
        {
            self.play_bound_sound(
                LevelSound::new("minecraft:entity.sheep.shear", feet, 1.0, 1.0)
                    .in_source(SoundSource::Neutral),
                id,
                None,
            );
        }
        if let Some((s, volume, pitch)) = eat {
            let name = full_id(&s);
            if eating_sound_is_bound(species) {
                let id = self.common.mobs[index].id;
                let sound =
                    LevelSound::new(&name, feet, volume, pitch).in_source(SoundSource::Neutral);
                self.play_bound_sound(sound, id, None);
            } else {
                self.server_play_sound(&name, feet, volume, pitch);
            }
        }
        if consume && !creative {
            self.consume_player_slot(selected, 1);
        }
        if let Some(g) = give {
            let mut stack = ItemStack {
                item: g.into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            if let Some(effects) = give_stew {
                stack.data_mut().stew_effects = effects;
            }
            if self.common.stack_in(selected).is_none() {
                *self.common.stack_in_mut(selected) = Some(stack);
            } else if let Some(rest) = self.add_stack_to_inventory(stack) {
                let at = self.common.eye_position().unwrap_or(center);
                self.spawn_item(rest, at, [0.0, 0.0, 0.0], 40);
            }
        }
        if let Some(stack) = armor_drop {
            // Mob.shearItem: PLAYER_SHEARED_EQUIPMENT with the armor and the wolf.
            let target = self.mob_entity(&self.common.mobs[index]);
            self.trigger_item_entity("minecraft:player_sheared_equipment", &stack, &target);
            self.spawn_item(stack, [center[0], center[1] + 0.5, center[2]], [0.0; 3], 10);
        }
        if let Some(equip) = equip_event {
            let id = self.common.mobs[index].id;
            let pos = self.common.mobs[index].position;
            self.mob_equip_event(id, [pos.x, pos.y, pos.z], equip);
        }
        let mut drops: Vec<ItemStack> = drops
            .into_iter()
            .map(|(item, count)| ItemStack::new(&item, count))
            .collect();
        if !drops.is_empty() {
            let player = Some(self.native_acting_id());
            self.native_use_drops(
                &drops_table,
                feet,
                None,
                Some((mob.0, &mob.1)),
                player,
                held.as_ref(),
                &mut drops,
            );
        }
        for stack in drops {
            for _ in 0..stack.count {
                // `spawnAtLocation(stack, 1.0)`: an ItemEntity a block above
                // the feet that starts with the constructor's own motion, and
                // `setDefaultPickUpDelay`. Sheep.shear adds its spread on top;
                // MushroomCow.shear builds the entity by hand instead, so its
                // mushrooms keep the constructor's motion and no pickup delay.
                let mut spread = [0.0; 3];
                if wool {
                    spread[0] = (self.common.world.random.next_float()
                        - self.common.world.random.next_float())
                        as f64
                        * 0.1;
                    spread[1] = self.common.world.random.next_float() as f64 * 0.05;
                    spread[2] = (self.common.world.random.next_float()
                        - self.common.world.random.next_float())
                        as f64
                        * 0.1;
                }
                self.spawn_item_own(
                    ItemStack {
                        count: 1,
                        ..stack.clone()
                    },
                    [feet[0], feet[1] + 1.0, feet[2]],
                    if wool { 10 } else { 0 },
                );
                if let Some(e) = self.just_spawned_item() {
                    e.velocity = e
                        .velocity
                        .add_vector(Vec3::new(spread[0], spread[1], spread[2]));
                }
            }
        }
        Ok(true)
    }
    pub(in crate::game::server_side) fn killer_uuid(&mut self, conn: Option<u32>) -> Option<u128> {
        match conn {
            // During that guest's own turn, the player in place has their UUID.
            Some(conn) if self.server.lan_acting != Some(conn) => {
                Some(self.server.lan.as_ref()?.hub.guest(conn)?.uuid)
            }
            Some(_) => Some(self.native_acting_id()),
            None if self.server.lan_acting.is_none() && self.common.player.is_some() => {
                Some(self.native_acting_id())
            }
            None => None,
        }
    }
    /// A mob's projectile enters the level.
    pub fn launch_mob_projectile(&mut self, mut p: Projectile) {
        self.server.next_entity += 1;
        p.id = self.server.next_entity;
        self.spawn_mob_projectile(p);
    }
    /// `LivingEntity.die` / `dropAllDeathLoot`. Does not finish a hurt.
    pub fn mob_die(&mut self, m: &Mob) {
        // Death reads the remembered player without consuming it. A body
        // still in the level can answer getKillCredit after this call.
        let source = (!mod_api::__private::mods().is_empty()).then(|| self.mob_damage_source(m));
        let killer = self
            .server
            .last_hurt_by_player
            .get(&m.id)
            .copied()
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|credit| self.resolve_player_credit(credit));
        if self.mob_death_canceled(m) {
            return;
        }
        // LivingEntity.die: a sleeper gets up, and its bed is free again.
        if let Some(bed) = sunrise_sim::mobs::villager::sleeping_pos(m) {
            self.wake_villager(m.id);
            // The body may be gone already.
            self.set_bed_occupied(bed, false);
        }
        // TamableAnimal.die, before the rest of LivingEntity.die.
        self.tell_owner_pet_died(m, killer);
        self.server.love_cause.remove(&m.id);
        let by_player = m.ai.last_hurt_by_player > 0;
        let mut dying = m.clone();
        // LivingEntity.die: the death is heard, and a sculk catalyst nearby
        // takes the experience (before the loot is dropped). It takes what
        // would drop, whoever killed the mob. Experience the catalyst takes is
        // never dropped, so living_experience_drop does not hear of it.
        self.sculk_mob_died(m);
        let mut catalyst_xp = 0;
        let consumed =
            self.catalyst_takes_experience([m.position.x, m.position.y, m.position.z], |game| {
                let base = sunrise_sim::mobs::drops::base_experience(&mut dying);
                let reward = game.mob_kill_experience(&mut dying, base);
                catalyst_xp = if sunrise_sim::mobs::drops::drops_experience(&dying) {
                    reward.max(0) as u32
                } else {
                    0
                };
                catalyst_xp
            });
        // CatalystListener: with experience to take, the player who last hurt
        // the mob gets KILL_MOB_NEAR_SCULK_CATALYST.
        if consumed && catalyst_xp > 0 {
            let who = match killer {
                Some(Some(conn)) if self.server.lan_acting == Some(conn) => {
                    Some(super::commands::Target::Player)
                }
                Some(Some(conn)) => Some(super::commands::Target::Guest(conn)),
                Some(None) if self.server.lan_acting.is_none() => {
                    Some(super::commands::Target::Player)
                }
                _ => None,
            };
            if let Some(who) = who {
                self.for_player(who, |g| {
                    // CatalystListener: trigger(player, killedEntity, damageSource).
                    let view = g.mob_entity(m);
                    let blow = g.last_blow(m);
                    g.trigger_killed_as(
                        "minecraft:kill_mob_near_sculk_catalyst",
                        &view,
                        blow.as_ref(),
                    );
                });
            }
        }
        if let Some(killer) = killer {
            // Only the resolved player receives statistics and advancements.
            match killer {
                Some(conn) if self.server.lan_acting != Some(conn) => {
                    self.as_guest(conn, |g| g.credit_kill(m));
                }
                _ => self.credit_kill(m),
            }
        }
        // The body keeps its scores until it is removed (`ServerLevel.
        // EntityCallbacks.onDestroyed` → `Scoreboard.entityRemoved`, run by
        // `score_destroyed_entities`): LivingEntity.die does not reset them.
        // Raider.die's hero and Villager.tellWitnessesThatIWasMurdered go to
        // the causing entity, not the credited player: a tame wolf's kill is
        // neither its owner's heroism nor a murder by them.
        let slain_by_player = by_player && !killed_by_mob(m);
        self.raider_died(m, slain_by_player, killer.flatten());
        self.villager_murdered(m, slain_by_player, killer.flatten());
        let pos = [m.position.x, m.position.y + 0.5, m.position.z];
        let killer_entity = self.killer_loot_entity(killer.flatten());
        // LAST_DAMAGE_PLAYER only while the player is still in the world and
        // the hurt memory lasts (LivingEntity.dropFromLootTable). Experience
        // and equipment use the timer alone (dropExperience / playerKilled):
        // a guest who has left still counts.
        let player = killer
            .filter(|_| m.ai.last_hurt_by_player > 0)
            .and_then(|conn| self.killer_uuid(conn));
        if let Some(player) = player.filter(|_| m.extra.leash.is_some()) {
            self.server
                .lead_killers
                .insert(m.id, (player, self.common.dim));
        }
        self.drop_death_loot(
            &mut dying,
            player,
            m.ai.last_hurt_by_player > 0,
            killer_entity,
            consumed,
            pos,
            source,
        );
        self.finish_mob_death(m, &mut dying, source, player);
    }

    /// `AbstractCubeMob.remove` and `LivingEntity.remove`: split, run removal effects,
    /// then retire the body. A failed callback cannot queue the removal for another tick.
    fn complete_mob_removal(&mut self, join: JoinSeq, split: bool) {
        let Some(index) = self.mobs.locate(join) else {
            return;
        };
        let removal = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut random = if split {
                let mut dying = self.mobs[index].clone();
                self.split_cube_children(&mut dying);
                dying.random
            } else {
                self.mobs[index].random.clone()
            };
            // LivingEntity.remove sees the body's current effects and pose, after the split
            // and child join listeners have had a chance to edit it. Keep the split's draws.
            let Some(index) = self.mobs.locate(join) else {
                return;
            };
            let parent = &self.mobs[index];
            if !parent.extra.effects.is_empty() {
                let names: Vec<_> = parent
                    .extra
                    .effects
                    .iter()
                    .map(|e| e.id.trim_start_matches("minecraft:"))
                    .collect();
                self.effects_on_death(
                    &names,
                    parent.bounds(),
                    parent.silent,
                    Some(parent.id),
                    Some(&mut random),
                );
            }
        }));
        if let Some(index) = self.common.mobs.locate(join) {
            let mob = &self.common.mobs[index];
            let id = mob.id;
            if !mob.removed && (mob.extra.mount.is_some() || self.has_seated_passengers(id)) {
                self.finish_mob_removal(id);
            }
            if let Some(index) = self.common.mobs.locate(join) {
                self.common.mobs.remove(index);
            }
            self.server.last_hurt_by_player.remove(&id);
        }
        if let Err(payload) = removal {
            std::panic::resume_unwind(payload);
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn complete_mob_removal_for_test(&mut self, join: JoinSeq, split: bool) {
        self.complete_mob_removal(join, split);
    }

    fn mob_weapon_attack(&mut self, hit: sunrise_sim::mobs::WeaponHit) -> bool {
        use sunrise_sim::mobs::{WeaponAttack, WeaponHit};
        let WeaponHit {
            target,
            source,
            weapon,
            amount,
            knockback,
            direction,
            attack: action,
        } = hit;
        let (victim, kind) = match target {
            Who::Player => (self.own_entity_id() as u32, Arc::from("minecraft:player")),
            Who::Mob(id) => {
                let Some(mob) = self.mob_object(id).filter(|mob| !mob.removed) else {
                    return false;
                };
                (id, Arc::<str>::from(mob.kind.id.as_str()))
            }
        };
        let attack = self.attack_of_who(source.attacker);
        let damage_type = source_damage_type(source);
        let hurt_source = HurtSource {
            damage_type,
            causing: attack.attacker,
            direct: attack.attacker,
            direct_type: attack.direct_type.as_deref(),
            position: source.from,
        };
        let ctx = attack.enchantment_context(hurt_source, &kind);
        let amount = self.evaluate_weapon_damage(&weapon, Some(victim), ctx, amount);
        let hurt = !matches!(
            action,
            WeaponAttack::Stab {
                deals_damage: false,
                ..
            }
        ) && match target {
            Who::Player => self.hurt_player_from_inner(
                amount,
                source.from,
                damage_type,
                match source.attacker {
                    Some(Who::Mob(id)) => KnownCause::Entity(id),
                    _ => KnownCause::None,
                },
                false,
                false,
            ),
            Who::Mob(id) => self.mob_hits_mob(id, amount, source),
        };
        let push = |game: &mut Self, power: f32, apply_blocking| {
            if power > 0.0 {
                game.mob_event_knockback(
                    target,
                    f64::from(power),
                    direction,
                    source,
                    apply_blocking,
                );
                if let Some(Who::Mob(id)) = source.attacker
                    && let Some(attacker) = game.mob_object_mut(id)
                {
                    attacker.velocity.x *= 0.6;
                    attacker.velocity.z *= 0.6;
                }
            }
        };
        let stab_push = matches!(
            action,
            WeaponAttack::Stab {
                deals_knockback: true,
                ..
            }
        );
        if stab_push {
            push(self, 0.4, None);
        }
        if stab_push || (hurt && matches!(action, WeaponAttack::Melee { .. })) {
            let power = self.evaluate_weapon_value(
                &weapon,
                "minecraft:knockback",
                Some(victim),
                ctx,
                knockback,
            ) / 2.0;
            push(self, power, stab_push.then_some(amount));
        }
        if matches!(
            action,
            WeaponAttack::Stab {
                dismounts: true,
                ..
            }
        ) {
            self.mount_event(&MobEvent::StabDismount { target });
        }
        if hurt {
            self.post_attack_from_hand(victim, &kind, &attack, hurt_source);
            if let WeaponAttack::Melee {
                effect: Some((effect, ticks, amplifier)),
            } = action
            {
                let cause = source.attacker.and_then(|who| match who {
                    Who::Mob(id) => self.entity(id),
                    Who::Player => self.entity(self.own_entity_id() as u32),
                });
                match target {
                    Who::Player => self.give_player_effect_from(effect, ticks, amplifier, cause),
                    Who::Mob(id) => {
                        if let Some(def) = sunrise_level::brewing::effect(effect) {
                            self.add_mob_effect(
                                id,
                                sunrise_level::effects::EffectInstance::from_def(
                                    def, ticks, amplifier,
                                ),
                                cause,
                            );
                        }
                    }
                }
            }
        }
        hurt
    }

    fn mob_event_knockback(
        &mut self,
        target: Who,
        power: f64,
        [x, z]: [f64; 2],
        source: Damage,
        apply_blocking: Option<f32>,
    ) {
        match target {
            // Ram knockback checks blocking independently of the hurt call.
            Who::Player => self.mob_pushes_player(power, x, z, source, apply_blocking),
            Who::Mob(id) => {
                let aim = self.attacker_aim(source.attacker);
                let attack = self.attack_of_who(source.attacker);
                let plan =
                    self.prepare_knockback_credit(id as i32, None, self.server.combat.0.attack());
                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    m.attacker_aim = aim;
                    let level = crate::game::entity_server::damage_level_of!(
                        self,
                        &attack,
                        &mut credit,
                        None
                    );
                    m.knockback(level, power, x, z, &source, 0.0);
                    m.attacker_aim = None;
                }
            }
        }
    }

    /// Melee or ranged damage to another mob; whether the hit landed.
    pub fn mob_hits_mob(&mut self, target: u32, amount: f32, mut source: Damage) -> bool {
        if source.attacker_species.is_none()
            && let Some(Who::Mob(id)) = source.attacker
        {
            source.attacker_species = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species());
        }
        let mut killed = false;
        let mut settle = false;
        let mut hurt = false;
        let mut infested = Vec::new();
        let mut sounds = Vec::new();
        let mut center = [0.0; 3];
        let catch_up = self.catch_up_countdowns(source.attacker, target);
        let aim = self.attacker_aim(source.attacker);
        // So `living_knockback` sees who hit and what they hold.
        let attack = self.attack_of_who(source.attacker);
        let plan = self.prepare_knockback_credit(target as i32, source.attacker, &attack);
        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
        let mut target_index = self.common.mobs.iter().position(|m| m.id == target);
        // One subscription decision for the hit's attacker loan.
        let listened = sunrise_sim::health::hit_host_listened();
        let attacker_index = match source.attacker {
            Some(Who::Mob(id)) if listened => self.common.mobs.iter().position(|m| m.id == id),
            _ => None,
        };
        if let Some(ti) = target_index {
            // The attacker's own random is lent through a live borrow beside the victim's: a
            // mob attacker from the same list, the attacking player from its own field.
            let (t, lend) = match attacker_index.filter(|&ai| ai != ti) {
                Some(ai) => {
                    let [t, a] = self
                        .common
                        .mobs
                        .get_disjoint_mut([ti, ai])
                        .expect("two different mobs");
                    (t, sunrise_sim::health::HitLend::mob(a))
                }
                None => (
                    &mut self.common.mobs[ti],
                    listened
                        .then(|| {
                            sunrise_sim::health::HitLend::attacking_player(
                                self.common.player.as_mut(),
                                attack.attacker,
                                &self.common.potions.effects,
                                acting_equipment!(self.common),
                            )
                        })
                        .flatten(),
                ),
            };
            let previous_invulnerable = t.invulnerable;
            let hit = t.damage_aimed(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &attack,
                    &mut credit,
                    lend
                ),
                amount,
                source,
                aim,
            );
            let Some((ti, hit)) = self.complete_mob_hurt(ti, hit) else {
                return false;
            };
            target_index = Some(ti);
            let t = &mut self.common.mobs[ti];
            hurt = hit;
            // Squid.hurtServer can report false after its superclass accepted a lethal hit.
            settle = t.damage_completion.is_some() || t.has_protected_hurt();
            if hit && catch_up && t.invulnerable > previous_invulnerable {
                t.hurt_time = (t.hurt_time - 1).max(0);
                t.invulnerable = (t.invulnerable - 1).max(0);
            }
            if hit {
                killed = !t.alive();
                // The hurt or death sound the mob raised.
                center = t.center();
                for event in std::mem::take(&mut t.outbox) {
                    match event {
                        MobEvent::Sound(s, _, volume, pitch) => sounds.push((s, volume, pitch)),
                        MobEvent::Infested { at, velocities } => infested.push((at, velocities)),
                        other => {
                            // Left for the end of the tick.
                            t.outbox.push(other);
                        }
                    }
                }
            }
        }
        if let Some(index) = target_index {
            self.settle_villager_hurt(index);
        }
        for (s, volume, pitch) in sounds {
            self.server_play_sound(&full_id(s), center, volume, pitch);
        }
        if settle && let Some(index) = target_index {
            self.finish_mob_damage_at(index);
            killed &= self
                .common
                .mobs
                .get(index)
                .is_some_and(|m| m.id == target && !m.alive());
        }
        for (at, velocities) in infested {
            self.spawn_infested(at, velocities);
        }
        if hurt
            && let Some(Who::Mob(attacker)) = source.attacker
            && let Some(m) = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == attacker && m.species() == Species::Axolotl)
        {
            let at = [m.position.x, m.position.y, m.position.z];
            self.server_play_sound("minecraft:entity.axolotl.attack", at, 1.0, 1.0);
        }
        if killed && let Some(Who::Mob(killer)) = source.attacker {
            self.zombie_killed(killer, target);
        }
        self.settle_mob_hurt(target);
        hurt
    }
    /// `dropAllDeathLoot` through `mobs::drops`: the entity loot table from the
    /// world's random sequence, equipment by drop chance, species extras and
    /// the experience orbs, spawned in order.
    /// The UUID of the player `last_hurt_by_player` keeps: the guest on
    /// connection `conn` (None once they left), else the player in place
    /// (None on a server, which has none of its own).
    /// The entity id of the player `killer_uuid` names for `conn`.
    pub(in crate::game) fn player_entity_id_by_connection(&self, conn: Option<u32>) -> Option<i32> {
        match conn {
            Some(conn) if self.server.lan_acting != Some(conn) => {
                Some(self.server.lan.as_ref()?.hub.guest(conn)?.entity_id)
            }
            Some(_) => Some(self.own_entity_id()),
            None if self.server.lan_acting.is_none() && self.common.player.is_some() => {
                Some(self.own_entity_id())
            }
            None => None,
        }
    }
    /// The game's own player's uuid and entity id — the identity behind
    /// [`Self::player_view`], so a tick that lends no guest's view names its
    /// player (#1077). None where the view stands for nobody: no player of
    /// the game's own, a fake one, or a guest's turn acting in its place.
    pub(in crate::game::server_side) fn player_identity(&self) -> Option<([i32; 4], u32)> {
        if self.server.lan_acting_fake
            || self.server.lan_acting_uuid.is_some()
            || self.common.player.is_none()
        {
            return None;
        }
        let uuid = self.common.level.player_uuid?;
        let id = u32::try_from(self.common.player.as_ref()?.entity_id).ok()?;
        Some((uuid, id))
    }
    pub fn player_view(&self) -> Option<PlayerView> {
        if self.server.lan_acting_fake {
            return None;
        }
        let p = self.common.player.as_ref()?;
        let w = f64::from(p.dimensions.width) / 2.0;
        let bounds = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        );
        Some(PlayerView {
            position: p.position,
            bounds,
            eye: [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height),
                p.position.z,
            ],
            alive: !p.health.dead_or_dying(),
            invulnerable: p.invulnerable
                || p.health.permanently_invulnerable
                || p.health.invulnerable_time > 0,
            // EntitySelector.NO_CREATIVE_OR_SPECTATOR.
            creative: self.common.creative || self.common.spectator(),
            sneaking: self.server_acting_sneak(),
            swimming: p.swimming,
            look: view_vector(self.common.yaw, self.common.pitch),
            main_hand: self.common.held().map(|s| s.item.clone()),
            off_hand: self.common.offhand.as_ref().map(|s| s.item.clone()),
            head: self.common.armor[0].as_ref().map(|s| s.item.clone()),
            chest: self.common.armor[1].as_ref().map(|s| s.item.clone()),
            legs: self.common.armor[2].as_ref().map(|s| s.item.clone()),
            feet: self.common.armor[3].as_ref().map(|s| s.item.clone()),
            armor_cover: self.common.armor.iter().filter(|a| a.is_some()).count() as f32 / 4.0,
            invisible: self.common.potions.effects.get("invisibility").is_some(),
            spectator: self.common.spectator(),
            gold_armor: self.wearing_gold(),
            hurt_by: self.player_hurt_by(),
            last_hurt: p.health.last_hurt_mob,
            velocity: p.velocity,
            yaw: self.common.yaw,
            sprinting: p.sprinting,
            riding: self.common.riding.is_some(),
            hero: self
                .common
                .potions
                .effects
                .get("hero_of_the_village")
                .is_some(),
            effects: self
                .common
                .potions
                .effects
                .list
                .iter()
                .map(|e| e.id)
                .collect(),
            uuid: self.acting_uuid(),
            allies: std::sync::Arc::new(self.player_allies()),
            remote: false,
            sleeping: self.common.sleeping.is_some(),
            sleep_timer: self.common.sleep_timer,
            passenger: self.common.riding.is_some(),
            riding_vehicle: self.common.riding,
            vehicle: self.common.riding.and_then(|id| {
                if let Some(b) = self.common.boats.iter().find(|b| b.id == id) {
                    // A boat's box: 1.375 wide and 0.5625 high around its position.
                    let w = 0.6875;
                    return Some(sunrise_sim::mobs::Vehicle {
                        nautilus: false,
                        bounds: Aabb::new(
                            Vec3::new(b.position.x - w, b.position.y, b.position.z - w),
                            Vec3::new(b.position.x + w, b.position.y + 0.5625, b.position.z + w),
                        ),
                        velocity: b.velocity,
                    });
                }
                let m = self.common.mobs.iter().find(|m| m.id == id)?;
                matches!(m.species(), Species::Nautilus | Species::ZombieNautilus).then(|| {
                    sunrise_sim::mobs::Vehicle {
                        nautilus: true,
                        bounds: m.bounds(),
                        velocity: m.velocity,
                    }
                })
            }),
        })
    }
    pub(in crate::game) fn projectile_hit_landed(
        &mut self,
        p: &Projectile,
        hit: Hit,
        deferred: Option<&mut Option<LandedPlayerArrow>>,
    ) -> Result<()> {
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, at, face } = hit
            && face != [0; 3]
        {
            self.projectile_block_impact(&ProjectileHitter::of(p), pos, at, face)?;
            self.projectile_breaks_block(p, pos);
        }
        self.server.combat.0.projectile_owner = match p.owner {
            Some(Who::Mob(id)) => Some(id),
            _ => None,
        };
        self.server.combat.0.projectile_direct = match hit {
            Hit::Entity {
                who: Who::Player, ..
            } => Some(self.target_name(super::commands::Target::Projectile(p.id))),
            _ => None,
        };
        let at = match hit {
            Hit::Entity { at, .. } | Hit::Block { at, .. } => at,
            Hit::None => return Ok(()),
        };
        // Projectile.onHitBlock: BlockState.onProjectileHit, for every
        // projectile (a wind charge out of the world's top has no face).
        if let Hit::Block { pos, face, .. } = hit
            && face != [0; 3]
        {
            self.target_block_hit(&ProjectileHitter::of(p), pos, at, face);
        }
        if let (ProjectileKind::Arrow { .. }, Hit::Entity { who, at }) = (&p.kind, hit) {
            return self.finish_arrow_step(p.clone(), who, at, deferred);
        }
        let wind = matches!(
            p.kind,
            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
        );
        // Projectile.hitTargetOrDeflectSelf: Breeze deflects every projectile except wind charges,
        // checked after ProjectileImpactEvent and guarded by lastDeflectedBy.
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && !wind
            && self
                .common
                .mobs
                .iter()
                .any(|m| m.id == id && m.alive() && m.species() == Species::Breeze)
        {
            let mut deflected = p.clone();
            if deflected.deflect_by(id, sunrise_sim::projectiles::Turn::Reverse, [1.0; 3]) {
                let sound = LevelSound::new(
                    "minecraft:entity.breeze.deflect",
                    [at.x, at.y, at.z],
                    1.0,
                    1.0,
                )
                .in_source(SoundSource::Hostile);
                self.play_bound_sound(sound, id, None);
            }
            deflected.turn_bullet();
            if deflected.arrow.is_some() {
                self.finish_arrow_hit(deflected, EntityHit::Deflected);
            } else {
                let (pid, from, to) = (deflected.id, deflected.previous, deflected.position);
                self.common.reinsert_continuing_projectile(deflected);
                self.entity_section_step(pid, from.components(), to.components());
            }
            return Ok(());
        }
        // Projectile.onHit: redirect any `#redirectable_projectile` before family onHitEntity.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
        {
            let momentum =
                matches!(p.kind, ProjectileKind::ShulkerBullet { .. }).then_some(p.velocity);
            self.redirect_projectile(id, p.owner, p.owner_uuid, p.guest, momentum);
        }
        if let Hit::Entity {
            who: Who::Mob(id),
            at,
        } = hit
            && self.lan_projectile_hit(p, id, at)
        {
            return Ok(());
        }
        if wind {
            return self.wind_charge_hit(p, hit);
        }
        let atp = [at.x, at.y, at.z];
        // A wither skull's hit on a part or crystal goes through `wither_skull_hit` below.
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
            && self.is_special_target(id)
            && !matches!(
                p.kind,
                ProjectileKind::WitherSkull { .. } | ProjectileKind::DragonFireball
            )
        {
            let resolved_owner = self.resolved_projectile_owner(p.owner, p.guest);
            let damage_type = self.projectile_damage_type(p);
            if matches!(p.kind, ProjectileKind::LlamaSpit) && resolved_owner.is_none() {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                return Ok(());
            }
            if p.arrow.is_some() {
                let mut trident = p.clone();
                let victim = self.entity_kind_for_who(Who::Mob(id));
                let amount = self.shot_damage(&mut trident, victim.as_deref(), Who::Mob(id));
                let _save_lease = self
                    .common
                    .is_crystal_id(id)
                    .then(|| self.common.creatures.projectiles.visibility_lease());
                let outcome = self.special_arrow_hit(&mut trident, id, amount);
                self.finish_arrow_hit(trident, outcome);
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                return Ok(());
            }
            let amount = match p.kind {
                ProjectileKind::SmallFireball => 5.0,
                ProjectileKind::LargeFireball { .. } => 6.0,
                ProjectileKind::ShulkerBullet { .. } => 4.0,
                ProjectileKind::LlamaSpit => 1.0,
                ProjectileKind::Thrown(t) => t.damage,
                _ => 0.0,
            };
            let kind = match p.kind {
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => {
                    sunrise_sim::mobs::DamageKind::Fireball
                }
                _ => sunrise_sim::mobs::DamageKind::Thrown,
            };
            let hit_special = |game: &mut Self| {
                game.hit_special_full(
                    id,
                    amount,
                    resolved_owner,
                    p.guest,
                    kind,
                    Some(damage_type),
                    Some(super::dragon::DirectHit::from_projectile(p)),
                );
            };
            if id >= SPECIAL
                && mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath)
                && self.server.combat.0.attack().projectile != Some(p.id as i32)
            {
                let attack = self.projectile_attack(p);
                self.with_attack(attack, hit_special);
            } else {
                hit_special(self);
            }
            if !matches!(
                p.kind,
                ProjectileKind::EnderPearl
                    | ProjectileKind::Egg
                    | ProjectileKind::Snowball
                    | ProjectileKind::Thrown(_)
                    | ProjectileKind::LargeFireball { .. }
            ) {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                if matches!(p.kind, ProjectileKind::ShulkerBullet { .. }) {
                    // ShulkerBullet.onHit calls destroy(), which posts ENTITY_DAMAGE at its position.
                    self.entity_event_by(
                        "minecraft:entity_damage",
                        [p.position.x, p.position.y, p.position.z],
                        Some(super::commands::Target::Projectile(p.id)),
                    );
                }
                return Ok(());
            }
        }
        let source_kind = self.common.projectile_damage_kind(p);
        let attacker_species = match p.owner {
            Some(Who::Mob(id)) => self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species()),
            _ => None,
        };
        let source = Damage {
            kind: source_kind,
            attacker: p.owner,
            from: Some([p.previous.x, p.previous.y, p.previous.z]),
            attacker_species,
            direct: Some(p.entity_type()),
        };
        let aim = self.attacker_aim(source.attacker);
        match (&p.kind, hit) {
            // ThrownTrident.onHitEntity: 8 damage (Impaling on aquatic mobs), then bounce.
            (ProjectileKind::Trident, Hit::Entity { who, .. }) => {
                let mut trident = p.clone();
                let victim = self.entity_kind_for_who(who);
                let amount = self.shot_damage(&mut trident, victim.as_deref(), who);
                let mut living_hit = false;
                let mut dodged = false;
                let hurt = match who {
                    Who::Player => {
                        living_hit = true;
                        self.mob_hits_player(
                            amount,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:trident",
                        )
                    }
                    Who::Mob(id) => {
                        let was_stand = self.common.armor_stands.iter().any(|a| a.id == id);
                        let info = ProjectileHitInfo::from_projectile(
                            &trident,
                            self.projectile_damage_type(p),
                            amount,
                            None,
                        );
                        match self.projectile_hurts_non_mob(&info, id) {
                            NonMobHit::Hurt => {
                                living_hit = was_stand;
                                true
                            }
                            NonMobHit::Refused => false,
                            NonMobHit::NotObject => {
                                living_hit = true;
                                let hurt =
                                    self.damage_projectile_with_weapon(p, id, amount, source, aim);
                                dodged = !hurt
                                    && self
                                        .common
                                        .mobs
                                        .iter()
                                        .find(|m| m.id == id)
                                        .is_some_and(|m| m.species() == Species::Enderman);
                                hurt
                            }
                        }
                    }
                };
                let outcome = if hurt {
                    if living_hit {
                        self.shot_post_attack(p, who, "minecraft:trident");
                    }
                    EntityHit::Hurt
                } else {
                    arrow_miss(dodged)
                };
                self.burning_arrow_primes(p, who);
                self.finish_arrow_hit(trident, outcome);
                self.settle_hurt_teleports()?;
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
            (ProjectileKind::Trident | ProjectileKind::Arrow { .. }, Hit::Block { pos, .. }) => {
                let weapon = p.arrow.as_ref().and_then(|arrow| {
                    if matches!(p.kind, ProjectileKind::Trident) {
                        Some(&arrow.pickup_item)
                    } else {
                        arrow.fired_from_weapon.as_ref()
                    }
                });
                if let Some(weapon) = weapon {
                    let holder = match p.owner {
                        Some(Who::Mob(id)) => Some(id),
                        Some(Who::Player) => self
                            .player_entity_id_by_connection(p.guest)
                            .map(|id| id as u32),
                        None => None,
                    };
                    self.hit_block_enchantment_effects(
                        weapon,
                        holder,
                        p.id,
                        p.entity_type(),
                        at,
                        pos,
                    );
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
            (ProjectileKind::Snowball, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && let Some(i) = self.common.mob_index.find(&self.common.mobs, id)
                {
                    // Snowball.onHitEntity: 3 damage to blazes, 0 otherwise.
                    let amount = if self.common.mobs[i].species() == Species::Blaze {
                        3.0
                    } else {
                        0.0
                    };
                    if self.damage_projectile_with_weapon(p, id, amount, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:snowball", amount);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    let info = ProjectileHitInfo::from_projectile(
                        p,
                        self.projectile_damage_type(p),
                        0.0,
                        None,
                    );
                    let _ = self.projectile_hurts_non_mob(&info, id);
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.thrown_break_event(p.id, atp);
            }
            // a mod's copy of Snowball: its damage to anything it hits.
            (ProjectileKind::Thrown(t), hit) => {
                match hit {
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => {
                        if self.common.mob_index.find(&self.common.mobs, id).is_some() {
                            // LivingEntity.hurtServer: PLAYER_HURT_ENTITY.
                            if self.damage_projectile_with_weapon(p, id, t.damage, source, aim)
                                && p.owner == Some(Who::Player)
                                && let Some(m) = self.common.mobs.iter().find(|m| m.id == id)
                            {
                                let victim = self.mob_entity(m);
                                let direct = sunrise_sim::advancements::EntityView {
                                    kind: t.id.into(),
                                    position: [p.position.x, p.position.y, p.position.z],
                                    ..Default::default()
                                };
                                self.player_hit(
                                    &victim,
                                    "minecraft:thrown",
                                    Some(direct),
                                    t.damage,
                                );
                            }
                        } else {
                            let info = ProjectileHitInfo::from_projectile(
                                p,
                                self.projectile_damage_type(p),
                                t.damage,
                                None,
                            );
                            let _ = self.projectile_hurts_non_mob(&info, id);
                        }
                    }
                    // Player.hurtServer observes invulnerability before refusing 0 damage.
                    Hit::Entity {
                        who: Who::Player, ..
                    } => {
                        let from = [p.previous.x, p.previous.y, p.previous.z];
                        self.mob_hits_player(t.damage, from, "minecraft:thrown");
                    }
                    _ => {}
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.thrown_break_event(p.id, atp);
            }
            (ProjectileKind::Egg, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && self.common.mob_index.find(&self.common.mobs, id).is_some()
                {
                    if self.damage_projectile_with_weapon(p, id, 0.0, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:egg", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    let info = ProjectileHitInfo::from_projectile(
                        p,
                        self.projectile_damage_type(p),
                        0.0,
                        None,
                    );
                    let _ = self.projectile_hurts_non_mob(&info, id);
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                // ThrownEgg.onHit: 1/8 chance of a chick, 1/32 of four.
                if self.common.world.random.next_int_bound(8).unwrap_or(1) == 0 {
                    let n = if self.common.world.random.next_int_bound(32).unwrap_or(1) == 0 {
                        4
                    } else {
                        1
                    };
                    let chicken_variant = p
                        .egg_variant
                        .as_deref()
                        .and_then(|id| self.world_variants().chicken.lookup(id))
                        .or_else(|| {
                            self.world_variants()
                                .chicken
                                .default_or_any("minecraft:temperate")
                        });
                    for _ in 0..n {
                        if let Some(id) = self.spawn_mob_raw("minecraft:chicken", atp, true)
                            && let Some(c) = self.common.mobs.iter_mut().find(|m| m.id == id)
                        {
                            c.extra.age = -24000;
                            c.extra.chicken_variant = chicken_variant;
                        }
                    }
                }
                self.thrown_break_event(p.id, atp);
            }
            (ProjectileKind::EnderPearl, hit) => {
                if let Hit::Entity {
                    who: Who::Player, ..
                } = hit
                {
                    self.mob_hits_player(
                        0.0,
                        [p.previous.x, p.previous.y, p.previous.z],
                        "minecraft:thrown",
                    );
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                    && self.common.mob_index.find(&self.common.mobs, id).is_some()
                {
                    if self.damage_projectile_with_weapon(p, id, 0.0, source, aim) {
                        self.thrown_hurt_mob(p, id, "minecraft:ender_pearl", 0.0);
                    }
                } else if let Hit::Entity {
                    who: Who::Mob(id), ..
                } = hit
                {
                    let info = ProjectileHitInfo::from_projectile(
                        p,
                        self.projectile_damage_type(p),
                        0.0,
                        None,
                    );
                    let _ = self.projectile_hurts_non_mob(&info, id);
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                let mut to = p.previous;
                let mut damage = 5.0;
                let mut cancel = false;
                if p.owner == Some(Who::Player)
                    && let Some(from) = self.common.player.as_ref().map(|pl| pl.position)
                    && let Some(owner) = self.native_acting_player().map(|player| player.entity())
                {
                    let dimension = self.common.dim;
                    let target;
                    (target, damage, cancel) =
                        self.native_pearl_teleport(owner, p.id, from, to, damage, hit);
                    cancel |= self.common.dim != dimension
                        || self
                            .common
                            .player
                            .as_ref()
                            .is_none_or(|pl| pl.position != from)
                        || self.native_acting_player().map(|player| player.entity()) != Some(owner);
                    if target.iter().all(|v| v.is_finite()) {
                        to = Vec3::new(target[0], target[1], target[2]);
                    }
                }
                let taken = self.common.player.as_ref().is_some_and(|pl| {
                    sunrise_sim::projectiles::owner_can_take_pearl(
                        !pl.health.dead_or_dying(),
                        self.common.sleeping.is_some(),
                    )
                });
                if p.owner == Some(Who::Player) && taken && !cancel {
                    if self.common.world.random.next_float() < 0.05
                        && self
                            .common
                            .ui
                            .rules
                            .bool_at(sunrise_game_data::gamerules::vanilla::SPAWN_MONSTERS)
                        && self.common.ui.difficulty != 0
                        && let Some(pl) = self.common.player.as_ref()
                    {
                        let from = pl.position;
                        self.spawn_mob_raw("minecraft:endermite", [from.x, from.y, from.z], false);
                    }
                    section_steps::teleport_player(self, to);
                    self.hurt_player_by("minecraft:ender_pearl", damage);
                    self.server_play_sound(
                        "minecraft:entity.player.teleport",
                        [to.x, to.y, to.z],
                        1.0,
                        1.0,
                    );
                }
                if let Some(Who::Mob(id)) = p.owner {
                    self.mob_pearl_teleport(p, id, hit);
                }
            }
            (ProjectileKind::SmallFireball, hit) => {
                match hit {
                    Hit::Entity { who, .. } => match who {
                        Who::Player => {
                            // SmallFireball.onHitEntity: ignite and restore on refusal.
                            let remaining = self.common.player.as_ref().map(|p| p.fire_ticks);
                            if let Some(p) = self.common.player.as_mut() {
                                p.ignite_for_ticks(100)?;
                            }
                            let hurt = self.mob_hits_player(
                                5.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                self.projectile_damage_type(p),
                            );
                            if hurt {
                                self.shot_post_attack(p, who, "minecraft:fireball");
                            } else if let Some(remaining) = remaining
                                && let Some(pl) = self.common.player.as_mut()
                            {
                                pl.set_remaining_fire_ticks(remaining);
                            }
                        }
                        Who::Mob(id) => {
                            if let Some(i) = self.common.mob_index.find(&self.common.mobs, id) {
                                let m = &mut self.common.mobs[i];
                                let (join, remaining) = (m.join, m.fire_ticks);
                                if !m.fire_immune() {
                                    m.ignite_for_ticks(100);
                                }
                                if self.damage_projectile_with_weapon(p, id, 5.0, source, aim) {
                                    self.shot_post_attack(p, who, "minecraft:fireball");
                                } else if let Some(i) =
                                    self.common.mob_index.find(&self.common.mobs, id)
                                    && self.common.mobs[i].join == join
                                    && !self.common.mobs[i].removed
                                {
                                    self.common.mobs[i].fire_ticks = remaining;
                                }
                            } else {
                                let info = ProjectileHitInfo::from_projectile(
                                    p,
                                    self.projectile_damage_type(p),
                                    5.0,
                                    Some(5.0),
                                );
                                let _ = self.projectile_hurts_non_mob(&info, id);
                            }
                        }
                    },
                    Hit::Block { pos, face, .. } => {
                        // SmallFireball.onHitBlock: ignite the empty adjacent block.
                        let fire = [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]];
                        let may = match p.owner {
                            Some(Who::Mob(owner)) => self.can_entity_grief(Some(owner)),
                            _ => true,
                        };
                        if may
                            && self
                                .common
                                .res
                                .facts()
                                .is_air(self.common.world.block(fire[0], fire[1], fire[2]))
                        {
                            let at = BlockPos::new(fire[0], fire[1], fire[2]);
                            let state = self
                                .with_level(|b, level, ticks| b.fire_state_for(level, ticks, at));
                            self.level_set(fire, state)?;
                        }
                    }
                    Hit::None => {}
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
            (ProjectileKind::LargeFireball { power }, hit) => {
                let owner = match p.owner {
                    Some(Who::Mob(id)) => Some(id),
                    _ => None,
                };
                if let Hit::Entity { who, .. } = hit {
                    let dmg_type = self.projectile_damage_type(p);
                    match who {
                        Who::Player => {
                            self.mob_hits_player(
                                6.0,
                                [p.previous.x, p.previous.y, p.previous.z],
                                dmg_type,
                            );
                            self.shot_post_attack(p, who, "minecraft:fireball");
                        }
                        Who::Mob(id) => {
                            if self.common.mob_index.find(&self.common.mobs, id).is_some() {
                                self.damage_projectile_with_weapon(p, id, 6.0, source, aim);
                                self.shot_post_attack(p, who, "minecraft:fireball");
                            } else {
                                let info =
                                    ProjectileHitInfo::from_projectile(p, dmg_type, 6.0, None);
                                let _ = self.projectile_hurts_non_mob(&info, id);
                                // LargeFireball.onHitEntity runs doPostAttackEffects unconditionally.
                                self.shot_post_attack(p, who, "minecraft:fireball");
                            }
                        }
                    }
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.server.combat.0.explosion_owner = owner;
                self.server.combat.0.explosion_in_water = p.touching_water;
                self.set_blast_by_player(p.owner == Some(Who::Player));
                let fire = self.large_fireball_fire(p.owner, p.guest);
                self.server.combat.0.explosion_direct = Some((p.id, p.entity_type()));
                let result = self.explode_mob(
                    atp,
                    *power as f32,
                    fire,
                    super::explosion::ExplosionSource::default()
                        .with_silence(self.entity_silent(p.id))
                        .with_entity(
                            p.id,
                            "minecraft:fireball",
                            sunrise_sim::health::DirectEntity::LargeFireball,
                        ),
                );
                self.server.combat.0.explosion_direct = None;
                self.set_blast_by_player(false);
                self.server.combat.0.explosion_owner = None;
                self.server.combat.0.explosion_in_water = false;
                result?;
            }
            (kind @ (ProjectileKind::Potion(..) | ProjectileKind::ExperienceBottle), hit) => {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                let bottle;
                let stack = match kind {
                    ProjectileKind::Potion(stack, _) => stack,
                    _ => {
                        bottle = Box::new(ItemStack::new("minecraft:experience_bottle", 1));
                        &bottle
                    }
                };
                let block = match hit {
                    Hit::Block { pos, face, .. } => Some((
                        pos,
                        Direction::approximate_nearest(Vec3::new(
                            f64::from(face[0]),
                            f64::from(face[1]),
                            f64::from(face[2]),
                        )),
                    )),
                    _ => None,
                };
                let mob = match hit {
                    Hit::Entity {
                        who: Who::Player, ..
                    } => self.common.player.as_ref().map(|pv| pv.position),
                    Hit::Entity {
                        who: Who::Mob(id), ..
                    } => self
                        .common
                        .mobs
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| m.position),
                    _ => None,
                };
                let owner = self.potion_owner(p.owner);
                let item = stack.item.to_string();
                self.splash(
                    stack,
                    at,
                    mob,
                    block,
                    p.age,
                    p.id,
                    &owner,
                    self.entity_silent(p.id),
                    p.velocity,
                );
                if stack.item.as_ref() != "minecraft:experience_bottle" {
                    self.native_potion_splash(&item, [at.x, at.y, at.z]);
                }
            }
            // LlamaSpit.onHitEntity hurts only when the resolved owner is a live LivingEntity.
            (ProjectileKind::LlamaSpit, Hit::Entity { .. })
                if self.resolved_projectile_owner(p.owner, p.guest).is_none() =>
            {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
            (ProjectileKind::LlamaSpit, Hit::Entity { who, .. }) => {
                match who {
                    Who::Player => {
                        if self.mob_hits_player(
                            1.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:spit",
                        ) {
                            self.shot_post_attack(p, who, "minecraft:spit");
                        }
                    }
                    Who::Mob(id) => {
                        if self.common.mob_index.find(&self.common.mobs, id).is_some() {
                            if self.damage_projectile_with_weapon(p, id, 1.0, source, aim) {
                                self.shot_post_attack(p, who, "minecraft:spit");
                            }
                        } else {
                            let info = ProjectileHitInfo::from_projectile(
                                p,
                                self.projectile_damage_type(p),
                                1.0,
                                None,
                            );
                            let _ = self.projectile_hurts_non_mob(&info, id);
                        }
                    }
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
            // ShulkerBullet.onHitEntity: 4 damage, then levitation for 10 s.
            (ProjectileKind::ShulkerBullet { .. }, Hit::Entity { who, .. }) => {
                let effect_source = match p.owner {
                    Some(Who::Mob(id)) => self.entity(id),
                    Some(Who::Player) => self
                        .player
                        .as_ref()
                        .and_then(|player| self.entity(player.entity_id as u32)),
                    None => None,
                }
                .or_else(|| self.entity(p.id));
                match who {
                    Who::Player => {
                        if self.mob_hits_player(
                            4.0,
                            [p.previous.x, p.previous.y, p.previous.z],
                            "minecraft:mob_projectile",
                        ) {
                            self.shot_post_attack(p, who, "minecraft:mob_projectile");
                            self.give_player_effect_from(
                                "minecraft:levitation",
                                200,
                                0,
                                effect_source,
                            );
                        }
                    }
                    Who::Mob(id) => {
                        if let Some(index) = self.common.mob_index.find(&self.common.mobs, id) {
                            let join = self.common.mobs[index].join;
                            let same_target = |m: &Mob| m.id == id && m.join == join && !m.removed;
                            if self.damage_projectile_with_weapon(p, id, 4.0, source, aim) {
                                if let Some(m) =
                                    self.common.mobs.iter_mut().find(|m| same_target(m))
                                {
                                    sunrise_sim::mobs::shulker::hit_by_bullet(m);
                                }
                                self.settle_hurt_teleports()?;
                                self.shot_post_attack(p, who, "minecraft:mob_projectile");
                                if self.mob_object(id).is_some_and(same_target) {
                                    self.add_mob_effect(
                                        id,
                                        sunrise_level::effects::EffectInstance::new(
                                            "levitation",
                                            200,
                                            0,
                                        ),
                                        effect_source,
                                    );
                                }
                            }
                        } else {
                            let info = ProjectileHitInfo::from_projectile(
                                p,
                                self.projectile_damage_type(p),
                                4.0,
                                None,
                            );
                            let _ = self.projectile_hurts_non_mob(&info, id);
                        }
                    }
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.entity_event_by(
                    "minecraft:entity_damage",
                    [p.position.x, p.position.y, p.position.z],
                    Some(super::commands::Target::Projectile(p.id)),
                );
            }
            (ProjectileKind::DragonFireball, _) => {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.dragon_fireball_on_hit(at);
            }
            (ProjectileKind::WitherSkull { .. }, hit) => {
                self.wither_skull_hit(p, hit, atp)?;
            }
            (ProjectileKind::ShulkerBullet { .. }, Hit::Block { .. }) => {
                self.server_play_sound("minecraft:entity.shulker_bullet.hit", atp, 1.0, 1.0);
                if let Some(ty) =
                    sunrise_level::particle_options::ParticleType::from_name("explosion")
                {
                    for _ in 0..2 {
                        let r = &mut self.common.world.random;
                        let at = [
                            atp[0] + r.next_gaussian() * 0.2,
                            atp[1] + r.next_gaussian() * 0.2,
                            atp[2] + r.next_gaussian() * 0.2,
                        ];
                        self.server_particle_at(
                            sunrise_sim::lan::server_effects::ServerParticle::simple(ty),
                            at,
                            [0.0; 3],
                        );
                    }
                }
                self.projectile_hit_landed_event(p.id, p.owner, hit);
                self.entity_event_by(
                    "minecraft:entity_damage",
                    [p.position.x, p.position.y, p.position.z],
                    Some(super::commands::Target::Projectile(p.id)),
                );
            }
            _ => {
                self.projectile_hit_landed_event(p.id, p.owner, hit);
            }
        }
        Ok(())
    }
    /// Dispatch with the projectile's actual owner context, including a guest shooter.
    pub(in crate::game) fn projectile_shoot_event(
        &mut self,
        owner: Option<Who>,
        guest: Option<u32>,
        at: [f64; 3],
    ) {
        if let Some(conn) = guest.filter(|conn| self.server.lan_acting != Some(*conn)) {
            let dispatched = self
                .as_guest(conn, |g| {
                    g.projectile_event("minecraft:projectile_shoot", owner, at);
                })
                .is_some();
            // An owner who left cannot resolve to the host. The event
            // still occurs, with no resolved owner (Projectile.getOwner).
            if !dispatched {
                self.projectile_event("minecraft:projectile_shoot", None, at);
            }
        } else {
            self.projectile_event("minecraft:projectile_shoot", owner, at);
        }
    }
    /// Remember the acting player for a mob's `lastHurtByPlayer`.
    pub fn remember_mob_player_credit(&mut self, victim: u32) {
        if let Some(uuid) = self.player_uuid_of(self.server.lan_acting) {
            self.server.last_hurt_by_player.insert(
                victim,
                PlayerCredit(sunrise_sim::lan::playerdata::ints_uuid(uuid)),
            );
        }
    }
    /// Resolve `EntityReference.getEntity` across levels. None inside means
    /// the player in place; Some is a guest connection.
    pub fn resolve_player_credit(&self, credit: PlayerCredit) -> Option<Option<u32>> {
        crate::game::server_side::entity_server::player_resolver!(self)
            .resolve(credit.0)
            .map(|guest| guest.or(self.server.lan_acting))
    }
    /// `LivingEntity.resolvePlayerResponsibleForDamage` after `source` hurt a mob, through the
    /// hit's credit loan (a tame wolf's hit is its owner's; an ownerless tame wolf clears it):
    /// what an accepted hit's site commits, for tests that set up the credit without a hit.
    #[cfg(any(test, feature = "test-support"))]
    pub fn resolve_player_responsible(&mut self, victim: u32, source: Damage) {
        if !matches!(source.attacker, Some(Who::Mob(_))) {
            return;
        }
        let plan = self.prepare_knockback_credit(
            victim as i32,
            source.attacker,
            self.server.combat.0.attack(),
        );
        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
        if let Some(mob) = self.common.mobs.iter_mut().find(|mob| mob.id == victim) {
            sunrise_sim::health::KnockbackCredit::accepted_mob_hit(&mut credit, mob);
        }
    }
    /// `AgeableMob.setAgeLocked`: a golden dandelion on mob `index` flips its
    /// age lock (a tadpole's or an ageable mob's own `setAgeLockedData`) and is
    /// used up; a locked mob stays (`setPersistenceRequired`), and the lock's
    /// use or unuse sound plays at its block for everyone near.
    pub(in crate::game) fn set_age_locked(&mut self, index: usize) {
        let m = &mut self.common.mobs[index];
        if m.species() == Species::Tadpole {
            sunrise_sim::mobs::aquatic::set_age_locked_data(m);
        } else {
            sunrise_sim::mobs::ai::set_age_locked_data(m);
        }
        self.consume_held();
        let m = &mut self.common.mobs[index];
        let locked = m.extra.age_locked;
        if locked {
            m.persistent = true;
        }
        // Level.playSound(null, blockPosition, ..): the middle of the block.
        let at = [
            m.position.x.floor() + 0.5,
            m.position.y.floor() + 0.5,
            m.position.z.floor() + 0.5,
        ];
        let chunk = (
            m.position.x.floor() as i32 >> 4,
            m.position.z.floor() as i32 >> 4,
        );
        self.common.world.modified.insert(chunk);
        let sound = if locked {
            "minecraft:item.golden_dandelion.use"
        } else {
            "minecraft:item.golden_dandelion.unuse"
        };
        self.server_play_sound(sound, at, 1.0, 1.0);
    }
    /// Throwables and spawn eggs (SnowballItem/EggItem/EnderpearlItem/SpawnEggItem.use).
    pub(in crate::game::server_side) fn throw_item(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        let kind = match item {
            "minecraft:snowball" => ProjectileKind::Snowball,
            "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => ProjectileKind::Egg,
            "minecraft:ender_pearl" => ProjectileKind::EnderPearl,
            "minecraft:wind_charge" => {
                let thrown = self.throw_wind_charge(stack)?;
                if thrown {
                    self.apply_use_cooldown(stack);
                }
                return Ok(thrown);
            }
            _ if item.ends_with("_spawn_egg") => return self.use_spawn_egg(stack),
            _ => match sunrise_sim::projectiles::thrown_by_item(item) {
                Some(t) => ProjectileKind::Thrown(t),
                None => return Ok(false),
            },
        };
        let Some(eye) = self.common.eye_position() else {
            return Ok(false);
        };
        let Some(p) = self.common.player.as_ref() else {
            return Ok(false);
        };
        // Projectile.shootFromRotation adds getKnownMovement: what the thrower last moved.
        let on_ground = p.ground.on_ground;
        let velocity = self.known_player_movement();
        let start = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let mut proj = Projectile::new(kind.clone(), start, Some(Who::Player));
        proj.set_egg_item(stack);
        proj.shoot_from_rotation(
            self.common.pitch,
            self.common.yaw,
            0.0,
            1.5,
            1.0,
            velocity,
            on_ground,
        );
        self.spawn_projectile(proj, stack);
        // SnowballItem.use / EggItem.use / EnderpearlItem.use: awardStat(ITEM_USED).
        self.award_stat("used", item, 1);
        // ItemStack.use's applyAfterUseComponentSideEffects: the pearl's
        // second is the `use_cooldown` component, as for any thrown item.
        self.apply_use_cooldown(stack);
        let sound = match kind {
            ProjectileKind::Snowball | ProjectileKind::Thrown(_) => {
                "minecraft:entity.snowball.throw"
            }
            ProjectileKind::Egg => "minecraft:entity.egg.throw",
            _ => "minecraft:entity.ender_pearl.throw",
        };
        let pitch = 0.4 / (self.common.world.random.next_float() * 0.4 + 0.8);
        self.server_play_sound(sound, eye, 0.5, pitch);
        self.swing_arm();
        self.consume_held();
        if kind == ProjectileKind::Egg
            && let (Some(player), Some(name)) = (
                self.server.lan_acting_uuid,
                self.server.lan_acting_name.clone(),
            )
        {
            self.native_player_egg_throw(player, &name);
        }
        Ok(true)
    }
    /// The entity phase of `tick_creatures`: every mob's tick, deaths, and the
    /// projectiles and effects that follow from them.
    pub fn tick_creature_entities(&mut self) -> Result<()> {
        // Its steps as spans for `sunrise-report` (nothing is read while spans are off).
        let mut laps = sunrise_level::spans::laps();
        let blocked = self.mobs_below_off_level();
        let level_id = self.common.dim.id();
        // `EntityEvent.Size` for the sizes that changed outside the mob ticks (spawns,
        // commands, a player's interaction), before the ticks move by them.
        self.native_mob_sizes();
        self.native_entity_sizes();
        // Hit-driven deaths already ran inside the hurt. Handle deaths from other
        // state changes before the mob ticks.
        let killed: Vec<Mob> = self
            .common
            .mobs
            .iter()
            .filter(|m| !m.alive() && !m.removed && !self.server.died.contains(&m.id))
            .cloned()
            .collect();
        for m in killed {
            self.server.died.insert(m.id);
            self.mob_die(&m);
        }
        // Their victims have been looted and mourned: no charged creeper is
        // waiting any more.
        // Resolve saved attacker UUIDs before the mobs run baseTick.
        self.resolve_last_hurt_by_refs();
        self.server.combat.0.charged_exploders.clear();
        self.server.combat.0.exploded.clear();
        self.server.combat.0.exploded_kinds.clear();
        let mut round = self.native_tick_round();
        let sleeping = mod_api::__private::listened(mod_api::HookId::CanContinueSleeping);
        if !round.pre() && sleeping {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(m) = self.common.mobs.get(i) {
                let (x, z) = (m.position.x.floor() as i32, m.position.z.floor() as i32);
                if !blocked.contains(&m.id) && self.common.world.loaded(x, z) && ticking(x, z) {
                    self.sync_sleeping_mob(i);
                }
                i += 1;
            }
        }
        self.common.init_sulfur_attribute_maps();
        let uuid = self.common.level.player_uuid;
        // LivingEntity.isPushable reads onClimbable live; a mob's cached one
        // is only refreshed by its own push step, so one that has not ticked
        // yet (the first tick after a load, a fresh spawn) or has moved since
        // would be pushed out of a ladder or scaffolding. Refreshed here, from
        // where each mob stands now, before the snapshot others push it by.
        let climbing: Vec<bool> = {
            let w = self.mob_world_base(&[]);
            self.common
                .mobs
                .iter()
                .map(|m| m.on_climbable(&w))
                .collect()
        };
        let inactive_equipment = if self.common.is_logical_server() {
            Some((
                self.entity_ticking_area(),
                world_items!(self),
                &self.common.motion.attributes,
            ))
        } else {
            None
        };
        for (m, climbing) in self.common.mobs.iter_mut().zip(climbing) {
            m.on_climbable = climbing;
            if let Some((ticking, items, catalog)) = &inactive_equipment
                && !m.removed
                && (!m.extra.equipment.is_empty() || m.extra.live.has_worn())
                && !ticking(m.position.x.floor() as i32, m.position.z.floor() as i32)
            {
                super::living_attributes::collect_equipment(catalog, items, m);
            }
        }
        self.flush_pending_mob_parts();
        let mut snapshot = std::mem::take(&mut self.common.creatures.seen_buf);
        snapshot.extend(self.common.mobs.iter().map(|m| seen(m, uuid, self.items())));
        let mut events = std::mem::take(&mut self.common.creatures.event_buf);
        events.clear();
        let passenger_objects = self.retained_passenger_snapshot(uuid);
        let mut pending = self
            .common
            .creatures
            .pending
            .remove(level_id)
            .unwrap_or_default();
        let primable = self.sulfur_cubes_adopted_explosive();
        let mut mobs = self.common.mobs.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        let mut dead = Vec::new();
        let mut died = std::mem::take(&mut self.server.died);
        let mut ticked = std::mem::take(&mut self.common.creatures.ticked);
        ticked.clear();
        // With guests on, each mob sees its nearest player (game/network/lan_mobs.rs).
        let mut guests = self.lan_guest_views();
        // What stands in a guest's slot while a mob holds that guest's view for
        // its tick (see `lent` below).
        let mut spare = guests.first().map(|g| g.1.clone());
        let mut bindings = match self.server.lan.as_mut() {
            Some(host) if !guests.is_empty() => std::mem::take(&mut host.mob_players),
            _ => Default::default(),
        };
        // The guests' entity ids by connection, for the shulker bullets fired
        // at the guest a mob sees.
        let mut guest_ids: Vec<(u32, u32)> = match self.server.lan.as_ref() {
            Some(host) if !guests.is_empty() => host
                .hub
                .guests
                .iter()
                .map(|g| (g.conn, g.entity_id as u32))
                .collect(),
            _ => Vec::new(),
        };
        let mut guest_events = Vec::new();
        let mut player_credit = std::mem::take(&mut self.server.last_hurt_by_player);
        // The block writes since the last tick: mobs re-path around them.
        let mut path_writes =
            std::mem::take(self.common.world.path_writes.get_or_insert_with(Vec::new));
        // The mobs whose bodies go this tick: (id, feet, width, height, split).
        // 🤖 Agent: the last field is the grown slime, magma cube or sulfur
        // cube the body leaves behind (`AbstractCubeMob.remove`), captured as
        // the body goes because it has left the list by the time it splits.
        let mut gone = Vec::new();
        let mut restocks = Vec::new();
        // The level's random and attachments, lent to the mobs (brain behaviours draw from
        // the random, and a hit's damage hooks get both) and taken back after their ticks.
        let lent = sunrise_sim::health::LentLevel::take(&mut self.common.world);
        let mut stand_result = Ok(());
        let (lent, dispatch_panic) = self.tick_borrowed_creatures(borrowed_tick::BorrowedMobTick {
            mobs: &mut mobs,
            snapshot: &mut snapshot,
            passenger_objects: &passenger_objects,
            blocked: &blocked,
            events: &mut events,
            restocks: &mut restocks,
            pushes: &mut pending.pushes,
            dead: &mut dead,
            died: &mut died,
            ticked: &mut ticked,
            guests: &mut guests,
            spare: &mut spare,
            bindings: &mut bindings,
            guest_ids: &mut guest_ids,
            guest_events: &mut guest_events,
            player_credit: &mut player_credit,
            path_writes: &mut path_writes,
            gone: &mut gone,
            lent,
            laps: &mut laps,
            uuid,
            level_id,
            round: &mut round,
            stand_result: &mut stand_result,
        });
        let aborted = dispatch_panic.is_some() || stand_result.is_err();
        if aborted {
            // The aborted pass never applies retention, including previously
            // finished despawn answers. Do not carry those into the retry.
            for m in &mut mobs {
                m.despawn = mod_api::Despawn::default();
            }
        }
        lent.give_back(&mut self.common.world);
        self.server.last_hurt_by_player = player_credit;
        if let Some(host) = self.server.lan.as_mut()
            && !guests.is_empty()
        {
            // A set: a scan of every mob for each binding is quadratic, and a
            // server's loaded mobs run into the thousands.
            let live: sunrise_level::fast_hash::FxHashSet<u32> =
                mobs.iter().map(|m| m.id).collect();
            bindings.retain(|id, _| live.contains(id));
            host.mob_players = bindings;
        }
        // The mobs added during the tick (into the emptied `self.mobs`) come
        // first, as `append` put them; with none, the ticked list goes back
        // as it is instead of being copied into a new one, every mob a tick.
        self.common
            .mobs
            .restore(mobs, sunrise_sim::join_seq::RestoreOrder::SpawnsFirst);
        if aborted {
            // These death callbacks have not run, and the borrowed block writes
            // must still invalidate paths on the next attempted pass.
            for mob in &dead {
                died.remove(&mob.id);
            }
            self.common
                .world
                .path_writes
                .get_or_insert_with(Vec::new)
                .extend(path_writes);
        }
        self.server.died = died;
        snapshot.clear();
        self.common.creatures.seen_buf = snapshot;
        self.common.creatures.ticked = ticked;
        // A work step's restock lands on the screen now, before any callback of
        // this tick can edit the offers (those edits are then the last write).
        for restock in restocks {
            self.villager_offers_open(restock);
        }
        // Completed brain actions are never fed back into the producer pass.
        if !events.is_empty() || !guest_events.is_empty() {
            pending.batches.push_back(MobOutputBatch {
                normal: NormalMobActions::new(std::mem::take(&mut events)),
                guests: std::mem::take(&mut guest_events),
            });
        }
        if !aborted {
            // The successful final sweep consumed all pushes for surviving mobs.
            pending.pushes = Default::default();
        }
        self.common.creatures.pending.insert(level_id, pending);
        self.common.creatures.event_buf = events;
        if let Some(payload) = dispatch_panic {
            self.emit_death_poofs(gone);
            std::panic::resume_unwind(payload);
        }
        stand_result?;
        self.tick_armor_stands_before(sunrise_sim::join_seq::JoinSeq::MAX)?;
        // ... and entity event 60 goes out as it is: the poof.
        // 🤖 Agent: the split follows the poof, as `LivingEntity.tickDeath`
        // broadcasts event 60 before it calls `remove`.
        self.emit_death_poofs(gone);
        laps.lap("creature_retain");
        self.sync_riders();
        self.tick_skeleton_traps();
        self.villager_item_sense();
        self.villager_pickups();
        laps.lap("creature_villagers");
        for m in dead {
            self.mob_die(&m);
        }
        self.deliver_pending_mob_actions(level_id)?;
        self.finish_creature_entities(&mut laps, &primable)
    }
    /// Advance every mob one tick, then apply their events.
    pub fn tick_creatures(&mut self) -> Result<()> {
        // Hurts dealt since the last creature tick (explosions, commands, the
        // systems after it) teleport an enderman before it ticks, not in its AI step.
        self.settle_hurt_teleports()?;
        self.server.attack_ticks = self.server.attack_ticks.wrapping_add(1);
        self.refresh_gear_knockback_resistance();
        self.tick_leads();
        // Vanilla's spawning and raids run before ChunkMap.tick.
        self.tick_spawners();
        let ticked =
            sunrise_level::spans::time("tick_creature_entities", || self.tick_creature_entities());
        self.dismount_underwater_mob_riders();
        ticked
    }

    /// Hands the tick's lifted projectiles back for a hook told about one of them, so its handles
    /// read the live rows, and takes them again. Whether they came back as they went: the same
    /// rows in the same order, unwritten (the list's write stamp), with no spawn of the tick put
    /// before them.
    fn projectiles_told(
        &mut self,
        projectiles: sunrise_sim::join_seq::JoinList<Projectile, false>,
        tell: impl FnOnce(&mut Self),
    ) -> (sunrise_sim::join_seq::JoinList<Projectile, false>, bool) {
        #[cfg(any(test, feature = "test-support"))]
        PROJECTILE_HANDBACKS.with(|count| count.set(count.get() + 1));
        let no_spawns = self.common.creatures.projectiles.is_empty();
        self.common.creatures.projectiles.restore(
            projectiles,
            sunrise_sim::join_seq::RestoreOrder::SpawnsFirst,
        );
        let restored = self.common.creatures.projectiles.stamp();
        tell(self);
        let same = no_spawns && self.common.creatures.projectiles.stamp() == restored;
        (self.common.creatures.projectiles.lift(), same)
    }
    pub fn tick_projectiles_now(&mut self) -> Result<()> {
        if self.common.creatures.projectiles.is_empty() {
            self.flush_lan_explosions();
            return self.flush_wind_bursts();
        }
        let ticking = self.entity_ticking_area();
        // `EntityTickList.forEach`: a projectile that joined during this
        // tick's entity pass (a mob's shot) first ticks on the next one; a
        // player's or a dispenser's, which join before the pass, tick now.
        let through = self.server.ticked_entities_through;
        let mut round = self.native_tick_round();
        let told = round.told_listening();
        let mut hear = self.native_hears_impacts();
        // ThrownEnderpearl.tick: a dead player's pearl is discarded in
        // place of its tick while ender_pearls_vanish_on_death is on.
        let vanish = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::ENDER_PEARLS_VANISH_ON_DEATH);
        if !hear && !told {
            return self.tick_projectiles_live(through, ticking, vanish);
        }
        let mut projectiles = self.common.creatures.projectiles.lift();
        let mut ticked = sunrise_level::fast_hash::FxHashSet::default();
        let mut swept = sunrise_level::fast_hash::FxHashSet::default();
        let mut hits = Vec::new();
        let mut arrow_events = Vec::new();
        let mut shoot_events = Vec::new();
        let mut bullet_hurts = Vec::new();
        let mut bubble_tops = Vec::new();
        let mut water_splashes = Vec::new();
        let mut snow_contacts = Vec::new();
        let mut section_steps = section_steps::begin(self);
        let mut keep_flags = vec![true; projectiles.len()];
        let mut winds = std::mem::take(&mut self.common.creatures.pickable_projectile_targets);
        let mut scratch = std::mem::take(&mut self.common.creatures.arrow_hit_scratch);
        let mut impacts: Vec<Impact> = Vec::new();
        let mut vehicle_winds = self.vehicle_winds();
        let mut hand_back = told.then(|| ProjectileHandBack::new(&projectiles));
        for slot in 0..projectiles.len() {
            let i = match hand_back.as_deref() {
                None => slot,
                Some(back) => match back.place(slot) {
                    Some(i) => i,
                    None => continue,
                },
            };
            if projectiles[i].id > through
                || hear && self.projectile_removal_pending(projectiles[i].id)
            {
                continue;
            }
            // ServerLevel.tick: losing an entity-ticking ticket must not
            // advance projectile flags or motion while its chunk unloads.
            let at = projectiles[i].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                projectiles.update(i, |p| p.previous = at);
                continue;
            }
            let i = match hand_back.as_deref_mut() {
                None => i,
                Some(back) => {
                    let (id, kind) = (projectiles[i].id, projectiles[i].entity_type());
                    if !round.pre() {
                        if round.post() {
                            back.told = Some((id, kind));
                        }
                        i
                    } else {
                        let mut ticks = true;
                        let same;
                        (projectiles, same) = self.projectiles_told(projectiles, |game| {
                            ticks = game.native_tick_pre_one(&mut round, id, kind);
                        });
                        hear = self.native_hears_impacts();
                        if !same {
                            back.follow(&projectiles, &mut keep_flags, &mut impacts);
                        }
                        match back.place(slot) {
                            Some(at) if ticks => {
                                if round.post() {
                                    back.told = Some((id, kind));
                                }
                                at
                            }
                            _ => continue,
                        }
                    }
                }
            };
            if round.canceled(projectiles[i].id) {
                continue;
            }
            ticked.insert(projectiles[i].id);
            let section_from = projectiles[i].position;
            let skip_wind = matches!(
                projectiles[i].kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            );
            winds.clear();
            for (j, q) in projectiles.iter().enumerate() {
                if j != i
                    && keep_flags[j]
                    && is_pickable_projectile(q)
                    && (!skip_wind
                        || !matches!(
                            q.kind,
                            ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                        ))
                {
                    winds.push(sunrise_sim::projectiles::TargetCandidate {
                        who: Who::Mob(q.id),
                        bounds: q.bounds(),
                        position: q.position,
                        order: sunrise_sim::projectiles::TargetOrder::entity(
                            q.position,
                            q.join.get(),
                        ),
                    });
                }
            }
            {
                let host_dead = self.player_view().is_some_and(|p| !p.alive);
                projectiles.update(i, |p| {
                    if vanish
                        && matches!(p.kind, ProjectileKind::EnderPearl)
                        && p.owner == Some(Who::Player)
                        && match p.guest {
                            Some(c) => self
                                .server
                                .lan
                                .as_ref()
                                .and_then(|h| h.hub.guest(c))
                                .is_some_and(|g| g.dead),
                            None => host_dead,
                        }
                    {
                        keep_flags[i] = false;
                        return;
                    }
                    // The projectile as it was, to fly on if a mod cancels its impact.
                    let before = (hear && !p.stuck).then(|| p.clone());
                    let wind_before = vehicle_winds.before(p);
                    let mob_shot = !matches!(p.owner, Some(Who::Player));
                    // A player's arrows, snowballs and eggs meet every player; a
                    // guest's flies from that guest (Projectile.getOwner), not
                    // from the host, until it has left them.
                    let pvp = !mob_shot && super::lan_pvp::meets_players(&p.kind);
                    let shooter = p.guest.filter(|_| !mob_shot).and_then(|c| {
                        self.server
                            .lan
                            .as_ref()
                            .and_then(|h| h.hub.guest(c))
                            .map(|g| Who::Mob(g.entity_id as u32))
                    });
                    if shooter.is_some() {
                        p.owner = shooter;
                    }
                    let use_guests = mob_shot || pvp;
                    let query = TargetQuery::for_projectile(p, use_guests);
                    let live = self.live_proj_world(query, &winds);
                    let (keep, hit) = if p.arrow.is_some() {
                        let rain = self.arrow_rain();
                        let scene = self.arrow_scene(&rain);
                        sunrise_sim::entities::arrows::tick_to_block_contact_live(
                            p,
                            &scene,
                            &live,
                            hear,
                            &mut scratch,
                        )
                    } else {
                        p.tick_before_hit(&live)
                    };
                    if (p.is_thrown() && !matches!(p.kind, ProjectileKind::LlamaSpit))
                        || p.is_arrow() && p.arrow.as_ref().is_none_or(|a| !a.no_physics)
                    {
                        for block in sunrise_sim::powder_snow::contacts(
                            p.previous,
                            p.position,
                            p.bounds(),
                            false,
                            |at| live.w.block_name(at.x, at.y, at.z) == "minecraft:powder_snow",
                        ) {
                            snow_contacts.push((p.id, block, p.position, p.velocity));
                        }
                    }
                    if shooter.is_some() {
                        p.owner = Some(Who::Player);
                    }
                    if let Some(at) = p.take_shoot_event() {
                        shoot_events.push((p.owner, p.guest, [at.x, at.y, at.z]));
                    }
                    water_splashes.append(&mut p.water_splashes);
                    if let Some((hurt_at, is_lava)) = p.take_bullet_hurt() {
                        swept.insert(p.id);
                        let silent = self.entity_silent(p.id);
                        let ctx = sunrise_level::sculk::Context {
                            owner: p.owner,
                            owner_uuid: p.owner_uuid,
                            ..self.entity_event_context(p.id, None)
                        };
                        bullet_hurts.push((hurt_at.components(), is_lava, silent, ctx));
                    }
                    // ThrownEnderpearl.onAboveBubbleColumn / onInsideBubbleColumn.
                    if keep && hit == Hit::None && matches!(p.kind, ProjectileKind::EnderPearl) {
                        let (at, h) = (p.position, 0.125);
                        sunrise_sim::motion::bubble_column_push_over(
                            live.w.world,
                            live.w.registry,
                            live.w.facts,
                            live.w.collision,
                            Aabb::new(
                                Vec3::new(at.x - h, at.y, at.z - h),
                                Vec3::new(at.x + h, at.y + 2.0 * h, at.z + h),
                            ),
                            &mut p.velocity,
                            |pos| bubble_tops.push(pos),
                        );
                    }
                    let events_from = arrow_events.len();
                    let mut shot_after_hit = false;
                    if let Some(a) = p.arrow.as_mut() {
                        for event in a.events.drain(..) {
                            if event == sunrise_sim::entities::arrows::ArrowEvent::ShootEvent {
                                if matches!(hit, Hit::Block { .. }) {
                                    shot_after_hit = true;
                                } else {
                                    shoot_events.push((p.owner, p.guest, p.position.components()));
                                }
                            } else {
                                arrow_events.push((p.id, p.position, event));
                            }
                        }
                    }
                    let wind = matches!(
                        p.kind,
                        ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
                    );
                    // AbstractWindCharge.tick: out of the world's top it bursts.
                    if wind
                        && keep
                        && let Some(hit) = self.wind_ceiling_contact(p)
                    {
                        swept.insert(p.id);
                        hits.push(ProjectileHit {
                            projectile: p.clone(),
                            hit,
                            shot_after_hit: false,
                        });
                        keep_flags[i] = false;
                        return;
                    }
                    let wind_vehicle = vehicle_winds.met(wind_before, hit);
                    if hit != Hit::None {
                        if let Some(before) = before
                            && matches!(
                                hit,
                                Hit::Block { .. }
                                    | Hit::Entity {
                                        who: Who::Mob(_),
                                        ..
                                    }
                            )
                        {
                            impacts.push(Impact {
                                index: Some(i),
                                keep,
                                before,
                                hit,
                                target: self.native_impact_target(hit),
                                wind_vehicle,
                                hit_slot: hits.len(),
                                events: events_from..arrow_events.len(),
                            });
                        }
                        hits.push(ProjectileHit {
                            projectile: p.clone(),
                            hit,
                            shot_after_hit,
                        });
                    }
                    keep_flags[i] = keep;
                });
            }
            keep_flags[i] = keep_flags[i]
                && section_steps::record(
                    self,
                    &mut section_steps,
                    projectiles.get(i),
                    section_from,
                    keep_flags[i],
                );
            if let Some(back) = hand_back.as_deref_mut()
                && let Some((id, kind)) = back.told.take()
            {
                if ticked.contains(&id)
                    && !swept.contains(&id)
                    && let Some(pos) = projectiles.iter().position(|p| p.id == id)
                {
                    swept.insert(id);
                    let mut destroyed = false;
                    let mut swept_row = None;
                    projectiles.update_with_rest(pos, |shot, before, after| {
                        let sources = |src_id| {
                            before
                                .iter()
                                .chain(after)
                                .find(|p| {
                                    p.id == src_id && crate::game::entity_server::shot_alive(p)
                                })
                                .map(|p| {
                                    (
                                        crate::game::commands::Target::Projectile(src_id),
                                        p.position.components(),
                                    )
                                })
                        };
                        self.sweep_projectile_inside(shot, Some(&sources));
                        destroyed = shot.destroyed;
                        swept_row = Some(shot.clone());
                    });
                    if let Some(row) = swept_row {
                        for h in &mut hits {
                            if h.projectile.join == row.join {
                                h.projectile.clone_from(&row);
                            }
                        }
                    }
                    if destroyed {
                        keep_flags[pos] = false;
                    }
                }
                let same;
                (projectiles, same) = self.projectiles_told(projectiles, |game| {
                    game.native_tick_post_one(&round, id, kind);
                });
                hear = self.native_hears_impacts();
                if !same {
                    back.follow(&projectiles, &mut keep_flags, &mut impacts);
                }
            }
        }
        self.common.creatures.pickable_projectile_targets = winds;
        self.common.creatures.arrow_hit_scratch = scratch;
        for impact in &mut impacts {
            if let Some(i) = impact.index {
                impact.keep = keep_flags[i];
                if !projectiles[i].destroyed {
                    keep_flags[i] = true;
                }
            }
        }
        let mut flags = keep_flags.into_iter();
        projectiles.retain(|_| flags.next().unwrap());
        for i in 0..projectiles.len() {
            let id = projectiles[i].id;
            if !ticked.contains(&id)
                || swept.contains(&id)
                || hear && self.projectile_removal_pending(id)
            {
                continue;
            }
            swept.insert(id);
            projectiles.update_with_rest(i, |shot, before, after| {
                let sources = |src_id| {
                    before
                        .iter()
                        .chain(after)
                        .find(|p| p.id == src_id && crate::game::entity_server::shot_alive(p))
                        .map(|p| {
                            (
                                crate::game::commands::Target::Projectile(src_id),
                                p.position.components(),
                            )
                        })
                };
                self.sweep_projectile_inside(shot, Some(&sources));
            });
        }
        for h in &mut hits {
            let shot = &mut h.projectile;
            if hear && self.projectile_removal_pending(shot.id) {
                continue;
            }
            if swept.contains(&shot.id) {
                if let Some(row) = projectiles.iter().find(|p| p.join == shot.join) {
                    shot.clone_from(row);
                }
                continue;
            }
            if shot.arrow.is_some() {
                continue;
            }
            let sources = |src_id| {
                projectiles
                    .iter()
                    .find(|p| p.id == src_id && crate::game::entity_server::shot_alive(p))
                    .map(|p| {
                        (
                            crate::game::commands::Target::Projectile(src_id),
                            p.position.components(),
                        )
                    })
            };
            self.sweep_projectile_inside(shot, Some(&sources));
        }
        let mut destroyed_ids = sunrise_level::fast_hash::FxHashSet::default();
        for p in projectiles.iter() {
            if p.destroyed {
                destroyed_ids.insert(p.id);
            }
        }
        for h in &hits {
            if h.projectile.destroyed {
                destroyed_ids.insert(h.projectile.id);
            }
        }
        if !destroyed_ids.is_empty() {
            let mut hit_remap: Vec<Option<usize>> = Vec::with_capacity(hits.len());
            let mut next_slot = 0usize;
            hits.retain(|h| {
                if h.projectile.destroyed || destroyed_ids.contains(&h.projectile.id) {
                    hit_remap.push(None);
                    false
                } else {
                    hit_remap.push(Some(next_slot));
                    next_slot += 1;
                    true
                }
            });
            impacts.retain_mut(|impact| {
                if destroyed_ids.contains(&impact.before.id) {
                    return false;
                }
                let Some(slot) = hit_remap.get(impact.hit_slot).copied().flatten() else {
                    return false;
                };
                impact.hit_slot = slot;
                true
            });
            projectiles.retain(|p| !p.destroyed && !destroyed_ids.contains(&p.id));
        }
        self.common.creatures.bounced_followers.clear();
        for ProjectileHit {
            projectile: p, hit, ..
        } in &hits
        {
            if matches!(hit, Hit::Entity { .. })
                && let Some(i) = projectiles.iter().position(|q| q.id == p.id)
            {
                let followers = (i + 1..projectiles.len())
                    .filter(|&j| {
                        impacts
                            .iter()
                            .all(|imp| imp.before.join != projectiles[j].join || imp.keep)
                    })
                    .map(|j| projectiles[j].id)
                    .collect();
                self.common
                    .creatures
                    .bounced_followers
                    .push((p.id, followers));
            }
        }
        let hears_impact = !impacts.is_empty();
        let scope = self.storage_scope_guard(hears_impact);
        let game = &mut *scope.game;
        let pending = if hears_impact {
            game.native_settle_impacts(
                &mut projectiles,
                impacts,
                &mut hits,
                &mut arrow_events,
                &mut shoot_events,
            )
        } else {
            FxHashMap::default()
        };
        for (id, block, position, movement) in snow_contacts {
            if !destroyed_ids.contains(&id)
                && (!hears_impact || !game.projectile_removal_pending(id))
            {
                game.powder_snow_particle(block, position, movement);
            }
        }
        for (owner, guest, at) in shoot_events {
            game.with_projectile_dispatch(&mut projectiles, |game| {
                game.projectile_shoot_event(owner, guest, at);
            });
        }
        // ShulkerBullet.hurtServer: hurt sound (unless silent) and crits, discard, ENTITY_DAMAGE, then burn sound if lava.
        for (at, is_lava, silent, ctx) in bullet_hurts {
            game.shulker_bullet_hurt(at, silent);
            game.post_game_event("minecraft:entity_damage", at, ctx);
            if is_lava && !silent {
                game.lava_hurt_sound(at);
            }
        }
        for pos in bubble_tops {
            game.bubble_column_particles(pos);
        }
        for splash in water_splashes {
            game.server_apply_item_splash(splash);
        }
        for (id, at, event) in arrow_events {
            if !destroyed_ids.contains(&id)
                && (!hears_impact
                    || !projectile_impact_event(&event)
                    || !game.projectile_removal_pending(id))
            {
                game.server_arrow_event(at, event);
            }
        }
        game.common.creatures.projectiles.restore(
            projectiles,
            sunrise_sim::join_seq::RestoreOrder::SpawnsFirst,
        );
        section_steps::finish(game, section_steps);
        let result = game.apply_projectile_hits(hits, &mut vehicle_winds, pending, hears_impact);
        scope.close(result.is_ok());
        self.common.creatures.bounced_followers.clear();
        result?;
        self.flush_lan_explosions();
        self.flush_wind_bursts()
    }
    pub fn touch_arrows(&mut self) {
        use sunrise_sim::entities::arrows::Touch;
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        if p.health.dead_or_dying() || self.common.spectator() {
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let reach = Aabb::new(
            Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
            Vec3::new(
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ),
        )
        .inflate(Vec3::new(1.0, 0.5, 1.0));
        let creative = self.common.creative;
        let mut projectiles = self.common.creatures.projectiles.lift();
        #[cfg(any(test, feature = "test-support"))]
        crate::game::server_side::entity_server::taken_pass_tests::after_take(self);
        projectiles.retain(|shot| {
            let Some(a) = shot.arrow.as_ref().filter(|a| !a.removed) else {
                return true;
            };
            let at = shot.position;
            let bounds = Aabb::new(
                Vec3::new(at.x - 0.25, at.y, at.z - 0.25),
                Vec3::new(at.x + 0.25, at.y + 0.5, at.z + 0.25),
            );
            if !bounds.intersects(reach) {
                return true;
            }
            let taken = match a.player_touch(creative) {
                Touch::Ignore => false,
                Touch::Take => true,
                Touch::Give(stack) => self.add_stack_to_inventory(stack).is_none() || creative,
            };
            if taken {
                let r = &mut self.common.world.random;
                let pitch = (r.next_float() - r.next_float()) * 1.4 + 2.0;
                self.server_play_sound(
                    "minecraft:entity.item.pickup",
                    [at.x, at.y, at.z],
                    0.2,
                    pitch,
                );
            }
            !taken
        });
        self.common.creatures.projectiles.restore(
            projectiles,
            sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst,
        );
    }
    /// `SpawnEggItem.useOn` for a block, or `use` for a fluid source.
    pub(in crate::game::server_side) fn use_spawn_egg(
        &mut self,
        stack: &ItemStack,
    ) -> Result<bool> {
        let fluid = matches!(self.server.guest_use, Some(super::GuestUse::Item(_)));
        let Some(hit) = (if fluid {
            self.spawn_egg_fluid_hit()
        } else {
            self.pick()
        }) else {
            return Ok(false);
        };
        // ItemStack.useOn in adventure mode: an egg used on a block without a
        // matching `can_place_on` passes to `use`, whose fluid ray finds none.
        if !fluid && !self.may_use_on(hit.pos) {
            return Ok(false);
        }
        let items = self.common.res.shared_items();
        let Some(data) = sunrise_sim::block_entity_item::item_entity_data(stack, &items)
            .filter(|data| self.common.spawn_egg_allowed(&data.kind))
        else {
            self.server.use_failed = true;
            return Ok(true);
        };
        if fluid
            && !self.may_build()
            && !self.can_place_on_in_adventure([
                hit.pos[0] - hit.face[0],
                hit.pos[1] - hit.face[1],
                hit.pos[2] - hit.face[2],
            ])
        {
            self.server.use_failed = true;
            return Ok(true);
        }
        // SpawnEggItem.useOn on a spawner sets its mob (and an op's egg data).
        let spawner_data = self.acting_player_is_op().then_some(data.compound);
        if let Some(done) = self.use_spawn_egg_on_spawner(hit.pos, &data.kind, spawner_data) {
            if done {
                self.swing_arm();
                self.award_stat("used", &stack.item, 1);
                self.consume_held();
            }
            return Ok(true);
        }
        // useOn: a clicked block without a collision shape (vines, grass)
        // takes the mob itself, any other the one on the clicked face.
        let p = if fluid || self.collision_empty(hit.pos) {
            hit.pos
        } else {
            [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ]
        };
        let at = [
            f64::from(p[0]) + 0.5,
            f64::from(p[1]),
            f64::from(p[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob_spawned(
            &data.kind,
            at,
            SpawnSite::Finalizing(SpawnReason::SpawnItemUse),
        ) else {
            return Ok(false);
        };
        // SpawnEggItem.spawnMob: `EntityType.create` finalizes the mob, then
        // `PostSpawnProcessor` puts the egg's components on it.
        let variants = self.server.world_tables.0.as_ref().map_or_else(
            || self.common.res.variants(),
            |tables| tables.variants.as_ref(),
        );
        if let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            sunrise_sim::entity_nbt::apply_item_components(m, stack);
            sunrise_sim::entity_nbt::apply_item_variant_components(m, stack, variants);
        }
        self.apply_spawn_egg_entity_data(id, &data);
        self.post_block_game_event("minecraft:entity_place", p, Some(Who::Player), None);
        self.award_stat("used", &stack.item, 1);
        self.swing_arm();
        self.consume_held();
        Ok(true)
    }
    pub(in crate::game) fn finish_creature_entities(
        &mut self,
        laps: &mut sunrise_level::spans::Laps,
        primable: &[(u32, i32)],
    ) -> Result<()> {
        self.drain_entity_inside();
        laps.lap("creature_events");
        self.ordered_before(super::ordered_tick::Batch::Projectiles)?;
        self.tick_projectiles()?;
        self.drain_entity_inside();
        self.tick_fangs();
        laps.lap("creature_projectiles");
        self.tick_allays();
        self.tick_pandas();
        self.tick_foxes();
        self.tick_dolphins();
        self.tick_mob_looting();
        self.tick_sniffer_server();
        self.piglin_pickups();
        self.sulfur_cube_pickups();
        self.sulfur_cube_power_priming(primable);
        laps.lap("creature_systems");
        // Hurt and death sounds of damage dealt outside the mob ticks.
        let mut queued = Vec::new();
        for m in &mut self.common.mobs {
            queued.append(&mut m.outbox);
        }
        self.handle_mob_events(queued)?;
        // The enderman teleports those hurts asked for (`Enderman.hurtServer`).
        self.settle_hurt_teleports()?;
        laps.lap("creature_outbox");
        // And for the sizes the ticks changed (a pose, a baby grown, a scale).
        self.native_mob_sizes();
        self.native_entity_sizes();
        Ok(())
    }

    /// Server-side arrow events (broadcast sounds, TNT priming, block reactions, game events).
    pub(in crate::game) fn server_arrow_event(
        &mut self,
        at: Vec3,
        event: sunrise_sim::entities::arrows::ArrowEvent,
    ) {
        use sunrise_sim::entities::arrows::{ArrowEvent, BlockReaction};
        match event {
            ArrowEvent::Particle { .. } => {}
            ArrowEvent::PotionPuff { id, .. } => {
                self.lan_entity_event(id as i32, 0, [at.x, at.y, at.z]);
            }
            ArrowEvent::Sound {
                sound,
                volume,
                pitch,
            } => {
                self.server_play_sound_on(
                    6,
                    arrow_sound_name(sound),
                    [at.x, at.y, at.z],
                    volume,
                    pitch,
                );
            }
            ArrowEvent::BubbleColumnParticles { pos } => self.bubble_column_particles(pos),
            ArrowEvent::Block {
                pos,
                reaction,
                owner,
            } => match reaction {
                BlockReaction::PrimeTnt => {
                    if self.tnt_prime_allowed(pos) {
                        self.spawn_primed_tnt(pos, false, owner, None);
                        self.with_level(|b, level, ticks| b.remove_block(level, ticks, pos, false));
                        let _ = self.server_process_level_events();
                    }
                }
                BlockReaction::Light(lit) => {
                    let _ = self.level_set([pos.x, pos.y, pos.z], lit);
                }
            },
            ArrowEvent::SplashEvent => {
                self.projectile_event("minecraft:splash", None, [at.x, at.y, at.z]);
            }
            ArrowEvent::ShootEvent | ArrowEvent::Drop => {}
        }
    }

    /// The level as server arrows see it.
    pub fn arrow_scene<'s>(
        &'s self,
        rain: &'s dyn Fn(BlockPos) -> bool,
    ) -> sunrise_sim::entities::arrows::ArrowScene<'s, 's> {
        self.common.arrow_scene_with(
            rain,
            self.grief_hooked()
                .then_some(self as &dyn sunrise_sim::mobs::MobHooks),
            self,
        )
    }

    pub(in crate::game) fn server_shulker_lid_push(
        &mut self,
        shulker: u32,
        area: Aabb,
        delta: [f64; 3],
    ) {
        let lid = Lid::Shulker(shulker);
        let movement = Vec3::new(delta[0], delta[1], delta[2]);
        self.lid_push(lid, area, movement);
    }

    pub(in crate::game) fn server_handle_mob_events_reusable(
        &mut self,
        events: &mut Vec<MobEvent>,
    ) -> Result<()> {
        let mut actions = NormalMobActions::new(std::mem::take(events));
        let result = self.handle_mob_events_pending(&mut actions);
        *events = actions.root.events.into();
        result
    }

    /// `Entity.sendBubbleColumnParticles` at the top of the column block `pos`.
    pub(in crate::game) fn bubble_column_particles(&mut self, pos: BlockPos) {
        for _ in 0..2 {
            let r = &mut self.common.world.random;
            let splash = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            let bubble = [
                f64::from(pos.x) + r.next_double(),
                f64::from(pos.y + 1),
                f64::from(pos.z) + r.next_double(),
            ];
            self.server_level_particles(
                &sunrise_sim::lan::server_effects::ServerParticle::simple(ParticleType::Splash),
                splash,
                [0.0; 3],
                1.0,
                1,
            );
            self.server_level_particles(
                &sunrise_sim::lan::server_effects::ServerParticle::simple(ParticleType::Bubble),
                bubble,
                [0.0, 0.01, 0.0],
                0.2,
                1,
            );
        }
    }

    /// `MushroomCow.shear`'s explosion puff over a sheared mooshroom.
    pub(in crate::game) fn mooshroom_shear_puff(&mut self, at: [f64; 3], height: f64) {
        let at = [at[0], at[1] + 0.5 * height, at[2]];
        self.server_level_particles(
            &sunrise_sim::lan::server_effects::ServerParticle::simple(ParticleType::Explosion),
            at,
            [0.0; 3],
            0.0,
            1,
        );
    }

    /// `ThrownEnderpearl.onHit` for a living non-player owner.
    pub(in crate::game) fn mob_pearl_teleport(&mut self, pearl: &Projectile, id: u32, hit: Hit) {
        let Some(owner) = self.entity(id) else { return };
        let Some(from) = self
            .common
            .mobs
            .iter()
            .find(|m| {
                m.id == id && m.alive() && !m.removed && !sunrise_sim::mobs::villager::sleeping(m)
            })
            .map(|m| m.position)
        else {
            return;
        };
        let dimension = self.common.dim;
        let failures = mod_api::runtime::isolated_failures();
        let _: std::result::Result<(), ()> = self.storage_transaction(|game, _| {
            let (mut target, _, cancel) =
                game.native_pearl_teleport(owner, pearl.id, from, pearl.previous, 0.0, hit);
            if mod_api::runtime::isolated_failures() != failures {
                return Err(());
            }
            if cancel
                || game.dim != dimension
                || game.entity(id) != Some(owner)
                || !game.mobs.iter().any(|m| {
                    m.id == id
                        && m.alive()
                        && !m.removed
                        && m.position == from
                        && !sunrise_sim::mobs::villager::sleeping(m)
                })
            {
                return Ok(());
            }
            if !target.iter().all(|v| v.is_finite()) {
                target = [pearl.previous.x, pearl.previous.y, pearl.previous.z];
            }
            if !crate::game::native_entity_moves::in_bounds(target) {
                return Ok(());
            }
            let _ = game.place_teleport_filed(
                crate::game::commands::Target::Mob(id),
                target,
                None,
                [false; 3],
                true,
            );
            if let Some(m) = game.mobs.iter_mut().find(|m| m.id == id) {
                m.fall_distance = 0.0;
                m.previous_yaw = m.yaw;
                m.previous_head_pitch = m.head_pitch;
            }
            game.server_play_sound("minecraft:entity.player.teleport", target, 1.0, 1.0);
            Ok(())
        });
    }

    /// Create a mob of `kind` at a position, finalized like a natural spawn.
    pub fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
    ) -> Option<u32> {
        let mob = self.create_mob_reason(kind, at, persistent, site, None)?;
        self.insert_spawned_mob(mob)
    }
    /// [`spawn_mob`](Self::spawn_mob) for a creation that snaps the mob to a
    /// yaw (`snapTo(pos, yaw, 0)`): the facing takes no draw from the level.
    pub(in crate::game) fn spawn_mob_facing(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        site: SpawnSite,
        yaw: f32,
    ) -> Option<u32> {
        let mob = self.create_mob_reason(kind, at, persistent, site, Some(yaw))?;
        self.insert_spawned_mob(mob)
    }
    /// Create a mob of `kind` at a position without `finalizeSpawn`'s rolls,
    /// matching vanilla `EntityType.create(level, reason)`.
    pub fn spawn_mob_raw(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32> {
        self.spawn_mob(kind, at, persistent, SpawnSite::Raw)
    }
    /// `SummonCommand` and `SpawnEggItem`: vanilla spawns the mob, but it
    /// never calls `Mob.setPersistenceRequired` for it, so the mob is not
    /// persistent: the despawn rule holds it like any other mob, and only
    /// its NBT can make it persistent.
    pub fn spawn_mob_spawned(&mut self, kind: &str, at: [f64; 3], site: SpawnSite) -> Option<u32> {
        // Dragons have a separate entity implementation, but eggs use the same spawn path.
        if matches!(kind, "ender_dragon" | "minecraft:ender_dragon") {
            return self.spawn_dragon_spawned(at, site);
        }
        // The spawn is not a natural one (no jockeys, the patrol leader roll
        // stays), but the mob is not persistent: vanilla's SummonCommand
        // never calls setPersistenceRequired. The flag is false from the
        // start, before the mob joins the level, so a mod's own write in the
        // entity_join hook survives it - clearing it after the join would
        // wipe what the hook set (Mob.setPersistenceRequired from
        // entity_join_level).
        self.spawn_mob(kind, at, false, site)
    }
    /// [`spawn_mob_spawned`](Self::spawn_mob_spawned) for a mob that starts
    /// moving or turned: `velocity` (`Entity.setDeltaMovement`) and, when
    /// given, `[yaw, pitch]` in place of the level random's yaw, set before
    /// it joins. The kind resolves as `spawn_mob_spawned`'s (`cow` is
    /// `minecraft:cow`), and the ender dragon takes its own creation, facing
    /// the given yaw; it has no momentum or pitch of its own here, so a
    /// dragon given a velocity or a pitch is refused.
    pub(in crate::game) fn spawn_mob_spawned_moving(
        &mut self,
        kind: &str,
        at: [f64; 3],
        site: SpawnSite,
        velocity: Vec3,
        rotation: Option<[f32; 2]>,
    ) -> Option<u32> {
        if matches!(kind, "ender_dragon" | "minecraft:ender_dragon") {
            if velocity != Vec3::ZERO || rotation.is_some_and(|[_, pitch]| pitch != 0.0) {
                return None;
            }
            return self.spawn_dragon_facing(at, site, rotation.map(|[yaw, _]| yaw));
        }
        // Not persistent, as `spawn_mob_spawned`'s mob.
        let mut mob = self.create_mob_reason(kind, at, false, site, rotation.map(|r| r[0]))?;
        mob.velocity = velocity;
        if let Some([_, pitch]) = rotation {
            (mob.head_pitch, mob.previous_head_pitch) = (pitch, pitch);
        }
        self.insert_spawned_mob(mob)
    }

    pub(in crate::game) fn insert_spawned_mob(&mut self, mob: Mob) -> Option<u32> {
        let id = mob.id;
        self.common.mobs.push(mob);
        self.admit_new_mobs(self.common.mobs.len() - 1);
        if !self.common.mobs.iter().any(|m| m.id == id) {
            return None;
        }
        Some(id)
    }
    /// `Mob.burnUndead` for a mob in daylight with an item on its sun
    /// protection slot (the head, or the body for a zombie horse or
    /// nautilus): `hurtAndBreak(random.nextInt(2))` on a damageable item, and
    /// when it breaks the slot empties and `onEquippedItemBroken` broadcasts
    /// the break event.
    pub fn wear_sun_protection(&mut self, id: u32) {
        use sunrise_core::equipment_slot::EquipmentSlot;
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let slot = m.sun_protection_slot();
        let body = slot == EquipmentSlot::Body;
        let Some(stack) = m.item_by_slot(slot) else {
            return;
        };
        if !sunrise_sim::stations::is_damageable(self.common.res.items(), &stack) {
            return;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let amount = m.random.next_int_bound(2).unwrap_or(0) as u32;
        if amount == 0 {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, amount, id);
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(slot).as_ref() != Some(&stack) {
            return;
        }
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            // LivingEntity.entityEventForEquipmentBreak: head 49, body 65.
            self.lan_entity_event(id as i32, if body { 65 } else { 49 }, at);
        }
    }
    /// `LivingEntity.updateFallFlying`'s wear, `ItemStack.hurtAndBreak(1, this,
    /// slot)` on the glider `MobEvent::GliderWear` picked: Unbreaking (from the
    /// level's random, `Enchantment.modifyItemFilteredCount`) may save it, and
    /// the rest goes on the stack where it is worn. Only a part's `break_now`
    /// breaks one (`canGlideUsing` picks none a point would break): emptied, with its event.
    pub fn wear_mob_glider(&mut self, id: u32, slot: sunrise_core::equipment_slot::EquipmentSlot) {
        let Some(stack) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| sunrise_sim::mobs::finalize::equipment_stack(m, slot))
        else {
            return;
        };
        if !sunrise_sim::stations::is_damageable(self.common.res.items(), &stack) {
            return;
        }
        let (updated, broken) = self.damage_living_stack(&stack, 1, id);
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        m.set_equipment_stack(slot as usize, updated);
        if broken.is_some() {
            let event = sunrise_sim::mobs::item_break::equipment_break_event(slot);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// `LivingEntity.hurtArmor` -> `LivingEntity.doHurtEquipment`, from the hit
    /// `MobEvent::ArmorHurt` reported. Only `Horse` overrides `hurtArmor` for a
    /// mob, and its override asks for the BODY slot alone, so the body piece is
    /// the only one this wears: it loses `max(1, damage / 4)` durability,
    /// Unbreaking applied, and at its maximum the slot empties and the break
    /// event goes out (`LivingEntity.entityEventForEquipmentBreak`: 65 for the
    /// body — 49 to 52 are the four armour slots, which only a player's own
    /// pieces reach). A fire hit leaves a fire-resistant piece alone
    /// (`ItemStack.canBeHurtBy`).
    pub fn hurt_mob_armor(&mut self, id: u32, damage: f32, source: &HurtSource<'_>) {
        use sunrise_core::equipment_slot::EquipmentSlot;
        if damage <= 0.0 {
            return;
        }
        let Some(m) = self.common.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        let at = [m.position.x, m.position.y, m.position.z];
        let amount = ((damage / 4.0) as u32).max(1);
        let name = source.damage_type;
        // A piece's maximum, or none when it may not lose durability at all.
        // `doHurtEquipment` starts from the item's `equippable` component
        // (`Equippable equippable = item.get(DataComponents.EQUIPPABLE)`), so a
        // damageable item with no such component — a sword put in a horse's body
        // slot by `/item replace` — never wears; `equip_damage_on_hurt` alone
        // cannot say that, since it defaults to true when the component is absent.
        let wearable = |me: &Self, stack: &ItemStack, name: &str| -> Option<u32> {
            let info = me.res.items().get(&stack.item)?;
            if info.equip_slot.is_none()
                || !sunrise_sim::item_sync::damage_on_hurt(stack, info.equip_damage_on_hurt)
                || info.max_damage == 0
                || !sunrise_sim::stations::is_damageable(me.res.items(), stack)
                || !me.stack_can_be_hurt_by(stack, name).unwrap_or(true)
            {
                return None;
            }
            Some(info.max_damage)
        };
        let Some(body) = self
            .common
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
        else {
            return;
        };
        if wearable(self, &body, name).is_none() {
            return;
        }
        // ArmorHurtEvent for a mob's body armor.
        let mut amount = amount;
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let Ok(body_snapshot) = body.try_into_api()
        {
            let kind = self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(sunrise_gameplay_common::game::native_hooks::mob_kind)
                .unwrap_or_default();
            let mut entries = vec![mod_api::ArmorHurt {
                slot: EquipmentSlot::Body,
                stack: body_snapshot,
                damage: sunrise_gameplay_common::game::durability_damage(damage),
                new_damage: sunrise_gameplay_common::game::durability_damage(damage),
            }];
            let mut cancel = false;
            self.native_armor_hurt(None, id, &kind, source, &mut entries, &mut cancel);
            if cancel {
                return;
            }
            amount =
                sunrise_gameplay_common::game::answered_wear(&entries, EquipmentSlot::Body, amount);
            if amount == 0 {
                return;
            }
            // A mod may have swapped the piece: it is not worn by this hit.
            if self
                .common
                .mobs
                .iter()
                .find(|m| m.id == id)
                .and_then(|m| m.item_by_slot(EquipmentSlot::Body))
                != Some(body.clone())
            {
                return;
            }
        }
        let (updated, broken) = self.damage_living_stack(&body, amount, id);
        let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        if m.item_by_slot(EquipmentSlot::Body).as_ref() != Some(&body) {
            return;
        }
        m.set_equipment_stack(EquipmentSlot::Body as usize, updated);
        if broken.is_some() {
            // LivingEntity.onEquippedItemBroken through the level's broadcast.
            let event = sunrise_sim::mobs::item_break::equipment_break_event(EquipmentSlot::Body);
            self.lan_entity_event(id as i32, event, at);
        }
    }
    /// `Mob.convertTo(type, ConversionParams.single(...))`: a new mob of
    /// `kind`, made without spawn finalization unless its conversion asks, takes
    /// its place with what `conversion::single` carries over, and the old
    /// mob's riders move onto it. A zombie that comes out of it rolls
    /// `Zombie.handleAttributes(special, CONVERSION)` (the
    /// `postConversionStep` of its `ConversionTracker`). `pre_checked` skips
    /// `LivingConversionEvent.Pre`, which the lightning asks before it picks
    /// its outcome.
    pub fn convert_to(
        &mut self,
        from: &Mob,
        kind: &str,
        params: sunrise_sim::mobs::conversion::Params,
        pre_checked: bool,
        after: impl FnOnce(&mut Self, &mut Mob),
    ) -> Option<u32> {
        self.convert_to_moved(from, kind, params, pre_checked, None, after)
    }

    /// The `summon` command: spawn a mob in front of the player or at a position.
    pub(in crate::game) fn summon_command_mob(
        &mut self,
        kind: &str,
        at: Option<[f64; 3]>,
        baby: bool,
    ) -> Result<u32> {
        let at = match at {
            Some(a) => a,
            None => self.common.default_summon_position()?,
        };
        let id = self
            .spawn_mob_spawned(kind, at, SpawnSite::Finalizing(SpawnReason::Command))
            .ok_or_else(|| format!("unknown or unsupported mob {kind}"))?;
        if baby && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.age = if m.species() == Species::Sniffer {
                sunrise_sim::mobs::sniffer::BABY_START_AGE
            } else {
                -24000
            };
        }
        Ok(id)
    }

    /// What the generator finalizes this level's structure mobs with.
    pub(in crate::game) fn share_spawn_conditions(&self) {
        if let Some(terrain) = self.common.end.locator() {
            terrain.set_spawn_conditions(self.difficulty_id() as u8, self.common.world.day_time);
        }
    }
    /// Fires `FinalizeSpawnEvent`, lets listeners skip class initialization, and returns whether the mob may be admitted.
    pub fn finalize_with(
        &mut self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::Spawner>,
    ) -> bool {
        self.flush_unjoined_mob_parts(mob);
        let mut group = None;
        let (outcome, difficulty) = self.finalize_spawn_event(mob, reason, spawner, &mut group);
        if !outcome.skip_initialization {
            self.finalize_grouped(mob, &difficulty, reason, group);
        }
        !outcome.veto_admission
    }
    /// `Mob.finalizeSpawn`'s event, the half listeners own: they hear the
    /// spawn, cancel it, take it over, set its difficulty, edit the mob
    /// through its lend and change the pack's `group`, which finalization then
    /// reads. The decision and the difficulty the spawn finalizes with come
    /// back together.
    pub(in crate::game) fn finalize_spawn_event(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::Spawner>,
        group: &mut Option<Group>,
    ) -> (
        sunrise_sim::mobs::finalize::SpawnEvent,
        spawning::LocalDifficulty,
    ) {
        let at = [mob.position.x, mob.position.y, mob.position.z];
        let difficulty = self.common.local_difficulty(at);
        self.finalize_spawn_with_difficulty(mob, reason, spawner, difficulty, group)
    }
    pub(in crate::game) fn finalize_spawn_with_difficulty(
        &self,
        mob: &mut Mob,
        reason: SpawnReason,
        spawner: Option<mod_api::Spawner>,
        difficulty: spawning::LocalDifficulty,
        group: &mut Option<Group>,
    ) -> (
        sunrise_sim::mobs::finalize::SpawnEvent,
        spawning::LocalDifficulty,
    ) {
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Default::default(), difficulty);
        }
        let at = mob.position.components();
        let kind = mob.kind.id.clone();
        self.finalize_spawn_lent(&kind, at, reason, spawner, difficulty, Some(mob), group)
    }

    /// Query the composed cluster size after the mob joins.
    pub(in crate::game) fn spawn_cluster_size(
        &self,
        mob: mod_api::MobRef,
        kind: &str,
        vanilla: i32,
    ) -> i32 {
        if !mod_api::__private::has_scoped_hook(mod_api::HookId::SpawnClusterSize)
            || !mod_api::__private::entity_part_hooked(
                mod_api::HookId::SpawnClusterSize,
                "spawn_cluster_size",
                kind,
            )
        {
            return vanilla;
        }
        let mut size = vanilla;
        self.native_spawn_cluster_size(
            mod_api::__private::SpawnClusterSizeCall {
                mob,
                vanilla,
                size: &mut size,
            },
            kind,
        );
        size
    }
    /// The shared finalization decision for a mob type the game keeps apart
    /// from mobs (the dragon), before the entity joins the level: its lend has
    /// a kind and a position and no mob.
    pub(in crate::game) fn finalize_spawn_at(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::Spawner>,
    ) -> (
        sunrise_sim::mobs::finalize::SpawnEvent,
        spawning::LocalDifficulty,
    ) {
        let difficulty = self.common.local_difficulty(at);
        if !mod_api::__private::listened(mod_api::HookId::FinalizeSpawn) {
            return (Default::default(), difficulty);
        }
        self.finalize_spawn_lent(kind, at, reason, spawner, difficulty, None, &mut None)
    }
    /// Fires `finalize_spawn` with `mob` lent to it and the group data as the
    /// API's; a group the listener changed comes back as the game's.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn finalize_spawn_lent(
        &self,
        kind: &str,
        at: [f64; 3],
        reason: SpawnReason,
        spawner: Option<mod_api::Spawner>,
        difficulty: spawning::LocalDifficulty,
        mob: Option<&mut Mob>,
        group: &mut Option<Group>,
    ) -> (
        sunrise_sim::mobs::finalize::SpawnEvent,
        spawning::LocalDifficulty,
    ) {
        let mut heard = mod_api::Difficulty::from(difficulty);
        let (mut spawn_cancelled, mut skip_finalize) = (false, false);
        let given = group.as_ref().and_then(|g| {
            crate::game::native_unspawned_mob::native_spawn_group(
                g,
                self.lend_epoch(),
                self.world_variants(),
            )
        });
        let mut native_group = given.clone();
        let mut borrowed = self.borrowed_unspawned_mob_mut(kind, at, mob);
        self.native_finalize_spawn(mod_api::__private::FinalizeSpawnCall {
            dimension: self.common.dim.id(),
            kind,
            reason,
            pos: at,
            spawner,
            difficulty: &mut heard,
            spawn_cancelled: &mut spawn_cancelled,
            mob: mod_api::UnspawnedMobEditor::new(&mut borrowed),
            group: &mut native_group,
            cancel: &mut skip_finalize,
        });
        if native_group != given {
            *group = native_group.as_ref().and_then(|g| {
                crate::game::native_unspawned_mob::spawn_group(g, self.world_variants())
            });
        }
        (
            sunrise_sim::mobs::finalize::SpawnEvent {
                skip_initialization: skip_finalize,
                veto_admission: spawn_cancelled,
            },
            if skip_finalize {
                difficulty
            } else {
                heard.into()
            },
        )
    }
    /// Generation's animals for the chunks generated since, once lit: chunk
    /// generation places them whether the level ticks or not.
    pub(in crate::game) fn spawn_fresh_chunks(&mut self) {
        if !self.common.world.chunks.has_fresh() {
            return;
        }
        let fresh: Vec<(i32, i32)> = self
            .common
            .world
            .chunks
            .fresh_keys_ordered()
            .into_iter()
            .filter(|k| {
                self.common
                    .world
                    .chunks
                    .get(k)
                    .is_some_and(|c| c.fresh && c.lit)
            })
            .collect();
        for key in fresh {
            if let Some(c) = self.common.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.common.world.chunks.remove_fresh(&key);
            self.spawn_generation_creatures(key.0, key.1);
        }
    }

    pub(in crate::game) fn deliver_pending_mob_actions(
        &mut self,
        level_id: &'static str,
    ) -> Result<()> {
        let mut pending = self
            .common
            .creatures
            .pending
            .remove(level_id)
            .unwrap_or_default();
        let delivery = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            while let Some(batch) = pending.batches.front_mut() {
                self.common.creatures.deferred_hits = true;
                let result = self.handle_mob_events_pending(&mut batch.normal);
                self.common.creatures.deferred_hits = false;
                result?;
                self.lan_guest_mob_events(&mut batch.guests);
                self.common.creatures.event_buf = pending
                    .batches
                    .pop_front()
                    .unwrap()
                    .normal
                    .root
                    .events
                    .into();
            }
            if !pending.pushes.is_empty() {
                for mob in &mut self.common.mobs {
                    sunrise_sim::mobs::apply_mob_pushes(mob, &mut pending.pushes);
                }
                // As in the producer's successful final sweep, removed targets
                // have no remaining owner to receive a push.
                pending.pushes = Default::default();
            }
            Ok(())
        }));
        self.common.creatures.pending.insert(level_id, pending);
        match delivery {
            Ok(result) => result,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    fn emit_death_poofs(&mut self, gone: impl IntoIterator<Item = (u32, Vec3, f64, f64)>) {
        for (id, at, _, _) in gone {
            self.lan_entity_event(id as i32, 60, at.components());
        }
    }

    /// Run the owning-level steps of a suspended hit, then let its caller see the real result.
    pub(in crate::game) fn complete_mob_hurt(
        &mut self,
        index: usize,
        hurt: bool,
    ) -> Option<(usize, bool)> {
        self.complete_mob_hurt_with_weapon(index, hurt, None)
    }

    /// `AbstractCubeMob.remove`: split only as the dead body is removed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn split_dead_cube(&mut self, mut dying: Mob) {
        if matches!(
            dying.species(),
            Species::Slime | Species::MagmaCube | Species::SulfurCube
        ) && dying.extra.size > 1
        {
            self.split_cube_children(&mut dying);
        }
    }
    /// The killer's side of a mob's death: `ServerPlayer.awardKillScore`'s
    /// statistics and scores, and the `player_killed_entity` trigger.
    pub(in crate::game) fn credit_kill(&mut self, m: &Mob) {
        // Player.killedEntity: the killed statistic goes to the causing
        // entity, so a kill by a mob (a tame wolf credited to its owner)
        // leaves it out; ServerPlayer.awardKillScore counts the rest.
        if !killed_by_mob(m) {
            self.award_stat("killed", &m.kind.id, 1);
        }
        self.award_custom_stat("mob_kills", 1);
        self.score_player_killed(crate::game::commands::Target::Mob(m.id));
        let view = self.mob_entity(m);
        let blow = self.last_blow(m);
        self.trigger_killed(&view, blow.as_ref());
    }
    /// The damage source a mob died to (`die`'s), its direct entity included when the mob
    /// recorded one ([`Mob::last_direct`]).
    pub(in crate::game) fn last_blow(
        &self,
        m: &Mob,
    ) -> Option<sunrise_sim::advancements::DamageView> {
        let (d, _) = m.last_damage?;
        let mut view = self.damage_view(d);
        if let Some(kind) = m.last_direct {
            view.direct = Some(sunrise_sim::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            });
            view.is_direct = false;
        }
        Some(view)
    }
    /// A mob's damage source as advancement predicates see it: its type, the
    /// causing entity, and the projectile when one dealt it.
    pub(in crate::game) fn damage_view(&self, d: Damage) -> sunrise_sim::advancements::DamageView {
        let source = match d.attacker {
            // Its callers are KilledTrigger's: the source as their killing_blow sees it.
            Some(Who::Player) => Some(self.player_entity_for_any(&[
                "minecraft:player_killed_entity",
                "minecraft:kill_mob_near_sculk_catalyst",
            ])),
            Some(Who::Mob(id)) => self
                .common
                .mobs
                .iter()
                .find(|o| o.id == id)
                .map(|o| self.mob_entity(o)),
            None => None,
        };
        let projectile = match d.kind {
            DamageKind::Arrow => Some("minecraft:arrow"),
            DamageKind::Fireball => Some("minecraft:fireball"),
            DamageKind::WitherSkull => Some("minecraft:wither_skull"),
            DamageKind::Fireworks => Some("minecraft:firework_rocket"),
            _ => None,
        };
        let direct = match projectile {
            Some(kind) => Some(sunrise_sim::advancements::EntityView {
                kind: kind.into(),
                position: d.from.unwrap_or_default(),
                ..Default::default()
            }),
            None => source.clone(),
        };
        sunrise_sim::advancements::DamageView {
            kind: source_damage_type(d).to_owned(),
            // DamageSource.isDirect: the causing entity is the direct one.
            is_direct: projectile.is_none(),
            direct,
            source,
            ..Default::default()
        }
    }
    /// `LivingEntity.dropExperience`: the experience a dying mob or player is worth once
    /// the mods have had their say; 0 when they cancel it.
    pub(in crate::game) fn living_experience_drop(
        &mut self,
        entity: u32,
        kind: &str,
        attacker: Option<u128>,
        original: i32,
    ) -> i32 {
        let (mut amount, mut cancel) = (original, false);
        self.native_living_experience_drop(
            entity,
            kind,
            attacker,
            original,
            &mut amount,
            &mut cancel,
        );
        if cancel { 0 } else { amount.max(0) }
    }

    /// Adds the items a dying entity dropped to the level, each at its own
    /// position. With mods, they first hear the list (`native_living_drops`):
    /// edited or canceled.
    pub(in crate::game) fn drop_death_items(
        &mut self,
        dying: &DyingDrops<'_>,
        stacks: Vec<(ItemStack, [f64; 3], i32)>,
        pickup_delay: i32,
        mut velocity: impl FnMut(&mut Self) -> Option<[f64; 3]>,
    ) {
        let Some(source) = &dying.source else {
            for (stack, position, age) in stacks {
                let mut random =
                    sunrise_gameplay_common::game::spill_common::prepare_item_random(None);
                random.state.age = age;
                let motion = velocity(self).unwrap_or(random.velocity);
                self.add_item_entity_prepared(
                    stack,
                    position,
                    Some(motion),
                    pickup_delay,
                    Some(random),
                );
            }
            return;
        };
        // Vanilla builds each item entity (and draws its motion) when it
        // collects the drop, before the event sees the list.
        let mut drops: Vec<sunrise_gameplay_common::game::spill_common::BlockItemDrop> = stacks
            .into_iter()
            .map(|(stack, position, age)| {
                let random = sunrise_gameplay_common::game::spill_common::prepare_item_random(None);
                sunrise_gameplay_common::game::spill_common::BlockItemDrop {
                    random: Some(random),
                    stack,
                    position,
                    velocity: velocity(self).unwrap_or(random.velocity),
                    pickup_delay,
                    state: sunrise_gameplay_common::game::spill_common::DropState {
                        age,
                        ..Default::default()
                    },
                }
            })
            .filter(|drop| drop.stack.count > 0 && &*drop.stack.item != "minecraft:air")
            .collect();
        let mut cancel = false;
        if mod_api::__private::listened(mod_api::HookId::LivingDrops)
            && let Ok(mut snapshots) = drops.try_into_api()
        {
            self.native_living_drops(mod_api::__private::LivingDropsCall {
                entity: dying.entity,
                kind: dying.kind,
                player: dying.player,
                source,
                recently_hit: dying.recently_hit,
                drops: &mut snapshots,
                cancel: &mut cancel,
            });
            if let Ok(mut next) = snapshots.try_into_engine(self.items()) {
                for (drop, original) in next.iter_mut().zip(&drops) {
                    drop.random = original.random;
                }
                drops = next;
            } else {
                cancel = false;
            }
        }
        if cancel {
            return;
        }
        for drop in drops {
            if drop.stack.count > 0 && &*drop.stack.item != "minecraft:air" {
                self.add_item_drop(drop);
            }
        }
    }
    /// Apply reactions that need the game in `hurtServer`, without waiting for
    /// a mob tick: a Creaking's heart response or a Warden's anger and targeting.
    /// Wardens can react even when the base hurt call returned false.
    pub(in crate::game) fn settle_mob_hurt(&mut self, id: u32) {
        use sunrise_sim::mobs::{creaking, warden};

        let Some(index) = self.common.mobs.iter().position(|m| {
            m.id == id
                && (creaking::has_pending_hurt(m)
                    || m.extra
                        .warden
                        .as_ref()
                        .is_some_and(|st| st.has_pending_hurt()))
        }) else {
            return;
        };
        if self.common.mobs[index].species() == Species::Creaking {
            self.finish_creaking_hurt(index);
            return;
        }
        let mut events = Vec::new();
        let uuid = self.common.level.player_uuid;
        let others: Vec<_> = self
            .common
            .mobs
            .iter()
            .map(|m| seen(m, uuid, self.items()))
            .collect();
        let mut mob = self.common.mobs.swap_remove(index);
        {
            let world = self.mob_world_base(&others);
            warden::handle_hurt(&mut mob, &world, &mut events);
        }
        self.common.mobs.push(mob);
        let last = self.common.mobs.len() - 1;
        if index < last {
            self.common.mobs.swap(index, last);
        }
        if !events.is_empty() {
            let _ = self.handle_mob_events(events);
        }
    }

    pub(in crate::game) fn settle_creaking_hurt(&mut self, id: u32) {
        if let Some(index) = self.common.mob_index.find(&self.common.mobs, id)
            && sunrise_sim::mobs::creaking::has_pending_hurt(&self.common.mobs[index])
        {
            self.finish_creaking_hurt(index);
        }
    }

    pub(in crate::game) fn finish_creaking_hurt(&mut self, index: usize) {
        let mob = &mut self.common.mobs[index];
        let (id, join) = (mob.id, mob.join);
        let mut events = Vec::new();
        let reaction = sunrise_sim::mobs::creaking::take_hurt(mob, &mut events);
        let _ = self.handle_mob_events(events);
        if let Some((home, by_player)) = reaction
            && self
                .common
                .mob_object(id)
                .is_some_and(|mob| mob.join == join && !mob.removed)
        {
            self.creaking_hurt(home, id, by_player);
        }
    }

    /// `TurtleEggBlock.decreaseEggs` after an entity's step or landing won its
    /// roll (`destroyEgg`): the break sound and the remaining eggs (a stack of
    /// one is destroyed after it).
    pub(in crate::game) fn crack_turtle_egg(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let eggs = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .filter(|s| s.name == "minecraft:turtle_egg")
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|e| e.parse::<i32>().ok());
        if let Some(eggs) = eggs {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.turtle_egg_destroyed_by_player(level, ticks, at, state);
                if eggs <= 1 {
                    b.destroy_block(level, ticks, at, false);
                }
            });
            self.server_process_level_events()?;
        }
        Ok(())
    }
    /// `TurtleEggBlock.randomTick` at the last hatch stage: one baby turtle
    /// per egg, a day from grown, with the egg as its home (the sound, the
    /// crumbs and the block's removal came from the level).
    pub fn hatch_turtles(&mut self, pos: [i32; 3], state: sunrise_world::registry::BlockStateId) {
        let eggs = self
            .common
            .pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("eggs"))
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(1);
        for i in 0..eggs {
            let at = [
                f64::from(pos[0]) + 0.3 + f64::from(i) * 0.2,
                f64::from(pos[1]),
                f64::from(pos[2]) + 0.3,
            ];
            if let Some(id) = self.spawn_mob_raw("minecraft:turtle", at, true)
                && let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id)
            {
                m.extra.age = -24000;
                m.extra.home = Some(pos);
                m.yaw = 0.0;
                m.body_yaw = 0.0;
                m.head_yaw = 0.0;
            }
        }
    }

    /// `Animal.finalizeSpawnChildFromBreeding`'s single experience orb, drawn
    /// from the breeding parent after the rule check.
    pub fn award_breeding_experience(&mut self, parent: u32, at: [f64; 3]) {
        let mob_drops = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_DROPS);
        if let Some(amount) = self
            .common
            .mobs
            .iter_mut()
            .find(|m| m.id == parent)
            .and_then(|m| sunrise_sim::mobs::breeding_experience(m, mob_drops))
        {
            self.add_orb(at, amount);
        }
    }
    pub(in crate::game) fn breeding_orb(
        &mut self,
        parent: u32,
        at: [f64; 3],
        experience: sunrise_sim::mobs::BreedExperience,
    ) {
        match experience {
            sunrise_sim::mobs::BreedExperience::Deferred => {
                self.award_breeding_experience(parent, at)
            }
            sunrise_sim::mobs::BreedExperience::Drawn(Some(amount)) => self.add_orb(at, amount),
            sunrise_sim::mobs::BreedExperience::Drawn(None) => {}
        }
    }
    /// `ServerPlayer.tellNeutralMobsThatIDied`: the mobs within 32 blocks
    /// across and 10 up and down get `NeutralMob.playerDied`, when the
    /// `forgive_dead_players` rule is on. A mob's `Who::Player` is the guest
    /// it is bound to in `bindings` (game/network/lan_mobs.rs), else the host's
    /// player, so only the mobs whose binding is `conn` (`None` for the
    /// host's player) hear it.
    pub(in crate::game) fn tell_neutral_mobs_player_died(
        &mut self,
        feet: [f64; 3],
        conn: Option<u32>,
    ) {
        if !self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::FORGIVE_DEAD_PLAYERS)
        {
            return;
        }
        let block = feet.map(f64::floor);
        let area = Aabb::new(
            Vec3::new(block[0], block[1], block[2]),
            Vec3::new(block[0] + 1.0, block[1] + 1.0, block[2] + 1.0),
        )
        .inflate(Vec3::new(32.0, 10.0, 32.0));
        let Some(host) = self.server.lan.as_ref() else {
            return;
        };
        let ids: Vec<u32> = self
            .common
            .mobs
            .iter()
            .filter(|m| {
                host.mob_players.get(&m.id).copied() == conn
                    && m.bounds().intersects(area)
                    && sunrise_sim::mobs::ai::forgives_dead_player(m)
            })
            .map(|m| m.id)
            .collect();
        for id in ids {
            let Some(epoch) = self.common.native_lifecycle.epoch() else {
                if let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                    sunrise_sim::mobs::ai::forgive_dead_player(mob);
                }
                continue;
            };
            let mob_ref = mod_api::MobRef::new(mod_api::EntityRef::new(epoch, id));
            let change = self
                .native_mob_target_event(mob_ref, None, false)
                .and_then(|target| self.native_mob_bind_target(id, target))
                .map_or(
                    sunrise_sim::mobs::TargetChange::Keep,
                    sunrise_sim::mobs::TargetChange::Set,
                );
            if let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                sunrise_sim::mobs::ai::forgive_dead_player_with_change(mob, change);
            }
        }
    }

    /// A status effect from a mob, arrow or splash potion (`addEffect`).
    pub fn give_player_effect(&mut self, effect: &'static str, ticks: i32, amplifier: i32) {
        self.give_mob_effect(effect, ticks, amplifier, false);
    }

    /// [`Self::give_player_effect`] with an explicit causing entity for `MobEffectEvent.Added`.
    pub(in crate::game) fn give_player_effect_from(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        source: Option<mod_api::EntityRef>,
    ) {
        self.give_mob_effect_from(effect, ticks, amplifier, false, source);
    }
    /// A mob's effect on the player. Creative and spectator players shrug off
    /// the damaging effects here, but the nautilus's ambient Breath is plain
    /// `addEffect` and lands in every game mode.
    pub(in crate::game) fn give_mob_effect(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        ambient: bool,
    ) {
        self.give_mob_effect_from(effect, ticks, amplifier, ambient, None);
    }

    fn give_mob_effect_from(
        &mut self,
        effect: &'static str,
        ticks: i32,
        amplifier: i32,
        ambient: bool,
        source: Option<mod_api::EntityRef>,
    ) {
        let immune = self.common.creative || self.common.spectator();
        if let Some(instance) = mob_effect_instance(effect, ticks, amplifier, ambient, immune) {
            self.add_player_effect_from(instance, source);
        }
    }
    /// `WitherRoseBlock.entityInside` for the player: on a server level that
    /// isn't peaceful, `getBeeInteractionEffect` (Wither, 40 ticks) unless the
    /// player is invulnerable to wither damage (creative, spectator). A
    /// server's player gets it from the server.
    pub(in crate::game) fn wither_rose_contact(&mut self) {
        if self.common.ui.difficulty == 0 {
            return;
        }
        self.give_player_effect("minecraft:wither", 40, 0);
    }

    /// Single-projectile live vanilla tick loop (when no impact/alone hook requires batching).
    pub(in crate::game) fn tick_projectiles_live(
        &mut self,
        through: u32,
        ticking: impl Fn(i32, i32) -> bool,
        vanish: bool,
    ) -> Result<()> {
        let cohort: Vec<JoinSeq> = self
            .common
            .creatures
            .projectiles
            .iter()
            .filter(|p| p.id <= through)
            .map(|p| p.join)
            .collect();
        for join in cohort {
            let Some(pos) = self.common.creatures.projectiles.locate(join) else {
                continue;
            };
            let at = self.common.creatures.projectiles[pos].position;
            if !ticking(at.x.floor() as i32, at.z.floor() as i32) {
                self.common
                    .creatures
                    .projectiles
                    .update(pos, |p| p.previous = at);
                continue;
            }
            let mut _save_lease = Some(
                self.common
                    .creatures
                    .projectiles
                    .visibility_lease()
                    .ok_or_else(sunrise_sim::join_seq::visibility::PassLease::enter),
            );
            let Some(mut p) = self.common.creatures.projectiles.extract(join) else {
                continue;
            };
            let pid = p.id;
            if vanish
                && matches!(p.kind, ProjectileKind::EnderPearl)
                && p.owner == Some(Who::Player)
                && match p.guest {
                    Some(c) => self
                        .server
                        .lan
                        .as_ref()
                        .and_then(|h| h.hub.guest(c))
                        .is_some_and(|g| g.dead),
                    None => self.player_view().is_some_and(|pv| !pv.alive),
                }
            {
                self.common
                    .creatures
                    .projectiles
                    .retire_extracted(pid, join);
                continue;
            }
            let section_from = p.position;
            let before = (self.native_hears_impacts() && !p.stuck).then(|| p.clone());
            let mob_shot = !matches!(p.owner, Some(Who::Player));
            let pvp = !mob_shot && crate::game::lan_pvp::meets_players(&p.kind);
            let shooter = p.guest.filter(|_| !mob_shot).and_then(|c| {
                self.server
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guest(c))
                    .map(|g| Who::Mob(g.entity_id as u32))
            });
            if shooter.is_some() {
                p.owner = shooter;
            }
            let use_guests = mob_shot || pvp;
            let query = TargetQuery::for_projectile(&p, use_guests);
            let mut scratch = std::mem::take(&mut self.common.creatures.arrow_hit_scratch);
            let (keep, hit, snow_contacts, bubble_tops) = {
                let live = self.live_proj_world(query, &[]);
                let (keep, hit) = if p.arrow.is_some() {
                    let rain = self.arrow_rain();
                    let scene = self.arrow_scene(&rain);
                    sunrise_sim::entities::arrows::tick_to_block_contact_live(
                        &mut p,
                        &scene,
                        &live,
                        false,
                        &mut scratch,
                    )
                } else {
                    p.tick_before_hit(&live)
                };
                let mut snow = Vec::new();
                if (p.is_thrown() && !matches!(p.kind, ProjectileKind::LlamaSpit))
                    || p.is_arrow() && p.arrow.as_ref().is_none_or(|a| !a.no_physics)
                {
                    for block in sunrise_sim::powder_snow::contacts(
                        p.previous,
                        p.position,
                        p.bounds(),
                        false,
                        |at| live.w.block_name(at.x, at.y, at.z) == "minecraft:powder_snow",
                    ) {
                        snow.push((block, p.position, p.velocity));
                    }
                }
                let mut bubbles = Vec::new();
                if keep && hit == Hit::None && matches!(p.kind, ProjectileKind::EnderPearl) {
                    let (at, h) = (p.position, 0.125);
                    sunrise_sim::motion::bubble_column_push_over(
                        live.w.world,
                        live.w.registry,
                        live.w.facts,
                        live.w.collision,
                        Aabb::new(
                            Vec3::new(at.x - h, at.y, at.z - h),
                            Vec3::new(at.x + h, at.y + 2.0 * h, at.z + h),
                        ),
                        &mut p.velocity,
                        |bpos| bubbles.push(bpos),
                    );
                }
                (keep, hit, snow, bubbles)
            };
            self.common.creatures.arrow_hit_scratch = scratch;
            if shooter.is_some() {
                p.owner = Some(Who::Player);
            }
            if hit != Hit::None
                && let Some(mut before) = before
            {
                let snapshot = p.clone();
                let target = self.native_impact_target(hit);
                drop(_save_lease.take());
                let place = pos.min(self.common.creatures.projectiles.len());
                self.common.creatures.projectiles.put_back(place, p);
                let canceled = self.native_projectile_impact_canceled(&snapshot, hit);
                _save_lease = Some(
                    self.common
                        .creatures
                        .projectiles
                        .visibility_lease()
                        .ok_or_else(sunrise_sim::join_seq::visibility::PassLease::enter),
                );
                let unchanged = !self.projectile_removal_pending(pid)
                    && self
                        .common
                        .creatures
                        .projectiles
                        .locate(join)
                        .is_some_and(|i| self.common.creatures.projectiles[i] == snapshot)
                    && target.is_some()
                    && target == self.native_impact_target(hit);
                if !unchanged {
                    drop(_save_lease.take());
                    continue;
                }
                if canceled {
                    before.has_been_shot = snapshot.has_been_shot;
                    if let (Some(arrow), Some(shot)) = (&mut before.arrow, &snapshot.arrow) {
                        arrow.has_been_shot = shot.has_been_shot;
                    }
                    let (flown, flies, events) = self.native_projectile_through(before);
                    let (owner, guest, to) = (flown.owner, flown.guest, flown.position);
                    if let Some(i) = self.common.creatures.projectiles.locate(join) {
                        if flies {
                            self.common.creatures.projectiles.store_back(i, flown);
                            if !keep
                                && let Some(row) = self.common.creatures.projectiles.extract(join)
                            {
                                let end = self.common.creatures.projectiles.len();
                                self.common.creatures.projectiles.put_back(end, row);
                            }
                        } else {
                            self.common.creatures.projectiles.remove(i);
                        }
                    }
                    drop(_save_lease.take());
                    if flies {
                        self.entity_section_step(pid, section_from.components(), to.components());
                    }
                    for (at, event) in events {
                        if event == sunrise_sim::entities::arrows::ArrowEvent::ShootEvent
                            && matches!(hit, Hit::Block { .. })
                        {
                            self.projectile_shoot_event(owner, guest, at.components());
                        } else {
                            self.server_arrow_event(at, event);
                        }
                    }
                    continue;
                }
                let Some(row) = self.common.creatures.projectiles.extract(join) else {
                    drop(_save_lease.take());
                    continue;
                };
                p = row;
            }
            for (block, position, movement) in snow_contacts {
                self.powder_snow_particle(block, position, movement);
            }
            if let Some(at) = p.take_shoot_event() {
                self.projectile_shoot_event(p.owner, p.guest, [at.x, at.y, at.z]);
            }
            for splash in p.water_splashes.drain(..) {
                self.server_apply_item_splash(splash);
            }
            if let Some((hurt_at, is_lava)) = p.take_bullet_hurt() {
                let at = hurt_at.components();
                let silent = self.entity_silent(pid);
                let ctx = sunrise_level::sculk::Context {
                    owner: p.owner,
                    owner_uuid: p.owner_uuid,
                    ..self.entity_event_context(pid, None)
                };
                self.shulker_bullet_hurt(at, silent);
                self.common
                    .creatures
                    .projectiles
                    .retire_extracted(pid, join);
                drop(_save_lease.take());
                self.post_game_event("minecraft:entity_damage", at, ctx);
                if is_lava && !silent {
                    self.lava_hurt_sound(at);
                }
                continue;
            }
            // Clients generate projectile smoke trails from synced entity state.
            for bpos in bubble_tops {
                self.bubble_column_particles(bpos);
            }
            let mut shot_after_hit = false;
            let mut pending_arrow_events = Vec::new();
            if let Some(a) = p.arrow.as_mut() {
                for event in a.events.drain(..) {
                    if event == sunrise_sim::entities::arrows::ArrowEvent::ShootEvent {
                        if matches!(hit, Hit::Block { .. }) {
                            shot_after_hit = true;
                        } else {
                            self.projectile_shoot_event(p.owner, p.guest, p.position.components());
                        }
                    } else {
                        pending_arrow_events.push(event);
                    }
                }
            }
            for event in pending_arrow_events {
                self.server_arrow_event(p.position, event);
            }
            let wind = matches!(
                p.kind,
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge
            );
            if wind
                && keep
                && let Some(ceiling) = self.wind_ceiling_contact(&p)
            {
                self.common
                    .creatures
                    .projectiles
                    .retire_extracted(pid, join);
                drop(_save_lease.take());
                self.apply_projectile_hit(&p, ceiling)?;
                continue;
            }
            if keep || hit != Hit::None && p.arrow.is_none() {
                self.sweep_projectile_inside(&mut p, None);
                if p.destroyed {
                    self.common
                        .creatures
                        .projectiles
                        .retire_extracted(pid, join);
                    continue;
                }
            }
            if matches!(hit, Hit::Entity { .. }) && (!keep || p.arrow.is_some()) {
                let followers: Vec<u32> = self
                    .common
                    .creatures
                    .projectiles
                    .iter()
                    .skip(pos.min(self.common.creatures.projectiles.len()))
                    .map(|q| q.id)
                    .collect();
                self.common
                    .creatures
                    .bounced_followers
                    .push((pid, followers));
            }
            if keep {
                let to = p.position;
                let place = pos.min(self.common.creatures.projectiles.len());
                if hit == Hit::None {
                    self.common.creatures.projectiles.put_back(place, p);
                    drop(_save_lease.take());
                    self.entity_section_step(pid, section_from.components(), to.components());
                    continue;
                }
                let hit_snapshot = p.clone();
                self.common.creatures.projectiles.put_back(place, p);
                drop(_save_lease.take());
                self.entity_section_step(pid, section_from.components(), to.components());
                let result = self.apply_projectile_hit(&hit_snapshot, hit);
                if result.is_ok() && shot_after_hit {
                    self.projectile_shoot_event(
                        hit_snapshot.owner,
                        hit_snapshot.guest,
                        hit_snapshot.position.components(),
                    );
                }
                self.common.creatures.bounced_followers.clear();
                result?;
                continue;
            }
            if hit == Hit::None {
                self.common
                    .creatures
                    .projectiles
                    .retire_extracted(pid, join);
                continue;
            }
            let result = self.apply_projectile_hit(&p, hit);
            if let Some(re_idx) = self.common.creatures.projectiles.locate(join) {
                self.common.creatures.projectiles.update(re_idx, |live| {
                    if !live.attachment_lifetime.is_bound() {
                        std::mem::swap(&mut live.attachment_lifetime, &mut p.attachment_lifetime);
                    }
                });
            } else {
                self.common
                    .creatures
                    .projectiles
                    .retire_extracted(pid, join);
            }
            drop(_save_lease.take());
            if result.is_ok() && shot_after_hit {
                self.projectile_shoot_event(p.owner, p.guest, p.position.components());
            }
            self.common.creatures.bounced_followers.clear();
            result?;
        }
        self.flush_lan_explosions();
        self.flush_wind_bursts()
    }

    pub(in crate::game) fn apply_projectile_hits(
        &mut self,
        hits: Vec<ProjectileHit>,
        vehicle_winds: &mut crate::game::wind::VehicleWinds,
        mut pending: FxHashMap<JoinSeq, PendingImpact>,
        hears_impact: bool,
    ) -> Result<()> {
        for ProjectileHit {
            projectile: p,
            hit,
            shot_after_hit,
        } in hits
        {
            let (id, join) = (p.id, p.join);
            if hears_impact && self.projectile_removal_pending(id) {
                continue;
            }
            let contact = pending.remove(&join);
            let deferred_arrow =
                contact.is_some() && p.arrow.is_some() && matches!(hit, Hit::Block { .. });
            let wind_flight = vehicle_winds.take(id, hit);
            let landed = match (contact, wind_flight) {
                (Some(contact), Some(before)) => {
                    self.apply_vehicle_wind_impact(p, hit, contact, before)
                }
                (Some(contact), None) => self
                    .apply_captured_impact(p, hit, contact)
                    .map(|p| (p, hit)),
                (None, Some(before)) if self.wind_vehicle_broken(hit) => {
                    let from = before.position;
                    let (mut p, keep, hit) = self.broken_vehicle_retick(before);
                    if keep {
                        if let Some(ceiling) = self.wind_ceiling_contact(&p) {
                            Some((p, ceiling))
                        } else {
                            self.sweep_projectile_inside(&mut p, None);
                            let (pid, to) = (p.id, p.position);
                            self.common.reinsert_continuing_projectile(p);
                            self.entity_section_step(pid, from.components(), to.components());
                            None
                        }
                    } else {
                        (hit != Hit::None).then_some((p, hit))
                    }
                }
                (None, _) => Some((p, hit)),
            };
            if let Some((p, hit)) = landed {
                if let Err(error) = self.apply_projectile_hit(&p, hit) {
                    if self.common.creatures.projectiles.locate(join).is_none() {
                        self.common.creatures.projectiles.retire_extracted(id, join);
                    }
                    return Err(error);
                }
                if shot_after_hit {
                    self.projectile_shoot_event(p.owner, p.guest, p.position.components());
                }
            }
            if deferred_arrow {
                self.finish_deferred_arrow_tick(id, join);
            }
        }
        Ok(())
    }

    pub(in crate::game) fn apply_projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        // Arrows dispatch each entity in `finish_arrow_step` directly so mixed mob/player batches
        // preserve shooter context and exact pierced entity ids.
        if matches!(p.kind, ProjectileKind::Arrow { .. }) && matches!(hit, Hit::Entity { .. }) {
            return self.projectile_hit(p, hit);
        }
        // A player's snowball/egg/thrown on another player lands as the victim's hurt.
        if let Some(victim) = self.projectile_pvp_victim(p, hit) {
            return self.projectile_hits_player(p, hit, victim);
        }
        // A guest's projectile lands as theirs: a pearl takes them, what it
        // hurts answers to them. One hitting the host stays the host's hit.
        match p.guest {
            Some(conn)
                if !matches!(
                    hit,
                    Hit::Entity {
                        who: Who::Player,
                        ..
                    }
                ) =>
            {
                self.guest_projectile_hit(conn, p, hit)?;
            }
            _ => self.projectile_hit(p, hit)?,
        }
        // LlamaSpit.tick lands the hit before replacing the live velocity with
        // its pre-hit movement, scaled by drag and affected by gravity.
        if p.kind == ProjectileKind::LlamaSpit
            && matches!(hit, Hit::Entity { .. })
            && !self.projectile_removal_pending(p.id)
            && let Some(i) = self.common.creatures.projectiles.locate(p.join)
        {
            let (from, to) = self.common.creatures.projectiles.update(i, |spit| {
                let from = spit.position;
                spit.finish_spit_motion(p.velocity);
                (from, spit.position)
            });
            self.entity_section_step(p.id, from.components(), to.components());
        }
        Ok(())
    }
    /// `LivingEntity.hurtServer` for a snowball, egg or pearl (`kind`) that hurt the mob `id`,
    /// 0 damage included: the accepted hit already credited its owner; for a player,
    /// also fire PLAYER_HURT_ENTITY.
    pub(in crate::game) fn thrown_hurt_mob(
        &mut self,
        p: &Projectile,
        id: u32,
        kind: &str,
        amount: f32,
    ) {
        if p.owner == Some(Who::Player)
            && let Some(m) = self.common.mobs.iter().find(|m| m.id == id)
        {
            let victim = self.mob_entity(m);
            let direct = sunrise_sim::advancements::EntityView {
                kind: kind.into(),
                position: [p.position.x, p.position.y, p.position.z],
                ..Default::default()
            };
            self.player_hit(&victim, "minecraft:thrown", Some(direct), amount);
        }
    }
    /// `projectile_hit` for a projectile the guest `conn` shot, run as them;
    /// as the host's when they have left.
    pub(in crate::game) fn guest_projectile_hit(
        &mut self,
        conn: u32,
        p: &Projectile,
        hit: Hit,
    ) -> Result<()> {
        let result = self.as_guest(conn, |g| g.projectile_hit(p, hit));
        // The mob hit now answers to the guest (LivingEntity.lastHurtByPlayer).
        if result.is_some()
            && self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_some_and(|guest| guest.participates_online())
            && let Hit::Entity {
                who: Who::Mob(id), ..
            } = hit
            && self.common.mobs.iter().any(|m| m.id == id)
            && let Some(host) = self.server.lan.as_mut()
        {
            host.mob_players.insert(id, conn);
        }
        match result {
            Some(r) => r,
            None => self.projectile_hit(p, hit),
        }
    }
    /// `EnchantmentHelper.doPostAttackEffects` after the player's shot `p`
    /// (its `direct` entity, a hit of `damage_type`) hurt the mob `who`: the
    /// victim's own post-attack enchantments answer the player (Thorns).
    pub(in crate::game) fn shot_post_attack(
        &mut self,
        p: &Projectile,
        who: Who,
        damage_type: &str,
    ) {
        let attack = self.current_attack();
        let (victim, kind) = match who {
            Who::Player => (self.own_entity_id() as u32, "minecraft:player".into()),
            Who::Mob(id) => {
                let Some(target) = self.level_entity(id as i32) else {
                    return;
                };
                (id, self.target_type(target))
            }
        };
        self.shot_post_attack_against(p, victim, &kind, &attack, damage_type);
    }

    fn shot_post_attack_against(
        &mut self,
        p: &Projectile,
        victim: u32,
        kind: &str,
        attack: &Attack,
        damage_type: &str,
    ) {
        let source = attack.player_source(damage_type, attack.attacker);
        let trident = matches!(p.kind, ProjectileKind::Trident);
        let weapon = p.arrow.as_ref().and_then(|arrow| {
            if trident {
                Some(&arrow.pickup_item)
            } else {
                arrow.fired_from_weapon.as_ref()
            }
        });
        if p.arrow.is_some() {
            self.post_attack(victim, kind, attack, source, weapon, trident);
        } else {
            self.post_attack_from_hand(victim, kind, attack, source);
        }
    }
    /// `BellBlock.onProjectileHit`: a projectile that strikes the bell proper
    /// rings it, and a player's shot counts as the player's ring.
    pub(in crate::game) fn bell_projectile_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let Some(face) = Direction::ALL.into_iter().find(|d| d.normal() == face) else {
            return;
        };
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let rang = self
            .with_level(|b, level, ticks| b.bell_projectile_hit(level, ticks, block, face, at.y));
        if rang {
            let _ = self.server_process_level_events();
            if p.owner == Some(Who::Player) {
                self.award_custom_stat("bell_ring", 1);
            }
        }
    }
    /// `AmethystBlock.onProjectileHit`: a projectile that strikes an amethyst
    /// block rings `block.amethyst_block.chime` there, pitched by the level random.
    pub(in crate::game) fn amethyst_projectile_hit(&mut self, pos: [i32; 3]) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if &*self.common.res.facts().get(state).name != "minecraft:amethyst_block" {
            return;
        }
        let pitch = 0.5 + self.common.world.random.next_float() * 1.2;
        self.common
            .level_events
            .push(sunrise_world::level::LevelEvent::Sound {
                pos: BlockPos::new(pos[0], pos[1], pos[2]),
                sound: "minecraft:block.amethyst_block.chime".into(),
                source: None,
                volume: 1.0,
                pitch,
            });
        let _ = self.server_process_level_events();
    }
    /// `onProjectileHit` of chorus flowers, decorated pots and speleothems
    /// (pointed dripstone, sulfur spikes): an impact projectile
    /// (`#impact_projectiles`) breaks the block when `Projectile.mayInteract`
    /// and `Projectile.mayBreak` allow it, which takes the
    /// `projectiles_can_break_blocks` rule and a shooter that may break blocks
    /// (`canBreakBlockInAdventureMode`: a resolved mob's arrow needs its
    /// actual projectile predicate and mob_griefing). Other owner/kind paths
    /// retain their existing policy. A pot is cracked first (flags 260).
    pub fn projectile_breaks_block(&mut self, p: &Projectile, pos: [i32; 3]) {
        use sunrise_command::args::GameMode;
        let impact = matches!(
            p.kind,
            ProjectileKind::Arrow { .. }
                | ProjectileKind::Trident
                | ProjectileKind::Snowball
                | ProjectileKind::Thrown(_)
                | ProjectileKind::Egg
                | ProjectileKind::SmallFireball
                | ProjectileKind::LargeFireball { .. }
                | ProjectileKind::WitherSkull { .. }
                | ProjectileKind::WindCharge
                | ProjectileKind::BreezeWindCharge
        );
        use sunrise_sim::entities::arrows::OwnerLookup;
        let shooter_may_break = match p.owner {
            Some(Who::Player) => self.common.mode != GameMode::Adventure,
            Some(Who::Mob(_)) => {
                matches!(p.kind, ProjectileKind::Arrow { .. })
                    && self
                        .common
                        .ui
                        .rules
                        .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING)
                    && p.arrow
                        .as_ref()
                        .and_then(|a| a.can_break.as_ref())
                        .is_some_and(|predicate| {
                            crate::game::game_mode::projectile_predicate_supported(predicate)
                                && self.adventure_predicate_at(predicate, pos)
                        })
            }
            None => !p.owner_uuid.is_some_and(|u| self.non_player_entity(u)),
        };
        if !impact
            || !shooter_may_break
            || !self
                .common
                .ui
                .rules
                .bool_at(sunrise_game_data::gamerules::vanilla::PROJECTILES_CAN_BREAK_BLOCKS)
        {
            return;
        }
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        // `destroyBlock(pos, true, projectile)`; a speleothem's names no entity.
        let mut entity = Some(p.id);
        match &*self.common.res.facts().get(state).name {
            "minecraft:chorus_flower"
                if matches!(
                    p.kind,
                    ProjectileKind::Arrow { .. } | ProjectileKind::Snowball
                ) =>
            {
                // `ChorusFlowerBlock.onProjectileHit` supplies the projectile,
                // even when it has no owner, as the direct block's loot entity.
                let _ = self.destroy_block_by(pos, Some(p.id));
                return;
            }
            "minecraft:chorus_flower" => {}
            // SpeleothemBlock.onProjectileHit (pointed dripstone and sulfur spikes): a
            // thrown trident faster than 0.6 breaks it.
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
                if matches!(p.kind, ProjectileKind::Trident)
                    && p.arrow.as_ref().is_some_and(|a| a.hit_speed > 0.6) =>
            {
                entity = None;
            }
            "minecraft:decorated_pot" => {
                let registry = &self.common.pack.assets.registry;
                let Some(mut pot) = registry.state(state).cloned() else {
                    return;
                };
                pot.properties.insert("cracked".into(), "true".into());
                let Ok(cracked) = registry.resolve(&pot) else {
                    return;
                };
                self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
            }
            _ => return,
        }
        let _ = self.destroy_block_by(pos, entity);
    }

    /// Flush queued arrow sounds, particles, block reactions, drops, and shoot events.
    pub(in crate::game) fn flush_arrow_events(&mut self, arrow: &mut Projectile) {
        let Some(a) = arrow.arrow.as_mut() else {
            return;
        };
        let (events, item) = (std::mem::take(&mut a.events), a.pickup_item.clone());
        for event in events {
            if event == sunrise_sim::entities::arrows::ArrowEvent::Drop {
                self.spawn_at_location(arrow.position, f64::from(0.1f32), item.clone());
            } else if event == sunrise_sim::entities::arrows::ArrowEvent::ShootEvent {
                self.projectile_shoot_event(arrow.owner, arrow.guest, arrow.position.components());
            } else {
                self.server_arrow_event(arrow.position, event);
            }
        }
    }

    /// The rest of the tick of an arrow that hit an entity (`arrows::finish_entity_hit`).
    pub fn finish_arrow_hit(&mut self, mut arrow: Projectile, outcome: EntityHit) {
        let keep = {
            let rain = self.arrow_rain();
            sunrise_sim::entities::arrows::finish_entity_hit(
                &mut arrow,
                &self.arrow_scene(&rain),
                outcome,
            )
        };
        self.flush_arrow_events(&mut arrow);
        if keep {
            self.common.reinsert_continuing_projectile(arrow);
        } else if self
            .common
            .creatures
            .projectiles
            .locate(arrow.join)
            .is_none()
        {
            self.common
                .creatures
                .projectiles
                .retire_extracted(arrow.id, arrow.join);
        }
    }

    /// Dispatch the same crystal and dragon-part hit for arrows and mob tridents.
    pub(in crate::game) fn special_arrow_hit(
        &mut self,
        arrow: &mut Projectile,
        id: u32,
        amount: f32,
    ) -> EntityHit {
        let owner = self.resolved_projectile_owner(arrow.owner, arrow.guest);
        let kind = self.common.projectile_damage_kind(arrow);
        let damage_type = self.projectile_damage_type(arrow);
        // EndCrystal.hurtServer can push this arrow before its next hit or deflection.
        // Lend the contact state to the level walk, as for an impact callback.
        let crystal = self.common.is_crystal_id(id);
        let join = arrow.join;
        if crystal {
            let mut contact = arrow.clone();
            std::mem::swap(
                &mut contact.attachment_lifetime,
                &mut arrow.attachment_lifetime,
            );
            let followers = self
                .common
                .creatures
                .bounced_followers
                .iter()
                .find(|(id, _)| *id == arrow.id)
                .map_or(&[][..], |(_, followers)| followers.as_slice());
            let projectiles = &mut self.common.creatures.projectiles;
            let index = bounced_place(projectiles, followers);
            projectiles.put_back(index, contact);
        }
        let hit = |game: &mut Self| {
            game.hit_special_full(
                id,
                amount,
                owner,
                arrow.guest,
                kind,
                Some(damage_type),
                Some(crate::game::dragon::DirectHit::from_projectile(arrow)),
            )
        };
        let hurt = if !crystal
            && id >= SPECIAL
            && mod_api::__private::enabled_listened(mod_api::HookId::LivingDeath)
            && self.server.combat.0.attack().projectile != Some(arrow.id as i32)
        {
            let attack = self.projectile_attack(arrow);
            self.with_attack(attack, hit)
        } else {
            hit(self)
        };
        if crystal {
            if !self.projectile_removal_pending(arrow.id)
                && let Some(live) = self.common.creatures.projectiles.extract(join)
            {
                *arrow = live;
            } else if let Some(a) = arrow.arrow.as_mut() {
                a.removed = true;
            }
        }
        match hurt {
            crate::game::dragon::Hurt::Landed { .. } => EntityHit::Hurt,
            crate::game::dragon::Hurt::Refused => EntityHit::Refused,
            crate::game::dragon::Hurt::Shrugged => {
                if let Some(a) = arrow.arrow.as_mut() {
                    a.ignite_for_seconds(1.0);
                }
                EntityHit::Refused
            }
        }
    }

    /// `setLastHurtMob` at the arrow/wind charge's onHitEntity site.
    pub(in crate::game) fn remember_projectile_target(&mut self, p: &Projectile, who: Who) {
        let target = match who {
            Who::Player => self
                .common
                .player
                .as_ref()
                .map(|body| body.entity_id as u32),
            Who::Mob(id) => self.last_hurt_living_target(id).map(|_| id),
        };
        match p.owner {
            Some(Who::Mob(id)) => self.remember_mob_target(id, target),
            Some(Who::Player) => {
                if let Some(shooter) = self.projectile_player_owner_id(p.guest) {
                    self.remember_player_projectile_target(shooter, target);
                }
            }
            None => {
                // These living bodies can be assigned as owners, but the projectile's
                // combat shortcut (`Who`) only represents mobs and players.
                let Some(owner) = p.owner_uuid.and_then(|uuid| {
                    self.target_by_uuid(sunrise_sim::lan::playerdata::ints_uuid(uuid))
                }) else {
                    return;
                };
                match owner {
                    crate::game::commands::Target::Stand(id) => {
                        if let Some(stand) =
                            self.common.armor_stands.iter_mut().find(|s| s.id == id)
                        {
                            stand.last_hurt_mob = target.map(|id| (id, stand.tick_count));
                        }
                    }
                    crate::game::commands::Target::Dragon(id) => {
                        if let Some(dragon) = self.common.dragon.as_mut().filter(|d| d.id == id) {
                            dragon.last_hurt_mob = target.map(|id| (id, dragon.ticks));
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// A projectile's hit, landed in the attack of its owner.
    pub fn projectile_hit(&mut self, p: &Projectile, hit: Hit) -> Result<()> {
        self.projectile_hit_deferring_player_arrow(p, hit, None)
    }
    pub(in crate::game) fn projectile_hit_deferring_player_arrow(
        &mut self,
        p: &Projectile,
        hit: Hit,
        deferred: Option<&mut Option<LandedPlayerArrow>>,
    ) -> Result<()> {
        let attack = self.projectile_attack(p);
        let result = self.with_attack(attack, |g| g.projectile_hit_landed(p, hit, deferred));
        self.settle_dealt_absorbed(p.owner == Some(Who::Player));
        result?;
        if let Hit::Entity {
            who: Who::Mob(id), ..
        } = hit
        {
            self.settle_mob_hurt(id);
        }
        // The hit's hurt teleports an enderman or a shulker now (`hurtServer`).
        self.settle_hurt_teleports()
    }
    pub(in crate::game) fn resume_guest_arrow_hit(&mut self, conn: u32, landed: LandedPlayerArrow) {
        let LandedPlayerArrow {
            victim,
            attack,
            arrow,
        } = landed;
        self.shot_post_attack_against(
            &arrow,
            victim,
            "minecraft:player",
            &attack,
            "minecraft:arrow",
        );
        self.guest_hit_by(conn, |g| {
            g.with_attack(attack, |g| g.arrow_post_hurt_effects(&arrow, Who::Player));
        });
    }

    /// `Snowball`/`ThrownEgg.onHit`: entity event 3 for the guests, before the
    /// projectile is discarded (the clients lose it when the tracker next runs).
    pub(in crate::game) fn thrown_break_event(&mut self, id: u32, at: [f64; 3]) {
        if id != 0 {
            self.lan_entity_event(id as i32, 3, at);
        }
    }

    /// `SnowGolem.mobInteract` and `Bogged.mobInteract` with shears: the
    /// pumpkin or the mushrooms come off (the shearing loot table), the shear
    /// game event is posted and the tool takes one damage.
    pub(in crate::game) fn interact_shear_mob(&mut self, index: usize, shears: bool) -> bool {
        let m = &self.common.mobs[index];
        if !shears || !m.alive() || m.extra.sheared {
            return false;
        }
        let (sound, table, height) = match m.species() {
            Species::SnowGolem => (
                "minecraft:entity.snow_golem.shear",
                "minecraft:shearing/snow_golem",
                m.eye_height(),
            ),
            Species::Bogged => (
                "minecraft:entity.bogged.shear",
                "minecraft:shearing/bogged",
                f64::from(m.height()),
            ),
            _ => return false,
        };
        let at = m.position;
        let mob = (
            m.id,
            sunrise_gameplay_common::game::native_hooks::mob_kind(m),
        );
        let tool = self.common.held().cloned();
        // shear: level.playSound(null, this, sound, PLAYERS, 1, 1).
        let shear =
            LevelSound::new(sound, [at.x, at.y, at.z], 1.0, 1.0).in_source(SoundSource::Players);
        self.play_bound_sound(shear, mob.0, None);
        self.common.mobs[index].extra.sheared = true;
        let mut stacks = self.sequence_loot(table, table, tool.as_ref());
        let origin = [at.x, at.y, at.z];
        let player = Some(self.native_acting_id());
        self.native_use_drops(
            table,
            origin,
            None,
            Some((mob.0, &mob.1)),
            player,
            tool.as_ref(),
            &mut stacks,
        );
        for stack in stacks {
            self.spawn_at_location(at, height, stack);
        }
        let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Player), None);
        self.post_game_event("minecraft:shear", [at.x, at.y, at.z], ctx);
        self.damage_held_tool();
        true
    }
    /// `ItemStack.hurtAndBreak(1, ...)` on the held tool: Unbreaking, break sound and stat.
    pub(in crate::game) fn damage_held_tool(&mut self) {
        self.damage_held(1);
    }
    /// `Mob.checkAndHandleImportantInteractions` with a spawn egg of mob
    /// `index`'s own kind (`SpawnEggItem.spawnOffspringFromSpawnEgg`): a baby
    /// at the mob's feet, taking the parent's variant as `getBreedOffspring`
    /// does, and one egg used. False (PASS) when the mob has no baby form.
    pub(in crate::game) fn spawn_egg_offspring(&mut self, index: usize) -> bool {
        let Some(stack) = self.common.held().cloned() else {
            return false;
        };
        let parent = &self.common.mobs[index];
        if !parent.alive()
            || !stack.item.ends_with("_spawn_egg")
            || sunrise_sim::block_entity_item::item_entity_data(&stack, self.common.res.items())
                .is_none_or(|data| data.kind != parent.kind.id)
            || self.common.ui.difficulty == 0 && !parent.kind.allowed_in_peaceful()
            || !sunrise_sim::mobs::species::can_be_a_baby(parent.species())
            // WanderingTrader.getBreedOffspring returns null: no offspring, the
            // click carries on to trading.
            || parent.species() == Species::WanderingTrader
        {
            return false;
        }
        let (kind, at) = (
            parent.kind.id.clone(),
            [parent.position.x, parent.position.y, parent.position.z],
        );
        let (species, variant, color) =
            (parent.species(), parent.extra.variant, parent.extra.color);
        let (wolf_variant, pig_variant, cow_variant, chicken_variant) = (
            parent.extra.wolf_variant,
            parent.extra.pig_variant,
            parent.extra.cow_variant,
            parent.extra.chicken_variant,
        );
        // `MushroomCow.getBreedOffspring(level, this)`: the egg's mate is the
        // parent itself, so the colour always rolls `nextInt(1024) == 0` to
        // flip, and every other roll spends the coin flip that can only pick
        // that same colour again. Both come from the parent's own random.
        let variant = if species == Species::Mooshroom {
            let random = &mut self.common.mobs[index].random;
            if random.next_int_bound(1024).unwrap_or(1) == 0 {
                if variant == 1 { 0 } else { 1 }
            } else {
                let _ = random.next_boolean();
                variant
            }
        } else {
            variant
        };
        let target = self.mob_entity(&self.common.mobs[index]);
        // SpawnEggItem.spawnOffspringFromSpawnEgg: `type.create(level, SPAWN_ITEM_USE)`
        // creates the baby without finalizeSpawn, and `snapTo(pos, 0, 0)` faces
        // it south without a level-random draw.
        let Some(child) = self.create_mob_reason(&kind, at, false, SpawnSite::Raw, Some(0.0))
        else {
            return false;
        };
        let Some(id) = self.insert_spawned_mob(child) else {
            return false;
        };
        if let Some(child) = self.common.mobs.iter_mut().find(|m| m.id == id) {
            sunrise_sim::mobs::finalize::spawn_egg_offspring(child, species, variant, color);
            child.extra.wolf_variant = wolf_variant;
            child.extra.pig_variant = pig_variant;
            child.extra.cow_variant = cow_variant;
            child.extra.chicken_variant = chicken_variant;
        }
        // `Mob.checkAndHandleImportantInteractions` then calls
        // `onOffspringSpawnedFromEgg`: `Fox`'s makes the baby trust the player
        // who used the egg (a fox egg used on a block trusts nobody).
        if species == Species::Fox {
            let spawner = self.acting_player_uuid();
            if let Some(child) = self.common.mobs.iter_mut().find(|m| m.id == id) {
                sunrise_sim::mobs::fox::add_trusted_entity(child, spawner);
            }
        }
        self.trigger_item_entity("minecraft:player_interacted_with_entity", &stack, &target);
        self.consume_held();
        // `Mob.interact`: the egg's `SUCCESS_SERVER` consumes the click, so the
        // interaction posts `ENTITY_INTERACT` with the player, at the mob.
        let ctx = self.who_context(Some(Who::Player), None);
        self.post_game_event("minecraft:entity_interact", at, ctx);
        true
    }
    /// `SpawnEggItem.use`: the SOURCE_ONLY ray must hit a liquid block.
    pub(in crate::game) fn spawn_egg_fluid_hit(
        &self,
    ) -> Option<sunrise_gameplay_common::game::Hit> {
        let hit = self.bucket_hit()?;
        matches!(
            &*self.block_name(hit.pos),
            "minecraft:water" | "minecraft:lava"
        )
        .then_some(hit)
    }

    /// `EntityType.updateCustomEntityTag`: the egg's `minecraft:entity_data` on
    /// the mob it spawned, once the guards let it through, as
    /// `TypedEntityData.loadInto`: the mob's own compound, the data merged over
    /// it, loaded back into the same mob (`/data merge entity`'s loader).
    pub(in crate::game) fn apply_spawn_egg_entity_data(
        &mut self,
        id: u32,
        data: &sunrise_sim::block_entity_item::ItemEntityData<'_>,
    ) {
        use sunrise_core::nbt::Tag;
        // A type-only component has no custom fields to apply.
        if data.compound.len() == 1 || !self.acting_player_is_op() && data.op_only() {
            return;
        }
        let Some(i) = self
            .common
            .mobs
            .iter()
            .position(|m| m.id == id && m.kind.id == data.kind)
        else {
            return;
        };
        if let Tag::Compound(mut tag) = self.mob_tree_nbt(i) {
            sunrise_sim::data_cmd::merge_compound(&mut tag, data.compound);
            // Nothing to report: the mob's own compound always loads back.
            let _ =
                self.apply_entity_nbt(crate::game::commands::Target::Mob(id), Tag::Compound(tag));
        }
    }
}

include!("mob_query_hooks.rs");

impl crate::game::ServerGame<'_> {
    /// `EventHooks.canEntityGrief(level, entity)`: the `mob_griefing` rule,
    /// which a mod may change for `entity` (`EntityMobGriefingEvent`). No
    /// entity: the rule alone, as NeoForge.
    pub fn can_entity_grief(&self, entity: Option<u32>) -> bool {
        let rule = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::MOB_GRIEFING);
        entity.map_or(rule, |entity| self.entity_grief(entity, rule))
    }
    /// Whether a large fireball's blast lights fire (`LargeFireball.onHit`):
    /// [`Self::can_entity_grief`] for its owner, as `getOwner()` gives it. That
    /// is the mob that shot it, or the player who owns it (the guest on `guest`,
    /// else the player in place). An owner that is gone, or none, reads the rule.
    pub fn large_fireball_fire(&self, owner: Option<Who>, guest: Option<u32>) -> bool {
        self.can_entity_grief(match owner {
            Some(Who::Mob(id)) => Some(id),
            Some(Who::Player) => self
                .player_entity_id_by_connection(guest)
                .map(|id| id as u32),
            None => None,
        })
    }
    /// Whether a mod may answer `can_entity_grief` differently from the rule:
    /// the sites that skip their whole work while griefing is off skip it only
    /// when no mod listens.
    pub(in crate::game) fn grief_hooked(&self) -> bool {
        self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::EntityMobGriefing)
    }
    /// [`Self::can_entity_grief`] for `entity`, with the rule already read.
    pub(in crate::game) fn entity_grief(&self, entity: u32, rule: bool) -> bool {
        if !self.grief_hooked() {
            return rule;
        }
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            return rule;
        };
        let mut can_grief = rule;
        self.native_entity_mob_griefing(mod_api::EntityRef::new(epoch, entity), &mut can_grief);
        can_grief
    }
}

impl sunrise_sim::entities::arrows::OwnerLookup for ServerGame<'_> {
    fn non_player_entity(&self, uuid: [i32; 4]) -> bool {
        let found = self.target_by_uuid(sunrise_sim::lan::playerdata::ints_uuid(uuid));
        !matches!(
            found,
            None | Some(
                crate::game::commands::Target::Player | crate::game::commands::Target::Guest(_)
            )
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "pending_birth_matrix_tests_server.rs"]
pub mod pending_birth_matrix_tests;

#[cfg(test)]
#[path = "death_loot_context_tests_server.rs"]
pub(in crate::game) mod death_loot_context_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "projectile_targets_tests_server.rs"]
pub mod projectile_targets_tests;

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::entity_sections::EntitySectionsGameExt1 as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::native_ticks::NativeTicksGameExt1 as _;

use sunrise_gameplay_common::game::pick_ray::PickRayGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod projectile_block_effects_tests;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static PROJECTILE_HANDBACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
pub use borrowed_tick::BORROWED_MOB_YIELDS;
