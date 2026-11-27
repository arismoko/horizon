//! `player_disconnect`: a server mod sends a player away with a reason, as `/kick` does.

use crate::game::ServerGame;
use mod_api::PlayerRef;
use sunrise_game_data::text::Text;
use sunrise_sim::lan::data::component_nbt;

impl ServerGame<'_> {
    /// Kicks a network or fake player with `reason` through the shared leave path.
    /// The calling hook finishes before the next poll removes the player, broadcasts
    /// their leave and fires `player_logged_out`; `player_kicked` hears the reason.
    /// False for a stale or unavailable player, the single-player owner or a player
    /// already leaving: the first reason stands, and nothing more is sent or heard.
    pub(in crate::game) fn player_disconnect(&mut self, player: PlayerRef, reason: Text) -> bool {
        let Some(conn) = self.disconnect_target(player) else {
            return false;
        };
        self.kick_conn(conn, component_nbt(&reason.component()))
    }

    /// The guest record of a network or fake player, including an acting or displaced one.
    fn disconnect_target(&self, player: PlayerRef) -> Option<u32> {
        (self.common.native_lifecycle.epoch()? == player.entity().epoch()).then_some(())?;
        let hub = &self.server.lan.as_ref()?.hub;
        let guest = hub.guests.get(
            hub.guests
                .identity_index(player.uuid(), player.entity().id())?,
        )?;
        (guest.placed
            && (guest.participates_online() || guest.is_fake())
            && hub.singleplayer_owner != Some(player.uuid()))
        .then_some(guest.conn)
    }
}
