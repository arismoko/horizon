//! Engine owners of detached placement and publication use the same frozen plan.
use super::super::Role;
use super::{Kind, Menu, custom_plan::MenuPlacement};
impl MenuPlacement {
    /// Physical slots whose old contents the placement operation may return.
    pub fn cleared_slots(&self) -> &[usize] {
        &self.clear
    }
    /// A `Mod` layout's plan before its part names the grid: only `for_mod` plans have no inputs.
    pub fn awaits_grid(&self) -> bool {
        self.inputs.is_empty()
    }
    /// Authoritative ingredient predicates, never display examples.
    pub fn requirements(&self) -> &crate::crafting::Placement {
        &self.placement
    }
}
impl Menu {
    /// Rebuild only visibility from the actual current mod menu. A cached
    /// machine may have hidden slots left over from an earlier linked view.
    pub fn reset_mod_visibility(&mut self, unlinked: impl IntoIterator<Item = usize>) {
        let Kind::Mod(kind) = self.state.kind else {
            return;
        };
        self.touch();
        let layout = self.state.kind;
        for (index, slot) in self.state.slots[..kind.slots].iter_mut().enumerate() {
            if slot.role == Role::Hidden {
                slot.role = layout.slot(index).unwrap().role;
            }
        }
        for slot in unlinked {
            self.hide_slot(slot);
        }
    }
}
