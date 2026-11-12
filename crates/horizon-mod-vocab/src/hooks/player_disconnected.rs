//! The `player_disconnected` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Why `player` (`name`) left: heard right after `player_logged_out`,
    /// for the same players, once each. `reason` is what ended the connection
    /// (the first cause kept, so a timeout after a kick stays a kick) and
    /// `message` the line the console's "lost connection" shows (a kick's
    /// text, `Timed out`, the read error; `Server closed` when the server
    /// stops). Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (PlayerEvent.PlayerLoggedOutEvent has no reason)",
        paper = "PlayerQuitEvent.getReason()"
    )]
    fn player_disconnected();
}
