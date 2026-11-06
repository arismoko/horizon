//! The `client_pause_read` area: host methods the game implements (`game/host/client_pause_read.rs`).
#![allow(unused_variables)]
/// The client host's methods of the `client_pause_read` area, implemented by the game and reached through [`ClientHost`](crate::ClientHost).
pub trait ClientPauseReadClientHost {
    /// Side: client.
    /// The applied client pause state, not whether a pause screen is open.
    fn is_paused(&self) -> bool {
        crate::phase::unprovided(
            "ClientHost::is_paused",
            "is_paused",
            crate::phase::LoadAccess::ClientSetup,
        )
    }
}
