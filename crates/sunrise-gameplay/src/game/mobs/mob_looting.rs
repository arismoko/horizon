//! `Mob.aiStep`'s looting loop: a mob with `canPickUpLoot` takes what lies
//! within its box grown by `getPickupReach` (1, 0, 1) and wears or holds it
//! when it beats what it has (`equipItemIfPossible`), dropping the replaced
//! stack by its drop chance. The species whose `pickUpItem` does something
//! else keep their own loops (`pandas.rs`, `piglins.rs`, `villagers.rs`,
//! `allays.rs`, `sulfur_cubes.rs`).

use crate::game::ServerGame;
use sunrise_gameplay::mob_looting::LootingLevel;
use sunrise_gameplay_common::game::equip_answers::EquipAnswersGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_sim::{
    entities::ItemEntity,
    mobs::{
        Mob,
        equip::{self, Facts, Slot},
    },
};

impl LootingLevel for ServerGame<'_> {
    fn grief_hooked(&self) -> bool {
        crate::game::ServerGame::grief_hooked(self)
    }
    fn equippable_in_slot(&self, stack: &ItemStack, slot: &str, kind: &str) -> bool {
        equip::equippable_in_slot(self.items(), stack, slot, kind)
    }
    fn can_equip(
        &self,
        stack: &ItemStack,
        slot: &str,
        entity: u32,
        kind: &str,
        data: bool,
    ) -> bool {
        let prepared = sunrise_gameplay_common::game::CommonGame::equip_stack(self, stack);
        EquipAnswersGameExt1::can_equip(self, &prepared, slot, entity, kind, data)
    }
    fn mob_equip_event(&mut self, id: u32, at: [f64; 3], equip: bool) {
        crate::game::ServerGame::mob_equip_event(self, id, at, equip);
    }
    fn stack_facts(&self, index: usize, stack: &ItemStack, slot: Slot) -> Facts {
        crate::game::ServerGame::stack_facts(self, &self.common.mobs[index], stack, slot)
    }
}

impl crate::game::ServerGame<'_> {
    /// `Mob.aiStep`'s looting block, once per mob per tick.
    pub(in crate::game) fn tick_mob_looting(&mut self) {
        sunrise_gameplay::mob_looting::tick_mob_looting(self);
    }

    /// `Mob.equipItemIfPossible`, as the tests ask it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn equip_item_if_possible(&mut self, index: usize, stack: &ItemStack) -> Option<u32> {
        sunrise_gameplay::mob_looting::equip_item_if_possible(self, index, stack)
    }

    /// The numbers `Mob.compareArmor` and `compareWeapons` read off `stack`
    /// in `slot` for the mob `m`.
    pub(in crate::game::server_side) fn stack_facts(
        &self,
        m: &Mob,
        stack: &ItemStack,
        slot: Slot,
    ) -> Facts {
        let (armor, toughness, attack) = self.attributes(m, stack, slot);
        Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: attack,
            enchantments: stack.enchantments().len(),
            damage: stack.damage as i32,
            custom_name: stack.custom_name().is_some(),
            prevents_armor_change: world_stations!(self)
                .enchantments
                .prevents_armor_change(stack),
        }
    }

    /// `Mob.getApproximateAttributeWith` for `stack` in `slot`: each
    /// attribute starts from the mob's own value. (armour, toughness, damage)
    fn attributes(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> (f64, f64, f64) {
        let modifiers = sunrise_sim::item_attributes::attribute_modifiers(stack, self.items());
        let modifiers = &*modifiers;
        (
            equip::attribute_with(modifiers, "minecraft:armor", f64::from(m.kind.armor), slot),
            equip::attribute_with(
                modifiers,
                "minecraft:armor_toughness",
                f64::from(m.kind.armor_toughness),
                slot,
            ),
            equip::attribute_with(
                modifiers,
                "minecraft:attack_damage",
                f64::from(m.attrs.attack_damage),
                slot,
            ),
        )
    }

    /// `LivingEntity.onItemPickup`: the advancement of whoever threw the item,
    /// whichever mob took it.
    pub(in crate::game::server_side) fn mob_item_pickup_trigger(
        &mut self,
        index: usize,
        e: &ItemEntity,
    ) {
        let Some(owner) = e.thrower else {
            return;
        };
        let stack = e.stack.clone();
        let target = self.mob_entity(&self.common.mobs[index]);
        self.for_each_player(|g| {
            if g.acting_uuid() == owner {
                g.trigger_item_entity("minecraft:thrown_item_picked_up_by_entity", &stack, &target);
            }
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;
