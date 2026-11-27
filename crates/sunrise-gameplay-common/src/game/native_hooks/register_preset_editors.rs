use super::*;

/// Collect attributed world-preset editor factories once at client startup
/// (NeoForge's `RegisterPresetEditorsEvent`).
pub fn native_register_preset_editors(
    context: &mod_api::RegistrationContext,
    editors: &mut Vec<Answered<PresetEditorFactoryRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_preset_editors(context, editors);
}
