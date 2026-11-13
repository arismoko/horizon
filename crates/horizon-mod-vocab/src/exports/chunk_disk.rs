mod chunk_disk;
pub use chunk_disk::{
    AttachmentDiskEntryWire, ChunkDiskContextWire, ChunkDiskData, ChunkDiskFailure, ChunkDiskHost,
    ChunkDiskKind, ChunkDiskPhase, ChunkDiskResultWire,
};
// hidden: engine-side save job and worker capability, used only by the persistence owner
#[doc(hidden)]
pub use chunk_disk::{ChunkDiskJob, OwningChunkDisk};
