//! The `server_stopped` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Fired once at the end of server teardown (NeoForge's
    /// `ServerStoppedEvent` via `ServerLifecycleHooks.handleServerStopped`),
    /// after `stopServer` completes (network admission stop, player save and
    /// disconnect, final world save for an adopted world, per-level close and
    /// `level_unload` when the save and level close succeed, and storage
    /// release), on both integrated and dedicated servers. Always fires once
    /// `runServer` enters, even when startup failed (including a dedicated TCP
    /// port bind failure before `server_about_to_start`), the tick loop errored
    /// or panicked, the final save failed, or a shutdown listener panicked.
    /// The world's Server configs are still loaded; `config_unloading`
    /// follows this event.
    /// Failed opens save only worlds already adopted by the server (the
    /// prepared level is installed; the session lock alone is not adoption).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_stopped",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerStoppedEvent",
        cancellable = false,
        version = 2
    )]
    fn server_stopped(dedicated: bool);
}
