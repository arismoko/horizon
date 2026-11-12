//! The player limit for server mods (`server_max_players`, Paper's `Server.getMaxPlayers`
//! and `setMaxPlayers`).
use super::Game;

impl Game<'_> {
    /// `PlayerList.getMaxPlayers`: the live login limit.
    pub(super) fn server_max_players(&self) -> Option<u32> {
        let host = self.lan.as_ref()?;
        Some(u32::try_from(host.hub.lan.shared.max_players()).unwrap_or(u32::MAX))
    }

    /// Paper's `Server.setMaxPlayers`: the live login limit, not `server.properties`.
    pub(super) fn server_set_max_players(&mut self, max_players: u32) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        host.hub.lan.shared.set_max_players(max_players as usize);
        true
    }
}
