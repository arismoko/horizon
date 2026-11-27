//! `Mob.aiStep`'s looting block: what a mob that can pick up loot takes and wears, on `MobLevel`.
use crate::mob_level::MobLevel;
use sunrise_core::{geometry::Vec3, random::RandomSource};
use sunrise_gameplay_common::item_areas::ItemAreas;
use sunrise_items::items::ItemStack;
use sunrise_sim::mobs::{
    Species,
    equip::{self, Facts, ItemTags, Slot},
    finalize::{self, set_drop_chance, set_item_stack, stored_drop_chance},
};

/// What looting asks of the level beyond `MobLevel`: the mod hooks and the tables only the game holds.
pub trait LootingLevel: MobLevel {
    /// `EntityMobGriefingEvent` has listeners: `Game::grief_hooked`, `fn(&self) -> bool`.
    fn grief_hooked(&self) -> bool;
    /// `Mob.isEquippableInSlot`'s own answer: `Game::stations().enchantments.entity_tags` with `equip::equippable_in_slot`.
    fn equippable_in_slot(&self, stack: &ItemStack, slot: &str, kind: &str) -> bool;
    /// `IItemExtension.canEquip`: `Game::can_equip(&self, &EquipStack, &str, u32, &str, bool) -> bool`.
    fn can_equip(&self, stack: &ItemStack, slot: &str, entity: u32, kind: &str, data: bool)
    -> bool;
    /// `Level.gameEvent(EQUIP)` of a mob: `Game::mob_equip_event(&mut self, u32, [f64; 3], bool)`.
    fn mob_equip_event(&mut self, id: u32, at: [f64; 3], equip: bool);
    /// The numbers `Mob.compareArmor` and `compareWeapons` read for mob `index`: `Game::stack_facts(&self, &Mob, &ItemStack, Slot) -> Facts`.
    fn stack_facts(&self, index: usize, stack: &ItemStack, slot: Slot) -> Facts;
}

/// `Mob.aiStep`'s looting block, once per mob per tick.
pub fn tick_mob_looting(level: &mut impl LootingLevel) {
    let griefing = level.mob_griefing();
    if level.parts().dropped.is_empty() || (!griefing && !level.grief_hooked()) {
        return;
    }
    let mut near = ItemAreas::default();
    for i in 0..level.parts().mobs.len() {
        let (id, reach, species, baby, passenger) = {
            let p = level.parts();
            let m = &p.mobs[i];
            if !m.can_pick_up_loot() || !m.alive() || m.removed || !p.ticked.contains(&m.id) {
                continue;
            }
            // `Mob.ITEM_PICKUP_REACH`.
            (
                m.id,
                m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0)),
                m.species(),
                m.is_baby(),
                m.extra.passenger,
            )
        };
        if !level.entity_grief(id, griefing) {
            continue;
        }
        let mut scan = {
            let p = level.parts();
            near.scan(p.dropped, p.next_entity, reach)
        };
        if scan.is_empty() {
            continue;
        }
        let mut j = 0;
        loop {
            let (stack, tags) = {
                let p = level.parts();
                j = scan.next_from(j, p.dropped.len());
                let Some(e) = p.dropped.get(j) else {
                    break;
                };
                if e.pickup_delay > 0 || !reach.intersects(e.bounds()) {
                    j += 1;
                    continue;
                }
                let stack = e.stack.clone();
                let item = &stack.item;
                let tags = ItemTags {
                    spear: p.items.item_in_tag("minecraft:spears", item),
                    egg: p.items.item_in_tag("minecraft:eggs", item),
                    disliked_weapon: p
                        .items
                        .item_in_tag("minecraft:wither_skeleton_disliked_weapons", item),
                };
                (stack, tags)
            };
            if !equip::wants_to_pick_up(species, baby, passenger, &stack.item, tags) {
                j += 1;
                continue;
            }
            // `Mob.pickUpItem`: nothing leaves the ground what it cannot use.
            let Some(taken) = equip_item_if_possible(level, i, &stack) else {
                j += 1;
                continue;
            };
            let before = level.parts().dropped[j].clone();
            let id = level.parts().mobs[i].id;
            level.take(id, before.clone());
            level.on_item_pickup(i, &before);
            let p = level.parts();
            let e = &mut p.dropped[j];
            e.stack.count -= taken;
            if e.stack.count == 0 {
                level.discard_item(j);
                scan.removed(j);
            } else {
                j += 1;
            }
        }
    }
}

/// `Mob.equipItemIfPossible`: what `slot` would take of `stack`, and how
/// many items that takes. `None` when the mob has no use for it.
pub fn equip_item_if_possible(
    level: &mut impl LootingLevel,
    index: usize,
    stack: &ItemStack,
) -> Option<u32> {
    // `getEquipmentSlotForItem`, then `isEquippableInSlot`: the stack's own `equippable`
    // component and its allowed entities, then the parts of its item.
    let worn = equip::equippable(level.parts().items, stack).map(|e| e.slot);
    let mut slot = equip::slot_for(worn)?;
    let (id, kind) = {
        let m = &level.parts().mobs[index];
        (m.id, m.kind.clone())
    };
    let data = level.equippable_in_slot(stack, slot.name(), &kind.id);
    if !level.can_equip(stack, slot.name(), id, &kind.id, data) {
        return None;
    }
    let mut replace = can_replace(level, index, stack, slot);
    // An armour piece that beats nothing it is worn with falls back to an
    // empty main hand.
    if slot.is_armor() && !replace {
        slot = Slot::MainHand;
        replace = level.parts().mobs[index].extra.main_hand_id().is_none();
    }
    if !replace {
        return None;
    }
    // `EquipmentSlot.limit`: an armour slot takes one item, a hand the
    // whole stack.
    let taken = if slot.is_armor() {
        1
    } else {
        stack.count.max(1)
    };
    let m = &mut level.parts().mobs[index];
    let old = finalize::item_stack(m, slot.index());
    let chance = stored_drop_chance(m, slot.index());
    // `DropChances.byEquipment` against the roll the equip makes.
    let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
    // The whole stack comes with it: its enchantments (a Sharpness V sword
    // stays sharp), damage, name and components.
    let mut equipped = stack.clone();
    equipped.count = taken;
    set_item_stack(m, slot.index(), equipped);
    // `setItemSlotAndDropWhenKilled`, then `persistenceRequired`.
    set_drop_chance(m, slot.index(), 2.0);
    m.persistent = true;
    let at = [m.position.x, m.position.y, m.position.z];
    let id = m.id;
    if drop_old && let Some(old) = old {
        level.spawn_item_own(old, at, 10);
    }
    level.mob_equip_event(id, at, true);
    Some(taken)
}

/// `Mob.canReplaceCurrentItem` for what mob `index` wears or holds in `slot`.
fn can_replace(level: &mut impl LootingLevel, index: usize, stack: &ItemStack, slot: Slot) -> bool {
    let (worn_stack, species) = {
        let m = &level.parts().mobs[index];
        let Some(worn_stack) = finalize::item_stack(m, slot.index()) else {
            return true;
        };
        (worn_stack, m.species())
    };
    let current = worn_stack.item.clone();
    // `Drowned.canReplaceCurrentItem` never parts with a nautilus shell.
    if species == Species::Drowned && &*current == "minecraft:nautilus_shell" {
        return false;
    }
    let new = level.stack_facts(index, stack, slot);
    let worn = level.stack_facts(index, &worn_stack, slot);
    let items = level.parts().items;
    let preferred = equip::preferred_weapon_tag(species).map(|tag| {
        (
            items.item_in_tag(tag, &stack.item),
            items.item_in_tag(tag, &current),
        )
    });
    equip::can_replace_current_item(&new, Some(&worn), slot, preferred)
}
