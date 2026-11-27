//! The `player_client_loaded_world` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player finished loading the world around them after joining, respawning or changing
    /// dimension. The guest's state is not swapped in. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_client_loaded_world",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerClientLoadedWorld)"
    )]
    fn player_client_loaded_world(player: u128, name: &str);
}
