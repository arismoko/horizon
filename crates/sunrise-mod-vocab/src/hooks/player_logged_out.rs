//! The `player_logged_out` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` (`name`) left the game (NeoForge's `PlayerLoggedOutEvent`), by
    /// any path: a lost connection, a kick, the server stopping or failing.
    /// Once for each player `player_logged_in` told of, and also for a kicked
    /// fake player without a preceding login. Silent `remove_fake` sends no logout.
    /// A real player is still in `host.players().server_players()` at `PlayerList.remove`
    /// entry, before their menu closes. A listener's edits persist only on leaves this server
    /// saves; a completed proxy hand-over or a crash writes no logout save. Only when the
    /// server stops can a player still get a last message. A kicked fake has already left the
    /// list and is not saved.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_logged_out",
        dispatch = "flat",
        side = "server",
        version = 3,
        contract = "listen",
        neoforge = "PlayerEvent.PlayerLoggedOutEvent"
    )]
    fn player_logged_out(player: u128, name: &str);
}
