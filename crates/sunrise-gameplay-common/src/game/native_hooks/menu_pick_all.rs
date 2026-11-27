use super::*;

pub trait GameHooks<'g>: crate::game::SideGame<'g> {
    /// Whether a double click's gather may take a mod menu type's slot: `menu` is the menu type,
    /// `slot` the backing index in `container`, `carried` the cursor's stack and `stack` the
    /// slot's. `allowed` arrives true; the menu type's parts may refuse. A read-only fire queried
    /// directly from the gather (`AbstractContainerMenu.canTakeItemForPickAll`).
    #[inline(never)]
    fn native_menu_pick_all(&self, call: mod_api::__private::MenuPickAllCall<'_>) {
        crate::game::native_hooks::fire::FireHooks::fire_menu_pick_all(self, call);
    }
}

impl<'g, G: crate::game::SideGame<'g>> GameHooks<'g> for G {}
