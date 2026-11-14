//! Scratch stacks (`ItemPlace::Scratch`, NeoForge's `ItemAccess.forStack`): a stack a mod holds
//! outside the world while a transaction is open, to run item capabilities against. The
//! outermost transaction keeps them; each transaction keeps a stack as it was before its first
//! write, as it keeps every other storage, so an abort puts it back, and the outermost close
//! drops them all. A write never changes a stack's item (`StackItemAccess`).
use super::{Game, HashMap, frame_token};
use horizon_client::items::ItemStack;
use mod_api::{ItemPlace, Transaction};
use std::sync::Arc;

/// One scratch stack: its item, fixed, and what of it is held, none once emptied.
#[derive(Clone)]
pub(super) struct Scratch {
    item: Arc<str>,
    stack: Option<ItemStack>,
}

impl Game<'_> {
    /// Keeps scratch stack `id` as it is, if the innermost transaction hasn't yet.
    fn keep_scratch(&mut self, id: u64) {
        let was = self
            .transactions
            .first()
            .and_then(|root| root.scratch.get(&id).cloned());
        if let Some(level) = self.transactions.last_mut() {
            level.scratch_kept.entry(id).or_insert(was);
        }
    }
    /// Puts back the scratch stacks an aborted transaction wrote, from what it kept.
    pub(super) fn scratch_roll_back(&mut self, kept: HashMap<u64, Option<Scratch>>) {
        // With the outermost aborted, its stacks went with it.
        let Some(root) = self.transactions.first_mut() else {
            return;
        };
        for (id, was) in kept {
            match was {
                Some(was) => root.scratch.insert(id, was),
                None => root.scratch.remove(&id),
            };
        }
    }
    /// `scratch_stack`: holds `stack` in the open transaction `tx`, as a new scratch place.
    pub(in crate::game) fn scratch_stack(
        &mut self,
        tx: &Transaction,
        stack: ItemStack,
    ) -> Option<ItemPlace> {
        if !self.transaction_holds(tx) || stack.count == 0 {
            return None;
        }
        let id = frame_token();
        self.keep_scratch(id);
        let scratch = Scratch {
            item: stack.item.clone(),
            stack: Some(stack),
        };
        self.transactions.first_mut()?.scratch.insert(id, scratch);
        Some(ItemPlace::Scratch(id))
    }
    /// `scratch_take`: the stack at scratch place `place`, which ends.
    pub(in crate::game) fn scratch_take(
        &mut self,
        tx: &Transaction,
        place: &ItemPlace,
    ) -> Option<ItemStack> {
        let ItemPlace::Scratch(id) = place else {
            return None;
        };
        if !self.transaction_holds(tx) || !self.transactions.first()?.scratch.contains_key(id) {
            return None;
        }
        self.keep_scratch(*id);
        self.transactions.first_mut()?.scratch.remove(id)?.stack
    }
    /// The stack scratch stack `id` holds.
    pub(in crate::game) fn scratch_held(&self, id: u64) -> Option<ItemStack> {
        self.transactions.first()?.scratch.get(&id)?.stack.clone()
    }
    /// Puts what of `offer` fits into scratch stack `id`: its own item only, empty or with the
    /// same components, up to the item's stack size. How many went.
    pub(in crate::game) fn scratch_insert(&mut self, id: u64, offer: &ItemStack) -> u32 {
        let Some(held) = self
            .transactions
            .first()
            .and_then(|root| root.scratch.get(&id))
        else {
            return 0;
        };
        if held.item != offer.item
            || held
                .stack
                .as_ref()
                .is_some_and(|held| !held.same_item_same_components(offer))
        {
            return 0;
        }
        let have = held.stack.as_ref().map_or(0, |held| held.count);
        let room = self.containers().max_stack(offer).saturating_sub(have);
        let put = room.min(offer.count);
        if put == 0 {
            return 0;
        }
        self.keep_scratch(id);
        if let Some(held) = self
            .transactions
            .first_mut()
            .and_then(|root| root.scratch.get_mut(&id))
        {
            held.stack = Some(ItemStack {
                count: have + put,
                ..offer.clone()
            });
        }
        put
    }
    /// Takes up to `want.count` of `want`'s item and components out of scratch stack `id`. How
    /// many came.
    pub(in crate::game) fn scratch_extract(&mut self, id: u64, want: &ItemStack) -> u32 {
        let Some(held) = self
            .scratch_held(id)
            .filter(|held| held.same_item_same_components(want))
        else {
            return 0;
        };
        let took = want.count.min(held.count);
        if took == 0 {
            return 0;
        }
        self.keep_scratch(id);
        if let Some(scratch) = self
            .transactions
            .first_mut()
            .and_then(|root| root.scratch.get_mut(&id))
        {
            scratch.stack = (held.count > took).then(|| ItemStack {
                count: held.count - took,
                ..held
            });
        }
        took
    }
}
