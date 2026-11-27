use super::*;

impl crate::game::ClientGame<'_> {
    /// Effective HUD and first-person hand visibility.
    pub fn hud_mode(&self) -> mod_api::HudMode {
        self.client.chat_ui.hud_mode()
    }
    /// Set HUD and hand visibility on the client window.
    pub fn set_hud_mode(&mut self, mode: mod_api::HudMode) {
        self.client.chat_ui.set_hud_mode(mode);
    }
}
