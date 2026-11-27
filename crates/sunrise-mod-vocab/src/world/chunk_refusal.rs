//! Refusals when a mod relights or resends chunks of a server level.

/// Why `level.relight_chunks` or `chunk.resend` did nothing: no chunk was queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(sunrise_mod_api_macros::Wire)]
pub enum ChunkRefusal {
    /// The level or chunk handle is of an earlier open of the world, or the level is not resident.
    Stale,
    /// The call was made on a client; chunks are the server's.
    Client,
    /// `chunk.resend` of a chunk that is not loaded.
    NotLoaded,
}

impl crate::NoBackend for ChunkRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }

    fn wrong_side() -> Self {
        Self::Client
    }
}
