use super::*;

/// Gather attributed item tint source providers at client startup, before resources load
/// (NeoForge's `RegisterColorHandlersEvent.ItemTintSources`).
pub fn native_register_item_tint_sources(
    context: &mod_api::RegistrationContext,
    sources: &mut Vec<Answered<ItemTintSourceRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_item_tint_sources(context, sources);
}
