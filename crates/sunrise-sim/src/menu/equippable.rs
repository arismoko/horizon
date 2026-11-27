//! `DataComponents.EQUIPPABLE` as a menu reads it from a stack: its slot and the entity types that
//! may wear it (`LivingEntity.isEquippableInSlot`), then the parts of its item
//! (`IItemExtension.canEquip`).
use super::{Env, Kind, MAIN, Menu, Slot};
use crate::items::ItemStack;
use crate::mobs::equip::equippable_in_slot;

/// The equipment slots of `Role::Armor`, by `EquipmentSlot.getIndex` (feet first).
pub(super) const ARMOR_SLOTS: [&str; 4] = ["feet", "legs", "chest", "head"];

/// Enforce the backing container's bound without asking placement again.
pub(super) fn limit_slot_max_for(
    kind: Kind,
    s: usize,
    slot: Slot,
    container_limit: Option<u32>,
    max: u32,
) -> u32 {
    if let Kind::Mod(layout) = kind
        && let Some(limit) = layout.owned_slots.as_ref().and_then(|limits| limits.get(s))
    {
        return max.min(u32::from(*limit));
    }
    if slot.container == MAIN {
        container_limit.map_or(max, |limit| max.min(limit))
    } else {
        max
    }
}

impl Menu {
    /// Apply the backing container's bound without asking placement again.
    pub(super) fn limit_slot_max(&self, s: usize, max: u32) -> u32 {
        let slot = self.state.slots[s];
        let container_limit = self
            .state
            .container_limits
            .get(slot.index)
            .copied()
            .flatten();
        limit_slot_max_for(self.state.kind, s, slot, container_limit, max)
    }

    /// `ArmorSlot.mayPlace`: `LivingEntity.isEquippableInSlot` for the player of `env`, armour
    /// slot `eq` of [`ARMOR_SLOTS`]. The stack's `equippable` component answers first and the
    /// parts of its item may change that (`IItemExtension.canEquip`, asked through `can_equip`).
    pub(super) fn armor_may_place_with(
        env: &Env,
        can_equip: Option<&dyn crate::mobs::equip::CanEquip>,
        eq: usize,
        stack: &ItemStack,
    ) -> bool {
        let data = equippable_in_slot(env.items, stack, ARMOR_SLOTS[eq], "minecraft:player");
        match (can_equip, env.player_entity_id) {
            (Some(parts), Some(player)) if parts.listens(&stack.item) => parts.can_equip(
                stack,
                ARMOR_SLOTS[eq],
                player as u32,
                "minecraft:player",
                data,
            ),
            _ => data,
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::tests::{stack, with_menu};
    use super::super::{Input, Kind};
    use sunrise_core::nbt::Tag;

    /// A stone whose `equippable` component has `fields`.
    pub(in crate::menu) fn equippable(fields: Vec<(&str, Tag)>) -> Option<crate::items::ItemStack> {
        let mut stone = stack("stone", 1)?;
        stone
            .data_mut()
            .set_other("minecraft:equippable", Some(Tag::compound(fields)));
        Some(stone)
    }

    /// A menu reads a stack's `EQUIPPABLE.slot`: a patched stone can go to
    /// the head slot and is equipped there.
    #[test]
    fn a_menu_reads_a_stacks_equippable_slot() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![("slot", Tag::String("head".into()))]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5).map(|s| &*s.item), Some("minecraft:stone"));
            assert_eq!(menu.slot(9), None);
            assert_eq!(menu.equipped.last().map(|(s, _)| *s), Some("head"));
        });
    }

    /// `allowed_entities` comes from the same stack component: a head item
    /// only a horse may wear stays out of the player's head slot.
    #[test]
    fn a_stacks_allowed_entities_keep_it_off_the_player() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![
                ("slot", Tag::String("head".into())),
                ("allowed_entities", Tag::String("horse".into())),
            ]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5), None);
        });
    }

    /// An iron helmet whose `equippable` is removed (`fields` None) or replaced by one with
    /// `fields`.
    fn helmet(fields: Option<Vec<(&str, Tag)>>) -> Option<crate::items::ItemStack> {
        let mut helmet = stack("iron_helmet", 1)?;
        let (id, value) = match fields {
            Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
            None => ("!minecraft:equippable", Tag::Compound(Default::default())),
        };
        helmet.data_mut().set_other(id, Some(value));
        Some(helmet)
    }

    /// The armour slots read the stack's own component before the item's: a removed one keeps
    /// an iron helmet off the head, an override for the feet moves it there, and an explicitly
    /// empty `allowed_entities` allows nobody while a component with none allows the player.
    #[test]
    fn the_armour_slots_read_the_stacks_own_equippable() {
        with_menu(Kind::Inventory, |env, menu| {
            let head = |slot: &str| ("slot", Tag::String(slot.into()));
            // Menu slots 5 to 8 are the head, chest, legs and feet.
            let takes = |stack: Option<crate::items::ItemStack>| {
                let stack = stack.unwrap();
                (5..=8)
                    .filter(|s| menu.may_place(env, *s, &stack))
                    .collect::<Vec<_>>()
            };
            assert_eq!(takes(stack("iron_helmet", 1)), [5]);
            assert_eq!(takes(helmet(None)), [] as [usize; 0]);
            assert_eq!(takes(helmet(Some(vec![head("feet")]))), [8]);
            let nobody = ("allowed_entities", Tag::List(Vec::new()));
            assert_eq!(
                takes(helmet(Some(vec![head("head"), nobody]))),
                [] as [usize; 0]
            );
            let player = ("allowed_entities", Tag::String("player".into()));
            assert_eq!(takes(helmet(Some(vec![head("head"), player]))), [5]);
        });
    }

    /// What a lent `CanEquip` was asked, answering the opposite of the data.
    #[derive(Default)]
    struct Contrary(std::cell::RefCell<Vec<(String, u32, String, bool)>>);
    impl crate::mobs::equip::CanEquip for Contrary {
        fn can_equip(
            &self,
            _: &crate::items::ItemStack,
            slot: &str,
            entity: u32,
            kind: &str,
            data: bool,
        ) -> bool {
            (self.0.borrow_mut()).push((slot.to_owned(), entity, kind.to_owned(), data));
            !data
        }
        fn with_stack(
            &self,
            _: &crate::items::ItemStack,
            body: &mut dyn FnMut(&dyn crate::mobs::equip::CanEquip),
        ) {
            body(self);
        }
    }

    /// `ArmorSlot.mayPlace` asks the `CanEquip` the game lent the click, for the menu's player
    /// with the component's answer, and takes its answer either way; with none lent, the
    /// component decides.
    #[test]
    fn the_armour_slots_ask_the_lent_can_equip() {
        with_menu(Kind::Inventory, |env, menu| {
            let helmet = stack("iron_helmet", 1).unwrap();
            let parts = Contrary::default();
            let lent = super::Env {
                can_equip: Some(&parts),
                player_entity_id: Some(41),
                ..*env
            };
            assert!(!menu.may_place(&lent, 5, &helmet));
            assert!(menu.may_place(&lent, 8, &helmet));
            let player =
                |slot: &str, data| (slot.to_owned(), 41, "minecraft:player".to_owned(), data);
            assert_eq!(
                *parts.0.borrow(),
                [player("head", true), player("feet", false)]
            );
            assert!(menu.may_place(env, 5, &helmet));
            assert!(!menu.may_place(env, 8, &helmet));
        });
    }
}
