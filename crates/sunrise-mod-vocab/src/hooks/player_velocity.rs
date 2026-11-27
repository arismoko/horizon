//! The `player_velocity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server applied `velocity` to a player, when syncing changed motion to that player's client.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_velocity",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_velocity(player: u128, name: &str, velocity: [f64; 3]);
}
