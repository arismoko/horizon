use super::*;

/// Gather attributed atlas sprite source providers at client startup, before resources load
/// (NeoForge's `RegisterSpriteSourcesEvent`).
pub fn native_register_sprite_sources(
    context: &mod_api::RegistrationContext,
    sources: &mut Vec<Answered<SpriteSourceRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_sprite_sources(context, sources);
}
