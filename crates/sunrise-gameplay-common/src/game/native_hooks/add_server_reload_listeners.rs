use super::*;

#[inline(always)]
pub fn native_add_server_reload_listeners(
    context: &dyn mod_api::ReloadContext,
    register: impl FnMut(&mut dyn FnMut(&mut dyn mod_api::ServerReloadListeners) -> bool),
) {
    crate::game::native_hooks::fire::fire_add_server_reload_listeners(context, register);
}
