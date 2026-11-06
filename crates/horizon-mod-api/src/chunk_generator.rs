//! Chunk generators a native mod registers: vanilla's `ChunkGenerator` with
//! its own terrain, as
//! [`register_chunk_generator`](super::Registrar::register_chunk_generator)
//! hands it over once at startup. A level whose stem names `modid:void`, one
//! [`register_dimension`](super::Registrar::register_dimension) made or a data
//! pack's, then generates by the mod's code.
//!
//! There is no `Host` here: chunks fill on the worldgen worker threads, so the
//! mod sees only the [`ChunkAccess`] of the chunk it fills, and a generator
//! keeps no mutable state because two workers can fill neighbouring chunks at
//! once. The level gets terrain only: no surface, carvers or features.
pub use horizon_world::worldgen::mod_generators::ChunkAccess;
use horizon_world::worldgen::mod_generators::{ModChunkGenerator, ModChunkGenerators};
use std::sync::Arc;

/// A mod's chunk generator: vanilla's `ChunkGenerator`, whose code answers for
/// a level stem with `"generator": {"type": "modid:void"}`. Implemented once,
/// kept by the game; deterministic and stateless.
pub trait ChunkGenerator: Send + Sync + 'static {
    /// `ChunkGenerator.fillFromNoise`: write the terrain of `chunk`, which
    /// starts as air. The same seed and chunk give the same blocks.
    fn fill_from_noise(&self, chunk: &mut dyn ChunkAccess);
}

/// A mod's generator as the engine's.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct Adapter(pub(crate) Arc<dyn ChunkGenerator>);

#[cfg(not(target_arch = "wasm32"))]
impl ModChunkGenerator for Adapter {
    fn fill_from_noise(&self, chunk: &mut dyn ChunkAccess) {
        self.0.fill_from_noise(chunk);
    }
}

/// What the game knows about a registered generator: its namespaced id
/// (`modid:void`), which a level stem's `generator` names, and its engine side.
pub(crate) struct ChunkGeneratorInfo {
    pub(crate) id: Box<str>,
    pub(crate) generator: Arc<dyn ModChunkGenerator>,
}

/// The generators the mods registered at startup, as the engine reads them:
/// `modid:void` → generator. Empty before the parts are installed.
pub fn installed_chunk_generators() -> ModChunkGenerators {
    super::parts::installed_chunk_generator_list()
        .iter()
        .map(|info| (info.id.to_string(), info.generator.clone()))
        .collect()
}
