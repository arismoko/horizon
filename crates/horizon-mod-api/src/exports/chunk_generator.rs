mod chunk_generator;
pub use chunk_generator::{ChunkAccess, ChunkGenerator};
mod __private {
    /// The chunk generators the mods registered, for the executable to hand to
    /// the engine before any world opens.
    pub use crate::chunk_generator::installed_chunk_generators;
}
