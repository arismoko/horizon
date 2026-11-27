use super::*;

/// Gather attributed model loader callbacks at client startup, before resources load
/// (NeoForge's `ModelEvent.RegisterLoaders`).
pub fn native_register_model_loaders(
    context: &mod_api::RegistrationContext,
    loaders: &mut Vec<Answered<ModelLoaderRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_model_loaders(context, loaders);
}
