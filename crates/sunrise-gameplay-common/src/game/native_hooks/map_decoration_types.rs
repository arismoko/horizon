use super::*;

/// Collect the mods' map decoration types once at startup on each side.
pub fn native_map_decoration_types(
    context: &mod_api::RegistrationContext,
    types: &mut Vec<MapDecorationType>,
) {
    crate::game::native_hooks::fire::fire_map_decoration_types(context, types);
}
