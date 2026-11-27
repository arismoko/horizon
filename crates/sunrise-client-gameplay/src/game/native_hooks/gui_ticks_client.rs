impl crate::game::ClientGame<'_> {
    /// `Hud.getGuiTicks`, independent of the server and visual clocks.
    pub fn gui_ticks(&self) -> i32 {
        self.client.gui_ticks
    }
}
