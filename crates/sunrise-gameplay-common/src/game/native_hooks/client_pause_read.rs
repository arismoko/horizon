/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::CommonGame<'_> {
    /// The applied client pause state, not whether a pause screen is open.
    pub fn is_paused(&self) -> bool {
        self.pause
    }
}
