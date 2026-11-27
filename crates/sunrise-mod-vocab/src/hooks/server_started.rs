//! The `server_started` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server has opened its worlds and is about to run its tick loop
    /// (NeoForge's `ServerStartedEvent`): every level is loaded, no player has
    /// joined, and the first tick has not run. The integrated server's initial
    /// world save has completed before `server_starting` and this notice. It
    /// saves the world, not an already joined player. Fired once per successful
    /// open; failed startup skips it but still reaches `server_stopped`.
    /// A listener panic does not undo the initial save or fail startup. It
    /// discards that callback's journaled server writes, including nested
    /// commits, but not effects made outside the Host transaction journal.
    /// `dedicated` distinguishes a dedicated from an integrated server.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_server_started",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ServerStartedEvent",
        cancellable = false,
        version = 2
    )]
    fn server_started(dedicated: bool);
}
