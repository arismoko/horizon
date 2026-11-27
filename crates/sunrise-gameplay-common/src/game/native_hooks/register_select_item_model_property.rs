use super::*;

/// Gather attributed categorical `minecraft:select` item model property providers at client
/// startup, before resources load (NeoForge's `RegisterSelectItemModelPropertyEvent`).
pub fn native_register_select_item_model_property(
    context: &mod_api::RegistrationContext,
    properties: &mut Vec<Answered<SelectPropertyRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_select_item_model_property(context, properties);
}
