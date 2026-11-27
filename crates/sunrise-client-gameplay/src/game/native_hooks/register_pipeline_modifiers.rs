/// Collect the mods' pipeline modifier registrations once at startup.
pub fn native_register_pipeline_modifiers(
    context: &mod_api::RegistrationContext,
    modifiers: &mut Vec<mod_api::PipelineModifier>,
) {
    crate::game::native_hooks::fire::fire_register_pipeline_modifiers(context, modifiers);
}
