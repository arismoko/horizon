use super::*;

/// The fluid contents mods give their cauldron blocks, once the block registry is resolved.
pub fn native_register_cauldron_fluid_content(
    context: &mod_api::RegistrationContext,
    contents: &mut mod_api::CauldronFluidContentRegistry,
) {
    crate::game::native_hooks::fire::fire_register_cauldron_fluid_content(context, |listen| {
        contents.register_listener(listen)
    });
}
