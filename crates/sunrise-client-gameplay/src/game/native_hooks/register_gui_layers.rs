/// Registers HUD layers and their draw callbacks (NeoForge's `RegisterGuiLayersEvent`).
/// Asked once at client startup.
pub(in crate::game) fn native_register_gui_layers(
    context: &mod_api::RegistrationContext,
    layers: &mut sunrise_client::gui_layers::GuiLayers,
) {
    crate::game::native_hooks::fire::fire_register_gui_layers(context, |listen| {
        layers.register_listener(listen)
    });
}
