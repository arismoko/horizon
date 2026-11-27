use super::*;

impl crate::game::ServerGame<'_> {
    /// A menu's owner teardown (`AbstractContainerMenu.removed`): routed to the
    /// `MenuPart` owning `menu` (or `block` for a stock menu opened by a mod block),
    /// carrying any captured opening hand and detached owned slots.
    #[inline(never)]
    pub(in crate::game) fn native_menu_removed(
        &mut self,
        call: mod_api::__private::MenuRemovedCall<'_>,
    ) {
        self.fire_menu_removed(call);
    }
}
