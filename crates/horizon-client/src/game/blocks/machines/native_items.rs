//! The game's own item capability (`mod_api::ITEMS`, NeoForge's `Capabilities.Item.BLOCK` on
//! a vanilla container): a block entity's slots as one of its sides reaches them, numbered from
//! 0 for that side, by the rules hoppers follow. It acts where the world is simulated, never on
//! a client's copy, and only on the level in place.
use super::{Game, Kind, View};
use crate::game::transactions::After;
use horizon_client::items::ItemStack;
use mod_api::{BlockSide, LevelRef, Transaction};

/// Whether the item capability reaches this container's slots: not a composter's, nor a block
/// entity's without slots.
fn takes_from(view: &View) -> bool {
    !matches!(view.kind, Kind::Composter(_)) && view.len() > 0
}

impl Game<'_> {
    /// The level in place (`minecraft:overworld`), for a hook that isn't told
    /// which it runs in.
    pub(in crate::game) fn dimension(&self) -> &'static str {
        self.dim.id()
    }
    /// Whether this game simulates a world, as `native_set_block` asks
    /// (the fluid helpers ask it too).
    pub(in crate::game) fn native_simulates(&self) -> bool {
        self.is_logical_server()
    }
    /// The level in place, as a handle: where the game's capability lookups (a hopper's) are.
    /// None where no world is simulated.
    pub(super) fn native_level_here(&self) -> Option<LevelRef> {
        let epoch = self.native_lifecycle.epoch()?;
        self.native_simulates()
            .then(|| LevelRef::new(epoch, self.dim.key()))
    }
    /// Up to `count` out of slot `slot` of `view`, without `setChanged`.
    fn take_up_to(&mut self, view: &View, slot: usize, count: u32) -> Option<ItemStack> {
        let target = self.slot_mut_at(view, slot)?;
        let current = target.as_mut()?;
        let taken = count.min(current.count);
        let out = ItemStack {
            count: taken,
            ..current.clone()
        };
        current.count -= taken;
        if current.count == 0 {
            *target = None;
        }
        Some(out)
    }
    /// The container at `at` the item capability reaches: a block entity with slots in the
    /// level in place. Not a composter, which has no block entity, nor a block entity without
    /// slots (a sign, a banner, a mod's without a container).
    fn capability_view(&self, at: BlockSide) -> Option<View> {
        if !self.level_in_place(at.level) {
            return None;
        }
        let view = self
            .containers()
            .view((at.pos[0], at.pos[1], at.pos[2]), false)?;
        takes_from(&view).then_some(view)
    }
    /// How many slots of the block entity at `at` its side reaches (none: every slot), by its
    /// `WorldlyContainer` rules: a furnace's top reaches its input only. None where no block
    /// entity has slots, in a level not in place, and on a client's copy of the world.
    pub(in crate::game) fn container_size(&self, at: BlockSide) -> Option<usize> {
        Some(self.capability_view(at)?.slots_for_face(at.side).len())
    }
    /// The stack in slot `slot` of those `at`'s side reaches (`container_size`).
    pub(in crate::game) fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack> {
        let view = self.capability_view(at)?;
        let slot = *view.slots_for_face(at.side).get(slot)?;
        self.containers().get(&view, slot).cloned()
    }
    /// `view_changed` of a capability write, at the root commit of its transaction.
    fn view_told(&mut self, view: &View) {
        if let Kind::Composter(_) = view.kind {
            return;
        }
        for &(pos, _) in &view.parts {
            self.after_commit(After::Changed(pos));
        }
    }
    /// Puts as much of `stack` as slot `slot` of those `at`'s side reaches takes, as a hopper
    /// would (`canPlaceItem`, the slot's limit, the same item and components), in transaction
    /// `tx`; what didn't fit.
    pub(in crate::game) fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &ItemStack,
        amount: u32,
        tx: &Transaction,
    ) -> Option<ItemStack> {
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        let _ = tx;
        let Some(view) = self.capability_view(at) else {
            return Some(stack);
        };
        let Some(&slot) = view.slots_for_face(at.side).get(slot) else {
            return Some(stack);
        };
        let fits = self.containers().fits(&view, slot, &stack);
        if fits > 0 {
            let some = ItemStack {
                count: fits,
                ..stack.clone()
            };
            for &(part, _) in &view.parts {
                self.journal_block_entity(part);
            }
            // A jukebox's disc starts at the root commit (`JukeboxBlockEntity.setTheItem`).
            if self.move_in_item(None, &view, some, slot).1 {
                self.view_told(&view);
            }
        }
        (fits < stack.count).then(|| ItemStack {
            count: stack.count - fits,
            ..stack
        })
    }
    /// How many of `stack` slot `slot` of those `at`'s side reaches holds when empty, as
    /// NeoForge's `isValid` and `getCapacityAsLong` answer: none where it may not go in (a
    /// furnace's output, `canPlaceItem`), else the slot's limit for it.
    pub(in crate::game) fn container_capacity(
        &self,
        at: BlockSide,
        slot: usize,
        stack: &ItemStack,
    ) -> u32 {
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        view.slots_for_face(at.side)
            .get(slot)
            .map_or(0, |&slot| self.containers().capacity(&view, slot, stack))
    }
    /// Takes up to `max` out of slot `slot` of those `at`'s side reaches, in transaction `tx`;
    /// through a side, only what that side may take (`canTakeItemThroughFace`). What came out.
    pub(in crate::game) fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        tx: &Transaction,
    ) -> Option<ItemStack> {
        let _ = tx;
        let view = self.capability_view(at)?;
        let slot = *view.slots_for_face(at.side).get(slot)?;
        let c = self.containers();
        let stack = c.get(&view, slot)?;
        if max == 0
            || at
                .side
                .is_some_and(|face| !c.can_take(&view, slot, stack, face))
        {
            return None;
        }
        for &(part, _) in &view.parts {
            self.journal_block_entity(part);
        }
        let out = self.take_up_to(&view, slot, max)?;
        // A jukebox's song stops at the root commit (`JukeboxBlockEntity.removeItem`).
        self.view_told(&view);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::containers::ModContainer;
    use horizon_core::geometry::Direction;
    use std::sync::Arc;

    fn view(kind: Kind, slots: usize) -> View {
        View {
            parts: vec![((0, 0, 0), slots)],
            kind,
            container: (kind == Kind::Mod).then(|| {
                Arc::new(ModContainer {
                    slots,
                    ..Default::default()
                })
            }),
        }
    }

    #[test]
    fn a_composter_and_a_block_without_slots_give_nothing() {
        assert!(takes_from(&view(Kind::Generic, 27)));
        assert!(takes_from(&view(Kind::Mod, 3)));
        assert!(!takes_from(&view(Kind::Generic, 0)));
        assert!(!takes_from(&view(Kind::Composter(8), 1)));
    }

    #[test]
    fn a_face_only_narrows_the_slots_of_a_worldly_container() {
        assert_eq!(
            view(Kind::Furnace, 3).slots_to_add(Some(Direction::Up)),
            [0]
        );
        assert_eq!(
            view(Kind::Generic, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(
            view(Kind::Shulker, 2).slots_to_add(Some(Direction::Up)),
            [0, 1]
        );
        assert_eq!(view(Kind::Furnace, 3).slots_to_add(None), [0, 1, 2]);
    }
}
