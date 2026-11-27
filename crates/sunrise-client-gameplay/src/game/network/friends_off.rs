//! Play with Friends left out of the build (the `friends` feature off, as in the server build):
//! no world is shared, the pause menu has no Friends button (`friends_shown` is false, as on
//! another server) and `sunrise-ctl friends` says it isn't there. game/friends.rs is the real one.
use super::Screen;
use sunrise_client::hud::HudBatch;
use sunrise_game_data::Result;

pub const HELP: &[&str] = &[];

/// The pause menu's Friends icon is vanilla's, drawn inactive: its name for the widget tree.
pub const TITLE: &str = "Play with Friends";
pub(in crate::game::client_side) fn title() -> String {
    sunrise_game_data::lang::get_or("sunrise.friends.title", TITLE)
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn friends_lan_closed(&mut self) {}

    pub(in crate::game) fn tick_friends(&mut self) {}

    /// Only reached by naming the screen (`sunrise-ctl`): back to the pause menu.
    pub(in crate::game::client_side) fn draw_friends_screen(&mut self, batch: &mut HudBatch) {
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Background);
        self.draw_screen_phase(batch, sunrise_client::screen_draw::RenderPhase::Foreground);
        self.set_screen(Screen::Paused);
    }

    pub(in crate::game::client_side) fn friends_screen_click(&mut self) -> bool {
        false
    }

    pub(in crate::game::client_side) fn friends_shown(&self) -> bool {
        false
    }

    /// `sunrise-ctl friends ...`.
    pub(in crate::game::client_side) fn friends_control(
        &mut self,
        words: &[&str],
    ) -> Result<Option<serde_json::Value>> {
        if words[0] != "friends" {
            return Ok(None);
        }
        Err("this build has no Play with Friends (the `friends` feature)".into())
    }
}
