use super::*;

/// Collect F3 text declarations and renderer toggles with their registering mods, once at
/// client startup (NeoForge's `RegisterDebugEntriesEvent`).
pub fn native_register_debug_entries(
    context: &mod_api::RegistrationContext,
    entries: &mut Vec<Answered<DebugEntry>>,
) {
    crate::game::native_hooks::fire::fire_register_debug_entries(context, entries);
}
