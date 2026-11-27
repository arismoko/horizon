/// The hook points. Each does nothing in Sunrise.
#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// Before the in-world HUD, including under F1 (`RenderGuiEvent.Pre`). A veto skips
    /// the layer walk and its Post, and ordinary toasts, but not mandatory diagnostics.
    pub fn native_render_gui_pre(&self, canceled: &mut bool) {
        self.fire_render_gui_pre(canceled);
    }
}
