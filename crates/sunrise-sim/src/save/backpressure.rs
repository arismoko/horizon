//! Backpressure for asynchronous world saves.
use super::*;

const MAX_ASYNC_CHUNK_WRITES: usize = 512;

impl SaveDir {
    pub(super) fn encoding_for_save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<Arc<Encoding>> {
        biomes.ensure_saveable()?;
        if !chunks.is_empty() && self.active_chunk_writes() >= MAX_ASYNC_CHUNK_WRITES {
            // Half the cap, not zero: this pass only needs room for its own batch,
            // and flushing would hold it for every queued record the writer is
            // already working through. Resuming at half keeps the writer fed and
            // still leaves the queue headroom for the next pass.
            self.wait_for_chunk_writes_below(MAX_ASYNC_CHUNK_WRITES / 2)?;
        }
        Ok(Arc::new(Encoding {
            items: self.items_for_save()?.clone(),
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
        }))
    }
}
