//! Enchantment effects on gameplay, read from the data-driven effect
//! components: efficiency and aqua affinity (`Player.getDestroySpeed`),
//! unbreaking (`minecraft:item_damage`), sharpness/smite/bane and power
//! (`minecraft:damage`), knockback, fire aspect, infinity (`minecraft:ammo_use`),
//! thorns (`minecraft:post_attack`), mending (`minecraft:repair_with_xp`) and
//! the armor attribute effects (respiration, depth strider, ...), Soul
//! Speed's `location_changed` and `tick` effects and the
//! `minecraft:location_changed` disk effect (frost walker).
use super::commands::Target;
use super::durability::Wearer;
use crate::game::ServerGame;
use crate::game::server_side::lan_actions_server::CombatPlayer;
use mod_api::EquipmentSlot;
use serde_json::Value;
use sunrise_core::{
    geometry::Vec3,
    jmath::round_f32,
    position::BlockPos,
    random::{LegacyRandom, RandomSource},
};
use sunrise_gameplay_common::game::{
    commands_common::CommandsCommonGameExt2 as _,
    native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _, tables::WorldTablesExt as _,
    weather::WeatherGameExt as _,
};
use sunrise_items::{
    enchant::{EffectContext, Enchantments, EntityEffect, level_value},
    items::{ItemStack, Items},
};
use sunrise_level::random_source;
use sunrise_sim::health::{Attack, HurtSource};
use sunrise_sim::mobs::{Damage, DamageKind, Who};
use sunrise_world::{fluid::FluidKind, level::LevelAccess, registry::BlockStateId};

fn stand_by_hint(
    stands: &[sunrise_sim::entities::ArmorStand],
    hint: usize,
    id: u32,
) -> Option<&sunrise_sim::entities::ArmorStand> {
    if stands.get(hint).is_some_and(|s| s.id == id) {
        return stands.get(hint);
    }
    stands.iter().find(|s| s.id == id)
}

fn stand_by_hint_mut(
    stands: &mut sunrise_sim::join_seq::JoinList<sunrise_sim::entities::ArmorStand>,
    hint: usize,
    id: u32,
) -> Option<&mut sunrise_sim::entities::ArmorStand> {
    if stands.get(hint).is_some_and(|s| s.id == id) {
        return stands.get_mut(hint);
    }
    stands.iter_mut().find(|s| s.id == id)
}

/// `EquipmentSlot.getName` of each `mobs::finalize` slot (hands, feet to head).
pub(in crate::game) const MOB_SLOTS: [&str; 6] =
    ["mainhand", "offhand", "feet", "legs", "chest", "head"];

/// One of the player's equipment slots (`EnchantedItemInUse.inSlot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum PlayerSlot {
    Main,
    Off,
    /// An `armor` index (head first).
    Armor(usize),
}

/// `EquipmentSlot.VALUES` over what the player holds and wears: the main
/// hand, the off hand, then feet, legs, chest and head (`armor` holds head
/// first).
pub(in crate::game) const PLAYER_SLOTS: [PlayerSlot; 6] = [
    PlayerSlot::Main,
    PlayerSlot::Off,
    PlayerSlot::Armor(3),
    PlayerSlot::Armor(2),
    PlayerSlot::Armor(1),
    PlayerSlot::Armor(0),
];

impl PlayerSlot {
    /// `EquipmentSlot.getName`.
    pub(in crate::game::server_side) fn name(self) -> &'static str {
        match self {
            Self::Main => "mainhand",
            Self::Off => "offhand",
            Self::Armor(i) => ["head", "chest", "legs", "feet"][i],
        }
    }
}

/// `ApplyEntityImpulse`'s impulse for an entity looking along `yaw` and
/// `pitch`: the effect's `direction` turned by the look
/// (`Entity.getLookQuaternion`: pitch about x, then yaw about y), scaled by
/// `coordinate_scale` and the level's `magnitude`.
pub(in crate::game) fn look_impulse(effect: &Value, level: u32, yaw: f32, pitch: f32) -> Vec3 {
    let vec3 = |key: &str| [0, 1, 2].map(|i| effect[key][i].as_f64().unwrap_or(0.0));
    let (d, scale) = (vec3("direction"), vec3("coordinate_scale"));
    let magnitude = f64::from(level_value(&effect["magnitude"], level));
    // JOML Quaternionf.rotationY(...).rotateX(...).transform(Vector3d):
    // quaternion and matrix coefficients round to float before multiplying
    // the double direction. Do not replace this with double Euler rotation.
    let half_y = (-yaw * (std::f32::consts::PI / 180.0)) * 0.5;
    let half_x = (pitch * (std::f32::consts::PI / 180.0)) * 0.5;
    let sy = f64::from(half_y).sin() as f32;
    let sx = f64::from(half_x).sin() as f32;
    let cy = sunrise_assets::math::cos_from_sin(sy, half_y);
    let cx = sunrise_assets::math::cos_from_sin(sx, half_x);
    let (x, y, z, w) = (cy * sx, sy * cx, -sy * sx, cy * cx);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz, xw, yw, zw) = (x * y, x * z, y * z, x * w, y * w, z * w);
    let matrix = [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - zw), 2.0 * (xz + yw)],
        [2.0 * (xy + zw), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - xw)],
        [2.0 * (xz - yw), 2.0 * (yz + xw), 1.0 - 2.0 * (xx + yy)],
    ];
    let result = matrix
        .map(|row| f64::from(row[0]) * d[0] + f64::from(row[1]) * d[1] + f64::from(row[2]) * d[2]);
    Vec3::new(
        result[0] * scale[0] * magnitude,
        result[1] * scale[1] * magnitude,
        result[2] * scale[2] * magnitude,
    )
}
/// `Player.getDestroySpeed` multiplies by `BLOCK_BREAK_SPEED`; a player
/// without the attribute keeps the vanilla default of 1.0.
#[allow(dead_code)]
pub(in crate::game::server_side) fn block_break_speed_factor(value: Option<f64>) -> f32 {
    value.map_or(1.0, |value| value as f32)
}

/// Says once per item that its parts' `xp_repair_ratio` answered `ratio`,
/// which isn't finite.
pub(in crate::game::server_side) fn refuse_xp_repair_ratio(item: &str, ratio: f32) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item.to_owned())
    {
        log::warn!(
            "Sunrise: an xp_repair_ratio part of {item} answered {ratio}, which isn't finite; Mending's own repair is used"
        );
    }
}

struct PostAttack<'a> {
    victim: u32,
    kind: &'a str,
    attack: &'a Attack,
    source: HurtSource<'a>,
}

pub(in crate::game) struct EntityEffectUse<'a> {
    pub stack: &'a ItemStack,
    pub holder: Option<u32>,
    pub slot: Option<EquipmentSlot>,
    pub affected: u32,
    pub level: u32,
    pub position: Option<[f64; 3]>,
    pub attack: Option<(u32, HurtSource<'a>)>,
}

impl ServerGame<'_> {
    /// The wear `ItemStack.hurtAndBreak` goes on to apply for `amount` points
    /// on the stack `wearer` wears or uses: what Unbreaking leaves
    /// (`EnchantmentHelper.processDurabilityChange`), then what the item's
    /// parts make of it (`IItemExtension.damageItem`).
    pub(in crate::game) fn durability_loss(
        &mut self,
        stack: &ItemStack,
        amount: u32,
        wearer: Wearer,
    ) -> u32 {
        let loss = self.unbreaking_loss(stack, amount);
        if loss == 0 || !self.common.is_logical_server() {
            return loss;
        }
        // One table lookup tells whether any part of this item listens; with none,
        // the stack is not converted and no host is made.
        if !mod_api::__private::item_listens(mod_api::HookId::DamageItem, &stack.item) {
            return loss;
        }
        self.damage_item_loss(stack, loss, wearer)
    }
    /// `EnchantmentHelper.processDurabilityChange` (unbreaking) for `amount` points.
    pub(in crate::game::server_side) fn unbreaking_loss(
        &mut self,
        stack: &ItemStack,
        amount: u32,
    ) -> u32 {
        let e = &server_world_stations!(self).enchantments;
        if !e.has_component(stack, "minecraft:item_damage") {
            return amount;
        }
        let ctx = EffectContext {
            tool: Some(&stack.item),
            items: Some(server_world_items!(self)),
            ..Default::default()
        };
        e.modify_value(
            Some(stack),
            "minecraft:item_damage",
            &ctx,
            &mut self.common.world.random,
            amount as f32,
        )
        .max(0.0) as u32
    }
    /// `ItemStack.hurtAndBreak` for the player's `stack`: the wear Unbreaking
    /// leaves, and `ItemDurabilityTrigger` when some is left (`applyDamage`).
    pub fn wear(&mut self, stack: &ItemStack, amount: u32) -> u32 {
        // ItemStack.isDamageableItem: an unbreakable stack, or one without
        // damage/max_damage, takes no wear before Unbreaking or mods are asked.
        let component = |id| stack.data().and_then(|data| data.other(id));
        if component("minecraft:unbreakable").is_some()
            || component("!minecraft:max_damage").is_some()
            || component("!minecraft:damage").is_some()
        {
            return 0;
        }
        let loss = self.durability_loss(stack, amount, Wearer::Player);
        if loss > 0 {
            self.trigger_item_durability(stack, (stack.damage as i32).wrapping_add(loss as i32));
        }
        loss
    }

    /// `getEnchantedDamage - base`: damage bonus of the held weapon against a mob type.
    pub(in crate::game::server_side) fn enchant_damage_bonus(
        &mut self,
        target: &str,
        victim: Option<u32>,
    ) -> f32 {
        self.enchanted_damage(target, 0.0, victim)
    }
    /// `Player.getEnchantedDamage`: `base` run through the held weapon's
    /// damage effects against a mob type.
    pub fn enchanted_damage(&mut self, target: &str, base: f32, victim: Option<u32>) -> f32 {
        let Some(held) = self
            .common
            .held()
            .map(|stack| self.common.item_combat_stack(stack))
        else {
            return base;
        };
        let actors = self
            .common
            .player
            .as_ref()
            .and_then(|player| victim.map(|victim| (player.entity_id as u32, victim)));
        self.weapon_enchanted_damage(
            &held,
            actors,
            target,
            &self.common.stack_melee_damage_type(
                Some(&held),
                self.common
                    .player
                    .as_ref()
                    .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying)),
                true,
            ),
            base,
            self.common
                .player
                .as_ref()
                .map(|p| (p.fall_distance, p.fall_flying)),
        )
    }

    /// Evaluate a pre-hit weapon snapshot with actual actor identities supplied by the hit.
    pub(in crate::game::server_side) fn weapon_enchanted_damage(
        &mut self,
        held: &ItemStack,
        actors: Option<(u32, u32)>,
        target: &str,
        damage_type: &str,
        base: f32,
        direct_motion: Option<(f64, bool)>,
    ) -> f32 {
        let ctx = EffectContext {
            this: Some(target),
            attacker: Some("minecraft:player"),
            direct_attacker: Some("minecraft:player"),
            is_direct: true,
            damage_type: Some(damage_type),
            direct_motion,
            attacker_motion: direct_motion,
            items: None,
            ..Default::default()
        };
        self.evaluate_weapon_damage(held, actors.map(|(_, victim)| victim), ctx, base)
    }

    pub(in crate::game::server_side) fn evaluate_weapon_damage(
        &mut self,
        held: &ItemStack,
        victim: Option<u32>,
        ctx: EffectContext<'_>,
        base: f32,
    ) -> f32 {
        self.evaluate_weapon_value(held, "minecraft:damage", victim, ctx, base)
    }

    pub(in crate::game) fn evaluate_weapon_value(
        &mut self,
        held: &ItemStack,
        component: &str,
        victim: Option<u32>,
        mut ctx: EffectContext<'_>,
        base: f32,
    ) -> f32 {
        let levels = held.all_enchantments();
        if levels.is_empty() {
            return base;
        }
        let enchantments = self.stations().enchantments.clone();
        let mut level_random = self.common.world.random.clone();
        let mut fallback = (*level_random).clone();
        let authoritative = self.common.is_logical_server();
        if authoritative && ctx.this_motion.is_none() {
            ctx.this_motion = victim
                .and_then(|id| self.enchantment_actor(id as i32))
                .and_then(|actor| actor.motion);
        }
        let mut evaluate = |victim_random: &mut LegacyRandom| {
            enchantments.modify_entity_value(
                &levels,
                component,
                &ctx,
                &mut level_random,
                victim_random,
                base,
            )
        };
        let value = if authoritative && let Some(id) = victim {
            self.with_damage_victim_random(id, |random| evaluate(random))
                .unwrap_or_else(|| evaluate(&mut fallback))
        } else {
            evaluate(&mut fallback)
        };
        self.common.world.random = level_random;
        value
    }

    pub(in crate::game) fn enchantment_actor(
        &self,
        id: i32,
    ) -> Option<sunrise_sim::health::EnchantmentActor> {
        use sunrise_sim::health::EnchantmentActor;
        use sunrise_sim::join_seq::residency::table::EntityLookup;
        if id == 0 {
            return None;
        }
        if let Some(player) = self.combat_player(id as u32) {
            let motion = match player {
                CombatPlayer::InPlace => self.common.player.as_ref(),
                CombatPlayer::Displaced(_, body) => Some(body),
                CombatPlayer::Parked(guest) => guest.motion.as_deref(),
            };
            return Some(EnchantmentActor {
                id,
                kind: "minecraft:player".into(),
                motion: motion.map(|p| (p.fall_distance, p.fall_flying || p.flying)),
            });
        }
        if let Some(index) = self.common.mob_index.find_ref(&self.common.mobs, id as u32) {
            let mob = &self.common.mobs[index];
            return Some(EnchantmentActor {
                id,
                kind: mob.kind.id.clone().into(),
                motion: Some((mob.fall_distance, mob.extra.fall_flying)),
            });
        }
        let EntityLookup::Found { value, .. } = self.server_resident_entity(id as u32) else {
            return None;
        };
        Some(EnchantmentActor {
            id,
            kind: self.target_type(value).into(),
            motion: None,
        })
    }

    /// `Entity.getRandom` for the body that the damage effect is about: runs `effect` on it.
    fn with_damage_victim_random<R>(
        &mut self,
        id: u32,
        effect: impl FnOnce(&mut LegacyRandom) -> R,
    ) -> Option<R> {
        match self.combat_player(id) {
            Some(CombatPlayer::InPlace) => {
                return self.common.player.as_mut().map(|p| effect(&mut p.random));
            }
            Some(CombatPlayer::Displaced(depth, _)) => {
                return self.server.lan_displaced_players.0[depth]
                    .body
                    .as_mut()
                    .map(|p| effect(&mut p.random));
            }
            Some(CombatPlayer::Parked(guest)) => {
                let conn = guest.conn;
                return self
                    .server
                    .lan
                    .as_mut()?
                    .hub
                    .guests
                    .motion_mut(conn)
                    .map(|p| effect(&mut p.random));
            }
            None => {}
        }
        if let Some(index) = self.common.mob_index.find(&self.common.mobs, id) {
            return self
                .common
                .mobs
                .get_mut(index)
                .map(|mob| effect(&mut mob.random));
        }
        if let Some(dragon) = self.common.dragon.as_mut().filter(|dragon| dragon.id == id) {
            return Some(effect(&mut dragon.random));
        }
        if let Some(stand) = self
            .common
            .armor_stands
            .iter_mut()
            .find(|stand| stand.id == id)
        {
            return Some(effect(&mut stand.random));
        }
        if let Some(index) = self.common.boats.iter().position(|boat| boat.id == id) {
            return Some(
                self.common
                    .boats
                    .update(index, |boat| effect(&mut boat.random)),
            );
        }
        let index = self.common.carts.iter().position(|cart| cart.id == id)?;
        Some(
            self.common
                .carts
                .update(index, |cart| effect(&mut cart.random)),
        )
    }
    /// `Player.attack`'s extra knockback: `getKnockback` (the ATTACK_KNOCKBACK
    /// attribute, plus the weapon's knockback enchantment, halved) plus 0.5 for a
    /// sprinting full-strength hit, and whether the hit is that knockback attack.
    pub(in crate::game::server_side) fn extra_knockback(
        &mut self,
        target: &str,
        victim: u32,
        scale: f32,
        source: Option<&str>,
    ) -> (f32, bool) {
        let sprinting = self.common.player.as_ref().is_some_and(|p| p.sprinting);
        let base = self.player_attack_knockback();
        let enchanted = if self.common.is_logical_server()
            && let Some(held) = self.common.held().cloned()
        {
            let attack = self.attack_of(self.own_entity_id());
            let damage_type = source.map(str::to_owned).unwrap_or_else(|| {
                self.common.stack_melee_damage_type(
                    Some(&held),
                    self.common
                        .player
                        .as_ref()
                        .map_or((0.0, false), |p| (p.fall_distance, p.fall_flying)),
                    true,
                )
            });
            // Melee's direct and causing entities are the attacker even when the item overrides the type.
            let source = HurtSource {
                damage_type: &damage_type,
                causing: attack.attacker,
                direct: attack.attacker,
                direct_type: attack.direct_type.as_deref(),
                position: None,
            };
            self.evaluate_weapon_value(
                &held,
                "minecraft:knockback",
                Some(victim),
                attack.enchantment_context(source, target),
                base,
            )
        } else {
            base
        };
        sunrise_sim::attack_strength::extra_knockback(enchanted, scale, sprinting)
    }

    pub(in crate::game) fn player_attack_knockback(&mut self) -> f32 {
        self.common
            .player
            .as_mut()
            .and_then(|p| p.attribute("minecraft:attack_knockback").ok())
            .unwrap_or(0.0) as f32
    }
    /// `causeExtraKnockback`'s direction, `(sin yRot, -cos yRot)` from Mth's
    /// sine table: the target flies where the attacker looks, not away from
    /// it (straight along an axis when the attacker faces one).
    pub(in crate::game::server_side) fn extra_knockback_direction(&self) -> (f64, f64) {
        use sunrise_core::geometry::trig;
        let yaw = f64::from(self.common.yaw * trig::DEG_TO_RAD);
        (f64::from(trig::sin(yaw)), -f64::from(trig::cos(yaw)))
    }
    /// The accepted hit's post-attack continuation. Equipment is visited before the
    /// attacking item, and each requirement runs immediately before its own effect.
    pub(in crate::game) fn post_attack(
        &mut self,
        victim: u32,
        kind: &str,
        attack: &Attack,
        source: HurtSource<'_>,
        weapon: Option<&ItemStack>,
        attackerless_on_break: bool,
    ) {
        let actors = PostAttack {
            victim,
            kind,
            attack,
            source,
        };
        if let Some(target) = self.post_attack_target(victim) {
            for slot in EquipmentSlot::ALL {
                let stack = self
                    .equipment_stack_for(target, slot)
                    .map(|stack| stack.project().into_owned());
                if let Some(stack) = stack {
                    self.post_attack_item(&actors, &stack, Some(victim), Some(slot), "victim");
                }
            }
        }
        if let Some(weapon) = weapon {
            let holder = self
                .post_attack_target(source.causing as u32)
                .filter(|target| {
                    matches!(
                        target,
                        Target::Player
                            | Target::Guest(_)
                            | Target::Mob(_)
                            | Target::Dragon(_)
                            | Target::Stand(_)
                    )
                });
            if holder.is_some() {
                self.post_attack_item(
                    &actors,
                    weapon,
                    Some(source.causing as u32),
                    Some(EquipmentSlot::MainHand),
                    "attacker",
                );
            } else if attackerless_on_break {
                self.post_attack_item(&actors, weapon, None, None, "attacker");
            }
        }
    }

    pub(in crate::game) fn post_player_melee(
        &mut self,
        victim: u32,
        kind: &str,
        damage_type: &str,
        weapon: Option<&ItemStack>,
    ) {
        let attack = self.attack_of(self.own_entity_id());
        let source = attack.player_source(damage_type, attack.attacker);
        self.post_attack(victim, kind, &attack, source, weapon, false);
    }

    pub(in crate::game) fn post_attack_target(&self, id: u32) -> Option<Target> {
        if id == 0 {
            return None;
        }
        match self.combat_player(id) {
            Some(CombatPlayer::InPlace) => Some(Target::Player),
            Some(CombatPlayer::Parked(guest)) => Some(Target::Guest(guest.conn)),
            Some(CombatPlayer::Displaced(..)) => None,
            None => self.level_entity(id as i32),
        }
    }

    fn post_attack_item(
        &mut self,
        actors: &PostAttack<'_>,
        stack: &ItemStack,
        holder: Option<u32>,
        slot: Option<EquipmentSlot>,
        enchanted: &str,
    ) {
        let levels = stack.all_enchantments();
        let enchantments = self.stations().enchantments.clone();
        for (id, level) in levels.iter() {
            let Some(def) = enchantments.get(id) else {
                continue;
            };
            if let Some(slot) = slot {
                let name = sunrise_items::equipment::SLOT_NAMES[slot as usize];
                if !def.matches_slot(name) {
                    continue;
                }
            }
            for (index, entry) in def.effects["minecraft:post_attack"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                if entry["enchanted"] != enchanted {
                    continue;
                }
                let mut ctx = actors
                    .attack
                    .enchantment_context(actors.source, actors.kind);
                if entry.get("requirements").is_some() {
                    ctx.weather = Some(self.enchantment_weather());
                    ctx.sees_sky = self
                        .post_attack_target(actors.victim)
                        .and_then(|target| self.target_position(target))
                        .map(|at| self.enchantment_sees_sky(at));
                }
                if entry.get("requirements").is_some_and(|req| {
                    !enchantments.condition(req, *level, &ctx, &mut self.common.world.random)
                }) {
                    continue;
                }
                let affected = match entry["affected"].as_str() {
                    Some("attacker") => actors.source.causing as u32,
                    Some("damaging_entity") => actors.source.direct as u32,
                    Some("victim") => actors.victim,
                    _ => continue,
                };
                if affected == 0 {
                    continue;
                }
                let effect = &entry["effect"];
                if let Some(bound) = enchantments.entity_effect(id, "minecraft:post_attack", index)
                {
                    let invocation = EntityEffectUse {
                        stack,
                        holder,
                        slot,
                        affected,
                        level: *level,
                        position: None,
                        attack: Some((actors.victim, actors.source)),
                    };
                    self.apply_bound_entity_effect(bound, &invocation, &mut |game, effect| {
                        game.apply_post_attack(effect, *level, affected, holder, slot);
                    });
                } else {
                    self.apply_post_attack(effect, *level, affected, holder, slot);
                }
            }
        }
    }

    pub(in crate::game) fn apply_bound_entity_effect(
        &mut self,
        effect: &EntityEffect,
        invocation: &EntityEffectUse<'_>,
        builtin: &mut impl FnMut(&mut Self, &Value),
    ) {
        match effect {
            EntityEffect::Builtin(effect) => builtin(self, effect),
            EntityEffect::AllOf(effects) => {
                for effect in effects {
                    self.apply_bound_entity_effect(effect, invocation, builtin);
                }
            }
            EntityEffect::Native(effect) => {
                if !self.common.is_logical_server() {
                    return;
                }
                use sunrise_gameplay_common::game::api_values::TryIntoApi;
                let Some(epoch) = self.common.native_lifecycle.epoch() else {
                    return;
                };
                let entity = |id| (id != 0).then(|| mod_api::EntityRef::new(epoch, id));
                let Some(affected) = entity(invocation.affected) else {
                    return;
                };
                let position = invocation.position.or_else(|| {
                    self.post_attack_target(invocation.affected)
                        .and_then(|target| self.target_position(target))
                });
                let Some(position) = position else { return };
                let Ok(item) = invocation.stack.try_into_api() else {
                    return;
                };
                let context = mod_api::EnchantmentEntityEffectContext {
                    level: invocation.level,
                    item: &item,
                    affected,
                    position,
                    holder: invocation
                        .holder
                        .and_then(entity)
                        .map(mod_api::LivingRef::new),
                    slot: invocation.slot,
                    victim: invocation.attack.and_then(|(victim, _)| entity(victim)),
                    causing: invocation
                        .attack
                        .and_then(|(_, source)| entity(source.causing as u32)),
                    direct: invocation
                        .attack
                        .and_then(|(_, source)| entity(source.direct as u32)),
                };
                effect.apply(self, &context);
            }
        }
    }

    pub fn apply_post_attack(
        &mut self,
        effect: &Value,
        level: u32,
        affected: u32,
        holder: Option<u32>,
        slot: Option<EquipmentSlot>,
    ) {
        let target = self.post_attack_target(affected);
        let value = |key: &str| level_value(&effect[key], level);
        match effect["type"]
            .as_str()
            .map(|kind| kind.trim_start_matches("minecraft:"))
        {
            Some("all_of") => {
                for child in effect["effects"].as_array().into_iter().flatten() {
                    self.apply_post_attack(child, level, affected, holder, slot);
                }
            }
            Some("ignite") => {
                let ticks = (value("duration") * 20.0) as i32;
                match target {
                    Some(target @ (Target::Player | Target::Guest(_))) => {
                        self.for_player(target, |game| {
                            if let Some(p) = game.common.player.as_mut() {
                                let _ = p.ignite_for_ticks(ticks);
                            }
                        });
                    }
                    Some(Target::Mob(id)) => {
                        if let Some(mob) = self
                            .common
                            .mobs
                            .iter_mut()
                            .find(|mob| mob.id == id && !mob.fire_immune())
                        {
                            mob.ignite_for_ticks(ticks);
                        }
                    }
                    Some(Target::Stand(id)) => {
                        if let Some(stand) = self
                            .common
                            .armor_stands
                            .iter_mut()
                            .find(|stand| stand.id == id && !stand.mod_fire_immune)
                        {
                            stand.fire_ticks = stand.fire_ticks.max(ticks);
                        }
                    }
                    _ => {}
                }
            }
            Some("apply_mob_effect") => {
                self.apply_post_attack_mob_effect(effect, level, affected, target);
            }
            Some("summon_entity") => {
                if let Some(at) = target.and_then(|target| self.target_position(target)) {
                    self.summon_enchantment_entity(effect, at, holder);
                }
            }
            Some("play_sound") => {
                if let Some(at) = target.and_then(|target| self.target_position(target)) {
                    self.play_enchantment_sound(effect, level, affected, at);
                }
            }
            Some("explode") if effect["block_interaction"] == "trigger" => {
                let Some(feet) = target.and_then(|target| self.target_position(target)) else {
                    return;
                };
                let knockback = if effect["knockback_multiplier"].is_null() {
                    1.0
                } else {
                    value("knockback_multiplier")
                };
                let attributed = holder.filter(|_| effect["attribute_to_user"] == true);
                let cause = super::wind::BurstCause {
                    source: attributed,
                    owner: attributed,
                };
                let _ = self.wind_burst(
                    feet,
                    value("radius"),
                    knockback,
                    super::wind::Source::WIND_BURST.with_cause(cause),
                );
            }
            Some("damage_entity") => {
                let Some(target) = target else {
                    return;
                };
                let (min, max) = (value("min_damage"), value("max_damage"));
                let Some(damage) = self.with_damage_victim_random(affected, |random| {
                    if min >= max {
                        min
                    } else {
                        random.next_float() * (max - min) + min
                    }
                }) else {
                    return;
                };
                let cause = holder.and_then(|id| self.post_attack_target(id));
                let from = cause.and_then(|cause| self.target_position(cause));
                let ty = effect["damage_type"].as_str().unwrap_or("minecraft:thorns");
                if let (Some(Target::Player), Target::Mob(id), "minecraft:thorns") =
                    (cause, target, ty)
                    && let Some(index) = self.common.mob_index.find(&self.common.mobs, id)
                {
                    self.hurt_by_player(
                        index,
                        damage,
                        Damage {
                            kind: DamageKind::Thorns,
                            attacker: Some(Who::Player),
                            from,
                            direct: None,
                            attacker_species: None,
                        },
                    );
                } else {
                    self.hurt_target(target, damage, ty, cause, cause, from);
                }
            }
            Some("change_item_damage") => {
                let (Some(holder), Some(slot)) = (holder, slot) else {
                    return;
                };
                let amount = value("amount") as i32;
                match self.post_attack_target(holder) {
                    Some(target @ (Target::Player | Target::Guest(_))) => {
                        self.for_player(target, |game| game.damage_player_equipment(slot, amount));
                    }
                    Some(Target::Mob(id)) if (slot as usize) < MOB_SLOTS.len() => {
                        self.wear_mob_piece(id, slot as usize, amount.max(0) as u32);
                    }
                    Some(_) => {
                        if let Some(epoch) = self.common.native_lifecycle.epoch() {
                            let living =
                                mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, holder));
                            self.living_hurt_and_break(living, slot, amount);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// Use the causing entity's actual hand for hits with a living wielder.
    pub(in crate::game) fn mob_post_attack(
        &mut self,
        victim: u32,
        attacker: Option<Who>,
        direct: Option<(u32, &str)>,
        damage_type: Option<&str>,
    ) {
        let mut attack = self.attack_of_who(attacker);
        if let Some((id, kind)) = direct {
            attack.enchantment_direct = Some(sunrise_sim::health::EnchantmentActor {
                id: id as i32,
                kind: kind.to_owned().into(),
                motion: None,
            });
        }
        let causing = match attacker {
            Some(_) => attack.attacker,
            None => 0,
        };
        let source = HurtSource {
            damage_type: damage_type.unwrap_or("minecraft:mob_attack"),
            causing,
            direct: direct.map_or(causing, |(id, _)| id as i32),
            direct_type: direct
                .map(|(_, kind)| kind)
                .or(attack.direct_type.as_deref()),
            position: attack.position,
        };
        let Some(target) = self.post_attack_target(victim) else {
            return;
        };
        let kind = self.target_type(target);
        self.post_attack_from_hand(victim, &kind, &attack, source);
    }

    pub(in crate::game) fn post_attack_from_hand(
        &mut self,
        victim: u32,
        kind: &str,
        attack: &Attack,
        source: HurtSource<'_>,
    ) {
        let stack = self
            .post_attack_target(source.causing as u32)
            .and_then(|target| self.equipment_stack_for(target, EquipmentSlot::MainHand))
            .map(|stack| stack.project().into_owned());
        self.post_attack(victim, kind, attack, source, stack.as_ref(), false);
    }

    pub fn thorns(&mut self, attacker: u32) {
        self.mob_post_attack(
            self.own_entity_id() as u32,
            Some(Who::Mob(attacker)),
            None,
            Some("minecraft:mob_attack"),
        );
    }

    /// `EnchantmentHelper.onHitBlock`, at the block-clamped impact point.
    pub(in crate::game) fn hit_block_enchantment_effects(
        &mut self,
        stack: &ItemStack,
        holder: Option<u32>,
        projectile: u32,
        kind: &str,
        at: Vec3,
        block: [i32; 3],
    ) {
        const COMPONENT: &str = "minecraft:hit_block";
        let edge = f64::from(1.0e-5_f32);
        let mut position = at.components();
        for (value, origin) in position.iter_mut().zip(block) {
            *value = value.clamp(f64::from(origin) + edge, f64::from(origin) + 1.0 - edge);
        }
        let table = self.stations().enchantments.clone();
        // AbstractArrow supplies the state it hit, not the state a preceding effect writes.
        let [x, y, z] = block;
        let hit_state = self
            .common
            .pack
            .assets
            .registry
            .state(self.common.world.block(x, y, z));
        for (id, level) in stack.all_enchantments().iter() {
            let Some(entries) = table
                .get(id)
                .and_then(|def| def.effects[COMPONENT].as_array())
            else {
                continue;
            };
            for (index, entry) in entries.iter().enumerate() {
                let Some(bound) = table.entity_effect(id, COMPONENT, index) else {
                    continue;
                };
                let ctx = EffectContext {
                    this: Some(kind),
                    items: Some(server_world_items!(self)),
                    weather: Some(self.enchantment_weather()),
                    sees_sky: Some(self.enchantment_sees_sky(position)),
                    hit_block: hit_state.map(|state| state.name.as_str()),
                    hit_state,
                    ..Default::default()
                };
                if entry
                    .get("requirements")
                    .is_none_or(|r| table.condition(r, *level, &ctx, &mut self.common.world.random))
                {
                    let invocation = EntityEffectUse {
                        stack,
                        holder,
                        slot: None,
                        affected: projectile,
                        level: *level,
                        position: Some(position),
                        attack: None,
                    };
                    self.apply_bound_entity_effect(bound, &invocation, &mut |game, effect| {
                        match effect["type"]
                            .as_str()
                            .map(|s| s.trim_start_matches("minecraft:"))
                        {
                            Some("summon_entity") => {
                                game.summon_enchantment_entity(effect, position, holder)
                            }
                            Some("play_sound") => {
                                game.play_enchantment_sound(effect, *level, projectile, position);
                            }
                            _ => {}
                        }
                    });
                }
            }
        }
    }

    fn enchantment_weather(&self) -> (bool, bool) {
        let (rain, thunder) = self.weather_levels(1.0);
        (rain > 0.2, self.can_have_weather() && thunder > 0.9)
    }

    fn enchantment_sees_sky(&self, at: [f64; 3]) -> bool {
        sunrise_gameplay_common::game::native_world_read_common::can_see_sky(
            &self.common.world,
            at.map(|v| v.floor() as i32),
        )
    }

    /// `SummonEntityEffect`: the Channeling bolt credits the enchanted item's owner.
    fn summon_enchantment_entity(&mut self, effect: &Value, at: [f64; 3], holder: Option<u32>) {
        if effect["entity"] != "minecraft:lightning_bolt" {
            return;
        }
        let player = holder.and_then(|id| match self.combat_player(id)? {
            CombatPlayer::InPlace => Some((
                self.server.lan_acting,
                sunrise_sim::lan::playerdata::ints_uuid(self.acting_player_uuid()),
            )),
            CombatPlayer::Parked(guest) => Some((Some(guest.conn), guest.uuid)),
            CombatPlayer::Displaced(..) => None,
        });
        if self.lightning_strike_allowed(
            player.map(|(_, uuid)| uuid),
            at.map(|v| v.floor() as i32),
            "trident",
        ) {
            self.spawn_bolt(at, false);
            if let Some(last) = self.common.bolts.len().checked_sub(1) {
                self.common
                    .bolts
                    .update(last, |bolt| bolt.cause = player.map(|(conn, _)| conn));
            }
        }
    }

    /// `EnchantmentHelper.doPostPiercingAttackEffects`: the
    /// `minecraft:post_piercing_attack` effects of the weapon in hand after a
    /// jab (Lunge's dash).
    pub(in crate::game::server_side) fn post_piercing_attack(&mut self) {
        let Some(held) = self.common.held().cloned() else {
            return;
        };
        const COMPONENT: &str = "minecraft:post_piercing_attack";
        let table = self.stations().enchantments.clone();
        for (id, level) in held.all_enchantments().iter() {
            let level = *level;
            let Some(entries) = table
                .get(id)
                .filter(|def| def.matches_slot("mainhand"))
                .and_then(|def| def.effects[COMPONENT].as_array())
            else {
                continue;
            };
            for (index, entry) in entries.iter().enumerate() {
                let Some(p) = self.common.player.as_ref() else {
                    return;
                };
                let position = p.position.components();
                let actor = p.entity_id as u32;
                let applies = if id.as_ref() == "minecraft:lunge" {
                    self.common.riding.is_none()
                        && !p.fall_flying
                        && !p.fluids.was_touching_water
                        && (self.common.creative || p.health.food.food >= 7)
                } else {
                    let ctx = EffectContext {
                        this: Some("minecraft:player"),
                        this_motion: Some((p.fall_distance, p.fall_flying || p.flying)),
                        items: Some(server_world_items!(self)),
                        ..Default::default()
                    };
                    entry.get("requirements").is_none_or(|r| {
                        table.condition(r, level, &ctx, &mut self.common.world.random)
                    })
                };
                if !applies {
                    continue;
                }
                if let Some(bound) = table.entity_effect(id, COMPONENT, index) {
                    let invocation = EntityEffectUse {
                        stack: &held,
                        holder: Some(actor),
                        slot: Some(EquipmentSlot::MainHand),
                        affected: actor,
                        level,
                        position: Some(position),
                        attack: None,
                    };
                    self.apply_bound_entity_effect(bound, &invocation, &mut |game, effect| {
                        game.apply_piercing_effect(effect, level)
                    });
                } else {
                    self.apply_piercing_effect(&entry["effect"], level);
                }
            }
        }
    }

    /// `ApplyMobEffect.apply`: the affected entity's own random chooses the
    /// effect, then its duration and amplifier.
    fn apply_post_attack_mob_effect(
        &mut self,
        effect: &Value,
        level: u32,
        affected: u32,
        target: Option<Target>,
    ) {
        let ids: Vec<&str> = match &effect["to_apply"] {
            Value::String(id) => vec![id.as_str()],
            Value::Array(list) => list.iter().filter_map(Value::as_str).collect(),
            _ => return,
        };
        if ids.is_empty() {
            return;
        }
        let Some((pick, ticks, amplifier)) = self.with_damage_victim_random(affected, |random| {
            let pick = random.next_int_bound(ids.len() as i32).unwrap_or(0) as usize;
            let mut between = |min: &str, max: &str| {
                let (lo, hi) = (
                    level_value(&effect[min], level),
                    level_value(&effect[max], level),
                );
                if lo >= hi {
                    lo
                } else {
                    random.next_float() * (hi - lo) + lo
                }
            };
            let ticks = round_f32(between("min_duration", "max_duration") * 20.0);
            let amplifier = round_f32(between("min_amplifier", "max_amplifier")).max(0);
            (pick, ticks, amplifier)
        }) else {
            return;
        };
        let Some(def) = sunrise_level::brewing::effect(ids[pick]) else {
            return;
        };
        let effect = sunrise_level::effects::EffectInstance::from_def(def, ticks, amplifier);
        match target {
            Some(Target::Mob(id)) => {
                self.add_mob_effect(id, effect, None);
            }
            Some(target @ (Target::Player | Target::Guest(_))) => {
                self.for_player(target, |game| game.add_player_effect_from(effect, None));
            }
            _ => {}
        }
    }
    /// `ItemStack.hurtAndBreak` with no player for what the mob `id` has in
    /// `slot`: Unbreaking takes its share from the level's random, and a
    /// piece worn out leaves the slot with its break event
    /// (`LivingEntity.onEquippedItemBroken`).
    pub fn wear_mob_piece(&mut self, id: u32, slot: usize, amount: u32) {
        use sunrise_sim::mobs::finalize as f;
        let (at, stack, is_stand) = if let Some(m) = self.common.mobs.iter().find(|m| m.id == id) {
            let at = [m.position.x, m.position.y, m.position.z];
            let Some(stack) = f::item_stack(m, slot).filter(|s| {
                amount > 0 && sunrise_sim::stations::is_damageable(server_world_items!(self), s)
            }) else {
                return;
            };
            (at, stack, false)
        } else if let Some(a) = self.common.armor_stands.iter().find(|a| a.id == id) {
            let at = [a.position.x, a.position.y, a.position.z];
            let Some(stack) = a
                .equipment
                .get(slot)
                .filter(|s| {
                    amount > 0 && sunrise_sim::stations::is_damageable(server_world_items!(self), s)
                })
                .cloned()
            else {
                return;
            };
            (at, stack, true)
        } else {
            return;
        };
        let loss = self.durability_loss(&stack, amount, Wearer::Living(id));
        if loss == 0 {
            return;
        }
        let damage = stack.damage + loss;
        let max = server_world_items!(self)
            .get(&stack.item)
            .map_or(0, |i| i.max_damage);
        if is_stand {
            let Some(a) = self.common.armor_stands.iter_mut().find(|a| a.id == id) else {
                return;
            };
            if damage < max {
                if let Some(s) = a.equipment[slot].as_mut() {
                    s.damage = damage;
                }
                return;
            }
            a.equipment.set(slot, None);
        } else {
            let Some(m) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                return;
            };
            if damage < max {
                f::set_slot_damage(m, slot, damage);
                return;
            }
            sunrise_sim::mobs::drops::clear_slot(m, MOB_SLOTS[slot]);
        }
        // LivingEntity.entityEventForEquipmentBreak
        let event = [47, 48, 52, 51, 50, 49][slot];
        self.lan_entity_event(id as i32, event, at);
    }
    /// `ExperienceOrb.repairPlayerItems`: returns the experience left over.
    pub fn mending(&mut self, amount: i32) -> i32 {
        let mut amount = amount;
        let player = (self.server.serving.is_some() || self.common.options.dedicated.is_some())
            .then(|| self.native_acting_id());
        loop {
            // EnchantmentHelper.getRandomItemWith(REPAIR_WITH_XP, damaged), in
            // EquipmentSlot order, drawn from the player's own random.
            let slots = PLAYER_SLOTS.map(|slot| (slot, self.player_slot(slot)));
            let Some(slot) = pick_mending_slot(
                slots,
                &server_world_stations!(self).enchantments,
                &mut random_source::entity(),
            ) else {
                return amount;
            };
            let Some(mut stack) = self.player_slot(slot).clone() else {
                return amount;
            };
            let vanilla_repair = repair_from_xp(
                &server_world_stations!(self).enchantments,
                Some(server_world_items!(self)),
                &stack,
                amount,
                &mut self.common.world.random,
            );
            let to_repair_xp = self.xp_repair(player, &stack, amount.max(0) as u32, vanilla_repair);
            // No re-check that the slot still holds the picked stack, eligible
            // and damaged: nothing between the pick and here can change it. The
            // `xp_repair_ratio` parts are asked through `&self` with a read-only
            // host, and the `repair_with_xp` value only draws from the world's
            // random. The repair is written back as the picked stack. A path that
            // could change the slot in this window must bring the re-check back.
            if to_repair_xp == 0 {
                // Vanilla consumes the XP when its own repair effect returns zero.
                return if vanilla_repair == 0 { 0 } else { amount };
            }
            let (repaired, remaining) =
                repaired_and_left(amount.max(0) as u32, stack.damage, to_repair_xp);
            if repaired == 0 {
                return amount;
            }
            let remaining = remaining as i32;
            stack.damage -= repaired;
            let notice = player
                .filter(|_| mod_api::__private::listened(mod_api::HookId::PlayerItemMend))
                .and_then(|uuid| {
                    let player = self
                        .native_server_players()
                        .into_iter()
                        .find(|p| p.uuid() == uuid)?;
                    let name = self.player_name(player)?.to_owned();
                    Some((uuid, name, stack.item.to_string()))
                });
            *self.player_slot_mut(slot) = Some(stack);

            if let Some((player, name, item)) = notice {
                self.native_player_item_mend(player, &name, &item, repaired, amount.max(0) as u32);
            }
            if remaining == 0 {
                return 0;
            }
            amount = remaining;
        }
    }
    /// The repair Mending gives `stack` for `experience` points: `vanilla`
    /// (its `minecraft:repair_with_xp` repair) as the item's parts scale it
    /// (`IItemExtension.getXpRepairRatio`), `floor(experience × ratio)` within
    /// `0..=i32::MAX`. A ratio left as it arrived keeps `vanilla` exactly, and
    /// one that isn't finite is refused for it. `player` is the acting player,
    /// `None` when no server runs (no part is asked then).
    pub(in crate::game::server_side) fn xp_repair(
        &self,
        player: Option<u128>,
        stack: &ItemStack,
        experience: u32,
        vanilla: u32,
    ) -> u32 {
        let Some(player) = player else {
            return vanilla;
        };
        // One table lookup tells whether any part of this item listens; with none,
        // the stack is not converted and no host is made.
        if experience == 0
            || !mod_api::__private::item_listens(mod_api::HookId::XpRepairRatio, &stack.item)
        {
            return vanilla;
        }
        let arrived = vanilla as f32 / experience as f32;
        let mut ratio = arrived;
        self.native_xp_repair_ratio(player, stack, experience, &mut ratio);
        if ratio.to_bits() == arrived.to_bits() {
            return vanilla;
        }
        if !ratio.is_finite() {
            refuse_xp_repair_ratio(&stack.item, ratio);
            return vanilla;
        }
        // `as` saturates: a negative product repairs nothing.
        (f64::from(experience) * f64::from(ratio))
            .floor()
            .min(f64::from(i32::MAX)) as u32
    }
    /// `LivingEntity.getItemBySlot` for the player.
    pub(in crate::game::server_side) fn player_slot(&self, slot: PlayerSlot) -> &Option<ItemStack> {
        match slot {
            PlayerSlot::Main => &self.common.inventory[self.common.selected],
            PlayerSlot::Off => &self.common.offhand,
            PlayerSlot::Armor(i) => &self.common.armor[i],
        }
    }
    pub(in crate::game::server_side) fn player_slot_mut(
        &mut self,
        slot: PlayerSlot,
    ) -> &mut Option<ItemStack> {
        match slot {
            PlayerSlot::Main => &mut self.common.inventory[self.common.selected],
            PlayerSlot::Off => &mut self.common.offhand,
            PlayerSlot::Armor(i) => &mut self.common.armor[i],
        }
    }

    /// `ProjectileWeaponItem.draw`: count on the shooter, then ammo use on its stack.
    /// The caller debits the selected slot; copies are made before the first shot spends it.
    pub(in crate::game) fn draw_weapon_ammo(
        &mut self,
        weapon: &ItemStack,
        ammo: &ItemStack,
        shooter: u32,
        kind: &str,
        infinite: bool,
    ) -> (Vec<ItemStack>, u32) {
        let count =
            self.entity_enchantment_value(weapon, shooter, kind, "minecraft:projectile_count", 1.0)
                as i32;
        let count = count.max(0);
        if count == 0 {
            return (Vec::new(), 0);
        }
        let wanted = if infinite || !self.common.is_logical_server() {
            0
        } else {
            let ctx = EffectContext {
                tool: Some(&ammo.item),
                items: Some(server_world_items!(self)),
                ..Default::default()
            };
            server_world_stations!(self).enchantments.modify_value(
                Some(weapon),
                "minecraft:ammo_use",
                &ctx,
                &mut self.common.world.random,
                1.0,
            ) as i32
        };
        let mut drawn = Vec::new();
        let mut debit = 0;
        if let Ok(wanted) = u32::try_from(wanted)
            && wanted <= ammo.count
        {
            let mut first = sunrise_sim::entities::arrows::drawn_ammo(ammo, wanted == 0);
            first.count = wanted.max(1);
            drawn.push(first);
            debit = wanted;
        }
        drawn.extend((1..count).map(|_| sunrise_sim::entities::arrows::drawn_ammo(ammo, true)));
        (drawn, debit)
    }

    /// Entity-filtered effects process their value on the subject's stream.
    pub(in crate::game) fn entity_enchantment_value(
        &mut self,
        weapon: &ItemStack,
        shooter: u32,
        kind: &str,
        component: &str,
        base: f32,
    ) -> f32 {
        if !self.common.is_logical_server() {
            return base;
        }
        let ctx = EffectContext {
            this: Some(kind),
            this_motion: self
                .enchantment_actor(shooter as i32)
                .and_then(|actor| actor.motion),
            ..Default::default()
        };
        self.evaluate_weapon_value(weapon, component, Some(shooter), ctx, base)
    }

    /// `AbstractArrow` uses the drawn pickup stack as the tool and stores piercing as a byte.
    pub(in crate::game) fn projectile_pierce(
        &mut self,
        weapon: &ItemStack,
        ammo: &ItemStack,
    ) -> u8 {
        if !self.common.is_logical_server() {
            return 0;
        }
        let ctx = EffectContext {
            tool: Some(&ammo.item),
            items: Some(server_world_items!(self)),
            ..Default::default()
        };
        let value = server_world_stations!(self).enchantments.modify_value(
            Some(weapon),
            "minecraft:projectile_piercing",
            &ctx,
            &mut self.common.world.random,
            0.0,
        ) as i32;
        value.max(0) as u8
    }

    /// Soul Speed's *location-based* modifiers and boots wear belong to the separate
    /// location-effect registry. Its particles and sound run in equipment tick order below.
    pub(in crate::game) fn update_soul_speed_location(&mut self) {
        const ID: &str = "minecraft:soul_speed";
        let Some(player) = self.common.player.as_mut() else {
            return;
        };
        if player.client_side {
            return;
        }
        let level = self.common.armor[3]
            .as_ref()
            .map_or(0, |s| s.enchantment_level(ID));
        let position = player.position;
        let (on_ground, flying) = (player.ground.on_ground, player.flying);
        let fx = &mut player.location_effects;
        let on_soul = fx.on_soul_speed_block;
        let block = BlockPos::containing(position);
        let changed_block = fx.block.replace(block) != Some(block);
        let landed = std::mem::take(&mut fx.landed);
        // Changed boots: `stopLocationBasedEffects`, then the effects run again.
        let equipped = std::mem::replace(&mut fx.boots_level, level) != level;
        let was_active = fx.applied_level;
        let mut active = was_active > 0 && !equipped;
        let mut wear = false;
        if level > 0 && (changed_block || landed || equipped) {
            let riding = self.common.riding.is_some();
            let wanted = !riding && !flying && (on_soul || (active && !on_ground));
            active = wanted;
            // `random_chance` is the first term of the wear's `all_of`: it
            // draws from the level random before the block is looked at.
            let roll = self.common.world.random.next_float() < 0.04;
            wear = roll && on_ground && on_soul;
        }
        fx.applied_level = if active { level } else { 0 };
        // The attribute effects were applied to the player before: swap them.
        if was_active > 0 && (!active || equipped) {
            for (attribute, id, _, op) in
                soul_speed_modifiers(&server_world_stations!(self).enchantments, was_active)
            {
                let _ = player.set_modifier(&attribute, &id, 0.0, &op, false);
            }
        }
        if active && (was_active == 0 || equipped) {
            for (attribute, id, amount, op) in
                soul_speed_modifiers(&server_world_stations!(self).enchantments, level)
            {
                let _ = player.set_modifier(&attribute, &id, amount, &op, true);
            }
        }
        if wear && !self.common.creative {
            self.wear_boots(1);
        }
    }

    /// `EnchantmentHelper.tickEffects`: slot order, then enchantment and entry order.
    pub(in crate::game) fn tick_equipment_enchantments(&mut self, target: Target, actor: u32) {
        if !self.common.is_logical_server() {
            return;
        }
        const COMPONENT: &str = "minecraft:tick";
        let table = self.stations().enchantments.clone();
        let mut kind = None;
        for slot in EquipmentSlot::ALL {
            let Some(stack) = self.equipment_stack_for(target, slot) else {
                continue;
            };
            let stack = stack.project();
            if !table.has_component(&stack, COMPONENT) {
                continue;
            }
            let stack = stack.into_owned();
            let kind = kind.get_or_insert_with(|| self.target_type(target));
            for (id, level) in stack.all_enchantments().iter() {
                let level = *level;
                let name = sunrise_items::equipment::SLOT_NAMES[slot as usize];
                let Some(def) = table.get(id).filter(|def| def.matches_slot(name)) else {
                    continue;
                };
                for (index, entry) in def.effects[COMPONENT]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let Some(bound) = table.entity_effect(id, COMPONENT, index) else {
                        continue;
                    };
                    let Some(body) = self.enchantment_tick_body(target) else {
                        return;
                    };
                    let support = sunrise_sim::motion::block_support_at(
                        &self.common.world,
                        Some(&self.common.res.motion().blocks),
                        body.position,
                        body.support,
                        sunrise_world::movement_support::MOVEMENT_SUPPORT_OFFSET,
                    );
                    let movement_block = self.block_name([support.x, support.y, support.z]);
                    let position = body.position.components();
                    let ctx = EffectContext {
                        this: Some(kind.as_str()),
                        this_motion: Some(body.motion),
                        items: Some(server_world_items!(self)),
                        grounded: Some(body.grounded),
                        horizontal_speed: Some(body.movement.x.hypot(body.movement.z)),
                        movement_block: Some(&movement_block),
                        tick: Some(body.tick),
                        ..Default::default()
                    };
                    if entry.get("requirements").is_some_and(|r| {
                        !table.condition(r, level, &ctx, &mut self.common.world.random)
                    }) {
                        continue;
                    }
                    let invocation = EntityEffectUse {
                        stack: &stack,
                        holder: Some(actor),
                        slot: Some(slot),
                        affected: actor,
                        level,
                        position: Some(position),
                        attack: None,
                    };
                    self.apply_bound_entity_effect(bound, &invocation, &mut |game, effect| {
                        match effect["type"]
                            .as_str()
                            .map(|s| s.trim_start_matches("minecraft:"))
                        {
                            Some("spawn_particles")
                                if effect["particle"]["type"] == "minecraft:soul" =>
                            {
                                let Some((at, velocity)) =
                                    game.with_damage_victim_random(actor, |random| {
                                        soul_speed_particle(
                                            position,
                                            body.movement.components(),
                                            body.width,
                                            random,
                                        )
                                    })
                                else {
                                    return;
                                };
                                if let Some(lan) = game.server.lan.as_mut() {
                                    super::lan_particles::send_soul_particle(lan, at, velocity);
                                }
                            }
                            _ => game.apply_post_attack(
                                effect,
                                level,
                                actor,
                                Some(actor),
                                Some(slot),
                            ),
                        }
                    });
                }
            }
        }
    }

    /// The selected living body's entity loot context and particle geometry.
    fn enchantment_tick_body(&self, target: Target) -> Option<TickBody> {
        Some(match target {
            Target::Player => {
                let p = self.common.player.as_ref()?;
                TickBody {
                    position: p.position,
                    movement: p.position.subtract(p.old_position),
                    width: f64::from(p.dimensions.width),
                    tick: p.health.tick_count as u32,
                    grounded: p.ground.on_ground,
                    support: p.ground.supporting_block,
                    motion: (p.fall_distance, p.fall_flying || p.flying),
                }
            }
            Target::Mob(id) => {
                let mob =
                    &self.common.mobs[self.common.mob_index.find_ref(&self.common.mobs, id)?];
                TickBody {
                    position: mob.position,
                    movement: mob.position.subtract(mob.previous),
                    width: mob.kind.width,
                    tick: mob.age.wrapping_add(1),
                    grounded: mob.ground.on_ground,
                    support: mob.ground.supporting_block,
                    motion: (mob.fall_distance, mob.extra.fall_flying),
                }
            }
            Target::Stand(id) => {
                let stand = self
                    .common
                    .armor_stands
                    .iter()
                    .find(|stand| stand.id == id)?;
                TickBody {
                    position: stand.position,
                    movement: stand.position.subtract(stand.previous),
                    width: stand.bounds().size().x,
                    tick: stand.enchantment_tick,
                    grounded: stand.ground.on_ground,
                    support: stand.ground.supporting_block,
                    motion: (stand.enchantment_fall_distance, false),
                }
            }
            Target::Dragon(id) => {
                let d = self.common.dragon.as_ref().filter(|d| d.id == id)?;
                TickBody {
                    position: d.position,
                    movement: d.position.subtract(d.previous),
                    // EntityType.ENDER_DRAGON uses the 16-wide parent, not a part.
                    width: 16.0,
                    tick: d.ticks.wrapping_add(1) as u32,
                    grounded: false,
                    support: None,
                    motion: (0.0, false),
                }
            }
            _ => return None,
        })
    }

    pub fn play_enchantment_sound(&mut self, effect: &Value, level: u32, actor: u32, at: [f64; 3]) {
        if self.entity_silent(actor) {
            return;
        }
        let (target, sound) = if let Some(index) = self
            .common
            .creatures
            .projectiles
            .iter()
            .position(|p| p.id == actor)
        {
            let sound = self
                .common
                .creatures
                .projectiles
                .update(index, |p| enchantment_sound(effect, level, &mut p.random.0));
            (Some(Target::Projectile(actor)), sound)
        } else if let Some(trident) = self.common.tridents.iter_mut().find(|t| t.id == actor) {
            (
                Some(Target::Trident(actor)),
                enchantment_sound(effect, level, &mut trident.random),
            )
        } else {
            (
                self.post_attack_target(actor),
                self.with_damage_victim_random(actor, |random| {
                    enchantment_sound(effect, level, random)
                })
                .flatten(),
            )
        };
        if let Some((sound, volume, pitch)) = sound
            && let Some(source) = target.and_then(|target| self.target_sound_source(target))
        {
            self.server_play_sound_on(source.index(), sound, at, volume, pitch);
        }
    }
    /// `ItemStack.hurtAndBreak` on the worn boots (`change_item_damage`):
    /// Unbreaking applies, and they break when worn out.
    pub fn wear_boots(&mut self, amount: u32) {
        self.damage_player_equipment(
            mod_api::EquipmentSlot::Feet,
            amount.min(i32::MAX as u32) as i32,
        );
    }
    /// `EnchantmentHelper.runLocationChangedEffects` for the worn armor of the
    /// player, run on the server once the player has moved: `LivingEntity.baseTick`
    /// runs it when the block position changed, `checkFallDamage` when it lands.
    /// `from` and `was_on_ground` are where and how the tick began.
    pub(in crate::game::server_side) fn location_changed_effects(
        &mut self,
        from: [f64; 3],
        was_on_ground: bool,
    ) {
        let Some(p) = self.common.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let on_ground = p.ground.on_ground;
        let cell = |v: [f64; 3]| v.map(|c| c.floor() as i32);
        // LivingEntity.collectEquipmentChanges: newly worn armor runs its
        // location_changed effects at once, even when standing still.
        let changed = armor_changes(&mut self.server.location_armor, &self.common.armor);
        let moved = cell(from) != cell(at) || (on_ground && !was_on_ground);
        if !moved && !changed.iter().any(|c| *c) {
            return;
        }
        let riding = self.common.riding.is_some();
        let slots = ["head", "chest", "legs", "feet"];
        let mut disks = Vec::new();
        for i in 0..4 {
            if moved || changed[i] {
                disks.extend(location_disks(
                    &self.stations().enchantments,
                    std::iter::once((&self.common.armor[i], slots[i])),
                    on_ground,
                    riding,
                ));
            }
        }
        for (effect, level) in disks {
            self.replace_disk(&effect, level, at, Who::Player);
        }
    }
    /// `LivingEntity.onChangedBlock`, landing and `collectEquipmentChanges`
    /// for an armor stand: run location effects before travel (including a
    /// piston push), on landing, and for newly equipped items.
    pub fn armor_stand_location_effects(&mut self, index: usize, landed: bool) {
        use super::native_mobs::equipment_stack_matches;

        let stand = &mut self.common.armor_stands[index];
        let block = BlockPos::containing(stand.position);
        let moved = !landed && stand.enchantment_block.replace(block) != Some(block);
        // `collectEquipmentChanges` walks every `EquipmentSlot`, body and
        // saddle included; only the six slots below wear enchantments.
        let any_initial_change = (0..8).any(|i| {
            !equipment_stack_matches(
                stand.equipment[i].as_ref(),
                stand.enchantment_equipment[i].as_ref(),
            )
        });
        if !moved && !landed && !any_initial_change {
            return;
        }
        let id = stand.id;
        let mut changed = [false; 8];
        if any_initial_change {
            if let Some(epoch) = self.living_equipment_notify_epoch() {
                let living = mod_api::LivingRef::new(mod_api::EntityRef::new(epoch, id));
                for slot in sunrise_core::equipment_slot::EquipmentSlot::ALL {
                    let i = slot as usize;
                    let Some(stand) = stand_by_hint_mut(&mut self.common.armor_stands, index, id)
                    else {
                        break;
                    };
                    if equipment_stack_matches(
                        stand.equipment[i].as_ref(),
                        stand.enchantment_equipment[i].as_ref(),
                    ) {
                        continue;
                    }
                    changed[i] = true;
                    let to = stand.equipment[i].clone();
                    if !mod_api::__private::enabled_listened(mod_api::HookId::LivingEquipmentChange)
                    {
                        stand.enchantment_equipment[i] = to;
                        continue;
                    }
                    let from = stand.enchantment_equipment[i].take();
                    self.dispatch_living_equipment_change(living, slot, from.as_ref(), to.as_ref());
                    // A listener can discard the stand while its row waits for the sweep: the
                    // authoritative residency decides, and a stand that left gets no history
                    // write, later notification or location effect.
                    if self.entity_residency(mod_api::EntityRef::new(epoch, id))
                        != mod_api::EntityResidency::InPlace
                    {
                        return;
                    }
                    if let Some(stand) = stand_by_hint_mut(&mut self.common.armor_stands, index, id)
                    {
                        stand.enchantment_equipment[i] = to;
                    }
                }
            } else {
                let stand = &mut self.common.armor_stands[index];
                for (i, is_changed) in changed.iter_mut().enumerate() {
                    if !equipment_stack_matches(
                        stand.equipment[i].as_ref(),
                        stand.enchantment_equipment[i].as_ref(),
                    ) {
                        *is_changed = true;
                        stand.enchantment_equipment[i] = stand.equipment[i].clone();
                    }
                }
            }
        }
        let Some(stand) = stand_by_hint(&self.common.armor_stands, index, id) else {
            return;
        };
        let at = [stand.position.x, stand.position.y, stand.position.z];
        let who = Who::Mob(stand.id);
        let equipment = stand
            .equipment
            .iter()
            .zip(sunrise_sim::entities::ArmorStand::SLOTS)
            .enumerate()
            .filter(|(i, _)| moved || landed || changed[*i])
            .filter(|(_, (stack, _))| {
                moved
                    || landed
                    || !stack.as_ref().is_some_and(|stack| {
                        let data = stack.data();
                        let component = |id| data.and_then(|data| data.other(id));
                        let max_damage = match component("minecraft:max_damage") {
                            Some(sunrise_core::nbt::Tag::Int(value)) => *value as u32,
                            _ => self
                                .common
                                .res
                                .items()
                                .get(&stack.item)
                                .map_or(0, |i| i.max_damage),
                        };
                        max_damage > 0
                            && component("minecraft:unbreakable").is_none()
                            && component("!minecraft:max_damage").is_none()
                            && component("!minecraft:damage").is_none()
                            && stack.damage >= max_damage
                    })
            })
            .map(|(_, item)| item);
        let disks = location_disks(
            &self.stations().enchantments,
            equipment,
            stand.ground.on_ground,
            stand.vehicle.is_some(),
        );
        for (effect, level) in disks {
            self.replace_disk(&effect, level, at, who);
        }
    }
    /// `ReplaceDisk.apply`: the blocks of the disk around `at` that pass the
    /// predicate become the effect's state, and each placement is a game event.
    pub(in crate::game::server_side) fn replace_disk(
        &mut self,
        effect: &Value,
        level: u32,
        at: [f64; 3],
        who: Who,
    ) {
        let registry = self.common.pack.assets.registry;
        let spec = &effect["block_state"];
        let Some(default) = spec["id"].as_str().and_then(|n| registry.default_state(n)) else {
            return;
        };
        let Some(mut state) = registry.state(default).cloned() else {
            return;
        };
        for (k, v) in spec["properties"].as_object().into_iter().flatten() {
            state
                .properties
                .insert(k.clone(), v.as_str().unwrap_or("").to_owned());
        }
        let new = registry.resolve(&state).unwrap_or(default);
        let radius = level_value(&effect["radius"], level) as i32;
        let height = level_value(&effect["height"], level) as i32;
        let offset = cell_offset(effect);
        let center = [0, 1, 2].map(|i| at[i].floor() as i32 + offset[i]);
        let event = effect["trigger_game_event"].as_str() == Some("minecraft:block_place");
        for dy in (height - 1).min(0)..=0 {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    let pos = [center[0] + dx, center[1] + dy, center[2] + dz];
                    let (x, z) = (
                        f64::from(pos[0]) + 0.5 - at[0],
                        f64::from(pos[2]) + 0.5 - at[2],
                    );
                    if x * x + z * z >= f64::from(radius * radius)
                        || !self.disk_predicate(&effect["predicate"], pos, new)
                        || self.level_set(pos, new).is_err()
                    {
                        continue;
                    }
                    if event {
                        self.post_block_game_event(
                            "minecraft:block_place",
                            pos,
                            Some(who),
                            Some(new),
                        );
                    }
                }
            }
        }
    }
    /// The `BlockPredicate` subset the disk effect uses, tested at `pos`; no
    /// predicate matches everything.
    pub(in crate::game::server_side) fn disk_predicate(
        &mut self,
        p: &Value,
        pos: [i32; 3],
        placed: BlockStateId,
    ) -> bool {
        if p.is_null() {
            return true;
        }
        let offset = cell_offset(p);
        let at = [0, 1, 2].map(|i| pos[i] + offset[i]);
        let state = self.common.world.block(at[0], at[1], at[2]);
        let b = self.common.res.behaviours();
        let names = |v: &Value| -> Vec<String> {
            match v {
                Value::String(s) => vec![s.clone()],
                Value::Array(a) => a
                    .iter()
                    .filter_map(|n| n.as_str().map(String::from))
                    .collect(),
                _ => Vec::new(),
            }
        };
        match p["type"].as_str().unwrap_or("") {
            "minecraft:all_of" => p["predicates"]
                .as_array()
                .into_iter()
                .flatten()
                .all(|q| self.disk_predicate(q, pos, placed)),
            // `MatchingBlockTagPredicate`
            "minecraft:matching_block_tag" => b
                .tag(p["tag"].as_str().unwrap_or("").trim_start_matches('#'))
                .is_some_and(|t| t.contains(b.block(state))),
            // `MatchingBlocksPredicate`
            "minecraft:matching_blocks" => {
                names(&p["blocks"]).iter().any(|n| n == b.table.name(state))
            }
            // `MatchingFluidsPredicate`: Fluids.WATER is the source.
            "minecraft:matching_fluids" => {
                let fluid = b.fluid(state);
                names(&p["fluids"]).iter().any(|n| match n.as_str() {
                    "minecraft:water" => fluid.kind() == FluidKind::Water && fluid.is_source(),
                    "minecraft:flowing_water" => {
                        fluid.kind() == FluidKind::Water && !fluid.is_source()
                    }
                    _ => false,
                })
            }
            // `UnobstructedPredicate`: no building-blocking entity in the block.
            "minecraft:unobstructed" => self.with_level(|_, level, _| {
                level.is_unobstructed(BlockPos::new(pos[0], pos[1], pos[2]), placed)
            }),
            _ => false,
        }
    }
    /// Enchantment attribute modifiers of active equipment, including held Efficiency.
    /// Each piece's modifier id ends with its slot (`EnchantmentAttributeEffect.idForSlot`),
    /// so the same enchantment on two pieces stacks instead of replacing itself.
    pub(in crate::game::server_side) fn apply_enchantment_attributes(&mut self) {
        let mut wanted: Vec<(String, String, f64, String)> = Vec::new();
        for (stack, slot) in self
            .common
            .armor
            .iter()
            .zip(["head", "chest", "legs", "feet"])
            .chain([
                (&self.common.inventory[self.common.selected], "mainhand"),
                (&self.common.offhand, "offhand"),
            ])
        {
            let Some(stack) = stack else { continue };
            for sunrise_items::item_data::ModifierRow {
                attribute: attr,
                id,
                amount,
                operation: op,
                slot: group,
            } in self.stations().enchantments.modifier_rows(stack)
            {
                if super::equipment_modifier_applies(&group, slot) {
                    wanted.push((attr, format!("{id}/{slot}"), amount, op));
                }
            }
        }
        let Some(p) = self.common.player.as_mut() else {
            return;
        };
        for (attr, id, _, op) in std::mem::take(&mut self.common.potions.enchant_modifiers) {
            let _ = p.set_modifier_unclamped(&attr, &id, 0.0, &op, false);
        }
        for (attr, id, amount, op) in &wanted {
            let _ = p.set_modifier_unclamped(attr, id, *amount, op, true);
        }
        let _ = p.clamp_health_to_max();
        self.common.potions.enchant_modifiers = wanted;
    }
}

/// `EnchantmentHelper.runLocationChangedEffects`: the replace-disk effects of
/// active equipment slots, with the wearer's ground/passenger requirements.
pub(in crate::game::server_side) fn location_disks<'a>(
    enchantments: &Enchantments,
    equipment: impl IntoIterator<Item = (&'a Option<ItemStack>, &'static str)>,
    on_ground: bool,
    riding: bool,
) -> Vec<(Value, u32)> {
    let mut disks = Vec::new();
    for (stack, slot) in equipment {
        let Some(stack) = stack else { continue };
        for (id, level) in stack.all_enchantments().iter() {
            let Some(def) = enchantments.get(id) else {
                continue;
            };
            let armor = matches!(slot, "head" | "chest" | "legs" | "feet");
            let hand = matches!(slot, "mainhand" | "offhand");
            if !def.slots.iter().any(|s| {
                s == slot || s == "any" || (s == "armor" && armor) || (s == "hand" && hand)
            }) {
                continue;
            }
            for effect in def.effects["minecraft:location_changed"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if effect["effect"]["type"] == "minecraft:replace_disk"
                    && location_requirements(&effect["requirements"], on_ground, riding)
                {
                    disks.push((effect["effect"].clone(), *level));
                }
            }
        }
    }
    disks
}

/// Soul Speed's `attribute` effects at `level` (attribute, id, amount, operation),
/// the id taking the boots slot like `EnchantmentAttributeEffect.idForSlot`.
pub(in crate::game::server_side) fn soul_speed_modifiers(
    enchantments: &Enchantments,
    level: u32,
) -> Vec<(String, String, f64, String)> {
    let Some(e) = enchantments.get("minecraft:soul_speed") else {
        return Vec::new();
    };
    e.effects["minecraft:location_changed"][0]["effect"]["effects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["type"] == "minecraft:attribute")
        .map(|a| {
            (
                a["attribute"].as_str().unwrap_or("").to_owned(),
                format!("{}/feet", a["id"].as_str().unwrap_or("")),
                f64::from(level_value(&a["amount"], level)),
                a["operation"].as_str().unwrap_or("add_value").to_owned(),
            )
        })
        .collect()
}

/// An effect's or predicate's `offset` vector (zero when absent).
pub(in crate::game::server_side) fn cell_offset(v: &Value) -> [i32; 3] {
    let at = |i: usize| {
        v["offset"]
            .get(i)
            .and_then(Value::as_i64)
            .map_or(0, |n| n as i32)
    };
    [at(0), at(1), at(2)]
}
/// The loot-condition subset a `location_changed` effect requires of the
/// wearer: standing on the ground and not riding (frost walker).
pub(in crate::game::server_side) fn location_requirements(
    req: &Value,
    on_ground: bool,
    riding: bool,
) -> bool {
    match req["type"].as_str().unwrap_or("") {
        "" if req.is_null() => true,
        "minecraft:all_of" => req["terms"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|t| location_requirements(t, on_ground, riding)),
        "minecraft:inverted" => !location_requirements(&req["term"], on_ground, riding),
        "minecraft:entity_properties" => {
            let p = &req["predicate"];
            p["minecraft:flags"]["is_on_ground"]
                .as_bool()
                .is_none_or(|v| v == on_ground)
                && (p["minecraft:vehicle"].is_null() || riding)
        }
        _ => false,
    }
}

/// `LivingEntity.collectEquipmentChanges`: which armor slots differ from the
/// `snapshot` taken at the last look, which is then brought up to date.
pub(in crate::game::server_side) fn armor_changes(
    snapshot: &mut [Option<ItemStack>; 4],
    armor: &[Option<ItemStack>; 4],
) -> [bool; 4] {
    let changed: [bool; 4] = std::array::from_fn(|i| armor[i] != snapshot[i]);
    // Only a changed slot is copied: the check runs every tick for every player.
    for ((kept, worn), changed) in snapshot.iter_mut().zip(armor).zip(changed) {
        if changed {
            kept.clone_from(worn);
        }
    }
    changed
}

pub(in crate::game::server_side) fn mending_entries(
    stack: &ItemStack,
    slot: PlayerSlot,
    enchantments: &Enchantments,
) -> usize {
    if stack.damage == 0 {
        return 0;
    }
    stack
        .all_enchantments()
        .iter()
        .filter(|(id, _)| {
            enchantments.get(id).is_some_and(|def| {
                def.effects.get("minecraft:repair_with_xp").is_some()
                    && def.slots.iter().any(|group| {
                        sunrise_gameplay_common::game::equipment_modifier_applies(
                            group,
                            slot.name(),
                        )
                    })
            })
        })
        .count()
}

/// `EnchantmentHelper.modifyDurabilityToRepairFromXp`: the experience `amount`
/// repairs at the stack's `repair_with_xp` xp-per-point rate. Vanilla's
/// `Enchantment.itemContext` puts the repaired stack in
/// `LootContextParams.TOOL`, so `minecraft:match_tool` judges that item.
pub(in crate::game::server_side) fn repair_from_xp(
    enchantments: &Enchantments,
    items: Option<&Items>,
    stack: &ItemStack,
    amount: i32,
    random: &mut LegacyRandom,
) -> u32 {
    let ctx = EffectContext {
        tool: Some(&stack.item),
        items,
        ..Default::default()
    };
    let repair = enchantments.modify_value(
        Some(stack),
        "minecraft:repair_with_xp",
        &ctx,
        random,
        amount as f32,
    );
    (repair as i32).max(0) as u32
}

/// `ExperienceOrb.repairPlayerItems`: the damage `to_repair` xp per point takes
/// off a stack with `damage`, and the experience left over after it
/// (`amount - repair * amount / toRepairFromXpAmount`, in vanilla's integer
/// order). Vanilla divides only when it repairs something, so a rate of zero
/// keeps all of `amount`.
pub(in crate::game::server_side) fn repaired_and_left(
    amount: u32,
    damage: u32,
    to_repair: u32,
) -> (u32, u32) {
    let repaired = to_repair.min(damage);
    if repaired == 0 {
        return (0, amount);
    }
    let left = amount
        .saturating_sub((u64::from(repaired) * u64::from(amount) / u64::from(to_repair)) as u32);
    (repaired, left)
}

/// `EnchantmentHelper.getRandomItemWith(REPAIR_WITH_XP, player, ItemStack::isDamaged)`:
/// collects damaged effect-bearing pieces in `EquipmentSlot.VALUES` order and picks
/// one with the player's own random (`Entity.random`), never the level's.
pub(in crate::game::server_side) fn pick_mending_slot(
    slots: [(PlayerSlot, &Option<ItemStack>); 6],
    enchantments: &Enchantments,
    random: &mut impl RandomSource,
) -> Option<PlayerSlot> {
    let candidates: Vec<PlayerSlot> = slots
        .into_iter()
        .flat_map(|(slot, stack)| {
            std::iter::repeat_n(
                slot,
                stack
                    .as_ref()
                    .map_or(0, |stack| mending_entries(stack, slot, enchantments)),
            )
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let count = candidates.len() as i32;
    let index = random.next_int_bound(count).unwrap_or(0) as usize;
    Some(candidates[index])
}

struct TickBody {
    position: Vec3,
    movement: Vec3,
    width: f64,
    tick: u32,
    grounded: bool,
    support: Option<BlockPos>,
    motion: (f64, bool),
}

/// `SpawnParticlesEffect.apply` for Soul Speed's in-bounds horizontal position.
fn soul_speed_particle(
    position: [f64; 3],
    moved: [f64; 3],
    width: f64,
    random: &mut LegacyRandom,
) -> ([f64; 3], [f64; 3]) {
    let x = position[0] + (random.next_double() - 0.5) * width;
    let z = position[2] + (random.next_double() - 0.5) * width;
    (
        [x, position[1] + 0.1, z],
        [moved[0] * -0.2, 0.1, moved[2] * -0.2],
    )
}

pub(in crate::game) fn enchantment_sound<'a>(
    effect: &'a Value,
    level: u32,
    random: &mut LegacyRandom,
) -> Option<(&'a str, f32, f32)> {
    use super::projectile_spawned::{float_provider, level_sound};
    let sound = level_sound(effect, level)?;
    let volume = float_provider(&effect["volume"])?;
    let pitch = float_provider(&effect["pitch"])?;
    Some((sound, volume.sample(random), pitch.sample(random)))
}

impl ServerGame<'_> {
    /// One `post_piercing_attack` effect on the player: `AllOf`,
    /// `ChangeItemDamage`, `ApplyExhaustion`, `ApplyEntityImpulse` and
    /// `PlaySoundEffect`.
    pub(in crate::game::server_side) fn apply_piercing_effect(
        &mut self,
        effect: &serde_json::Value,
        level: u32,
    ) {
        let amount = || level_value(&effect["amount"], level);
        match effect["type"].as_str() {
            Some("minecraft:all_of") => {
                for e in effect["effects"].as_array().into_iter().flatten() {
                    self.apply_piercing_effect(e, level);
                }
            }
            Some("minecraft:change_item_damage") => self.damage_held(amount() as u32),
            Some("minecraft:apply_exhaustion") => {
                if let Some(p) = self.common.player.as_mut() {
                    p.cause_food_exhaustion(amount());
                }
            }
            // The direction turned by the look (`Entity.getLookQuaternion`:
            // pitch about x, then yaw about y), scaled by coordinate_scale
            // and the magnitude, added to the velocity (the guest's client
            // is told of it by `guest_hit_by`).
            Some("minecraft:apply_impulse") => {
                let impulse = look_impulse(effect, level, self.common.yaw, self.common.pitch);
                if let Some(p) = self.common.player.as_mut() {
                    p.velocity = p.velocity.add_vector(impulse);
                }
            }
            // The sound for the level (a list picks by level), heard by all.
            Some("minecraft:play_sound") => {
                let sounds: Vec<&str> = match &effect["sound"] {
                    serde_json::Value::Array(list) => {
                        list.iter().filter_map(|s| s.as_str()).collect()
                    }
                    one => one.as_str().into_iter().collect(),
                };
                let index = (level as usize).saturating_sub(1);
                if let Some(sound) = sounds.get(index.min(sounds.len().saturating_sub(1))) {
                    let float = |key: &str| effect[key].as_f64().unwrap_or(1.0) as f32;
                    let at = self
                        .common
                        .player
                        .as_ref()
                        .map(|p| [p.position.x, p.position.y, p.position.z]);
                    if let Some(at) = at {
                        self.server_play_sound(sound, at, float("volume"), float("pitch"));
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::{
        Enchantments, armor_changes, block_break_speed_factor, enchantment_sound,
        pick_mending_slot, repair_from_xp, repaired_and_left, soul_speed_particle,
    };

    use sunrise_core::random::{LegacyRandom, RandomSource};
    use sunrise_items::items::ItemStack;
    use sunrise_world::loot::PackOverlay;

    /// The snapshot reports a newly worn piece once, then settles.
    #[test]
    fn armor_changes_settle_after_one_look() {
        let worn = [
            None,
            None,
            None,
            Some(ItemStack::new("minecraft:iron_boots", 1)),
        ];
        let mut snapshot: [Option<ItemStack>; 4] = Default::default();
        assert_eq!(
            armor_changes(&mut snapshot, &worn),
            [false, false, false, true]
        );
        assert_eq!(armor_changes(&mut snapshot, &worn), [false; 4]);
    }

    /// #4094: `BLOCK_BREAK_SPEED` is a multiplier on the destroy speed.
    #[test]
    fn block_break_speed_is_a_destroy_speed_multiplier() {
        assert_eq!(block_break_speed_factor(None), 1.0);
        assert_eq!(block_break_speed_factor(Some(1.0)), 1.0);
        assert_eq!(block_break_speed_factor(Some(0.5)), 0.5);
        assert_eq!(block_break_speed_factor(Some(2.0)), 2.0);
        assert_eq!(8.0 * block_break_speed_factor(Some(0.5)), 4.0);
        assert_eq!(8.0 * block_break_speed_factor(Some(2.0)), 16.0);
    }

    /// #11309: `EnchantmentHelper.getRandomItemWith` picks among damaged
    /// Mending items in `EquipmentSlot.VALUES` order using the player's RNG,
    /// leaving the world RNG untouched.
    #[test]
    fn mixed_repair_enchantments_weight_each_entry() {
        let definition = serde_json::json!({
            "description": "Repair", "supported_items": "minecraft:diamond_pickaxe",
            "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0},
            "slots": ["any"],
            "effects": {"minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 2}}]}
        });
        let enchantments = super::Enchantments::default()
            .with_entries([("test:first", &definition), ("test:second", &definition)])
            .unwrap()
            .unwrap();
        let mut main = ItemStack::new("minecraft:diamond_pickaxe", 1);
        main.damage = 20;
        main.set_crafting_enchantments(vec![("test:first".into(), 1), ("test:second".into(), 1)]);
        let mut off = ItemStack::new("minecraft:diamond_pickaxe", 1);
        off.damage = 20;
        off.set_crafting_enchantments(vec![("test:first".into(), 1)]);
        let main = Some(main);
        let off = Some(off);
        let none = None;
        use super::PlayerSlot::{Armor, Main, Off};
        let slots = [
            (Main, &main),
            (Off, &off),
            (Armor(3), &none),
            (Armor(2), &none),
            (Armor(1), &none),
            (Armor(0), &none),
        ];
        let expected_candidates = [Main, Main, Off];
        let mut actual_rng = LegacyRandom::new(20_002);
        let mut expected_rng = LegacyRandom::new(20_002);
        for _ in 0..32 {
            let index = expected_rng.next_int_bound(3).unwrap() as usize;
            assert_eq!(
                pick_mending_slot(slots, &enchantments, &mut actual_rng),
                Some(expected_candidates[index])
            );
        }
        assert_eq!(actual_rng.next_long(), expected_rng.next_long());
    }

    #[test]
    fn mending_picks_damaged_candidate_in_equipment_slot_order_from_player_rng() {
        let enchantments = super::Enchantments::default().with_entries([("minecraft:mending", &serde_json::json!({
            "description": "Mending", "supported_items": "minecraft:diamond_pickaxe", "weight": 1,
            "max_level": 1, "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0}, "anvil_cost": 1,
            "slots": ["any"], "effects": {"minecraft:repair_with_xp": [{"effect": {"type": "minecraft:multiply", "factor": 2}}]}
        }))]).unwrap().unwrap();
        let mending_item = |id: &str, damage: u32| {
            let mut stack = ItemStack::new(id, 1);
            stack.damage = damage;
            stack.set_crafting_enchantments(vec![("minecraft:mending".into(), 1)]);
            Some(stack)
        };
        let mut unbreaking = ItemStack::new("minecraft:diamond_leggings", 1);
        unbreaking.damage = 5;
        unbreaking.set_crafting_enchantments(vec![("minecraft:unbreaking".into(), 3)]);

        let main = mending_item("minecraft:diamond_pickaxe", 12);
        let off = mending_item("minecraft:shield", 3);
        let head = mending_item("minecraft:diamond_helmet", 4);
        let chest = mending_item("minecraft:diamond_chestplate", 0);
        let legs = Some(unbreaking);
        let feet = mending_item("minecraft:diamond_boots", 9);

        let slots = [
            (super::PlayerSlot::Main, &main),
            (super::PlayerSlot::Off, &off),
            (super::PlayerSlot::Armor(3), &feet),
            (super::PlayerSlot::Armor(2), &legs),
            (super::PlayerSlot::Armor(1), &chest),
            (super::PlayerSlot::Armor(0), &head),
        ];

        let mut world_rng = LegacyRandom::new(10_001);
        let mut expected_world = LegacyRandom::new(10_001);
        let mut player_rng = LegacyRandom::new(20_002);
        let mut expected_player = LegacyRandom::new(20_002);

        let expected_candidates = [
            super::PlayerSlot::Main,
            super::PlayerSlot::Off,
            super::PlayerSlot::Armor(3),
            super::PlayerSlot::Armor(0),
        ];
        for _ in 0..16 {
            let picked = pick_mending_slot(slots, &enchantments, &mut player_rng);
            let idx = expected_player.next_int_bound(4).unwrap() as usize;
            assert_eq!(picked, Some(expected_candidates[idx]));
        }
        assert_eq!(player_rng.next_long(), expected_player.next_long());
        assert_eq!(world_rng.next_long(), expected_world.next_long());

        let none: Option<ItemStack> = None;
        let empty = [
            (super::PlayerSlot::Main, &none),
            (super::PlayerSlot::Off, &chest),
            (super::PlayerSlot::Armor(3), &none),
            (super::PlayerSlot::Armor(2), &legs),
            (super::PlayerSlot::Armor(1), &chest),
            (super::PlayerSlot::Armor(0), &none),
        ];
        assert_eq!(
            pick_mending_slot(empty, &enchantments, &mut player_rng),
            None
        );
        assert_eq!(player_rng.next_long(), expected_player.next_long());
    }

    #[test]
    fn mending_pick_counts_each_matching_enchantment() {
        use super::{Enchantments, PLAYER_SLOTS, PlayerSlot};
        let definition = serde_json::json!({
            "description": "Repair", "supported_items": "minecraft:diamond_pickaxe",
            "weight": 1, "max_level": 1, "anvil_cost": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0},
            "max_cost": {"base": 1, "per_level_above_first": 0},
            "slots": ["any"], "effects": {"minecraft:repair_with_xp": [
                {"effect": {"type": "minecraft:multiply", "factor": 2}}
            ]}
        });
        let enchantments = Enchantments::default()
            .with_entries([
                ("minecraft:mending", &definition),
                ("test:patch", &definition),
            ])
            .unwrap()
            .unwrap();
        let mut main = ItemStack::new("minecraft:diamond_pickaxe", 1);
        main.damage = 20;
        main.set_crafting_enchantments(vec![
            ("minecraft:mending".into(), 1),
            ("test:patch".into(), 1),
        ]);
        let mut off = ItemStack::new("minecraft:diamond_pickaxe", 1);
        off.damage = 20;
        off.set_crafting_enchantments(vec![("minecraft:mending".into(), 1)]);
        let (main, off, empty) = (Some(main), Some(off), None);
        let slots = PLAYER_SLOTS.map(|slot| {
            (
                slot,
                match slot {
                    PlayerSlot::Main => &main,
                    PlayerSlot::Off => &off,
                    _ => &empty,
                },
            )
        });
        let candidates = [PlayerSlot::Main, PlayerSlot::Main, PlayerSlot::Off];
        let mut actual = LegacyRandom::new(3215);
        let mut expected = LegacyRandom::new(3215);
        for _ in 0..64 {
            let index = expected.next_int_bound(3).unwrap() as usize;
            assert_eq!(
                pick_mending_slot(slots, &enchantments, &mut actual),
                Some(candidates[index])
            );
        }
        assert_eq!(actual.next_long(), expected.next_long());
    }

    /// #11309: Soul Speed's tick effects draw the 35% sound requirement from
    /// the level RNG and sample particle coordinates and sound pitch from the
    /// wearer's RNG.
    #[test]
    fn soul_speed_samples_particle_and_pitch_from_wearer_rng() {
        let pos = [10.0, 64.0, -4.0];
        let moved = [0.25, 0.0, -0.5];
        let width = f64::from(0.6f32);
        let table = Enchantments::default();
        let requirement = serde_json::json!({"type": "minecraft:random_chance", "chance": 0.35});
        let sound = serde_json::json!({
            "sound": "minecraft:particle.soul_escape", "volume": 0.6,
            "pitch": {"type": "minecraft:uniform", "min_inclusive": 0.6, "max_exclusive": 1.0}
        });
        for seed in 0..32 {
            let mut l_rng = LegacyRandom::new(seed);
            let mut expected_level = l_rng.clone();
            let mut w_rng = LegacyRandom::new(seed + 1_000);
            let mut expected_wearer = w_rng.clone();
            let particle = soul_speed_particle(pos, moved, width, &mut w_rng);
            let want_x = pos[0] + (expected_wearer.next_double() - 0.5) * width;
            let want_z = pos[2] + (expected_wearer.next_double() - 0.5) * width;
            assert_eq!(
                particle,
                (
                    [want_x, pos[1] + 0.1, want_z],
                    [moved[0] * -0.2, 0.1, moved[2] * -0.2]
                )
            );
            let pitch = table
                .condition(
                    &requirement,
                    1,
                    &sunrise_items::enchant::EffectContext::default(),
                    &mut l_rng,
                )
                .then(|| enchantment_sound(&sound, 1, &mut w_rng).unwrap().2);
            let want_pitch = (expected_level.next_float() < 0.35)
                .then(|| expected_wearer.next_float() * (1.0 - 0.6) + 0.6);
            assert_eq!(pitch, want_pitch);
            assert_eq!(l_rng.next_long(), expected_level.next_long());
            assert_eq!(w_rng.next_long(), expected_wearer.next_long());
        }
    }

    /// A pack's `repair_with_xp` gated on swords: `Enchantment.itemContext`
    /// hands the repaired stack to the conditions as `LootContextParams.TOOL`,
    /// so the sword repairs at twice the xp and the axe at the plain rate.
    #[test]
    fn mending_repair_judges_match_tool_by_the_repaired_stack() {
        let files = [(
            "enchantment/pack:xp_repair".to_owned(),
            vec![
                br#"{"description":"xp repair","supported_items":"minecraft:diamond_sword","slots":["mainhand"],"effects":{"minecraft:repair_with_xp":[{"effect":{"type":"minecraft:multiply","factor":2.0},"requirements":{"type":"minecraft:match_tool","predicate":{"items":"minecraft:diamond_sword"}}}]}}"#
                    .to_vec(),
            ],
        )];
        let root = std::env::temp_dir().join(format!(
            "sunrise-mending-tool-context-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let defaults = sunrise_world::item_defaults::read(&root).unwrap();
        let e = Enchantments::load_with(&root, &PackOverlay::new(&files), &defaults).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        let gated = |id: &str| {
            let mut stack = ItemStack::new(id, 1);
            stack.damage = 20;
            stack.set_crafting_enchantments(vec![("pack:xp_repair".into(), 1)]);
            stack
        };
        let mut random = LegacyRandom::new(7);
        // The sword matches: 5 xp repairs 10 of the stack's 20 damage and uses
        // it all up; 2 damage would eat 1 xp (2 * 5 / 10, vanilla's integer
        // order) and leave 4.
        let sword = gated("minecraft:diamond_sword");
        let to_repair = repair_from_xp(&e, None, &sword, 5, &mut random);
        assert_eq!(to_repair, 10);
        assert_eq!(repaired_and_left(5, sword.damage, to_repair), (10, 0));
        assert_eq!(repaired_and_left(5, 2, to_repair), (2, 4));
        // The axe matches no tool condition: 1 xp per damage point, so 5 xp
        // repairs 5 damage; 2 damage would eat 2 xp and leave 3.
        let axe = gated("minecraft:diamond_axe");
        let to_repair = repair_from_xp(&e, None, &axe, 5, &mut random);
        assert_eq!(to_repair, 5);
        assert_eq!(repaired_and_left(5, axe.damage, to_repair), (5, 0));
        assert_eq!(repaired_and_left(5, 2, to_repair), (2, 3));
        // `minecraft:multiply` draws nothing: the level RNG is untouched.
        assert_eq!(random.next_long(), LegacyRandom::new(7).next_long());
    }
}

#[cfg(test)]
pub(in crate::game) mod impulse_rounding_tests {
    #[test]
    fn impulse_matches_pinned_joml_components() {
        let effect =
            serde_json::json!({"direction":[0,0,1],"coordinate_scale":[1,1,1],"magnitude":1});
        let v = super::look_impulse(&effect, 1, 45.0, 30.0);
        assert_eq!(
            [v.x, v.y, v.z],
            [-0.6123724579811096, -0.4999999701976776, 0.6123723983764648]
        );
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
