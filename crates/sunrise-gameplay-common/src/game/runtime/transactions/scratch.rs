//! Scratch stacks (`ItemPlace::Scratch`, NeoForge's `ItemAccess.forStack` and
//! `DispenseFluidContainer`): a stack a mod holds outside the world for the duration of one
//! scoped transaction (`HostTransaction::scratch_stack` or `HostTransaction::scratch_slots`).
//! Live loans are stored on `OpenTransactions` and owned by the frame that lent them; each
//! transaction snapshots still-live ancestor scratch slots before mutating them so aborts restore
//! them, while closed loans are removed before commit and never leak into outer journals. A
//! single-slot loan (`item: Some(..)`) never changes its item; a two-slot loan (`item: None`)
//! may change its item and catches what its first slot does not take in its second slot.
use super::{HashMap, StorageSettlement};
pub use mod_api::{ItemPlace, ScratchSlot, TransactionContext, TransactionScopeError};
use std::sync::Arc;
pub use sunrise_items::items::{ItemStack, Items};

/// One lent scratch item place: its fixed item (`Some` for a single slot, `None` for two
/// sequential dispenser slots), what of it is held (`None` once emptied), and a two-slot loan's
/// overflow slot.
#[derive(Clone)]
pub struct Scratch {
    item: Option<Arc<str>>,
    pub stack: Option<ItemStack>,
    pub overflow: Option<ItemStack>,
}

impl crate::game::CommonGame<'_> {
    /// Keeps scratch stack `id` as it is, if the innermost transaction hasn't yet.
    fn keep_scratch(&mut self, id: u64) {
        let Some(level) = self.transactions.levels.last_mut() else {
            return;
        };
        if level.scratch_kept.contains_key(&id) {
            return;
        }
        if let Some(was) = self.transactions.scratch.get(&id).cloned() {
            level.scratch_kept.insert(id, was);
        }
    }

    /// Puts back the still-live scratch stacks an aborted transaction wrote, from what it kept.
    pub fn scratch_roll_back(&mut self, kept: HashMap<u64, Scratch>) {
        for (id, was) in kept {
            if let Some(slot) = self.transactions.scratch.get_mut(&id) {
                *slot = was;
            }
        }
    }

    /// The first slot of lent scratch place `slot`.
    pub fn scratch_held(&self, slot: ScratchSlot) -> Option<&ItemStack> {
        self.transactions.scratch.get(&slot.id())?.stack.as_ref()
    }

    /// Takes up to `amount` of `resource`'s item and components out of lent scratch place `slot`.
    pub fn scratch_extract(&mut self, slot: ScratchSlot, resource: &ItemStack, amount: u32) -> u32 {
        let Some(held) = self.scratch_held(slot).filter(|held| {
            held.same_item_same_components_with_prototype(resource, self.res.items())
        }) else {
            return 0;
        };
        let took = amount.min(held.count);
        if took == 0 {
            return 0;
        }
        let remaining = held.count - took;
        let id = slot.id();
        self.keep_scratch(id);
        if let Some(scratch) = self.transactions.scratch.get_mut(&id) {
            if remaining == 0 {
                scratch.stack = None;
            } else if let Some(held) = &mut scratch.stack {
                held.count = remaining;
            }
        }
        took
    }

    /// Number of currently live lent scratch slots on this game's transaction journal.
    #[cfg(any(test, feature = "test-support"))]
    pub fn live_scratch_count(&self) -> usize {
        self.transactions.scratch.len()
    }

    /// Number of scratch rollback snapshots recorded on the innermost open transaction frame.
    #[cfg(any(test, feature = "test-support"))]
    pub fn current_scratch_kept_count(&self) -> usize {
        self.transactions
            .levels
            .last()
            .map_or(0, |level| level.scratch_kept.len())
    }
}

pub trait ScratchGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Runs `f` inside a new (or nested) storage transaction with a lent scratch [`ItemPlace`].
    /// Commits and returns the final slots when `f` returns `true`; aborts and returns `None` on
    /// `false` or unwind. Reports a refused commit separately from callback abort.
    fn scratch_scope(
        &mut self,
        item: Option<Arc<str>>,
        stack: ItemStack,
        f: impl for<'loan> FnOnce(&'loan mut Self, &'loan TransactionContext, &'loan ItemPlace) -> bool,
    ) -> Result<Option<Scratch>, TransactionScopeError> {
        let (_, settled) = self.run_storage_scope(|game, frame| {
            let slot = game.transactions.insert_scratch(Scratch {
                item,
                stack: Some(stack),
                overflow: None,
            });
            if let Some(level) = game.transactions.levels.last_mut() {
                level.owned_scratch = Some(slot.id());
            }
            let place = ItemPlace::Scratch(slot);
            let commit = frame.lend(|tx| f(game, tx, &place));
            ((), commit)
        });
        match settled {
            StorageSettlement::Committed(slots) => Ok(slots),
            StorageSettlement::Aborted => Ok(None),
            StorageSettlement::Refused => Err(TransactionScopeError::CommitRefused),
        }
    }

    /// Puts what of `offer` fits into lent scratch place `slot`: its own item only, empty or with
    /// the same components, up to the item's stack size; a two-slot copy takes any item into its
    /// first slot where that is empty or holds the same, and what doesn't fit into its second.
    /// How many went.
    fn scratch_insert(&mut self, slot: ScratchSlot, offer: &ItemStack) -> u32 {
        if offer.count == 0 {
            return 0;
        }
        let id = slot.id();
        let Some(held) = self.transactions.scratch.get(&id).cloned() else {
            return 0;
        };
        if held.item.as_ref().is_some_and(|item| *item != offer.item) {
            return 0;
        }
        let info = self.items().get(&offer.item);
        let max = match offer.effective_max_stack_size(info) {
            Some(max) => max.clamp(1, 99) as u32,
            None => info.map_or(64, |i| i.max_stack).max(1),
        };
        let mut stack = held.stack;
        let mut overflow = held.overflow;
        let items = self.res.items();
        let mut put = fill(&mut stack, offer, offer.count, max, items);
        if held.item.is_none() {
            put += fill(&mut overflow, offer, offer.count - put, max, items);
        }
        if put == 0 {
            return 0;
        }
        self.keep_scratch(id);
        if let Some(scratch) = self.transactions.scratch.get_mut(&id) {
            scratch.stack = stack;
            scratch.overflow = overflow;
        }
        put
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> ScratchGameExt1<'__sg> for G {}

/// Puts up to `count` of `offer` into `slot`, empty or holding the same item and components,
/// up to `max`: how many went.
fn fill(
    slot: &mut Option<ItemStack>,
    offer: &ItemStack,
    count: u32,
    max: u32,
    items: &Items,
) -> u32 {
    match slot {
        Some(held) if held.same_item_same_components_with_prototype(offer, items) => {
            let put = max.saturating_sub(held.count).min(count);
            held.count += put;
            put
        }
        Some(_) => 0,
        None => {
            let put = max.min(count);
            if put > 0 {
                *slot = Some(ItemStack {
                    count: put,
                    ..offer.clone()
                });
            }
            put
        }
    }
}

use crate::game::tables::GameItems as _;

use crate::game::transactions::TransactionsGameExt as _;
