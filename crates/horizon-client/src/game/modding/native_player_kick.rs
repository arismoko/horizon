//! A server mod's kick (`PlayerRef::kick`, Paper's `Player.kick`).
use super::Game;
use horizon_client::{lan::data::component_nbt, text::Text};
use mod_api::PlayerRef;

impl Game<'_> {
    pub(super) fn connected_guest_index(&self, player: PlayerRef) -> Option<usize> {
        let acting = self.native_acting_player() == Some(player);
        let host = self.lan.as_ref()?;
        if !acting {
            self.online_player_record(player)?;
        }
        host.hub
            .guests
            .iter()
            .position(|g| g.uuid == player.uuid() && g.participates_online() && !g.is_fake())
    }

    /// `ServerGamePacketListenerImpl.disconnect`: the player leaves with
    /// `reason` on their screen. False for a stale, unavailable or fake player.
    pub(super) fn player_kick(&mut self, player: PlayerRef, reason: Text) -> bool {
        let Some(i) = self.connected_guest_index(player) else {
            return false;
        };
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let conn = host.hub.guests[i].conn;
        host.hub.kick(conn, component_nbt(&reason.component()));
        true
    }
}
