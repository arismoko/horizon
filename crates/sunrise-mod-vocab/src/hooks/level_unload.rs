//! The `level_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `dimension`'s level closed (NeoForge's `LevelEvent.Unload`, posted after
    /// `level.close()` in `MinecraftServer.stopServer`): drop what the mod kept
    /// for it. Fires when an explicitly unloaded runtime level closes or during
    /// `MinecraftServer.stopServer`'s per-level close loop (in creation order,
    /// after network shutdown, player save/disconnect, and a successful final
    /// world save), only after that level's own close (`save.flush()`)
    /// succeeds. Idle non-Overworld levels evict chunks/entities without
    /// closing or firing `level_unload`. If the final world save fails/panics
    /// or a level's own close fails, its `level_unload` does not fire;
    /// `server_stopped` still fires unconditionally after retiring storage.
    /// Don't read its blocks. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_level_unload",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Unload",
        cancellable = false,
        version = 3
    )]
    fn level_unload(dimension: &str);
}
