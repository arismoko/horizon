//! Host authority for prepared payloads. Generations are process-unique, including
//! across guest instances; exhausted generations refuse instead of wrapping.
use std::sync::atomic::{AtomicU32, Ordering};

// Input: slot, generation, existing ItemStack Wire bytes and prepared kind strings.
// Reply: the validated (slot, generation) token.
pub(crate) const PREPARE: u32 = 0xffff_ff10;
// Input: slot, generation; empty reply.
pub(crate) const RELEASE: u32 = 0xffff_ff11;
// Matches the guest's PreparedSlots bound in horizon-mod-api/src/wire/prepared.rs.
const MAX_PREPARED_SLOTS: usize = 4096;
static NEXT_GENERATION: AtomicU32 = AtomicU32::new(1);

#[derive(Default)]
pub(crate) struct Slots(Vec<Option<Slot>>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Active(u32),
    Releasing,
}

impl Slots {
    pub(crate) fn prepare(&mut self) -> Option<(u32, u32)> {
        let index = self
            .0
            .iter()
            .position(Option::is_none)
            .unwrap_or(self.0.len());
        if index >= MAX_PREPARED_SLOTS {
            return None;
        }
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()?;
        if index == self.0.len() {
            self.0.push(Some(Slot::Active(generation)));
        } else {
            self.0[index] = Some(Slot::Active(generation));
        }
        Some((index as u32, generation))
    }
    pub(crate) fn check(&self, slot: u32, generation: u32) -> bool {
        self.0
            .get(slot as usize)
            .is_some_and(|entry| *entry == Some(Slot::Active(generation)))
    }
    pub(crate) fn release(&mut self, slot: u32, generation: u32) -> bool {
        if !self.check(slot, generation) {
            return false;
        }
        self.0[slot as usize] = Some(Slot::Releasing);
        true
    }

    pub(crate) fn finish_release(&mut self, slot: u32) {
        debug_assert!(self.0[slot as usize] == Some(Slot::Releasing));
        self.0[slot as usize] = None;
    }
}
