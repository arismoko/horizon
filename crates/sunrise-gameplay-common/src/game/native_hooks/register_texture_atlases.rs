use super::*;

pub fn native_register_texture_atlases(
    context: &mod_api::RegistrationContext,
    atlases: &mut Vec<Answered<TextureAtlasRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_texture_atlases(context, atlases);
}
