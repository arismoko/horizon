//! The `time_skip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A manual clock write is about to move the clock: `cause` is `command` for
    /// `/time` or `mod` for a level clock setter. Set `cancel` to keep `from`.
    /// Waking sleepers uses `sleep_finished_time` alone.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_time_skip",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        version = 2
    )]
    fn time_skip(dimension: &str, from: i64, to: i64, cause: &str, cancel: &mut bool);
}
