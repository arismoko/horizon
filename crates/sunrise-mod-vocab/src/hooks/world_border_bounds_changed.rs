//! The `world_border_bounds_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the server sets `level`'s border size. `lerp_ticks` is `None` for a fixed
    /// size and `Some(requested_ticks)` for a timed write, including a timed equal-size
    /// write. A timed equal-size write doesn't start a moving extent. Loads and client packets
    /// do not fire this hook. A panic is logged with the mod's ID, shown in an error toast and
    /// disables just that mod's callback for the session. Nested dispatch skips callbacks for
    /// this hook already running on the thread. Other refusals are logged each time.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_world_border_bounds_changed",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        independent_listener = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_bounds_changed(level: LevelRef, lerp_ticks: Option<i64>);
}
