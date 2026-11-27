use super::*;

/// Collect the mods' fluid models before the client bakes its initial or reloaded resources.
pub fn native_register_fluid_models(
    context: &mod_api::RegistrationContext,
    models: &mut Vec<mod_api::FluidModel>,
) {
    crate::game::native_hooks::fire::fire_register_fluid_models(context, models);
}
