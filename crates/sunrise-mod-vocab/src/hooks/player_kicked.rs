//! The `player_kicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A network or fake guest was kicked, after the hub recorded the rendered `reason`. The guest's state is
    /// swapped in unless they were already removed in the same poll.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_kicked",
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerKickEvent)"
    )]
    fn player_kicked(player: u128, name: &str, reason: &str);
}
