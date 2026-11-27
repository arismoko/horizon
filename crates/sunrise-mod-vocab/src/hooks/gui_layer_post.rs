//! The `gui_layer_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the complete body of a vanilla or mod HUD layer (`RenderGuiLayerEvent.Post`).
    /// Fires once with the ID from `gui_layer_pre`, even if F1 or another condition hid
    /// the vanilla drawing, or if a wrapper omitted its previous body. Does not fire
    /// when `gui_layer_pre` cancels the layer. If the body schedules deferred subtitle paint,
    /// Post still fires in the HUD walk, not when that paint runs. Use `event.draw` to
    /// paint after the body, or `render_gui_post` for a pass after the whole HUD walk.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_gui_layer_post",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 4,
        contract = "listen",
        neoforge = "RenderGuiLayerEvent.Post",
        cancellable = false
    )]
    fn gui_layer_post(layer: &str, draw: &mut dyn DrawContext);
}
