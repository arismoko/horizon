//! Chunk attachment snapshot encoding boundary.

pub trait Hooks {
    /// A committed chunk snapshot is about to encode on the chunk worker.
    /// `data` belongs only to that snapshot: typed edits change the disk record,
    /// never the live chunk. Exactly once per captured record, for full and proto
    /// chunks alike. The restricted Host cannot resolve live chunks or mutate the
    /// world. A failed job retains its original snapshot and the prior disk record.
    #[hook(
        side = "server",
        contract = "listen",
        context = "chunk_disk",
        target = "mod_api::chunk_disk::native_chunk_data_save",
        neoforge = "ChunkDataEvent.Save"
    )]
    fn chunk_data_save();
}
