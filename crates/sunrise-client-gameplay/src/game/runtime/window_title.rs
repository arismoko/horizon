//! `Minecraft.createTitle` and `updateTitle`: the window's title, as vanilla's reads.
use super::Screen;

/// `Minecraft* 26.3`: `ModCheck` reports a client that isn't Mojang's signed jar as modified.
pub(in crate::game) fn base() -> String {
    format!("Minecraft* {}", sunrise_core::VANILLA_VERSION)
}

/// The full title for a session part (`title.singleplayer`...), none while not connected.
fn title(session: Option<&str>) -> String {
    match session {
        Some(key) => format!("{} - {}", base(), sunrise_game_data::lang::get(key)),
        None => base(),
    }
}

impl crate::game::ClientGame<'_> {
    /// The session part `createTitle` adds while `getConnection` has a player and its connection is open.
    fn title_session(&self) -> Option<&'static str> {
        let live = self
            .client
            .remote
            .as_ref()
            .is_some_and(|r| r.session.connected());
        if !live || self.common.player.is_none() {
            return None;
        }
        Some(
            if self.client.integrated.is_some() && !self.is_published() {
                "title.singleplayer"
            } else if self.client.realm_session {
                "title.multiplayer.realms"
            } else if self.client.integrated.is_none() && !self.client.lan_session {
                "title.multiplayer.other"
            } else {
                "title.multiplayer.lan"
            },
        )
    }

    /// `updateTitle`: on a session change (level, publishing), and a new language once back to play.
    pub(in crate::game::client_side) fn update_window_title(&mut self) {
        let language = match self.client.window_title {
            Some((_, shown)) if self.client.screen != Screen::Playing => shown,
            _ => sunrise_game_data::lang::generation(),
        };
        let shown = (self.title_session(), language);
        if self.client.window_title == Some(shown) {
            return;
        }
        if let Some(window) = &self.client.window.0 {
            window.set_title(&title(shown.0));
            self.client.window_title = Some(shown);
        }
    }
}

use sunrise_gameplay_common::game::integrated_common::IntegratedCommonGameExt1 as _;
