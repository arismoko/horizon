//! The `chunk_unload` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Chunk `chunk` of `dimension` left the world (NeoForge's
    /// `ChunkEvent.Unload`), including eviction of idle columns while the level
    /// remains open. Don't read its blocks. Its block entities are still there, but changes made in
    /// this hook aren't saved: as in vanilla, the chunk is saved before it unloads.
    /// Not told when the world closes, which `level_unload` and
    /// `server_stopping`/`server_stopped` tell.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::on_chunk_unloaded::ServerGame<'_>::native_chunk_unload",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ChunkEvent.Unload"
    )]
    fn chunk_unload(dimension: &str, chunk: [i32; 2]);
}
