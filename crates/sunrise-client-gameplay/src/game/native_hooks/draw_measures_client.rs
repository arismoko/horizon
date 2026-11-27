use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::ClientGame<'_> {
    /// The width of `text` in GUI pixels, as `native_text` draws it on `draw` (`Font.width`).
    pub(in crate::game) fn native_text_width(&self, _draw: &HudBatch, text: &str) -> f32 {
        HudBatch::text_width(&self.client.gui.atlas, text)
    }
    /// The size of a GUI atlas sprite in its own pixels, if present.
    pub fn gui_sprite_pixels(&self, name: &str) -> Option<[f32; 2]> {
        let r = self.client.gui.atlas.gui_rect(name)?;
        Some([r.w as f32, r.h as f32])
    }
}
