//! The `player_deep_sleep` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player has slept for 100 ticks in a bed and now counts as deeply asleep for the
    /// night-skip check.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_deep_sleep",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_deep_sleep(player: u128, name: &str);
}
