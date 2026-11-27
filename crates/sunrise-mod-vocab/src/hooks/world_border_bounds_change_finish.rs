//! The `world_border_bounds_change_finish` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// When a moving server border reaches its target size at the start of `level`'s
    /// normal tick, after the server's tick functions, including on an idle parked level.
    /// Starting or replacing an extent does not finish it. Read the final bounds with
    /// `event.level.world_border(host)`. A panic is logged with the mod's ID, shown in an
    /// error toast and disables just that mod's callback for the session. Nested dispatch
    /// skips callbacks for this hook already running on the thread. Other refusals are
    /// logged each time.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_world_border_bounds_change_finish",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        independent_listener = true,
        neoforge = "none (NeoForge has no such event; Bukkit's WorldBorderBoundsChangeFinish)",
        cancellable = false
    )]
    fn world_border_bounds_change_finish(level: LevelRef);
}
