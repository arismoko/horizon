use super::*;

pub fn native_register_load_conditions(
    context: &mod_api::RegistrationContext,
    conditions: &mut mod_api::LoadConditionDeclarations,
) {
    crate::game::native_hooks::fire::fire_register_load_conditions(context, |listen| {
        conditions.register_listener(listen)
    });
}
