use super::*;

/// Collect typed client spectator post-effect registrations once at startup.
pub fn native_register_entity_spectator_shaders(
    context: &mod_api::RegistrationContext,
    shaders: &mut Vec<mod_api::EntitySpectatorShader>,
) {
    crate::game::native_hooks::fire::fire_register_entity_spectator_shaders(context, shaders);
}
