//! Inventory reads preserve CompoundContainer's short-circuit traversal.
use super::*;
use sunrise_gameplay_common::game::{container_loot_level::LootTarget, machine_containers::View};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn unpack_inventory_slot(&mut self, view: &View, slot: usize) {
        if let Some((pos, _)) = view.locate(slot) {
            self.unpack_loot(LootTarget::Block(pos), None);
        }
    }
    /// One pending lookup per visited half, without allocating a visited set.
    pub(in crate::game::server_side) fn unpack_next_inventory_part(
        &mut self,
        view: &View,
        slot: usize,
        prepared: &mut Option<Pos>,
    ) {
        if let Some((pos, _)) = view.locate(slot)
            && prepared.replace(pos) != Some(pos)
        {
            self.unpack_loot(LootTarget::Block(pos), None);
        }
    }
    /// `CompoundContainer.isEmpty` visits the other half only if needed.
    pub(in crate::game::server_side) fn inventory_is_empty(&mut self, view: &View) -> bool {
        let mut prepared = None;
        for slot in 0..view.len() {
            self.unpack_next_inventory_part(view, slot, &mut prepared);
            if self.containers().insertion_item(view, slot).is_some() {
                return false;
            }
        }
        true
    }
    /// Hopper's full check stops at the first non-full slot.
    pub(in crate::game::server_side) fn inventory_is_full(
        &mut self,
        view: &View,
        face: Direction,
    ) -> bool {
        let mut prepared = None;
        for slot in view.slots_for_face_iter(Some(face)) {
            self.unpack_next_inventory_part(view, slot, &mut prepared);
            let c = self.containers();
            if !c
                .insertion_item(view, slot)
                .is_some_and(|s| s.count >= c.max_stack(&s))
            {
                return false;
            }
        }
        true
    }
}
