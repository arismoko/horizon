//! The `world_border_center_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// After the server sets `level`'s border center, including an equal-value direct write.
    /// Read the named border with `event.level.world_border(host)`. Loads and client packets
    /// do not fire this hook. A panic is logged with the mod's ID, shown in an error toast and
    /// disables just that mod's callback for the session. Nested dispatch skips callbacks for
    /// this hook already running on the thread. Other refusals are logged each time.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_world_border_center_changed",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        independent_listener = true,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn world_border_center_changed(level: LevelRef);
}
