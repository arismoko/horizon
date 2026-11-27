//! The `world_save_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The server is starting a world save cycle, before anything is written: before the
    /// players, the levels' chunks and the world's metadata (level.dat). What a listener changes
    /// here goes out in this same cycle: block entities it marks changed, saved data and player
    /// data it updates, including on the stop's final save before leaving players are written.
    /// `wait` is the flag `world_saved` reports at the cycle's end: true when this save flushes
    /// its chunk writes before returning (`/save-all flush`, the initial save on open, the final
    /// save on stop), false for an autosave or an integrated pause. Once for every cycle, paired
    /// with one `world_saved` unless the cycle fails; a cycle the server refuses to start (a
    /// transfer still settling, or a nested save started from this notice) asks neither. A
    /// listener panic discards that listener's journaled writes and disables this callback for
    /// the session. Not asked on a client of a remote server.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_world_save_start",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        listener_transaction = "always",
        neoforge = "none (NeoForge has no such event; LevelEvent.Save fires per level, after the levels before it were saved)"
    )]
    fn world_save_start(wait: bool);
}
