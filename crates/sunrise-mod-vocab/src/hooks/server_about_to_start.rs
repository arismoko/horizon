//! The `server_about_to_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server is about to open its worlds: nothing of the world is
    /// loaded yet (NeoForge's `ServerAboutToStartEvent`, "called before the
    /// server begins loading anything"). Vanilla's `MinecraftServer.loadLevel`
    /// start. Server-side: the single player server thread once Play is
    /// pressed, a dedicated server after its listener has bound. A bind
    /// failure skips this event and still reaches `server_stopped`. On a successful
    /// open this is followed by `level_load`, `server_starting`, and
    /// `server_started`; a failed open skips `server_starting`,
    /// `server_started`, and `server_stopping`, still runs `stopServer`
    /// teardown (`level_unload` for opened levels), and reaches
    /// `server_stopped`. `dedicated` tells a dedicated server from the single
    /// player one.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_about_to_start",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerAboutToStartEvent",
        cancellable = false,
        version = 2
    )]
    fn server_about_to_start(dedicated: bool);
}
