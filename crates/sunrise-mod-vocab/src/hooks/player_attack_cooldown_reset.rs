//! The `player_attack_cooldown_reset` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player reset their attack strength by swinging (`handlePunch`). The swing and reset
    /// have already happened. The player's state is not swapped in. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_attack_cooldown_reset",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_attack_cooldown_reset(player: u128, name: &str);
}
