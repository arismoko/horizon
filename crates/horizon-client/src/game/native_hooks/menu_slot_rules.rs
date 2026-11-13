use super::*;

impl Game<'_> {
    /// Slot placement and stack limits for registered vanilla and mod menus: `menu` is the
    /// menu type, `slot` the backing container index, `stack` the stack being asked about.
    /// `may_place` and `max_stack` arrive holding the slot's own answers, which the menu type's
    /// parts may change; the click bounds the final maximum. A read-only fire: a click asks once
    /// for each slot and stack it consults while it plans, and never again to validate or apply.
    #[inline(never)]
    pub(in crate::game) fn native_menu_slot_rules(
        &self,
        menu: &str,
        slot: usize,
        stack: &mod_api::ItemStack,
        may_place: &mut bool,
        max_stack: &mut u32,
    ) {
        self.fire_menu_slot_rules(menu, slot, stack, may_place, max_stack);
    }
}
