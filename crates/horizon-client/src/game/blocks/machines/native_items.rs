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
    pub(in crate::game) fn container_set_stack_limit(
        &mut self,
        at: BlockSide,
        limit: u32,
        tx: &Transaction,
    ) -> bool {
        if !self.transaction_holds(tx) {
            return false;
        }
        if !(1..=99).contains(&limit) {
            return false;
        }
        let Some(view) = self.capability_view(at) else {
            return false;
        };
        for &(pos, _) in &view.parts {
            self.journal_block_entity(pos);
            self.world.set_container_stack_limit(pos, limit);
        }
        self.view_told(&view);
        true
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
    /// Keeps what a capability write to slot `slot` of `view` changes: that slot alone where a
    /// write there changes nothing else, else each block entity of the view whole.
    fn journal_view_slot(&mut self, view: &View, slot: usize) {
        let slot_only = match view.kind {
            // A chest's, barrel's, dispenser's, dropper's or shelf's slot, a shulker box's, a
            // mod container's: the slot is all a write changes (setChanged waits for the root
            // commit).
            Kind::Generic | Kind::Shulker | Kind::Mod => true,
            // State beside the slot: filling an empty input restarts a furnace's cooking; an
            // insert into an empty hopper sets its cooldown; a jukebox's disc starts its song;
            // a bookshelf's books are in its block state and last slot.
            Kind::Furnace | Kind::Hopper | Kind::Jukebox | Kind::Bookshelf => false,
            // Not shown to change only the slot (a crafter's disabled slots, a brewing stand's
            // brew), so kept whole as before.
            Kind::Crafter | Kind::Brewing => false,
            // A pot's item lives in its saved data, written through its own insertion view, not
            // its slots; a composter has no block entity (its level is its block state) and the
            // capability never reaches it. Listed so a new kind has to say where it goes.
            Kind::Pot | Kind::Composter(_) => false,
        };
        if slot_only && let Some((pos, slot)) = view.locate(slot) {
            return self.journal_block_entity_slot(pos, slot);
        }
        for &(part, _) in &view.parts {
            self.journal_block_entity(part);
        }
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
        if !self.transaction_holds(tx) {
            return Some(stack);
        }
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
            self.journal_view_slot(&view, slot);
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
    /// How much of `resource` the slots `at`'s side reaches hold in all (`STORAGE`'s amount),
    /// in one pass over one view of the container.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_amount(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let c = self.containers();
        view.slots_for_face_iter(at.side)
            .filter_map(|slot| c.get(&view, slot))
            .filter(|held| held.same_item_same_components(resource))
            .try_fold(0u64, |total, held| total.checked_add(u64::from(held.count)))
    }
    /// How much of `resource` an insert through `at`'s side takes now: each slot's room for it,
    /// by the rules `container_insert` follows, summed in one pass.
    /// None where the total does not fit in 64 bits.
    pub(in crate::game) fn container_kind_space(
        &self,
        at: BlockSide,
        resource: &ItemStack,
    ) -> Option<u64> {
        let Some(view) = self.capability_view(at) else {
            return Some(0);
        };
        let probe = ItemStack {
            count: u32::MAX,
            ..resource.clone()
        };
        let c = self.containers();
        view.slots_for_face_iter(at.side)
            .try_fold(0u64, |total, slot| {
                total.checked_add(u64::from(c.fits(&view, slot, &probe)))
            })
    }
    /// What the slots `at`'s side reaches hold, from its `from`th to before its
    /// `from + limit`th (fewer at the end), empty ones included: never more than
    /// `MAX_STORAGE_PAGE` slots, whatever `limit` asks, so a request cannot make the game
    /// allocate more than one page.
    pub(in crate::game) fn container_listing(
        &self,
        at: BlockSide,
        from: usize,
        limit: usize,
    ) -> Option<Vec<Option<ItemStack>>> {
        let view = self.capability_view(at)?;
        let c = self.containers();
        Some(
            view.slots_for_face_iter(at.side)
                .skip(from)
                .take(limit.min(mod_api::MAX_STORAGE_PAGE))
                .map(|slot| c.get(&view, slot).cloned())
                .collect(),
        )
    }
    /// Puts up to `amount` of `resource` into the slots `at`'s side reaches, those holding it
    /// first and then the empty ones, in transaction `tx`, on one view of the container: what
    /// went in. Each slot is kept and filled as `container_insert` keeps and fills it.
    pub(in crate::game) fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        if amount == 0 || !self.transaction_holds(tx) {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        let (mut left, mut told) = (amount, false);
        for holding in [true, false] {
            for slot in view.slots_for_face_iter(at.side) {
                if left == 0 {
                    break;
                }
                let pick = match self.containers().get(&view, slot) {
                    Some(held) => holding && held.same_item_same_components(resource),
                    None => !holding,
                };
                if !pick {
                    continue;
                }
                let stack = ItemStack {
                    count: left.min(u64::from(u32::MAX)) as u32,
                    ..resource.clone()
                };
                let fits = self.containers().fits(&view, slot, &stack);
                if fits == 0 {
                    continue;
                }
                self.journal_view_slot(&view, slot);
                let some = ItemStack {
                    count: fits,
                    ..resource.clone()
                };
                if self.move_in_item(None, &view, some, slot).1 {
                    left -= u64::from(fits);
                    told = true;
                }
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
    }
    /// Takes up to `amount` of `resource` out of the slots `at`'s side reaches and may take
    /// from, in transaction `tx`, on one view of the container: what came out.
    pub(in crate::game) fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &ItemStack,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        if amount == 0 || !self.transaction_holds(tx) {
            return 0;
        }
        let Some(view) = self.capability_view(at) else {
            return 0;
        };
        let (mut left, mut told) = (amount, false);
        for slot in view.slots_for_face_iter(at.side) {
            if left == 0 {
                break;
            }
            let c = self.containers();
            let Some(held) = c.get(&view, slot) else {
                continue;
            };
            if !held.same_item_same_components(resource)
                || at
                    .side
                    .is_some_and(|face| !c.can_take(&view, slot, held, face))
            {
                continue;
            }
            self.journal_view_slot(&view, slot);
            let ask = left.min(u64::from(u32::MAX)) as u32;
            if let Some(out) = self.take_up_to(&view, slot, ask) {
                left -= u64::from(out.count);
                told = true;
            }
        }
        if told {
            self.view_told(&view);
        }
        amount - left
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
        if !self.transaction_holds(tx) {
            return None;
        }
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
        self.journal_view_slot(&view, slot);
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
