impl crate::game::ServerGame<'_> {
    /// Non-result container slots of mod menu type `menu` changed on the server
    /// (`Container.setItem` / `AbstractContainerMenu.slotsChanged`). Listeners may push
    /// authoritative writes for declared result or owned slots into `outputs`.
    pub(in crate::game) fn native_menu_slot_changed(
        &mut self,
        call: mod_api::__private::MenuSlotChangedCall<'_>,
    ) {
        self.fire_menu_slot_changed(call);
    }
}
