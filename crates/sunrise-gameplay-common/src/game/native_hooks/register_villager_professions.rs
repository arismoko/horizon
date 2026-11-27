use super::*;

pub fn native_register_villager_professions(
    context: &mod_api::RegistrationContext,
    professions: &mut mod_api::ProfessionDeclarations,
) {
    crate::game::native_hooks::fire::fire_register_villager_professions(context, |listen| {
        professions.register_listener(listen)
    });
}
