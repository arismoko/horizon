use super::*;

/// Extra geometry in chunk section meshes (NeoForge's `AddSectionGeometryEvent`):
/// `register(renderer)` adds the mod's own `SectionGeometry`, which the mesher asks to add its
/// quads every time it meshes a section, on its worker threads. Asked once at start on the
/// client, before resources load.
pub fn native_add_section_geometry(
    context: &mod_api::RegistrationContext,
    renderers: &mut mod_api::SectionGeometryRegistry,
) {
    crate::game::native_hooks::fire::fire_add_section_geometry(context, |listen| {
        renderers.register_listener(listen)
    });
}
