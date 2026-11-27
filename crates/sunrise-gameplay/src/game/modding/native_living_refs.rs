//! Living entities for native server mods, through a `LivingRef`
//! (docs/modding/src/how-to/native-mods.md, "Entity and level handles", #7387): the equipment
//! of mobs, players and armor stands, throwing a stack, and whether a mob
//! is tamed. Reads return `None`, writes false, for a stale handle or on a
//! client; the exception is the reads marked side `both` (`living`, `with_equipment`,
//! `with_hand_item`, `health`, `max_health`, `attribute_value`, `attribute_base`,
//! `visit_attributes` and `visit_attribute_modifiers`), which on a client answer for the
//! local player from the client's own copy; and `effect`/`visit_effects`. Visitor reads
//! refuse with a typed error rather than returning `None`.
use super::commands::{Target, items::Owner};
use super::living_attributes::AttributeWrite;
use crate::game::ServerGame;
use mod_api::client::first_person::Hand;
use mod_api::{
    AttributeModifier, AttributeOperation, Direction, EntityRef, EquipmentSlot, Identifier,
    LivingRef, ModifierLifetime,
};
use sunrise_gameplay_common::game::native_equipment_read::hand_slot;
pub(in crate::game::server_side) use sunrise_gameplay_common::game::native_living_body::{
    mob_attribute_modifiers, visit_attribute_modifiers,
};
use sunrise_items::items::{EntityTypeTags, ItemStack};
use sunrise_sim::health::{Attack, DamageLevel, HitEffects, Hurting, Knock, NO_ATTACK};
use sunrise_sim::mobs::Who;
use sunrise_sim::mobs::{
    Mob,
    finalize::{self, EquipmentRead},
};
use sunrise_world::attributes::Operation;

/// A player's armour slots (`armor`, head first) by equipment slot.
pub(in crate::game::server_side) fn armor_index(slot: EquipmentSlot) -> Option<usize> {
    match slot {
        EquipmentSlot::Head => Some(0),
        EquipmentSlot::Chest => Some(1),
        EquipmentSlot::Legs => Some(2),
        EquipmentSlot::Feet => Some(3),
        _ => None,
    }
}

/// A player's stack in `slot`, from its inventory, armour and off hand.
pub(in crate::game::server_side) fn player_slot<'a>(
    inventory: &'a [Option<ItemStack>],
    selected: usize,
    armor: &'a [Option<ItemStack>; 4],
    offhand: &'a Option<ItemStack>,
    equipment: &'a sunrise_items::equipment::Equipment,
    slot: EquipmentSlot,
) -> Option<&'a ItemStack> {
    match slot {
        EquipmentSlot::MainHand => inventory.get(selected)?.as_ref(),
        EquipmentSlot::OffHand => offhand.as_ref(),
        EquipmentSlot::Body | EquipmentSlot::Saddle => equipment.get(slot as usize),
        _ => armor[armor_index(slot)?].as_ref(),
    }
}

/// The `/item` slot name of an equipment slot.
pub fn slot_name(slot: EquipmentSlot) -> &'static str {
    match slot {
        EquipmentSlot::MainHand => "weapon.mainhand",
        EquipmentSlot::OffHand => "weapon.offhand",
        EquipmentSlot::Head => "armor.head",
        EquipmentSlot::Chest => "armor.chest",
        EquipmentSlot::Legs => "armor.legs",
        EquipmentSlot::Feet => "armor.feet",
        EquipmentSlot::Body => "armor.body",
        EquipmentSlot::Saddle => "saddle",
    }
}

pub(in crate::game::server_side) fn world_equipment_slot(
    slot: EquipmentSlot,
) -> sunrise_core::equipment_slot::EquipmentSlot {
    use sunrise_core::equipment_slot::EquipmentSlot as WorldSlot;
    match slot {
        EquipmentSlot::MainHand => WorldSlot::MainHand,
        EquipmentSlot::OffHand => WorldSlot::OffHand,
        EquipmentSlot::Head => WorldSlot::Head,
        EquipmentSlot::Chest => WorldSlot::Chest,
        EquipmentSlot::Legs => WorldSlot::Legs,
        EquipmentSlot::Feet => WorldSlot::Feet,
        EquipmentSlot::Body => WorldSlot::Body,
        EquipmentSlot::Saddle => WorldSlot::Saddle,
    }
}

/// `LivingEntity.isAlive`'s health test: above 0, so a NaN health loaded from
/// a save is not alive either.
pub(in crate::game::server_side) fn alive_at(health: f32) -> bool {
    health > 0.0
}

pub(in crate::game::server_side) fn equipment_stack(stack: Option<ItemStack>) -> Option<ItemStack> {
    stack.filter(|s| s.count > 0 && &*s.item != "minecraft:air")
}

impl ServerGame<'_> {
    /// What a living handle names now: a player, a mob, an armor stand or the dragon.
    pub(in crate::game) fn native_living_target(&self, living: LivingRef) -> Option<Target> {
        self.native_entity_target(living.entity())
            .filter(|t| t.is_living())
    }
    /// A direct, non-damaging call to the short `LivingEntity.knockback` entry.
    pub(in crate::game) fn living_knockback(
        &mut self,
        living: LivingRef,
        strength: f64,
        [x, z]: [f64; 2],
    ) -> bool {
        if ![strength, x, z].into_iter().all(f64::is_finite) {
            return false;
        }
        let Some(target) = self.native_living_target(living) else {
            return false;
        };
        const SOURCE: mod_api::HurtSource<'static> = mod_api::HurtSource {
            damage_type: "minecraft:generic",
            causing: 0,
            direct: 0,
            position: None,
            direct_type: None,
        };
        let knock = Knock { strength, x, z };
        let id = living.id();
        match target {
            Target::Player | Target::Guest(_) => self
                .player_knockback_from_mod(living, target, knock, SOURCE)
                .is_some(),
            Target::Mob(_) => {
                let Some(i) = self.common.mob_index.find(&self.common.mobs, id) else {
                    return false;
                };
                let plan = self.prepare_knockback_credit(id as i32, None, &NO_ATTACK);
                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                let level = crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &NO_ATTACK,
                    &mut credit,
                    None
                );
                self.common.mobs[i].knockback(
                    level,
                    strength,
                    x,
                    z,
                    &sunrise_sim::mobs::Damage::of(sunrise_sim::mobs::DamageKind::Generic),
                    0.0,
                )
            }
            Target::Stand(id) => self.knock_back_armor_stand(id, knock, SOURCE, &NO_ATTACK),
            Target::Dragon(_) => {
                let resistance = self.dragon_knockback_resistance();
                let plan = self.prepare_knockback_credit(id as i32, None, &NO_ATTACK);
                let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
                let level = crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    &NO_ATTACK,
                    &mut credit,
                    None
                );
                self.common.dragon.as_mut().is_some_and(|d| {
                    crate::game::server_side::dragon::knockback(d, level, knock, SOURCE, resistance)
                })
            }
            _ => false,
        }
    }

    /// Lend the victim's own body and level to the player path without switching turns or levels.
    pub(in crate::game::server_side) fn player_knockback_from_mod(
        &mut self,
        living: LivingRef,
        target: Target,
        knock: Knock,
        source: mod_api::HurtSource<'_>,
    ) -> Option<()> {
        let id = living.id();
        // The hook's attacker falls back to the victim's `getLastHurtByMob`, resolved in any
        // level; the source stays unattributed and kill credit is planned on its own.
        let remembered = self
            .living_last_attacker(living)
            .map(|attacker| self.last_attacker_attack(attacker.id()));
        let attack = remembered.as_ref().unwrap_or(&NO_ATTACK);
        let hub = &self.server.lan.as_ref()?.hub;
        let player = match target {
            Target::Player => {
                hub.guest(self.server.lan_acting?)?;
                self.common.player.as_ref()?
            }
            Target::Guest(conn) => hub.guest(conn)?.motion.as_deref()?,
            _ => return None,
        };
        let mut random = player.random.clone();
        let plan = self.prepare_knockback_credit(id as i32, None, attack);
        let dimension = self
            .resident_target_dimension(target)
            .unwrap_or(self.common.dim);
        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
        let world = if dimension == self.common.dim {
            &mut self.common.world
        } else {
            &mut self.server.levels.0.parked_mut(dimension)?.world
        };
        let mut level = DamageLevel::of(
            world,
            self.common.pack.assets.registry,
            Some(server_world_items!(self)),
            Some(&self.common.pack.block_registries),
            &self.common.damage_hooks,
        )
        .attacked_by(attack)
        .with_knockback_credit(&mut credit)
        .with_effects(HitEffects::new(
            &mut self.server.pending_native_sounds,
            Some(hub),
            dimension.id(),
            self.common.native_lifecycle.epoch(),
            None,
        ));
        let velocity =
            sunrise_gameplay_common::game::creatures::knockback::player_knockback_velocity(
                player,
                &mut random,
                &mut level,
                knock,
                source,
            );
        // The retries drew from the victim's own random, landed or not.
        self.write_player_random(target, random);
        let velocity = velocity?;
        self.write_player_velocity(target, |_| Some(velocity))
            .then_some(())
    }

    /// [`Self::attack_of`] for a remembered attacker, which may be a mob in another loaded
    /// level: its main-hand stack is read there.
    pub(in crate::game::server_side) fn last_attacker_attack(&self, attacker: u32) -> Attack {
        let mut attack = self.attack_of(attacker as i32);
        if let Some((_, mob)) = self.mob_in_other_level(attacker)
            && !mod_api::__private::mods().is_empty()
        {
            attack.held[0] = mob.extra.main_hand_stack().cloned();
        }
        attack
    }

    pub(in crate::game::server_side) fn knock_back_armor_stand(
        &mut self,
        id: u32,
        mut knock: Knock,
        source: mod_api::HurtSource<'_>,
        attack: &Attack,
    ) -> bool {
        let Some(index) = self
            .common
            .armor_stands
            .iter()
            .position(|stand| stand.id == id)
        else {
            return false;
        };
        let resistance = self.common.armor_stands[index]
            .live
            .read(
                &self.common.motion.attributes,
                "minecraft:armor_stand",
                "minecraft:knockback_resistance",
                sunrise_world::attribute_map::ValueKind::Computed,
                Vec::new,
            )
            .unwrap_or(0.0);
        let plan = self.prepare_knockback_credit(id as i32, None, attack);
        let mut credit = plan.lend(&mut self.server.last_hurt_by_player);
        let level = crate::game::server_side::entity_server::damage_level_of!(
            self,
            attack,
            &mut credit,
            None
        );
        let pushed = Hurting::ArmorStand(id as i32, source);
        if level.knockback_canceled(pushed, &mut knock) {
            return false;
        }
        let stand = &mut self.common.armor_stands[index];
        knock.apply(
            &mut stand.velocity,
            resistance,
            stand.ground.on_ground,
            level.random(),
        )
    }

    /// Server arm for living handle resolution.
    pub(in crate::game) fn server_living_read_target(&self, living: LivingRef) -> Option<Target> {
        self.native_living_target(living)
    }

    /// Server arm for whether a remembered hurt target is still alive (`native_surface`).
    pub(in crate::game) fn server_living_target_alive(&self, id: u32) -> bool {
        self.last_hurt_target_alive(id)
    }

    /// Server arm for how long `living` has been using its active item (`native_living_blocking`).
    pub(in crate::game) fn server_living_use_ticks(
        &self,
        living: LivingRef,
    ) -> Option<Option<u32>> {
        match self.server_living_read_target(living)? {
            Target::Player => Some(self.common.using.map(|(_, ticks)| ticks)),
            Target::Guest(conn) => Some(
                self.server
                    .lan
                    .as_ref()?
                    .play
                    .get(&conn)?
                    .using()
                    .map(|(_, t)| t),
            ),
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id)?;
                Some(
                    sunrise_sim::mobs::spear_use::is_using_item(mob)
                        .then(|| mob.extra.using_item.unsigned_abs() - 1),
                )
            }
            _ => None,
        }
    }

    /// Server arm for `LivingEntity.getUsedItemHand` (`native_living_blocking`).
    pub(in crate::game) fn server_living_used_item_hand(&self, living: LivingRef) -> Option<Hand> {
        self.living_used_item_hand(living)
    }

    /// Server arm for `AttributeMap.getValue`.
    pub(in crate::game) fn server_living_attribute_value(
        &self,
        living: LivingRef,
        id: &str,
    ) -> Option<f64> {
        self.native_living_attribute(
            living,
            id,
            sunrise_world::attribute_map::ValueKind::Computed,
        )
    }

    /// Adds an attribute modifier (`AttributeInstance.addTransientModifier`
    /// or `addPermanentModifier`). A permanent one is saved with the entity;
    /// a transient one lasts until it unloads. It goes where `/attribute`
    /// writes (`Game::write_attribute`), so gameplay and the entity's clients
    /// see it. False when the entity already has a modifier with that id on
    /// the attribute (vanilla throws), for an attribute the entity type does
    /// not have, a non-finite amount, the dragon (whose attributes nothing
    /// reads yet), a stale handle, or on a client.
    pub(in crate::game) fn living_add_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, false)
    }

    /// Adds an attribute modifier in place of the one with the same id, if
    /// any (`addOrUpdateTransientModifier`, `addOrReplacePermanentModifier`):
    /// the new one's lifetime wins. False as `add_attribute_modifier` is,
    /// except that a taken id is replaced.
    pub(in crate::game) fn living_replace_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
    ) -> bool {
        self.native_living_add_modifier(living, attribute, modifier, lifetime, true)
    }

    /// Removes the modifier with id `id` from the attribute
    /// (`AttributeInstance.removeModifier`), whatever its lifetime. False when
    /// there was none, and in the cases `add_attribute_modifier` is.
    pub(in crate::game) fn living_remove_attribute_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        id: &Identifier,
    ) -> bool {
        self.native_living_write(living, attribute, AttributeWrite::Remove(id.to_string()))
    }

    /// Sets the attribute's base value (`AttributeInstance.setBaseValue`);
    /// the value stays within the attribute's range. False for a non-finite
    /// base, and in the cases `add_attribute_modifier` is.
    pub fn living_set_attribute_base(
        &mut self,
        living: LivingRef,
        attribute: &str,
        base: f64,
    ) -> bool {
        base.is_finite()
            && self.native_living_write(
                living,
                attribute,
                AttributeWrite::Base(self.mod_base_in_range(attribute, base)),
            )
    }

    /// A mod's base clamped to the attribute's range, which `/attribute base set` and the entity's own code skip.
    pub(in crate::game::server_side) fn mod_base_in_range(
        &self,
        attribute: &str,
        base: f64,
    ) -> f64 {
        self.common
            .motion
            .attributes
            .definition(attribute)
            .map_or(base, |d| d.sanitize(base))
    }

    pub(in crate::game::server_side) fn native_living_add_modifier(
        &mut self,
        living: LivingRef,
        attribute: &str,
        modifier: AttributeModifier,
        lifetime: ModifierLifetime,
        replace: bool,
    ) -> bool {
        let operation = match modifier.operation {
            AttributeOperation::AddValue => Operation::AddValue,
            AttributeOperation::AddMultipliedBase => Operation::AddMultipliedBase,
            AttributeOperation::AddMultipliedTotal => Operation::AddMultipliedTotal,
            _ => unreachable!("AttributeOperation: unhandled variant"),
        };
        let permanent = match lifetime {
            ModifierLifetime::Transient => false,
            ModifierLifetime::Permanent => true,
            _ => unreachable!("ModifierLifetime: unhandled variant"),
        };
        if !modifier.amount.is_finite() {
            return false;
        }
        AttributeWrite::add(
            &modifier.id.to_string(),
            modifier.amount,
            operation,
            permanent,
            replace,
        )
        .is_some_and(|write| self.native_living_write(living, attribute, write))
    }

    pub(in crate::game::server_side) fn native_living_write(
        &mut self,
        living: LivingRef,
        name: &str,
        write: AttributeWrite,
    ) -> bool {
        self.native_living_target(living)
            .and_then(|t| self.write_attribute(t, name, write))
            .unwrap_or(false)
    }

    /// `Entity.canUsePortal`: the decision the game's own portal trip runs —
    /// a player's (alive, nobody's passenger, not asleep: the engine's own
    /// `player_can_use_portal`, the one gate the client's portal entry, the
    /// guest trip and this read answer), a mob's walk gate
    /// (`mob_can_use_portal`, #1275's read: not a boss that never travels,
    /// not a creaking bound to its heart, not asleep in its bed, riding
    /// nothing, carrying no one this side holds back). None for a stale
    /// handle, an armor stand or the dragon: the portal walk carries
    /// neither as its own traveller.
    pub(in crate::game) fn living_can_use_portal(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            // The engine's own gate, the one the client's portal entry runs
            // (`player_can_use_portal`, #943).
            Target::Player => Some(self.common.player_can_use_portal()),
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                let p = g.motion.as_deref()?;
                Some(
                    self.common
                        .can_use_portal_of(p, g.vehicle.is_some(), g.sleeping),
                )
            }
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                Some(self.mob_portal_eligible(m))
            }
            _ => None,
        }
    }

    /// `Mob.isWithinMeleeAttackRange`: the melee goal's own reach check — the
    /// mob's box inflated by sqrt(2.04) − 0.6 over the target's box, or an
    /// active item's attack range for a mob (a spear's reach) — whether this
    /// living entity's next melee strike reaches `target`. The check is a
    /// mob's; a player's attack reach is another rule (the interaction range
    /// attribute), so a player answers None.
    pub(in crate::game) fn living_in_melee_range(
        &self,
        living: LivingRef,
        target: EntityRef,
    ) -> Option<bool> {
        let Target::Mob(id) = self.native_living_read_target(living)? else {
            return None;
        };
        let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
        let [min, max] = self.entity_bounding_box(target)?;
        let bounds = sunrise_core::geometry::Aabb::new(
            sunrise_core::geometry::Vec3::new(min[0], min[1], min[2]),
            sunrise_core::geometry::Vec3::new(max[0], max[1], max[2]),
        );
        let w = self.mob_world_base(&[]);
        Some(m.melee_reaches(&w, bounds))
    }

    /// `LivingEntity.canFreeze`: the decision the freeze tick itself runs — a
    /// mob's (`ai::can_freeze`: not `#freeze_immune_entity_types`, no
    /// `#freeze_immune_wearables` worn), a player's (`PlayerMotion::can_freeze`:
    /// not a spectator, no immune armor). None for a stale handle, an armor
    /// stand or the dragon.
    pub(in crate::game) fn living_can_freeze(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let w = self.mob_world_base(&[]);
                Some(sunrise_sim::mobs::ai::can_freeze(m, &w))
            }
            t => Some(self.native_player_body(t)?.can_freeze()),
        }
    }

    /// Membership in the live `#can_breathe_under_water` tag for a mob.
    /// The tag read only, not the whole drown decision. None for a stale handle,
    /// an armor stand or the dragon.
    pub(in crate::game) fn living_can_breathe_underwater(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => {
                let mob = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                Some(self.items().entity_type_tags.has_bit(
                    mob.kind.flags.entity_tag_slot,
                    EntityTypeTags::CAN_BREATHE_UNDER_WATER,
                ))
            }
            Target::Player | Target::Guest(_) => Some(false),
            _ => None,
        }
    }

    /// The bed a sleeper lies in, as its block faces
    /// (`LivingEntity.getBedOrientation` over `BedBlock.getBedOrientation`):
    /// the block at the sleeping position, its `facing` where that block is
    /// one of `#minecraft:beds`. None where the sleeper keeps no position,
    /// the block is not a bed, or the handle is stale.
    pub(in crate::game) fn living_bed_orientation(&self, living: LivingRef) -> Option<Direction> {
        let pos = self.living_sleeping_pos(living)?;
        let w = self.mob_world_base(&[]);
        let name = w.block_name(pos[0], pos[1], pos[2]);
        if !w.has_tag("minecraft:beds", name) {
            return None;
        }
        match w.prop(pos[0], pos[1], pos[2], "facing") {
            Some("north") => Some(Direction::North),
            Some("south") => Some(Direction::South),
            Some("west") => Some(Direction::West),
            Some("east") => Some(Direction::East),
            _ => None,
        }
    }

    /// Whether the entity is alive (`LivingEntity.isAlive`; Paper's
    /// `Entity#isDead` is its inverse). A dying mob, mid death
    /// animation, is not.
    pub(in crate::game) fn living_alive(&self, living: LivingRef) -> Option<bool> {
        self.living_health(living).map(alive_at)
    }

    /// Sets the entity's health (`LivingEntity.setHealth`; Paper's
    /// `Damageable#setHealth`), cut to its maximum health. No heal or damage
    /// hook runs. False for a stale handle, a non-finite value, an armor stand
    /// or the dragon.
    pub(in crate::game) fn living_set_health(&mut self, living: LivingRef, health: f32) -> bool {
        if !health.is_finite() {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    let max = g.player.as_mut()?.attribute("minecraft:max_health").ok()? as f32;
                    let health = health.min(max);
                    let revived = g.player.as_ref()?.health.health() <= 0.0 && health > 0.0;
                    g.journal_player_health();
                    g.player.as_mut()?.set_health(health, false).ok()?;
                    if revived
                        && let Some(conn) = g.server.lan_acting
                        && let Some(host) = g.server.lan.as_mut()
                        && let Some(gi) =
                            host.hub.guests.iter().position(|guest| guest.conn == conn)
                        && host.hub.guests[gi].death_canceled
                    {
                        host.hub.guests.update(gi, |guest| {
                            guest.death_canceled = false;
                            guest.death_time = 0;
                        });
                        g.player.as_mut()?.health.dead = false;
                    }
                    Some(())
                })
                .flatten()
                .is_some(),
            Some(Target::Mob(id)) => {
                self.journal_mob_health(id);
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                let revived =
                    m.death_canceled && m.health <= 0.0 && health.min(m.attrs.max_health) > 0.0;
                m.health = health.clamp(0.0, m.attrs.max_health);
                m.needs_sync = true;
                if revived {
                    m.death_canceled = false;
                    m.death_time = 0;
                    self.server.died.remove(&id);
                }
                true
            }
            _ => false,
        }
    }
    /// Heals the entity by `amount` (`LivingEntity.heal`), through the
    /// `living_heal` hook, which may change or cancel it. A dead or dying
    /// entity isn't healed, and health stops at the maximum. False for a stale
    /// handle, a non-finite amount, a zero-health entity, an armor stand or the dragon.
    pub(in crate::game) fn living_heal(&mut self, living: LivingRef, amount: f32) -> bool {
        if !amount.is_finite()
            || !self
                .living_health(living)
                .is_some_and(|health| health > 0.0)
        {
            return false;
        }
        match self.native_living_target(living) {
            Some(t @ (Target::Player | Target::Guest(_))) => self
                .for_player(t, |g| {
                    g.apply_effect_action(sunrise_level::effects::EffectAction::Heal(amount));
                })
                .is_some(),
            Some(Target::Mob(id)) => {
                let Some(m) = self
                    .common
                    .mobs
                    .iter_mut()
                    .find(|m| m.id == id && !m.removed)
                else {
                    return false;
                };
                m.heal(
                    &self.common.loop_context,
                    &|| crate::game::entity_server::hit_reads_of!(self),
                    amount,
                );
                m.needs_sync = true;
                true
            }
            _ => false,
        }
    }

    /// `LivingEntity.getKillCredit`: the remembered player takes precedence,
    /// even if the player cannot be resolved; otherwise the last hurting mob,
    /// in whichever loaded level it is now (`credited_entity`).
    pub(in crate::game) fn living_kill_credit(&self, living: LivingRef) -> Option<EntityRef> {
        let credit = match self.native_living_target(living)? {
            Target::Player => self
                .server
                .combat
                .0
                .player_credit(self.common.world.game_time),
            Target::Guest(conn) => self
                .server
                .guest_combat
                .0
                .get(&conn)
                .and_then(|combat| combat.player_credit(self.common.world.game_time)),
            Target::Mob(id) => self.server.last_hurt_by_player.get(&id).copied(),
            _ => None,
        };
        if let Some(credit) = credit {
            let conn = self.resolve_player_credit(credit)?;
            let id = self.player_entity_id_by_connection(conn)?;
            return self.entity(u32::try_from(id).ok()?);
        }
        self.living_last_attacker(living)
    }

    /// `LivingEntity.getLastHurtByMob`: the entity that last hurt this one,
    /// within a hundred ticks of the hurt (the window of
    /// `CombatTracker.getKiller`'s credit and a mob's hurt-by memory, as
    /// `armadillo.rs` reads it). None for a stale handle, a window that has
    /// passed, an attacker that is gone, an armor stand, the dragon or a
    /// client.
    pub(in crate::game) fn living_last_attacker(&self, living: LivingRef) -> Option<EntityRef> {
        match self.native_living_target(living)? {
            Target::Player => self
                .server
                .combat
                .0
                .kill_credit(self.common.world.game_time)
                .and_then(|id| self.credited_entity(id)),
            Target::Guest(conn) => self
                .server
                .guest_combat
                .0
                .get(&conn)?
                .kill_credit(self.common.world.game_time)
                .and_then(|id| self.credited_entity(id)),
            Target::Mob(id) => {
                let m = self.common.mobs.iter().find(|m| m.id == id && !m.removed)?;
                let who = m.recent_hurt_by()?;
                let attacker = match who {
                    Who::Mob(attacker) => {
                        return self.credited_entity(attacker);
                    }
                    // A player's hurt carries no id in `Who`: the hit's
                    // record names the player (`last_hurt_by_player`), the
                    // hub the guest's entity. The acting guest is resolved
                    // without the hub, which their turn takes out; the
                    // host's own record is displaced for that turn — the
                    // guest's body stands in as `player`, so the host's id
                    // is out of reach (the same None the other players'
                    // handles read while the hub is out).
                    Who::Player => {
                        let conn =
                            self.resolve_player_credit(*self.server.last_hurt_by_player.get(&id)?)?;
                        match conn {
                            None if self.server.lan_acting.is_some() => None,
                            None => Some(self.common.player.as_ref()?.entity_id as u32),
                            Some(conn) if self.server.lan_acting == Some(conn) => {
                                Some(self.server.lan_acting_entity?)
                            }
                            Some(conn) => Some(
                                self.server
                                    .lan
                                    .iter()
                                    .flat_map(|h| h.hub.guests.iter())
                                    .find(|g| g.conn == conn)?
                                    .entity_id as u32,
                            ),
                        }
                    }
                }?;
                self.entity(attacker)
            }
            _ => None,
        }
    }

    pub(in crate::game) fn living_last_hurt_mob(
        &self,
        living: LivingRef,
    ) -> Option<(EntityRef, i32)> {
        let (id, timestamp) = self.living_last_hurt_record(living)?;
        let target = self.entity(id)?;
        self.last_hurt_target_alive(id)
            .then_some((target, timestamp))
    }

    /// The holder's `lastHurtMob` id and `lastHurtMobTimestamp`, its target unresolved.
    pub(in crate::game::server_side) fn living_last_hurt_record(
        &self,
        living: LivingRef,
    ) -> Option<(u32, i32)> {
        match self.native_living_target(living)? {
            Target::Player => self.common.player.as_ref()?.health.last_hurt_mob,
            Target::Guest(conn) => {
                let guest = self.command_guest(conn)?;
                match &guest.motion {
                    Some(body) => body.health.last_hurt_mob,
                    None => guest.carried_last_hurt_mob,
                }
            }
            Target::Mob(id) => {
                let (_, join) = self.last_hurt_living_target(id)?;
                self.common.mobs[self.common.mobs.locate(join)?].last_hurt_mob
            }
            Target::Stand(id) => {
                let (_, join) = self.last_hurt_living_target(id)?;
                self.common.armor_stands[self.common.armor_stands.locate(join)?].last_hurt_mob
            }
            Target::Dragon(id) => {
                self.common
                    .dragon
                    .as_ref()
                    .filter(|d| d.id == id)?
                    .last_hurt_mob
            }
            _ => None,
        }
    }

    /// `EntityReference.getEntity` for a kill credit, which looks through
    /// every level (`getEntityInAnyDimension`): the entity `id` in the level
    /// in place, else [`Self::credited_elsewhere`].
    pub(in crate::game::server_side) fn credited_entity(&self, id: u32) -> Option<EntityRef> {
        self.entity(id).or_else(|| self.credited_elsewhere(id))
    }

    /// A credited mob that is not in the level in place under `id`: the same
    /// id in another loaded level (a vehicle keeps it), or the id the
    /// dimension changes since gave that same mob (`Levels::travelled`), in
    /// whichever loaded level it is now. Never another entity holding its UUID.
    pub(in crate::game::server_side) fn credited_elsewhere(&self, id: u32) -> Option<EntityRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let mut id = id;
        loop {
            if self.mob_in_other_level(id).is_some() {
                return Some(EntityRef::new(epoch, id));
            }
            id = self.server.levels.0.travelled(id)?;
            if let Some(found) = self.entity(id) {
                return Some(found);
            }
        }
    }

    /// Server arm for resolving one live equipment slot without copying.
    pub(in crate::game) fn server_living_equipment_stack(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'_>> {
        self.equipment_stack_for(self.server_living_read_target(living)?, slot)
    }

    /// Read one equipment slot of an already resolved target.
    pub(in crate::game) fn equipment_stack_for(
        &self,
        target: Target,
        slot: EquipmentSlot,
    ) -> Option<EquipmentRead<'_>> {
        let body = match target {
            // A removed mob still lends what it wears.
            Target::Mob(id) => {
                sunrise_sim::health::LivingBody::Mob(self.common.mobs.iter().find(|m| m.id == id)?)
            }
            target => self.native_living_body(target)?,
        };
        super::native_living_body::body_equipment(body, slot)
    }

    /// Read the drop chance for one equipment slot of a mob in the active level.
    pub(in crate::game) fn living_equipment_drop_chance(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
    ) -> Option<f32> {
        if !self.common.is_logical_server() {
            return None;
        }
        let Target::Mob(id) = self.native_living_target(living)? else {
            return None;
        };
        let mob = self.common.mobs.iter().find(|m| m.id == id)?;
        let slot = match slot {
            EquipmentSlot::MainHand => "mainhand",
            EquipmentSlot::OffHand => "offhand",
            EquipmentSlot::Feet => "feet",
            EquipmentSlot::Legs => "legs",
            EquipmentSlot::Chest => "chest",
            EquipmentSlot::Head => "head",
            EquipmentSlot::Body => "body",
            EquipmentSlot::Saddle => "saddle",
        };
        Some(sunrise_sim::mobs::drops::drop_chance(mob, slot))
    }

    /// Damage an equipped stack without an equip/unequip notification.
    pub fn living_hurt_and_break(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let target = self.native_living_target(living)?;
        if matches!(target, Target::Player | Target::Guest(_)) {
            return self
                .for_player(target, |game| game.damage_player_equipment(slot, amount))
                .flatten();
        }
        let stack = self
            .equipment_stack_for(target, slot)?
            .project()
            .into_owned();
        let wearer = super::durability::Wearer::Living(living.id());
        let (updated, broken) = self.damage_stack_worn(&stack, amount, wearer);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        if self
            .living_equipment_stack(living, slot)?
            .project()
            .as_ref()
            != &stack
        {
            return None;
        }
        match target {
            Target::Mob(id) => {
                self.common
                    .mobs
                    .iter_mut()
                    .find(|mob| mob.id == id)?
                    .set_equipment_stack(slot as usize, updated);
            }
            Target::Stand(id) => {
                self.common
                    .armor_stands
                    .iter_mut()
                    .find(|stand| stand.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            Target::Dragon(id) => {
                self.common
                    .dragon
                    .as_mut()
                    .filter(|dragon| dragon.id == id)?
                    .equipment
                    .set(slot as usize, updated);
            }
            _ => return None,
        }
        if broken.is_some() {
            let event =
                sunrise_sim::mobs::item_break::equipment_break_event(world_equipment_slot(slot));
            // Non-player equipment writes above are unjournaled and take effect immediately; keep
            // the coupled break event on the same immediate side of the rollback boundary.
            self.broadcast_entity_event_immediate(living.entity(), event as u8);
        }
        broken
    }

    pub fn damage_player_equipment(
        &mut self,
        slot: EquipmentSlot,
        amount: i32,
    ) -> Option<ItemStack> {
        let stack = player_slot(
            &self.common.inventory,
            self.common.selected,
            &self.common.armor,
            &self.common.offhand,
            &self.common.equipment,
            slot,
        )?
        .clone();
        let (updated, broken) = self.damage_stack(&stack, amount, true);
        if updated.as_ref() == Some(&stack) {
            return None;
        }
        let cell = match slot {
            EquipmentSlot::MainHand => &mut self.common.inventory[self.common.selected],
            EquipmentSlot::OffHand => &mut self.common.offhand,
            EquipmentSlot::Body | EquipmentSlot::Saddle => {
                if self.common.equipment.get(slot as usize) != Some(&stack) {
                    return None;
                }
                self.common.equipment.set(slot as usize, updated);
                if let Some(ref broken) = broken {
                    self.award_stat("broken", &broken.item, 1);
                    self.player_equipment_broke(world_equipment_slot(slot), broken);
                    self.player_item_destroyed(&stack, None);
                }
                return broken;
            }
            _ => &mut self.common.armor[armor_index(slot)?],
        };
        if cell.as_ref() != Some(&stack) {
            return None;
        }
        *cell = updated;
        if let Some(ref broken) = broken {
            self.award_stat("broken", &broken.item, 1);
            self.player_equipment_broke(world_equipment_slot(slot), broken);
            let hand = match slot {
                EquipmentSlot::MainHand => Some(Hand::Main),
                EquipmentSlot::OffHand => Some(Hand::Off),
                _ => None,
            };
            self.player_item_destroyed(&stack, hand);
            if armor_index(slot).is_some() {
                self.apply_equipment();
            }
        }
        broken
    }

    /// The effects of `LivingEntity.onEquipItem` once the guards passed (server
    /// side, not a spectator, not the first tick, another item): the equip
    /// sound when there is one, and the `equip` (the stack is equippable) or
    /// `unequip` game event.
    pub(in crate::game) fn equip_item_effects(
        &mut self,
        at: [f64; 3],
        sound: Option<String>,
        equips: bool,
        ctx: sunrise_level::sculk::Context,
    ) {
        if let Some(sound) = sound {
            self.server_play_sound(&sound, at, 1.0, 1.0);
        }
        self.post_game_event(
            if equips {
                "minecraft:equip"
            } else {
                "minecraft:unequip"
            },
            at,
            ctx,
        );
    }

    /// Puts `stack` in hand `hand` (`LivingEntity.setItemInHand`): the main
    /// hand's slot or the off hand's, written as `set_equipment`
    /// writes them, with its equip events and sound. None, air or a zero count
    /// empties the hand. False means unsupported or stale.
    pub fn living_set_hand_item(
        &mut self,
        living: LivingRef,
        hand: Hand,
        stack: Option<ItemStack>,
    ) -> bool {
        self.living_set_equipment(living, hand_slot(hand), stack)
    }
    /// Puts `stack` in `slot` (`LivingEntity.setItemSlot`); None, air or a
    /// zero count empties it. Players, guests, mobs, armor stands and the
    /// dragon store all eight slots. An armor stand stores body and saddle
    /// too: `ArmorStand.canUseSlot` only stops interaction, and
    /// `LivingEntity.setItemSlot` keeps whatever it is given. False means
    /// unsupported or stale.
    /// This is a direct write: it takes effect and is final immediately, even
    /// inside `host.transaction`. A mob's write makes any equip/unequip event
    /// and equip sound immediately; a stand's or the dragon's makes neither. API 382.
    pub fn living_set_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        self.write_living_equipment(living, slot, stack, false)
    }
    /// The write `native_living_set_equipment` makes (#932): with `defer`,
    /// its equip sound and game event wait for the root commit of the open
    /// transaction — an abort stays quiet — while the equipment and the
    /// attribute map change at once, as later steps of the same transaction
    /// read them.
    pub(in crate::game::server_side) fn write_living_equipment(
        &mut self,
        living: LivingRef,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
        defer: bool,
    ) -> bool {
        // A client's copy changes only when the server says so.
        if !self.common.is_logical_server() {
            return false;
        }
        let Some(t) = self.native_living_target(living) else {
            return false;
        };
        let stack = equipment_stack(stack);
        let equippable = stack.as_ref().and_then(|s| self.equippable_slot_sound(s));
        if let Target::Mob(id) = t {
            let Some(mob) = self.common.mobs.iter_mut().find(|m| m.id == id) else {
                return false;
            };
            let changed = {
                let old = if (slot as usize) <= finalize::HEAD {
                    finalize::item_stack(mob, slot as usize).map(std::borrow::Cow::Owned)
                } else {
                    mob.extra
                        .slot_stack(slot as usize)
                        .map(std::borrow::Cow::Borrowed)
                };
                match (old.as_deref(), &stack) {
                    (None, None) => false,
                    (Some(a), Some(b)) => !a.same_item_same_components(b),
                    _ => true,
                }
            };
            if !mob.set_equipment_stack(slot as usize, stack) {
                return false;
            }
            if changed && !mob.first_tick {
                let at = mob.position;
                let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                    (!mob.silent && *equip_slot as usize == slot as usize)
                        .then(|| {
                            super::dispenser::equip_sound(mob, *equip_slot, Some(sound.clone()))
                        })
                        .flatten()
                });
                if defer {
                    self.after_commit(sunrise_gameplay_common::game::transactions::After::Equip {
                        at: [at.x, at.y, at.z],
                        sound,
                        equips: equippable.is_some(),
                        who: id,
                    });
                } else {
                    let ctx = self.who_context(Some(sunrise_sim::mobs::Who::Mob(id)), None);
                    self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
                }
            }
            // The stack's attribute modifiers go on the live map at once, so
            // the next read — a hit's armor, a `LivingRef`'s — sees them (#941).
            let before = if defer {
                self.snapshot_mob_modifiers(id)
            } else {
                None
            };
            self.collect_mob_equipment_attributes(id, before);
            return true;
        }
        // `LivingEntity.onEquipItem` for the entities that keep no equip state
        // of their own: the stand and the dragon (not silent, never spectators).
        let equip_event_at = match t {
            Target::Stand(id) => self
                .common
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| (a.equipment.get(slot as usize), a.position, a.first_tick)),
            Target::Dragon(id) => self
                .common
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| (d.equipment.get(slot as usize), d.position, d.first_tick())),
            _ => None,
        }
        .and_then(|(old, at, first_tick)| {
            let same = match (old, stack.as_ref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_item_same_components(b),
                _ => false,
            };
            (!first_tick && !same).then_some(at)
        });
        let written = match self
            .slots_named(&Owner::Entity(t), slot_name(slot))
            .as_slice()
        {
            // This path owns onEquipItem below, including immediate writes.
            // The generic stand slot writer must not emit it a second time.
            [s] => self.slot_set_with_effects(s, stack, false),
            _ => false,
        };
        if let (true, Some(at)) = (written, equip_event_at) {
            let sound = equippable.as_ref().and_then(|(equip_slot, sound)| {
                (*equip_slot as usize == slot as usize).then(|| sound.clone())
            });
            let (Target::Stand(id) | Target::Dragon(id)) = t else {
                unreachable!("only a stand or the dragon gets here")
            };
            if defer {
                self.after_commit(sunrise_gameplay_common::game::transactions::After::Equip {
                    at: [at.x, at.y, at.z],
                    sound,
                    equips: equippable.is_some(),
                    who: id,
                });
            } else {
                let ctx_who = Some(sunrise_sim::mobs::Who::Mob(id));
                let ctx = self.who_context(ctx_who, None);
                self.equip_item_effects([at.x, at.y, at.z], sound, equippable.is_some(), ctx);
            }
        }
        written
    }

    /// The slot and sound `stack`'s `equippable` component names
    /// (`Equippable.slot`, `equipSound`).
    pub(in crate::game::server_side) fn equippable_slot_sound(
        &self,
        stack: &ItemStack,
    ) -> Option<(super::dispenser::Slot, String)> {
        let name = sunrise_sim::item_sync::equippable_slot(stack, self.items())?;
        let equip_slot = super::dispenser::Slot::from_name(&name)?;
        let sound = match stack.data().and_then(|d| d.other("minecraft:equippable")) {
            Some(component) => match component.get("equip_sound") {
                Some(sunrise_core::nbt::Tag::String(sound)) => Some(sound.to_string_lossy()),
                _ => None,
            },
            None => self
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_sound.clone()),
        }
        .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into());
        Some((equip_slot, sound))
    }

    /// Throws `stack`, which the mod holds, from the eyes (`LivingEntity.drop`,
    /// `ServerPlayer.drop`), never through a slot: along the view with a
    /// main-hand swing, or with `throw_randomly` scattered with none, as a
    /// death's drops. `include_thrower_name` sets the thrower, and a player's
    /// throw along the view then counts in its stats and fires `item_toss`.
    /// None for an empty stack, a stand or the dragon, a stale handle, or an
    /// item a mod kept out. API @API:living_refs@.
    pub fn living_drop(
        &mut self,
        living: LivingRef,
        stack: ItemStack,
        throw_randomly: bool,
        include_thrower_name: bool,
    ) -> Option<EntityRef> {
        if self.entity_ids_exhausted() {
            return None;
        }
        let target = self.native_living_target(living)?;
        let stack = equipment_stack(Some(stack))?;
        let mut item = None;
        match target {
            Target::Mob(id) => {
                if !throw_randomly {
                    self.living_swing(living, Hand::Main, true);
                }
                item = self.native_mob_drop(id, stack, throw_randomly, include_thrower_name);
            }
            Target::Player | Target::Guest(_) => {
                self.native_as_living_player(target, |g| {
                    item = g.native_player_drop(stack, throw_randomly, include_thrower_name);
                });
            }
            _ => {}
        }
        Some(EntityRef::new(living.entity().epoch(), item?))
    }
    /// `TamableAnimal.isTame`, `AbstractHorse.isTamed`: None for a kind that
    /// can't be tamed or a stale handle. API @API:living_refs@.
    pub fn living_is_tamed(&self, living: LivingRef) -> Option<bool> {
        self.native_tamable(living).map(|m| m.extra.tame)
    }
    /// The owner's UUID (`OwnableEntity.getOwnerReference`): None for a wild
    /// mob, a kind that can't be tamed, or a stale handle. API @API:living_refs@.
    pub fn living_owner(&self, living: LivingRef) -> Option<u128> {
        let owner = self.native_tamable(living)?.extra.owner?;
        Some(sunrise_sim::lan::playerdata::ints_uuid(owner))
    }
    /// The living entity as a player, with its UUID: the player in place (a guest's while it
    /// acts) or a guest. Resolved through [`Self::player`], so it names the same player
    /// `host.player(uuid)` does, and only when that player is this entity. None for any other
    /// living entity, a fake player acting, a stale handle or on a client.
    pub(in crate::game) fn living_player(&self, living: LivingRef) -> Option<mod_api::PlayerRef> {
        if !self.common.is_logical_server() {
            return None;
        }
        let uuid = match self.native_living_target(living)? {
            Target::Player => self.server.lan_acting_uuid.or_else(|| {
                self.common
                    .level
                    .player_uuid
                    .map(sunrise_sim::lan::playerdata::ints_uuid)
            })?,
            Target::Guest(conn) => self.command_guest(conn)?.uuid,
            _ => return None,
        };
        self.player(uuid)
            .filter(|player| player.entity() == living.entity())
    }
    /// The mob a handle names, when it is a `TamableAnimal` or an `AbstractHorse`.
    pub(in crate::game::server_side) fn native_tamable(&self, living: LivingRef) -> Option<&Mob> {
        let Target::Mob(id) = self.native_living_target(living)? else {
            return None;
        };
        self.common
            .mobs
            .iter()
            .find(|m| m.id == id && m.is_tamable())
    }
    /// Runs `f` as the player `target` names: the one acting now, or a guest
    /// in the level in place through `as_guest` (`native_as_player`). False
    /// when it didn't run.
    pub(in crate::game::server_side) fn native_as_living_player(
        &mut self,
        target: Target,
        f: impl FnOnce(&mut Self),
    ) -> bool {
        let uuid = match target {
            Target::Guest(conn) => match self.command_guest(conn) {
                Some(g) if g.dimension == self.common.dim.id() => g.uuid,
                _ => return false,
            },
            _ => self.native_acting_id(),
        };
        self.native_as_player(uuid, f)
    }
    /// The acting player's half of `living_drop`: the id of the item entity.
    pub fn native_player_drop(
        &mut self,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        if randomly {
            let eye = self.common.eye_position()?;
            self.throw_from_eyes(stack, eye, true, thrown_from_hand)
        } else if thrown_from_hand {
            self.player_drop(stack)
        } else {
            self.drop_plain(stack)
        }
    }
    /// A mob's half of `living_drop`: the id of the item entity.
    pub(in crate::game::server_side) fn native_mob_drop(
        &mut self,
        id: u32,
        stack: ItemStack,
        randomly: bool,
        thrown_from_hand: bool,
    ) -> Option<u32> {
        let m = self.common.mobs.iter_mut().find(|m| m.id == id)?;
        let v = sunrise_sim::entities::drop_motion(&mut m.random, m.yaw, m.head_pitch, randomly);
        let at = [m.position.x, m.eye_y() - f64::from(0.3f32), m.position.z];
        let thrower = thrown_from_hand.then(|| self.uuid_for(id));
        self.spawn_item(stack, at, v, 40);
        let item = self.just_spawned_item()?;
        item.thrower = thrower;
        Some(item.id)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests {
    use sunrise_gameplay_common::game::server_side::native_living_refs::tests::ZOMBIE_NBT;

    use super::*;

    pub fn summon_mob(game: &mut crate::game::ServerGame<'_>, kind: &str, nbt: &str) -> u32 {
        match game
            .summon_with_nbt(kind, [0.5, 70.0, 0.5], nbt)
            .unwrap()
            .unwrap()
        {
            Target::Mob(id) => id,
            other => panic!("{kind} summoned as {other:?}"),
        }
    }

    /// A pig hurt by a zombie of [`ZOMBIE_UUID`] in the Overworld, with the
    /// Nether's spawn chunk forced loaded so it stays parked beside it.
    pub fn hurt_pig(game: &mut crate::game::ServerGame<'_>) -> (u32, u32) {
        use std::sync::Arc;
        use sunrise_gameplay_common::game::Dim;
        use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
        use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
        game.server.world_root = game.server.save.0.root.clone();
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        game.block_ticks
            .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
            .unwrap();
        game.build_registries();
        assert!(game.as_console(|game| game.run_command("forceload add 0 0")));
        game.enter_level(Dim::Overworld).unwrap();
        fixture_resident_column(&mut game.world, (0, 0));
        let pig = summon_mob(game, "minecraft:pig", "{NoAI:1b}");
        let zombie = summon_mob(game, "minecraft:zombie", ZOMBIE_NBT);
        let hit = sunrise_sim::mobs::Damage::mob(zombie, [0.5, 70.0, 0.5]);
        assert!(game.mob_hits_mob(pig, 1.0, hit));
        let living = game.living(game.entity(pig).unwrap()).unwrap();
        assert_eq!(living.kill_credit(game).map(|e| e.id()), Some(zombie));
        (pig, zombie)
    }

    pub fn send_to_the_nether(game: &mut crate::game::ServerGame<'_>, mob: u32) {
        use {crate::game::vehicle_teleport::TeleportMotion, sunrise_gameplay_common::game::Dim};
        assert!(
            game.teleport_vehicle_across(
                Target::Mob(mob),
                Dim::Nether,
                [0.5, 70.0, 0.5],
                None,
                false,
                TeleportMotion::Stop,
            )
            .unwrap()
        );
        assert_eq!(game.dim, Dim::Overworld);
    }

    #[cfg(test)]
    #[test]
    fn alive_is_health_above_zero_as_in_vanilla() {
        assert!(alive_at(0.5));
        assert!(!alive_at(0.0));
        assert!(!alive_at(-1.0));
        assert!(!alive_at(f32::NAN));
    }

    #[cfg(test)]
    #[test]
    fn air_and_zero_count_are_empty_on_write() {
        assert!(equipment_stack(Some(ItemStack::new("minecraft:air", 1))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 0))).is_none());
        assert!(equipment_stack(Some(ItemStack::new("minecraft:stone", 1))).is_some());
    }

    #[cfg(test)]
    #[test]
    fn a_players_slots_come_from_its_inventory_armour_and_off_hand() {
        let stack = |id: &str| Some(ItemStack::new(id, 1));
        let inventory = vec![stack("minecraft:stone"), stack("minecraft:dirt")];
        let armor = [
            stack("minecraft:iron_helmet"),
            None,
            None,
            stack("minecraft:iron_boots"),
        ];
        let offhand = stack("minecraft:shield");
        let mut equipment = sunrise_items::equipment::Equipment::default();
        equipment.set(sunrise_items::equipment::SADDLE, stack("minecraft:saddle"));
        let at = |slot| player_slot(&inventory, 1, &armor, &offhand, &equipment, slot);
        let id = |slot| at(slot).map(|s| s.item.as_ref());
        assert!(std::ptr::eq(
            at(EquipmentSlot::MainHand).unwrap(),
            inventory[1].as_ref().unwrap()
        ));
        assert_eq!(id(EquipmentSlot::MainHand), Some("minecraft:dirt"));
        assert_eq!(id(EquipmentSlot::OffHand), Some("minecraft:shield"));
        assert_eq!(id(EquipmentSlot::Head), Some("minecraft:iron_helmet"));
        assert_eq!(id(EquipmentSlot::Feet), Some("minecraft:iron_boots"));
        assert_eq!(id(EquipmentSlot::Chest), None);
        assert_eq!(id(EquipmentSlot::Body), None);
        assert_eq!(id(EquipmentSlot::Saddle), Some("minecraft:saddle"));
    }

    #[cfg(test)]
    #[test]
    fn every_equipment_slot_has_its_own_item_command_name() {
        let names = EquipmentSlot::ALL.map(slot_name);
        for (i, n) in names.iter().enumerate() {
            assert!(!names[..i].contains(n), "{n} named twice");
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::block_props::BlockPropsGameExt2 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt2 as _;

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt5 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt1 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt3 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt4 as _;

use sunrise_gameplay_common::game::native_living_body::NativeLivingBodyGameExt6 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

use sunrise_gameplay_common::game::resident_state::ResidentStateGameExt1 as _;
