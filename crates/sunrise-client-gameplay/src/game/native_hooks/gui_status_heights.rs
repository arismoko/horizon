#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// NeoForge's `Hud.leftHeight` and `rightHeight` while the HUD layers draw, with what a mod
    /// set during the current layer.
    pub fn gui_status_heights(&self) -> Option<[i32; 2]> {
        self.client.hud_layer_next.map(|_| {
            self.client
                .hud_status_heights_set
                .get()
                .unwrap_or_else(|| self.client.hud_status_heights.heights())
        })
    }
    /// Assigning NeoForge's `Hud.leftHeight` and `rightHeight` from a HUD layer: rows drawn
    /// after it start from `heights`. False, setting nothing, outside the HUD layer pass.
    pub fn set_gui_status_heights(&self, heights: [i32; 2]) -> bool {
        if self.client.hud_layer_next.is_none() {
            return false;
        }
        self.client.hud_status_heights_set.set(Some(heights));
        true
    }
    /// Moves heights a mod set into the status columns, before a body that pushes rows draws
    /// and after each layer.
    pub(in crate::game) fn apply_gui_status_heights(&mut self) {
        if let Some(heights) = self.client.hud_status_heights_set.take() {
            self.client.hud_status_heights.set(heights);
        }
    }
}
