//! The `server_starting` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Fired once on both dedicated and integrated servers after configured
    /// world and level loading (`level_load`), adopted datapack and text setup,
    /// spawn preparation, and the initial world save, before `server_started`
    /// and before accepting play (matching NeoForge's `ServerStartingEvent` at
    /// the end of `DedicatedServer.initServer` and `IntegratedServer.initServer`).
    /// Does not fire if startup fails before that point; a failed startup still
    /// runs teardown and reaches `server_stopped`.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_starting",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerStartingEvent",
        cancellable = false,
        version = 3
    )]
    fn server_starting(dedicated: bool);
}
