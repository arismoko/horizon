use super::*;

/// The custom animation targets and interpolations a mod adds (NeoForge's
/// `RegisterJsonAnimationTypesEvent`), bound to their callbacks with the registering mod.
/// Asked once at start on the client, before resources load.
pub fn native_register_json_animation_types(
    context: &mod_api::RegistrationContext,
    types: &mut Vec<Answered<JsonAnimationTypeRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_json_animation_types(context, types);
}
