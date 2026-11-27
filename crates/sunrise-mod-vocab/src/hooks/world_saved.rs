//! The `world_saved` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server completed a world save cycle. `wait` says chunk writes were
    /// flushed before returning. This includes the integrated server's initial
    /// save before `server_starting`; the final save comes after player removal.
    /// A listener panic discards that listener's journaled writes and disables this
    /// callback for the session. It does not fail the save or skip level close.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_world_saved",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        version = 2
    )]
    fn world_saved(wait: bool);
}
