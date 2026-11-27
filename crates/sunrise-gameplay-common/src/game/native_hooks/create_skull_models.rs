use super::*;

/// Collect the mods' skull models once at client startup, after layer definitions freeze.
/// API @API:create_skull_models@.
pub fn native_create_skull_models(
    context: &mod_api::RegistrationContext,
    models: &mut Vec<SkullModel>,
) {
    crate::game::native_hooks::fire::fire_create_skull_models(context, models);
}
