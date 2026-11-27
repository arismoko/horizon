use super::*;

/// Collect typed client loading-screen registrations once at startup.
pub fn native_register_dimension_transition_screen(
    context: &mod_api::RegistrationContext,
    screens: &mut Vec<mod_api::Answered<mod_api::DimensionTransitionScreen>>,
) {
    crate::game::native_hooks::fire::fire_register_dimension_transition_screen(context, screens);
}
