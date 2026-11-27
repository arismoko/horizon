use sunrise_client::hud::HudBatch;

/// Post-HUD hook (`RenderGuiEvent.Post`) and drawn HUD layer bounds queries.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// After the complete in-world HUD layer walk (including under F1) and before screens,
    /// toasts, debug overlays and IME draw (`RenderGuiEvent.Post`).
    pub fn native_render_gui_post(&self, draw: &mut HudBatch) {
        self.fire_render_gui_post(draw);
    }

    /// Bounding rectangle `[x, y, width, height]` in GUI pixels of a HUD layer drawn
    /// during the current HUD pass (`ScreensClientHost::hud_layer_bounds`).
    pub fn hud_layer_bounds(
        &self,
        layer: &str,
    ) -> Result<Option<[f32; 4]>, mod_api::LevelReadError> {
        if !self.native_window() {
            return Err(mod_api::LevelReadError::Unavailable);
        }
        Ok(self
            .client
            .hud_layer_bounds
            .iter()
            .rev()
            .find_map(|&(id, rect)| (id == layer).then_some(rect)))
    }
}
