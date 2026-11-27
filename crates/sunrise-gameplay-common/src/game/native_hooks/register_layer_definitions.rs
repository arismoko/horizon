use super::*;

/// Gather attributed layer suppliers at client startup, before the layer table is frozen.
/// API @API:model_layers@.
pub fn native_register_layer_definitions(
    context: &mod_api::RegistrationContext,
    layers: &mut Vec<Answered<LayerDefinitionRegistration>>,
) {
    crate::game::native_hooks::fire::fire_register_layer_definitions(context, layers);
}
