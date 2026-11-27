use super::*;

/// Collect world-space debug renderers with their registering mods at each client
/// renderer-list refresh (NeoForge's `RegisterDebugRenderersEvent`).
pub fn native_register_debug_renderers(
    context: &mod_api::RegistrationContext,
    entries: &dyn mod_api::__private::DebugEntryStates,
    renderers: &mut Vec<Answered<DebugRenderer>>,
) {
    crate::game::native_hooks::fire::fire_register_debug_renderers(context, entries, renderers);
}
