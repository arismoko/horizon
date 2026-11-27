//! Durability lost by what the player wears and holds: armour hurt with its wearer
//! (`wear_armor`, `hurt_armor`, `hurt_equipment`, `hurt_player_equipment`) and the held stack's wear
//! (`damage_held`, `damage_held_in_slot`, `hurt_held_without_breaking`).
use super::*;
use sunrise_sim::motion::MotionEvent;

impl crate::game::ServerGame<'_> {
    /// Run equipment continuations with the acting player installed on the real host.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game::server_side) fn hurt_player_equipment(
        &mut self,
        source: sunrise_world::damage_source::DamageTypeId,
        amount: f32,
        blocking: Option<sunrise_sim::health::Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        attack: &sunrise_sim::health::Attack,
        armor_weapon: Option<sunrise_sim::health::ArmorWeapon>,
    ) -> sunrise_game_data::Result<sunrise_sim::health::BlockOutcome> {
        let rules = self.common.world.health_rules;
        // One decision about the hit's causing entity for its three levels.
        let decision =
            crate::game::server_side::entity_server::causer_decision!(self, attack.attacker);
        let helmet = self.common.armor[0]
            .as_ref()
            .is_some_and(|s| s.count > 0 && &*s.item != "minecraft:air");
        let mut hit = self
            .common
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .start_hurt(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    attack,
                    no_push,
                    crate::game::server_side::entity_server::player_victim_causer!(self, decision),
                    acting_equipment!(self.common)
                ),
                &self.common.health_resources,
                rules,
                source,
                amount,
                blocking,
                living_cause,
                causing,
                creative_source,
                helmet,
                !self.server.combat.0.refuse_player_hit,
                armor_weapon,
            )?;
        if let Some(damage) = hit.helmet_damage() {
            let hit_source = hit.source(attack);
            self.hurt_helmet(&hit_source, damage);
            self.refresh_armor_protection();
        }
        let armor_damage = self
            .common
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .select_equipment_hurt(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    attack,
                    no_push,
                    crate::game::server_side::entity_server::player_victim_causer!(self, decision)
                ),
                &self.common.health_resources,
                rules,
                &mut hit,
            )?;
        if let Some(damage) = armor_damage {
            let hit_source = hit.source(attack);
            self.hurt_armor(&hit_source, damage);
            self.refresh_armor_protection();
        }
        self.common
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .finish_hurt(
                crate::game::server_side::entity_server::damage_level_of!(
                    self,
                    attack,
                    no_push,
                    crate::game::server_side::entity_server::player_victim_causer!(self, decision),
                    acting_equipment!(self.common)
                ),
                &self.common.health_resources,
                rules,
                hit,
            )
    }
    /// The armor hits among a tick's motion events (`Player.hurtArmor` from
    /// `getDamageAfterArmorAbsorb`), whatever the damage: a mob, an explosion,
    /// a cactus, lava, a campfire.
    pub fn wear_armor(&mut self, events: &[MotionEvent]) {
        for event in events {
            if let MotionEvent::Health(sunrise_sim::health::HealthEvent::ArmorHurt {
                source,
                causing,
                direct,
                position,
                amount,
            }) = event
            {
                let Some(name) = self
                    .common
                    .health_resources
                    .damage
                    .definition(*source)
                    .map(|d| d.name.clone())
                else {
                    continue;
                };
                let source = sunrise_sim::health::HurtSource {
                    damage_type: &name,
                    causing: *causing,
                    direct: *direct,
                    position: *position,
                    direct_type: None,
                };
                self.hurt_armor(&source, *amount);
            }
        }
    }
    /// `Player.hurtArmor`: every armor slot wears.
    pub fn hurt_armor(&mut self, source: &sunrise_sim::health::HurtSource<'_>, damage: f32) {
        self.hurt_equipment(source, damage, &[3, 2, 1, 0]);
    }

    /// `Player.hurtHelmet`: the head slot alone wears.
    pub fn hurt_helmet(&mut self, source: &sunrise_sim::health::HurtSource<'_>, damage: f32) {
        self.hurt_equipment(source, damage, &[0]);
    }

    /// `LivingEntity.doHurtEquipment` of the `slots` of `armor`: each worn piece that
    /// wears when its wearer is hurt loses max(1, damage / 4) durability, except
    /// to a fire source when the piece resists fire (`ItemStack.canBeHurtBy`).
    pub(in crate::game::server_side) fn hurt_equipment(
        &mut self,
        source: &sunrise_sim::health::HurtSource<'_>,
        damage: f32,
        slots: &[usize],
    ) {
        if damage <= 0.0 {
            return;
        }
        let amount = if damage.is_nan() {
            0
        } else {
            (damage / 4.0).max(1.0) as i32
        };
        if amount <= 0 {
            return;
        }
        let amount = amount as u32;
        let name = source.damage_type;
        // The pieces that may wear (`doHurtEquipment`'s checks).
        let mut eligible: [Option<ItemStack>; 4] = std::array::from_fn(|i| {
            let stack = self.common.armor[i]
                .clone()
                .filter(|_| slots.contains(&i))?;
            let info = world_items!(self).get(&stack.item)?;
            let component = |id| stack.data().and_then(|data| data.other(id));
            // ItemStack.isDamageableItem: component removal and
            // unbreakable are checked before enchantment RNG or hooks.
            if stack.count == 0
                || !sunrise_sim::item_sync::damage_on_hurt(&stack, info.equip_damage_on_hurt)
                || component("minecraft:unbreakable").is_some()
                || component("!minecraft:max_damage").is_some()
                || component("!minecraft:damage").is_some()
                || !self.stack_can_be_hurt_by(&stack, name).unwrap_or(true)
            {
                return None;
            }
            let max = match component("minecraft:max_damage") {
                Some(sunrise_core::nbt::Tag::Int(value)) => u32::try_from(*value).ok()?,
                _ => info.max_damage,
            };
            if max == 0 {
                return None;
            }
            Some(stack)
        });
        // No equipment can wear: there is no armor damage event to dispatch.
        if eligible.iter().all(Option::is_none) {
            return;
        }
        // ArmorHurtEvent: once for the pieces that would wear, a server player's only.
        let mut amounts = [amount; 4];
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let (Some(uuid), Some(player)) =
                (self.server.lan_acting_uuid, self.server.lan_acting_entity)
        {
            let mut entries: Vec<mod_api::ArmorHurt> = eligible
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    Some(mod_api::ArmorHurt {
                        slot: armor_slot(i),
                        stack: e.as_ref()?.try_into_api().ok()?,
                        damage: durability_damage(damage),
                        new_damage: durability_damage(damage),
                    })
                })
                .collect();
            let mut cancel = false;
            self.native_armor_hurt(
                Some(uuid),
                player,
                "minecraft:player",
                source,
                &mut entries,
                &mut cancel,
            );
            if cancel {
                return;
            }
            for (i, amount) in amounts.iter_mut().enumerate() {
                *amount = answered_wear(&entries, armor_slot(i), *amount);
            }
        }
        // Each break is applied before the next slot's hook and RNG draw.
        for &index in slots {
            let Some(before) = eligible[index].take() else {
                continue;
            };
            if amounts[index] == 0 || self.common.armor[index].as_ref() != Some(&before) {
                continue;
            }
            let (updated, broken) =
                self.damage_stack(&before, amounts[index].min(i32::MAX as u32) as i32, true);
            if updated.as_ref() != Some(&before)
                && self.common.armor[index].as_ref() == Some(&before)
            {
                self.common.armor[index] = updated;
                if let Some(stack) = broken {
                    self.player_equipment_broke(armor_slot(index), &stack);
                    self.player_item_destroyed(&before, None);
                    self.apply_equipment();
                }
            }
        }
    }
    pub fn damage_held(&mut self, amount: u32) {
        self.damage_held_in_slot(
            amount,
            sunrise_core::equipment_slot::EquipmentSlot::MainHand,
        );
    }
    /// Hurt the acting hand's stack through ordinary wear/criterion/break handling,
    /// reporting its break in the caller's explicit equipment slot.
    pub fn damage_held_in_slot(
        &mut self,
        amount: u32,
        break_slot: sunrise_core::equipment_slot::EquipmentSlot,
    ) {
        let hand = self.common.hand;
        if let Some(original) = self.apply_held_damage(amount, break_slot) {
            self.player_item_destroyed(&original, Some(hand));
        }
    }

    /// Apply wear and its ordinary break effects, returning the pre-wear stack
    /// on a break. Mining reports that snapshot only after the block's drops.
    pub(in crate::game::server_side) fn apply_held_damage(
        &mut self,
        amount: u32,
        break_slot: sunrise_core::equipment_slot::EquipmentSlot,
    ) -> Option<ItemStack> {
        let slot = self.common.held_slot();
        let stack = self.common.stack_in(slot).cloned()?;
        let (updated, broken) = self.damage_stack(&stack, amount.min(i32::MAX as u32) as i32, true);
        if self.common.held_slot() != slot || self.common.stack_in(slot) != Some(&stack) {
            return None;
        }
        self.edit_block_use_stack(|slot| {
            if let Some(original) = slot.as_mut() {
                if let Some(updated) = &updated {
                    original.damage = original
                        .damage
                        .saturating_add(updated.damage.saturating_sub(stack.damage))
                        .saturating_sub(stack.damage.saturating_sub(updated.damage));
                    original.count = original
                        .count
                        .saturating_sub(stack.count.saturating_sub(updated.count));
                    if original.count == 0 {
                        *slot = None;
                    }
                } else {
                    *slot = None;
                }
            }
        });
        let broken = broken?;
        self.player_equipment_broke(break_slot, &broken);
        Some(stack)
    }
    /// `ItemStack.hurtWithoutBreaking`: Unbreaking applies, but the stack stops at
    /// one point of durability left instead of breaking.
    pub(in crate::game::server_side) fn hurt_held_without_breaking(&mut self, amount: u32) {
        if self.common.creative {
            return;
        }
        let Some(held) = self.common.held().cloned() else {
            return;
        };
        let amount = self.wear(&held, amount);
        let max = self.trident_max_damage(&held);
        if amount == 0 || max == 0 {
            return;
        }
        self.edit_block_use_stack(|slot| {
            if let Some(stack) = slot.as_mut() {
                stack.damage = (stack.damage + amount).min(max - 1);
            }
        });
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::item_use_stacks::ItemUseStacksGameExt1 as _;

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;
