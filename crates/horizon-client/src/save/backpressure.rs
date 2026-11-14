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
            self.flush()?;
        }
        Ok(Arc::new(Encoding {
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
        }))
    }
}

pub(super) fn compress_records(
    records: &[((i32, i32), &Record)],
    timestamp: u32,
) -> Vec<std::io::Result<Option<ChunkRecord>>> {
    // `RegionFile.getVersion`: the version `region-file-compression` selected
    // (`RegionFileVersion.getSelected`), zlib unless a dedicated server said
    // otherwise.
    let compression = Compression::selected();
    let compress = |record: &Record| {
        if let Record::Chunk(_, encoding, _) | Record::Proto(_, encoding) = record {
            encoding
                .biomes
                .ensure_saveable()
                .map_err(std::io::Error::other)?;
        }
        record
            .tag()
            .map(|tag| ChunkRecord::from_nbt(&tag, compression, timestamp, Limits::saved()))
            .transpose()
    };
    let helpers = horizon_threads::worker_count()
        .div_ceil(2)
        .min(records.len() / CHUNKS_PER_HELPER);
    if helpers <= 1 {
        return records.iter().map(|(_, r)| compress(r)).collect();
    }
    share_out(records.len(), helpers, |i| compress(records[i].1), || true)
}
