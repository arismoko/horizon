use super::*;

/// Bind each logical-client sprite-set factory to its common registered particle type.
/// The same provider creates, ticks and extracts the sprite particle. Registration runs once; resource reloads rebind sprites without replaying the hook.
pub fn native_register_particle_providers(
    context: &mod_api::RegistrationContext,
    providers: &mut mod_api::ParticleProviderRegistry,
) {
    crate::game::native_hooks::fire::fire_register_particle_providers(context, |listen| {
        providers.register_listener(listen)
    });
}
