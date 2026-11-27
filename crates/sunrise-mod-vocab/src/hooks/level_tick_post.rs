//! The `level_tick_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After a successful level tick pass on either logical side; never after a failed tick.
    /// The server ticks eligible levels in its loaded creation order.
    /// `host.server_has_time()` says whether optional work still fits in the tick
    /// (`LevelTickEvent.hasTime`); always true on a client.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_level_tick_post",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "LevelTickEvent.Post",
        version = 2
    )]
    fn level_tick_post(level: LevelRef);
}
