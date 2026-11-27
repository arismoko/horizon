/// Registers client listeners before resource staging begins.
#[cfg(feature = "client")]
#[inline(always)]
pub(in crate::game) fn native_add_client_reload_listeners(
    context: &dyn mod_api::ClientReloadListenerContext,
    register: impl FnMut(&mut dyn FnMut(&mut dyn mod_api::ClientReloadListeners) -> bool),
) {
    crate::game::native_hooks::fire::fire_add_client_reload_listeners(context, register);
}
