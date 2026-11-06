//! The `chunk_refs` area: host methods the game implements (`game/host/chunk_refs.rs`).
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `chunk_refs` area, implemented by the game and reached through [`Host`].
pub trait ChunkRefsHost {
    /// Side: both.
    /// Find a loaded chunk in an open server level.
    fn chunk(&self, level: LevelRef, pos: [i32; 2]) -> Option<ChunkRef> {
        crate::phase::unprovided("Host::chunk", "chunk", crate::phase::LoadAccess::WorldOnly)
    }
}
/// The handles' reads of the `chunk_refs` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
pub trait ChunkRefsBackend {
    /// Side: server.
    /// Whether this chunk is loaded; false on a client or for a stale handle.
    fn chunk_loaded(&self, chunk: ChunkRef) -> bool {
        crate::phase::unprovided(
            "Backend::chunk_loaded",
            "chunk_loaded",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: server.
    /// The status of a loaded chunk. Proto chunks are not exposed here.
    fn chunk_status(&self, chunk: ChunkRef) -> Option<ChunkStatus> {
        crate::phase::unprovided(
            "Backend::chunk_status",
            "chunk_status",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
    /// Side: both.
    /// Every loaded chunk of the level (`ServerChunkCache` loaded chunks), as handles ordered by
    /// chunk x then z. Server-side; None for a stale handle or on a client. Loads nothing, and the
    /// list is a snapshot: a chunk can unload after it, so read `chunk_loaded` again before use.
    fn level_loaded_chunks(&self, level: LevelRef) -> Option<Vec<ChunkRef>> {
        crate::phase::unprovided(
            "Backend::level_loaded_chunks",
            "level_loaded_chunks",
            crate::phase::LoadAccess::WorldOnly,
        )
    }
}
