use crate::game::client_side::native_entity_layers::LayerRegistration;

/// The layers a mod inspects, hides or adds on living entity renderers (NeoForge's
/// `EntityRenderersEvent.AddLayers`): `add(target, layer)`, `stock_layers(&target)` and
/// `hide(target, stock_layer)`. Every living entity of the type, or every player of the player
/// model, in view draws `EntityRenderLayer::submit` after its model, its unhidden stock layers
/// and the mod layers added before. `entity_types`, `is_living`, `player_models`,
/// `has_mod_renderer` and `stock_layers` tell what renderers this load built. A target that is
/// no living entity type, or a stock layer a renderer lacks, is refused by name. Asked on the
/// client on every resource load, the first one and each reload, after the renderers resolve
/// (`register_entity_renderers`): added layers and hidden stock layers don't carry over from
/// one load to the next, so register them again each time.
pub(in crate::game) fn native_register_entity_layers(
    context: &mod_api::RegistrationContext,
    layers: &mut LayerRegistration<'_>,
) {
    crate::game::native_hooks::fire::fire_register_entity_layers(context, |listen| {
        layers.register_listener(listen)
    });
}
