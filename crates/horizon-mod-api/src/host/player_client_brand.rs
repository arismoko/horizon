//! The `player_client_brand` area: host methods the game implements (`game/host/player_client_brand.rs`).
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `player_client_brand` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
pub trait PlayerClientBrandBackend {
    /// Side: both.
    /// The brand the player's client announced when it connected (`vanilla`, `fabric`,
    /// `horizon`), as the connection stored it; empty when the client sent none. Includes
    /// displaced connected players. Empty for an acting player with no known connection; None for
    /// a stale or unavailable player. Server-side.
    fn player_client_brand(&self, player: PlayerRef) -> Option<String> {
        crate::phase::unprovided(
            "Backend::player_client_brand",
            "player_client_brand",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
