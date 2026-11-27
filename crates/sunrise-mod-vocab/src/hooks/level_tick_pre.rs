//! The `level_tick_pre` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before a level's tick pass on either logical side. `level` identifies the level.
    /// The server ticks eligible levels in its loaded creation order.
    /// `host.server_has_time()` says whether optional work still fits in the tick
    /// (`LevelTickEvent.hasTime`); always true on a client.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_level_tick_pre",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "LevelTickEvent.Pre",
        version = 2
    )]
    fn level_tick_pre(level: LevelRef);
}
