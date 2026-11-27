//! The `player_shield_disable` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's shield was disabled by a blocked melee hit. The shield cooldown is set
    /// immediately after this call. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_shield_disable",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_shield_disable(player: u128, name: &str, ticks: u32);
}
