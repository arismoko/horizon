//! Chunk attachment decoding boundary.

pub trait Hooks {
    /// The disk record's attachments have decoded, before the chunk is published.
    /// Runs on a chunk worker with a restricted Host. Read and edit `data` through
    /// its typed methods; edits become the loaded store only if the entire job
    /// succeeds. Missing records and proto-to-full promotion do not fire this hook.
    /// A codec error, unwind or guest trap refuses publication of the whole store.
    #[hook(
        side = "server",
        contract = "listen",
        context = "chunk_disk",
        target = "mod_api::chunk_disk::native_chunk_data_load",
        neoforge = "ChunkDataEvent.Load"
    )]
    fn chunk_data_load();
}
