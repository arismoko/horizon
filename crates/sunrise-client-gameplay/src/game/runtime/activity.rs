//! What the player is doing, for the hooks above the engine (`Hooks::activity`: the Sunrise
//! layer's Discord presence shows it).
use super::*;
use sunrise_command::args::GameMode;
use sunrise_hooks::{Activity, Client, Dimension, Play};

/// At startup, for a game window (never a dedicated server's): the player's first activity, then
/// `Hooks::client_started`.
pub fn client_started(options: &Options) {
    if options.dedicated.is_some() {
        return;
    }
    let automated = options.benchmark.is_some()
        || options.offscreen
        || options.frames.is_some()
        || options.world_frames.is_some();
    sunrise_hooks::engine::activity(Activity::Menus);
    sunrise_hooks::engine::client_started(&Client::new(automated));
}

impl crate::game::ClientGame<'_> {
    /// Each frame: only a change reaches the hooks.
    pub(in crate::game::client_side) fn report_activity(&self) {
        sunrise_hooks::engine::activity(self.activity());
    }
    fn activity(&self) -> Activity {
        if self.client.title.0.is_some() {
            return Activity::Menus;
        }
        let Some(remote) = &self.client.remote else {
            return Activity::Menus;
        };
        let max = sunrise_sim::lan::MAX_PLAYERS;
        let play = if let Some(server) = &self.client.integrated {
            match server.published() {
                Some(_) => Play::Hosting {
                    players: remote.listed_player_count(),
                    max,
                },
                None => Play::Singleplayer,
            }
        } else {
            Play::Multiplayer
        };
        Activity::World {
            play,
            dimension: match self.common.dim {
                Dim::Overworld => Dimension::Overworld,
                Dim::Nether => Dimension::Nether,
                Dim::End => Dimension::End,
                Dim::Extra(_) => Dimension::Other,
            },
            mode: match self.common.mode {
                GameMode::Survival => sunrise_hooks::GameMode::Survival,
                GameMode::Creative => sunrise_hooks::GameMode::Creative,
                GameMode::Adventure => sunrise_hooks::GameMode::Adventure,
                GameMode::Spectator => sunrise_hooks::GameMode::Spectator,
            },
        }
    }
}
