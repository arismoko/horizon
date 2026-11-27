//! A living entity's reads, one function per field over its borrowed body
//! ([`LivingBody`]): the `LivingRef` reads (docs/modding/src/how-to/native-mods.md, "Entity
//! and level handles") find the body their handle names and call these, and a hit's
//! `LivingView` calls the same ones on the body the hit lends, so both answer alike.
use crate::game::{
    commands_common::Target,
    dragon_common::{Dragon, MAX_HEALTH},
    native_equipment_read::{hand_slot, lend_equipment},
};
use mod_api::{
    __private::LivingParty, ActiveEffect, AttributeModifierView, AttributeOperation, EffectKey,
    EntityReadError, EquipmentSlot, ItemStackView, LivingRef, Pose,
};
use std::{ops::ControlFlow, sync::Arc};
use sunrise_core::{geometry::Vec3, position::BlockPos};
use sunrise_level::effects::{EffectInstance, PlayerEffects};
use sunrise_sim::health::{FixedBody, LivingBody, PlayerBody, PlayerHealthRead};
use sunrise_sim::mobs::finalize::{self, EquipmentRead};
use sunrise_sim::{
    first_person_sim::Hand,
    mobs::{Mob, spear_use, villager},
};
use sunrise_world::attribute_map::ValueKind;
use sunrise_world::attributes::entity::Catalog;
use sunrise_world::{
    attributes::{Modifier, Operation},
    climbing::{self, ClimbingWorld},
    registry::BlockStateId,
};

/// `LivingEntity.getHealth`.
pub(in crate::game) fn body_health(body: LivingBody<'_>) -> Option<f32> {
    match body {
        LivingBody::Mob(m) => Some(m.health),
        LivingBody::Player(p) => match p.health {
            PlayerHealthRead::Synced(data, ids) => data.get(ids.health).ok(),
            PlayerHealthRead::Record { health, .. } => Some(*health),
        },
        // An armor stand carries the fixed 20 the game gives it: lava
        // does not hurt it and splash water reads the very field.
        LivingBody::Stand(s) | LivingBody::Dragon(s) => Some(*s.health),
    }
}

/// `LivingEntity.getAbsorptionAmount`: none for an armor stand or the dragon, which carry none.
pub fn body_absorption(body: LivingBody<'_>) -> Option<f32> {
    match body {
        LivingBody::Mob(m) => Some(m.absorption),
        LivingBody::Player(p) => match p.health {
            PlayerHealthRead::Synced(data, ids) => data.get(ids.absorption).ok(),
            PlayerHealthRead::Record {
                body,
                saved_absorption,
                ..
            } => Some(body.map_or(saved_absorption.unwrap_or(0.0), |h| h.absorption())),
        },
        LivingBody::Stand(_) | LivingBody::Dragon(_) => None,
    }
}

/// `LivingEntity.getMaxHealth`. Players read their attribute map; a mob, an armor stand and
/// the dragon their own fields (the armor stand the fixed 20 the game seeds it with, the
/// dragon its 200) until live attribute maps cover them.
pub(in crate::game) fn body_max_health(body: LivingBody<'_>) -> Option<f32> {
    match body {
        LivingBody::Mob(m) => Some(m.attrs.max_health),
        LivingBody::Player(p) => {
            player_attribute(p, "minecraft:max_health", ValueKind::Computed).map(|max| max as f32)
        }
        LivingBody::Stand(_) => Some(20.0),
        LivingBody::Dragon(_) => Some(MAX_HEALTH),
    }
}

/// `AttributeMap.getValue` or `getBaseValue` of `name`: None for an attribute the entity
/// type does not have. A mob, an armor stand and the dragon answer from their live attribute
/// map, built from `catalog` on the first read.
pub(in crate::game) fn body_attribute(
    body: LivingBody<'_>,
    catalog: &Arc<Catalog>,
    name: &str,
    kind: ValueKind,
) -> Option<f64> {
    match body {
        LivingBody::Mob(m) => m.extra.live.read(catalog, &m.kind.id, name, kind, || {
            sunrise_sim::entity_nbt::attribute_entries(m)
                .into_iter()
                .map(|(_, entry)| entry)
                .collect()
        }),
        LivingBody::Player(p) => player_attribute(p, name, kind),
        LivingBody::Stand(s) => s
            .live
            .read(catalog, "minecraft:armor_stand", name, kind, Vec::new),
        LivingBody::Dragon(d) => {
            d.live
                .read(catalog, "minecraft:ender_dragon", name, kind, Vec::new)
        }
    }
}

pub(in crate::game) fn player_attribute(
    player: PlayerBody<'_>,
    name: &str,
    kind: ValueKind,
) -> Option<f64> {
    let attributes = player.attributes?;
    let key = attributes.key(name)?;
    attributes.value(Some(key), kind).ok()
}

/// `LivingEntity.getEffect` of the effect at registry index `index`: none for an armor stand
/// or the dragon, which keep no effects.
pub(in crate::game) fn body_effect(body: LivingBody<'_>, index: usize) -> Option<ActiveEffect> {
    match body {
        LivingBody::Mob(m) => {
            let e = m
                .extra
                .effects
                .iter()
                .find(|e| e.registry_index == Some(index))?;
            Some(mob_effect(m, e))
        }
        LivingBody::Player(p) => p.effects?.get_registered(index).map(active_effect),
        LivingBody::Stand(_) | LivingBody::Dragon(_) => None,
    }
}

/// `LivingEntity.getActiveEffects`, in the order their store keeps them.
pub(in crate::game) fn body_effects(body: LivingBody<'_>) -> Option<Vec<ActiveEffect>> {
    match body {
        LivingBody::Mob(m) => Some(m.extra.effects.iter().map(|e| mob_effect(m, e)).collect()),
        LivingBody::Player(p) => Some(p.effects?.list.iter().map(active_effect).collect()),
        LivingBody::Stand(_) | LivingBody::Dragon(_) => None,
    }
}

/// `LivingEntity.hasEffect` of the effect at registry index `index`.
pub(in crate::game) fn body_potion_affected(body: LivingBody<'_>, index: usize) -> bool {
    match body {
        LivingBody::Mob(m) => m
            .extra
            .effects
            .iter()
            .any(|e| e.registry_index == Some(index)),
        LivingBody::Player(p) => p
            .effects
            .is_some_and(|effects| effects.get_registered(index).is_some()),
        LivingBody::Stand(_) | LivingBody::Dragon(_) => false,
    }
}

/// `LivingEntity.getItemBySlot`: the live stack, never a copy.
pub fn body_equipment(body: LivingBody<'_>, slot: EquipmentSlot) -> Option<EquipmentRead<'_>> {
    let stack = match body {
        LivingBody::Mob(m) => return finalize::equipment_slot(m, slot),
        LivingBody::Player(p) => *p.equipment.get(slot as usize)?,
        LivingBody::Stand(s) | LivingBody::Dragon(s) => s.equipment.get(slot as usize),
    }?;
    Some(EquipmentRead::resident(stack))
}

/// The type a living body is of (`Entity.getType`), by name: the one mapping the full host's
/// `entity_kind` and a hit's view both resolve.
pub fn body_kind_name<'a>(body: LivingBody<'a>) -> &'a str {
    match body {
        LivingBody::Mob(m) => &m.kind.id,
        LivingBody::Player(_) => PLAYER,
        LivingBody::Stand(_) => ARMOR_STAND,
        LivingBody::Dragon(_) => ENDER_DRAGON,
    }
}

/// A player's, an armor stand's and the dragon's type names.
pub const PLAYER: &str = "minecraft:player";
pub const ARMOR_STAND: &str = "minecraft:armor_stand";
pub const ENDER_DRAGON: &str = "minecraft:ender_dragon";

/// The registry key of the type a living body is of (`Entity.getType`), as the accepted
/// resources resolved it: a mob's type keeps its own, the other three are in `keys`. The one read
/// both the full host's `entity_kind` and a hit's view make.
pub(in crate::game) fn body_kind(
    body: LivingBody<'_>,
    keys: sunrise_sim::health::LivingTypeKeys,
) -> Option<mod_api::EntityTypeKey> {
    match body {
        LivingBody::Mob(m) => m.kind.type_key,
        LivingBody::Player(_) => keys.player,
        LivingBody::Stand(_) => keys.armor_stand,
        LivingBody::Dragon(_) => keys.ender_dragon,
    }
}

/// The holder of a living body's attachments: a player's by UUID where its borrower knows it,
/// any other entity's by id.
pub(in crate::game) fn body_holder(body: LivingBody<'_>) -> Option<mod_api::Holder> {
    Some(match body {
        LivingBody::Mob(m) => mod_api::Holder::Entity(m.id),
        LivingBody::Player(p) => mod_api::Holder::Player(p.uuid?),
        LivingBody::Stand(s) | LivingBody::Dragon(s) => mod_api::Holder::Entity(s.id),
    })
}

/// `Mob.isNoAi`: None for anything but a mob.
pub fn body_no_ai(body: LivingBody<'_>) -> Option<bool> {
    match body {
        LivingBody::Mob(m) => Some(m.no_ai),
        _ => None,
    }
}

/// An armor stand's body.
pub(in crate::game) fn stand_body(a: &sunrise_sim::entities::ArmorStand) -> LivingBody<'_> {
    LivingBody::Stand(FixedBody {
        id: a.id,
        health: &a.health,
        live: &a.live,
        equipment: &a.equipment,
    })
}

/// The dragon's body.
pub fn dragon_body(d: &Dragon) -> LivingBody<'_> {
    LivingBody::Dragon(FixedBody {
        id: d.id,
        health: &d.health,
        live: &d.live,
        equipment: &d.equipment,
    })
}

/// A lent living body with what naming and reading it needs: the one place a `LivingView` reads a
/// body, whichever hook lends it (a hit's victim or causer, a healed or moving entity).
#[derive(Clone, Copy)]
pub struct BodyReads<'a> {
    pub body: LivingBody<'a>,
    pub reads: sunrise_sim::health::HitReads<'a>,
}

/// The view's reads of `body`; `party` only says which body a hook lent, and a `BodyReads` holds
/// the one it names.
impl mod_api::__private::LivingReads for BodyReads<'_> {
    fn living_entity(&self, _: LivingParty) -> Option<mod_api::EntityRef> {
        let id = match self.body {
            LivingBody::Mob(m) => m.id,
            LivingBody::Player(p) => u32::try_from(p.id).ok()?,
            LivingBody::Stand(s) | LivingBody::Dragon(s) => s.id,
        };
        Some(mod_api::EntityRef::new(self.reads.epoch, id))
    }
    fn living_kind(&self, _: LivingParty) -> Option<mod_api::EntityTypeKey> {
        body_kind(self.body, self.reads.living_types)
    }
    fn living_holder(&self, _: LivingParty) -> Option<mod_api::Holder> {
        body_holder(self.body)
    }
    fn living_health(&self, _: LivingParty) -> Option<f32> {
        body_health(self.body)
    }
    fn living_absorption(&self, _: LivingParty) -> Option<f32> {
        body_absorption(self.body)
    }
    fn living_max_health(&self, _: LivingParty) -> Option<f32> {
        body_max_health(self.body)
    }
    fn living_effect(&self, _: LivingParty, effect: mod_api::EffectKey) -> Option<ActiveEffect> {
        let def = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)?;
        body_effect(self.body, def.index)
    }
    fn living_effects(&self, _: LivingParty) -> Option<Vec<ActiveEffect>> {
        body_effects(self.body)
    }
    fn living_potion_affected(&self, _: LivingParty, effect: mod_api::EffectKey) -> bool {
        mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
            .is_some_and(|def| body_potion_affected(self.body, def.index))
    }
    fn living_attribute_value(&self, _: LivingParty, id: &str) -> Option<f64> {
        body_attribute(self.body, self.reads.attributes, id, ValueKind::Computed)
    }
    fn living_hand_item(
        &self,
        _: LivingParty,
        hand: mod_api::Hand,
        read: &mut dyn for<'s> FnMut(mod_api::ItemStackView<'s>),
    ) -> Option<()> {
        let stack = body_equipment(self.body, hand_slot(hand))?;
        lend_equipment(stack, read)
    }
    fn living_no_ai(&self, _: LivingParty) -> Option<bool> {
        body_no_ai(self.body)
    }
}

/// Lend `lend`'s entity to a heal or movement hook's listeners as a [`mod_api::LivingView`]:
/// `dispatch` runs with it once its reads are built, and not at all where they cannot be (no
/// open world names the entity). Called only behind the hook's subscription check.
pub fn with_lent_view(
    lend: sunrise_sim::health::LivingLend<'_>,
    dispatch: impl FnOnce(&mod_api::LivingView<'_>),
) {
    let Some(reads) = (lend.reads)() else {
        return;
    };
    let lent = BodyReads {
        body: lend.living_body(&reads),
        reads,
    };
    if let Some(view) = mod_api::LivingView::new(&lent, LivingParty::Subject) {
        dispatch(&view);
    }
}

/// A player's effect as the API's load-owned key and instance data.
pub fn active_effect(e: &EffectInstance) -> ActiveEffect {
    ActiveEffect::from(mod_api::__private::built::ActiveEffect {
        id: mod_api::__private::effect_key_by_index(
            e.registry_index.expect("active effect registry index"),
        )
        .expect("effects resolved at load"),
        duration: e.duration,
        amplifier: e.amplifier,
        ambient: e.ambient,
        visible: e.visible,
        show_icon: e.show_icon,
    })
}

/// A mob's stored effect as the API's: ambient and particle flags remain in their own lists.
pub fn mob_effect(m: &Mob, e: &sunrise_sim::mobs::StoredEffect) -> ActiveEffect {
    ActiveEffect::from(mod_api::__private::built::ActiveEffect {
        id: mod_api::__private::effect_key_by_index(
            e.registry_index.expect("active effect registry index"),
        )
        .expect("effects resolved at load"),
        duration: e.duration,
        amplifier: e.amplifier,
        ambient: m.extra.ambient_effects.contains(&e.id),
        visible: !m.extra.hidden_effects.contains(&e.id),
        show_icon: e.show_icon,
    })
}

/// Project mob effects in store order, stopping before projecting the next on a break.
pub fn visit_mob_effects(
    mob: &Mob,
    visit: &mut dyn FnMut(ActiveEffect) -> std::ops::ControlFlow<()>,
) -> std::ops::ControlFlow<()> {
    mob.extra
        .effects
        .iter()
        .map(|e| mob_effect(mob, e))
        .try_for_each(visit)
}

pub trait NativeLivingBodyGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// The body a living target names now, where its reads are: the player in place's own
    /// fields, a guest's hub record, a mob that is not removed, an armor stand or the dragon.
    fn native_living_body<'__loan>(&'__loan self, target: Target) -> Option<LivingBody<'__loan>>
    where
        '__sg: '__loan,
    {
        Some(match target {
            Target::Player => {
                let p = self.player.as_ref()?;
                LivingBody::Player(PlayerBody {
                    id: p.entity_id,
                    uuid: None,
                    health: PlayerHealthRead::Synced(&p.health.data, p.health.ids),
                    attributes: Some(&p.attributes),
                    effects: self.native_living_player_effects(target),
                    equipment: sunrise_sim::health::player_equipment(
                        &self.inventory,
                        self.selected,
                        &self.armor,
                        &self.offhand,
                        &self.equipment,
                    ),
                    attachments: None,
                })
            }
            Target::Guest(conn) => {
                // The guest's hub record, the one read a hit's parked causer makes too; an acting
                // guest's effects are the game's.
                let mut body = PlayerBody::of_record(self.command_guest(conn)?);
                body.effects = self.native_living_player_effects(target);
                // The holder and its map are a hit's own loan; outside one the record names
                // none, as before.
                body.uuid = None;
                body.attachments = None;
                LivingBody::Player(body)
            }
            Target::Mob(id) => {
                LivingBody::Mob(self.mobs.iter().find(|m| m.id == id && !m.removed)?)
            }
            Target::Stand(id) => stand_body(self.armor_stands.iter().find(|a| a.id == id)?),
            Target::Dragon(id) => dragon_body(self.dragon.as_ref().filter(|d| d.id == id)?),
            _ => return None,
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt1<'__sg> for G {}

pub trait NativeLivingBodyGameExt2<'__sg>: crate::game::SideGame<'__sg> {
    /// Where a player's or a guest's effects are: the acting player's own
    /// are the game's (swapped in by the entry of `guest_turn`; a player handle names
    /// the acting player, and a game without a hub keeps its single
    /// player's the same way), any other guest's are their hub record's.
    /// None for another kind.
    fn native_living_player_effects<'__loan>(
        &'__loan self,
        t: Target,
    ) -> Option<&'__loan PlayerEffects>
    where
        '__sg: '__loan,
    {
        match t {
            Target::Player => Some(&self.potions.effects),
            Target::Guest(conn)
                if crate::game::Side::lan_acting(self.side_state()) == Some(conn) =>
            {
                Some(&self.potions.effects)
            }
            Target::Guest(conn) => self.side_guest_player_effects(conn),
            _ => None,
        }
    }

    /// The entity's instance of `effect` (`LivingEntity.getEffect`): its id
    /// namespaced as the registry has it, the ticks left (-1 infinite), the
    /// amplifier and the flags. The effect is a registration-slot key resolved at load.
    /// None when it has the effect not, before effect resolution, or for an armor
    /// stand, the dragon, a stale handle or a client.
    fn living_effect(&self, living: LivingRef, effect: EffectKey) -> Option<ActiveEffect> {
        let def = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)?;
        let target = self.native_living_read_target(living)?;
        body_effect(self.native_living_body(target)?, def.index)
    }

    /// Visit `LivingEntity.getActiveEffects` over the resident player, guest or mob store.
    fn living_visit_effects(
        &self,
        living: LivingRef,
        visit: &mut dyn FnMut(ActiveEffect) -> std::ops::ControlFlow<()>,
    ) -> Result<std::ops::ControlFlow<()>, mod_api::EntityReadError> {
        let result = match self.native_living_visit_target(living)? {
            t @ (Target::Player | Target::Guest(_)) => self
                .native_living_player_effects(t)
                .ok_or(mod_api::EntityReadError::Unavailable)?
                .list
                .iter()
                .map(active_effect)
                .try_for_each(visit),
            Target::Mob(id) => {
                let mob = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .ok_or(mod_api::EntityReadError::Stale)?;
                visit_mob_effects(mob, visit)
            }
            _ => return Err(mod_api::EntityReadError::Unavailable),
        };
        Ok(result)
    }

    /// Whether the entity has `effect` (`LivingEntity.hasEffect`), using its resolved key.
    /// False before effect resolution, for an armor stand, the dragon or a stale handle;
    /// on a client, for any entity but the local player, whose copy of what the server
    /// sent it is read.
    fn living_potion_affected(&self, living: LivingRef, effect: EffectKey) -> bool {
        let Some(def) = mod_api::__private::effect_index(effect)
            .and_then(sunrise_game_data::potions::effect_by_index)
        else {
            return false;
        };
        self.native_living_read_target(living)
            .and_then(|target| self.native_living_body(target))
            .is_some_and(|body| body_potion_affected(body, def.index))
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt2<'__sg> for G {}

pub trait NativeLivingBodyGameExt3<'__sg>: crate::game::SideGame<'__sg> {
    /// `LivingEntity.getUsedItemHand` while `isUsingItem`: the hand of the item
    /// a player or a mob uses, None when it uses none. On a client, the local
    /// player's only.
    fn living_used_item_hand(&self, living: LivingRef) -> Option<Hand> {
        match self.native_living_read_target(living)? {
            Target::Player => self.used_hand(),
            Target::Guest(conn) => self.side_guest_used_hand(conn),
            Target::Mob(id) => {
                let mob = self.mobs.iter().find(|m| m.id == id)?;
                spear_use::is_using_item(mob).then_some(if mob.extra.using_off_hand {
                    Hand::Off
                } else {
                    Hand::Main
                })
            }
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt3<'__sg> for G {}

pub trait NativeLivingBodyGameExt4<'__sg>: crate::game::SideGame<'__sg> {
    /// The ticks the entity has glided (`LivingEntity.getFallFlyingTicks`),
    /// counted from 0 again the tick after the glide ends. Players (including
    /// inactive guests) and mobs read their own stored body counter. None
    /// for an armor stand or the dragon.
    fn living_gliding_ticks(&self, living: LivingRef) -> Option<i32> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.fall_fly_ticks),
            Target::Guest(conn) => Some(
                self.command_guest(conn)?
                    .motion
                    .as_ref()
                    .map_or(0, |m| m.fall_fly_ticks),
            ),
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .extra
                    .fall_fly_ticks,
            ),
            _ => None,
        }
    }

    /// Whether the entity glides on elytra (`LivingEntity.isFallFlying`;
    /// Paper's `LivingEntity#isGliding`). The player in place and the guests
    /// answer from their fall-flying flag, a mob from its own. None for an
    /// armor stand or the dragon: the game keeps no glide for them.
    fn living_is_gliding(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.fall_flying),
            Target::Guest(conn) => Some(self.command_guest(conn)?.fall_flying),
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .extra
                    .fall_flying,
            ),
            _ => None,
        }
    }

    /// Whether the entity sleeps in a bed (`LivingEntity.isSleeping`): the
    /// player in place, a guest, or a villager or wandering trader asleep in
    /// its bed. Other mobs never sleep. None for an armor stand or the
    /// dragon.
    fn living_is_sleeping(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.sleeping.is_some()),
            Target::Guest(conn) => Some(self.command_guest(conn)?.sleeping),
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)
                    .map(villager::sleeping)
                    .unwrap_or(false),
            ),
            _ => None,
        }
    }

    /// The head of the bed the entity sleeps in (`LivingEntity.getSleepingPos`),
    /// as the block position it lies at. A guest keeps the bed it last slept
    /// in while it sleeps, a villager or wandering trader its own. None for
    /// an awake entity, another mob, an armor stand or the dragon.
    fn living_sleeping_pos(&self, living: LivingRef) -> Option<[i32; 3]> {
        match self.native_living_read_target(living)? {
            Target::Player => self.sleeping,
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                g.bed.filter(|_| g.sleeping)
            }
            Target::Mob(id) => {
                villager::sleeping_pos(self.mobs.iter().find(|m| m.id == id && !m.removed)?)
            }
            _ => None,
        }
    }

    /// Whether the entity jumps (`LivingEntity.isJumping`; Paper's
    /// `LivingEntity#isJumping`): the player in place answers from their
    /// own jump state. The guests' jump is their client's, and a mob has no
    /// jump controller in the game's own AI, so both read false. None for
    /// an armor stand or the dragon.
    fn living_is_jumping(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.jumping),
            Target::Guest(_) => Some(false),
            Target::Mob(_) => Some(false),
            _ => None,
        }
    }

    /// `LivingEntity.noJumpDelay`: ticks before a held jump can jump again.
    fn living_no_jump_delay(&self, living: LivingRef) -> Option<i32> {
        match self.native_living_read_target(living)? {
            Target::Player => Some(self.player.as_ref()?.no_jump_delay),
            Target::Mob(id) if self.is_logical_server() => {
                Some(self.mobs.iter().find(|m| m.id == id)?.ai.no_jump_delay)
            }
            _ => None,
        }
    }

    /// Whether the entity is on a climbable block this tick
    /// (`LivingEntity.onClimbable`; Paper's `LivingEntity#isClimbing`), by
    /// the block at its feet: a ladder, a vine, a scaffold or an open
    /// trapdoor above a ladder. A spectator never climbs; a gliding entity
    /// ignores blocks tagged to glide through, such as vines. None for an
    /// armor stand or the dragon.
    fn living_is_climbing(&self, living: LivingRef) -> Option<bool> {
        let target = self.native_living_read_target(living)?;
        let (position, spectator, gliding) = match target {
            Target::Player => {
                let p = self.player.as_ref()?;
                (p.position, p.spectator, p.fall_flying)
            }
            Target::Guest(conn) => {
                let g = self.command_guest(conn)?;
                (
                    Vec3::new(g.position[0], g.position[1], g.position[2]),
                    g.spectator(),
                    g.fall_flying,
                )
            }
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
                (m.position, false, m.extra.fall_flying)
            }
            _ => return None,
        };
        let mut world = FlagClimbing {
            world: &self.world,
            context: &self.loop_context,
            entity: living.id(),
            position: BlockPos::containing(position),
            spectator,
            gliding,
        };
        Some(climbing::on_climbable(&mut world, &self.motion.climbing).unwrap_or(false))
    }

    /// Whether the entity is a baby (`LivingEntity.isBaby`): a mob answers
    /// from its age, negative until it grows up. A player, an armor stand
    /// and the dragon are never babies.
    fn living_baby(&self, living: LivingRef) -> Option<bool> {
        match self.native_living_read_target(living)? {
            Target::Mob(id) => Some(
                self.mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .extra
                    .age
                    < 0,
            ),
            Target::Player | Target::Guest(_) | Target::Stand(_) | Target::Dragon(_) => Some(false),
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt4<'__sg> for G {}

/// `LivingEntity.onClimbable` reads the world through `ClimbingWorld`; this
/// walks the level the flag's entity stands in, at its feet.
pub struct FlagClimbing<'a> {
    pub world: &'a sunrise_level::world::World,
    pub context: &'a sunrise_world::native_load_context::LoopContext,
    pub entity: u32,
    pub position: BlockPos,
    pub spectator: bool,
    pub gliding: bool,
}

impl ClimbingWorld for FlagClimbing<'_> {
    type Error = std::convert::Infallible;
    fn is_spectator(&mut self) -> Result<bool, Self::Error> {
        Ok(self.spectator)
    }
    fn is_fall_flying(&mut self) -> Result<bool, Self::Error> {
        Ok(self.gliding)
    }
    fn block_position(&mut self) -> Result<BlockPos, Self::Error> {
        Ok(self.position)
    }
    fn in_block_state(&mut self) -> Result<BlockStateId, Self::Error> {
        self.block_state(self.position)
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId, Self::Error> {
        Ok(self.world.block(p.x, p.y, p.z))
    }
    fn set_last_climbable_position(&mut self, _: BlockPos) {}
    fn adjust_ladder(&mut self, position: BlockPos, state: BlockStateId, tagged: bool) -> bool {
        let mut ladder = tagged;
        sunrise_sim::motion::native_is_ladder(
            self.context,
            self.entity,
            position,
            state,
            &mut ladder,
        );
        ladder
    }
    fn adjust_trapdoor_ladder(
        &mut self,
        position: BlockPos,
        state: BlockStateId,
        trapdoor: BlockStateId,
        ladder: bool,
    ) -> bool {
        let mut climbable = ladder;
        sunrise_sim::motion::ladder::native_makes_open_trapdoor_above_climbable(
            self.context,
            state,
            position,
            trapdoor,
            &mut climbable,
        );
        climbable
    }
}

pub trait NativeLivingBodyGameExt5<'__sg>: crate::game::SideGame<'__sg> {
    /// `LivingEntity.getDimensions(Pose)`: the box the living entity has in
    /// `pose` — its width, height and eye height, `minecraft:scale`
    /// included, the fixed 0.2 box of a sleeping pose included. The species'
    /// own state still answers where its `getDefaultDimensions` reads it: a
    /// digging sniffer or an emerging warden is low in every pose. None for
    /// a stale or unavailable handle.
    fn living_dimensions_for_pose(&self, living: LivingRef, pose: Pose) -> Option<[f64; 3]> {
        let target = self.native_living_read_target(living)?;
        // LivingEntity.getDimensions is final: sleeping wins over every
        // species' default dimensions, but never makes a stale handle valid.
        if pose == Pose::Sleeping {
            return Some([f64::from(0.2_f32); 3]);
        }
        match target {
            t @ (Target::Player | Target::Guest(_)) => {
                // `Avatar.getDefaultDimensions(pose).scale(getScale())`.
                let scale = self.native_target_attribute(
                    t,
                    "minecraft:scale",
                    sunrise_world::attribute_map::ValueKind::Computed,
                )? as f32;
                let d = sunrise_sim::pose::Dimensions::avatar(pose, scale);
                Some([
                    f64::from(d.width),
                    f64::from(d.height),
                    f64::from(d.eye_height),
                ])
            }
            Target::Mob(id) => {
                let (w, h, e) = self
                    .mobs
                    .iter()
                    .find(|m| m.id == id && !m.removed)?
                    .dims_in_pose(pose);
                Some([w, h, e])
            }
            // ArmorStand.getDefaultDimensions ignores the pose: a marker's
            // box is empty, a small stand's BABY_DIMENSIONS, else the type's.
            Target::Stand(id) => {
                let a = self.armor_stands.iter().find(|a| a.id == id)?;
                let dims = if a.marker {
                    (0.0, 0.0, 0.0)
                } else {
                    a.probe_dimensions()
                };
                let scale = self.native_target_attribute(
                    target,
                    "minecraft:scale",
                    sunrise_world::attribute_map::ValueKind::Computed,
                )? as f32;
                Some([dims.0, dims.1, dims.2].map(|v| f64::from(v as f32 * scale)))
            }
            Target::Dragon(id) => {
                self.dragon.as_ref().filter(|d| d.id == id)?;
                // EntityTypes.ENDER_DRAGON: scalable(16, 8), default eye
                // height 8 * 0.85. Dragons are not in the ordinary mob table.
                let scale = self.native_target_attribute(
                    target,
                    "minecraft:scale",
                    sunrise_world::attribute_map::ValueKind::Computed,
                )? as f32;
                Some([16.0_f32, 8.0, 6.8].map(|v| f64::from(v * scale)))
            }
            _ => None,
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt5<'__sg> for G {}

/// Project each modifier without copying its canonical id out of the live map.
pub fn visit_attribute_modifiers(
    modifiers: &[Modifier],
    visit: &mut dyn FnMut(AttributeModifierView<'_>) -> ControlFlow<()>,
) -> ControlFlow<()> {
    modifiers.iter().try_for_each(|m| {
        visit(mod_api::__private::game::attribute_modifier_view(
            m.id(),
            m.amount(),
            match m.operation() {
                Operation::AddValue => AttributeOperation::AddValue,
                Operation::AddMultipliedBase => AttributeOperation::AddMultipliedBase,
                Operation::AddMultipliedTotal => AttributeOperation::AddMultipliedTotal,
            },
        ))
    })
}

/// Use the mob's saved-attribute seed if its map has not been built yet.
pub fn mob_attribute_modifiers<'a>(
    mob: &'a Mob,
    catalog: &Arc<Catalog>,
    name: &str,
) -> Option<&'a [Modifier]> {
    mob.extra.live.modifiers(catalog, &mob.kind.id, name, || {
        sunrise_sim::entity_nbt::attribute_entries(mob)
            .into_iter()
            .map(|(_, entry)| entry)
            .collect()
    })
}

pub trait NativeLivingBodyGameExt6<'__sg>: crate::game::SideGame<'__sg> {
    /// The base value of an attribute, before modifiers
    /// (`AttributeMap.getBaseValue`), read as the effective value is.
    fn living_attribute_base(&self, living: LivingRef, id: &str) -> Option<f64> {
        self.native_living_attribute(living, id, sunrise_world::attribute_map::ValueKind::Base)
    }

    /// Resolve the visitor's target. Only the local player has these reads on the client.
    fn native_living_visit_target(&self, living: LivingRef) -> Result<Target, EntityReadError> {
        let target = self.native_entity_visit_target(living.entity())?;
        if !matches!(target, Target::Player) && !self.is_logical_server() {
            return Err(EntityReadError::Unavailable);
        }
        Ok(target)
    }

    /// Lend the entity type's attribute ids in registry order.
    fn living_visit_attributes(
        &self,
        living: LivingRef,
        visit: &mut dyn FnMut(&str) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let entity = match self.native_living_visit_target(living)? {
            Target::Player | Target::Guest(_) => "minecraft:player",
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id && !m.removed)
                .ok_or(EntityReadError::Stale)?
                .kind
                .id
                .as_str(),
            Target::Stand(_) => "minecraft:armor_stand",
            Target::Dragon(_) => "minecraft:ender_dragon",
            _ => return Err(EntityReadError::Unavailable),
        };
        Ok(self
            .motion
            .attributes
            .attribute_names_for(entity)
            .try_for_each(visit))
    }

    /// Lend resident modifiers from the same attribute instances the value reads use.
    fn living_visit_attribute_modifiers(
        &self,
        living: LivingRef,
        id: &str,
        visit: &mut dyn FnMut(AttributeModifierView<'_>) -> ControlFlow<()>,
    ) -> Result<ControlFlow<()>, EntityReadError> {
        let catalog = &self.motion.attributes;
        let modifiers = match self.native_living_visit_target(living)? {
            Target::Player => self
                .player
                .as_ref()
                .ok_or(EntityReadError::Unavailable)?
                .attributes
                .modifiers(id),
            Target::Guest(conn) => self
                .command_guest(conn)
                .and_then(|guest| guest.motion.as_deref())
                .ok_or(EntityReadError::Unavailable)?
                .attributes
                .modifiers(id),
            Target::Mob(mob) => {
                let m = self
                    .mobs
                    .iter()
                    .find(|m| m.id == mob && !m.removed)
                    .ok_or(EntityReadError::Stale)?;
                mob_attribute_modifiers(m, catalog, id)
            }
            Target::Stand(stand) => self
                .armor_stands
                .iter()
                .find(|a| a.id == stand)
                .ok_or(EntityReadError::Stale)?
                .live
                .modifiers(catalog, "minecraft:armor_stand", id, Vec::new),
            Target::Dragon(dragon) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == dragon)
                .ok_or(EntityReadError::Stale)?
                .live
                .modifiers(catalog, "minecraft:ender_dragon", id, Vec::new),
            _ => None,
        }
        .ok_or(EntityReadError::Unavailable)?;
        Ok(visit_attribute_modifiers(modifiers, visit))
    }

    fn native_living_attribute(
        &self,
        living: LivingRef,
        name: &str,
        kind: sunrise_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        self.native_target_attribute(self.native_living_read_target(living)?, name, kind)
    }

    fn native_target_attribute(
        &self,
        target: Target,
        name: &str,
        kind: sunrise_world::attribute_map::ValueKind,
    ) -> Option<f64> {
        body_attribute(
            self.native_living_body(target)?,
            &self.motion.attributes,
            name,
            kind,
        )
    }

    /// The entity's health (`LivingEntity.getHealth`; Paper's
    /// `Damageable#getHealth`). During a player's turn only that player
    /// resolves: the hub is out for their action, and other players'
    /// handles read `None` until it is back.
    fn living_health(&self, living: LivingRef) -> Option<f32> {
        let target = self.native_living_read_target(living)?;
        body_health(self.native_living_body(target)?)
    }

    /// The entity's maximum health (`LivingEntity.getMaxHealth`; Paper's
    /// `Damageable#getMaxHealth`). Players and guests read their attribute
    /// map; a mob, an armor stand and the dragon their own fields (the
    /// armor stand the fixed 20 the game seeds it with, the dragon its
    /// 200) until live attribute maps cover them.
    fn living_max_health(&self, living: LivingRef) -> Option<f32> {
        let target = self.native_living_read_target(living)?;
        body_max_health(self.native_living_body(target)?)
    }

    /// Lend the stack in `slot` (`LivingEntity.getItemBySlot`).
    fn living_equipment(
        &self,
        living: LivingRef,
        slot: EquipmentSlot,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        lend_equipment(self.living_equipment_stack(living, slot)?, read)
    }

    /// Lend the stack in `hand` (`LivingEntity.getItemInHand`).
    fn living_hand_item(
        &self,
        living: LivingRef,
        hand: Hand,
        read: &mut dyn for<'a> FnMut(ItemStackView<'a>),
    ) -> Option<()> {
        self.living_equipment(living, hand_slot(hand), read)
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeLivingBodyGameExt6<'__sg> for G {}

use crate::game::side_ops::modapi_modding::ModapiModdingSideGameExt as _;

use crate::game::commands_common::CommandsCommonGameExt2 as _;

use crate::game::native_entity_refs_common::{
    NativeEntityRefsCommonGameExt1 as _, NativeEntityRefsCommonGameExt5 as _,
};
