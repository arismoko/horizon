//! `player_disconnect`: a server mod sends a player away with a reason, as `/kick` does.

use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use mod_api::PlayerRef;

impl Game<'_> {
    /// Disconnects that player with `reason` through the hub's kick (the client shows it, the
    /// log names it, `player_kicked` hears it) and records the server's `kick` event. Works
    /// for the acting player inside their own hook; the connection closes at the next poll.
    /// False for a stale or unavailable player, a fake player, the single-player owner and a
    /// player whose disconnect is already pending (`Guest::disconnect_pending`: the first
    /// reason stands, and nothing more is sent or heard).
    pub(super) fn player_disconnect(&mut self, player: PlayerRef, reason: Text) -> bool {
        let Some(conn) = self.disconnect_target(player) else {
            return false;
        };
        self.kick_conn(conn, component_nbt(&reason.component()))
    }

    /// The connection of a connected player other than the single-player owner: the acting
    /// player's own (unless fake), or a connected record's, displaced ones included.
    fn disconnect_target(&self, player: PlayerRef) -> Option<u32> {
        let conn = if self.native_acting_player() == Some(player) {
            (!self.lan_acting_fake).then_some(())?;
            self.lan_acting?
        } else {
            self.online_player_record(player)?.conn
        };
        let hub = &self.lan.as_ref()?.hub;
        let guest = hub.guest(conn)?;
        (guest.participates_online()
            && guest.uuid == player.uuid()
            && hub.singleplayer_owner != Some(player.uuid()))
        .then_some(conn)
    }
}
