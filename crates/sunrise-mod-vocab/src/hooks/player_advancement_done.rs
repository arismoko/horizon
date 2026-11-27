//! The `player_advancement_done` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player completed an advancement, after progress was recorded.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_advancement_done",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "AdvancementEvent.AdvancementEarnEvent",
        cancellable = false
    )]
    fn player_advancement_done(player: u128, name: &str, advancement: &str);
}
