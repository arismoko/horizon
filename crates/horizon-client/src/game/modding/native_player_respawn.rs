//! A server mod's respawn of a dead player (`PlayerRef::respawn`, Spigot's
//! `Player.spigot().respawn()`).
use super::Game;
use mod_api::PlayerRef;

impl Game<'_> {
    /// The Respawn button for a dead player, outside every player's hook.
    pub(super) fn player_respawn(&mut self, player: PlayerRef) -> bool {
        if self.remote.is_some() || self.lan_acting.is_some() {
            return false;
        }
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let (conn, dead) = (host.hub.guests[i].conn, host.hub.guests[i].dead);
        if !dead {
            return false;
        }
        self.guest_respawn(conn);
        self.lan.as_ref().is_some_and(|host| {
            host.hub
                .guests
                .iter()
                .any(|g| g.conn == conn && !g.dead)
        })
    }
}
