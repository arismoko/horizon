//! The `player_input` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest sent the server's current `PlayerInput` flags, after they were applied to the hub. The guest's state is swapped in. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_input",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerInputEvent)"
    )]
    fn player_input(player: u128, name: &str, flags: i32);
}
