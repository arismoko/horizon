//! Common player identity and local-body residency wrappers dispatching via `side_ops::player`.
use mod_api::PlayerRef;

pub trait PlayerIdentityGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Wire entity id of the acting server player or connected window player (0 if absent).
    fn own_entity_id(&self) -> i32 {
        self.side_own_entity_id()
    }

    /// The acting or window player's UUID, as servers know it.
    fn own_uuid(&self) -> u128 {
        self.own_profile_uuid().unwrap_or_default()
    }

    /// Profile UUID of the acting server player (or legacy local body) / connected client.
    fn own_profile_uuid(&self) -> Option<u128> {
        self.side_own_profile_uuid()
    }

    /// Profile name of the acting server player or connected window player.
    fn own_profile_name(&self) -> String {
        self.side_own_profile_name()
    }

    /// Logged-in window player handle (`None` on server roles).
    fn local_player(&self) -> Option<PlayerRef> {
        self.side_local_player()
    }

    /// Binds local player residency (and client pick-section / payload transport when on client).
    fn bind_local_player_residency(&mut self) {
        self.side_bind_local_player_residency();
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> PlayerIdentityGameExt1<'__sg> for G {}

use crate::game::side_ops::player::PlayerSideGameExt as _;
