use crate::game::cauldron_interactions::Registrar;

/// Declare cauldron interactions before publishing the resource snapshot.
pub fn native_register_cauldron_interaction(
    context: &mod_api::RegistrationContext,
    interactions: &mut Registrar<'_>,
) {
    crate::game::native_hooks::fire::fire_register_cauldron_interaction(context, |listen| {
        interactions.register_listener(listen)
    });
}
