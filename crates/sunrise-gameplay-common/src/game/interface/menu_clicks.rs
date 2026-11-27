//! Clicks on an open container menu: `AbstractContainerMenu.clicked` run by the menu engine
//! (`menu_session`, `click_open_inventory`, `menu_ops`), a shift-click's quick move, the creative screen's own
//! display and throw clicks, and a mouse button over the inventory (`click_inventory`).
use super::*;

pub trait MenuClicksGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `CrafterBlockEntity.setSlotState`: the menu's slot states land on the
    /// block entity, whose `setChanged` marks the chunk for saving.
    fn apply_crafter_slot_states(&mut self, menu: &sunrise_sim::menu::Menu) {
        let Some(key) = self.container else {
            return;
        };
        let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&key) else {
            return;
        };
        let mut changed = false;
        for (i, d) in c.disabled.iter_mut().enumerate() {
            let disabled = menu.data[i] == 1;
            changed |= *d != disabled;
            *d = disabled;
        }
        if changed {
            self.side_container_changed(key);
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> MenuClicksGameExt1<'__sg> for G {}

use crate::game::side_ops::events::EventsSideGameExt as _;
