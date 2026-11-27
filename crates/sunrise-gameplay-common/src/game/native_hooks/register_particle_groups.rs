use super::*;

/// Gather attributed group factories at startup, before the client builds particle engines.
pub fn native_register_particle_groups(
    context: &mod_api::RegistrationContext,
    groups: &mut Vec<Answered<ParticleGroupRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_particle_groups(context, groups);
}
