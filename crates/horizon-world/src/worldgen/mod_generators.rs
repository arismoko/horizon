//! Chunk generators a mod registers (vanilla's `ChunkGenerator`), handed over
//! once at startup and looked up when a level's stem names one. Runs on the
//! worldgen worker threads, so a generator keeps no mutable state and its
//! chunk carries no `Host`.
use crate::registry::BlockStateId;
use horizon_core::position::ChunkPos;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// The chunk a generator fills: vanilla's `ChunkAccess`, as far as terrain
/// needs it. Positions are world blocks.
pub trait ChunkAccess {
    /// `ChunkAccess.getPos`.
    fn pos(&self) -> ChunkPos;
    /// The world's seed.
    fn seed(&self) -> i64;
    /// `getMinY`: the level's lowest block Y.
    fn min_y(&self) -> i32;
    /// `getHeight`: the level's build height, from its dimension type.
    fn height(&self) -> i32;
    /// A block state by block name (`stone`, `minecraft:oak_log`), with its
    /// default properties; air for a block the game doesn't have.
    fn state(&self, name: &str) -> BlockStateId;
    /// A block state by block name with properties (`oak_log` with `axis=y`);
    /// missing properties keep defaults.
    fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId;
    /// `setBlockState`: write `state` at `pos`, answering whether it was
    /// written. A position outside the chunk or the build height is skipped.
    fn set_block(&mut self, pos: [i32; 3], state: BlockStateId) -> bool;
}

/// A chunk generator's engine side, as the executable's adapter sees it.
/// `Send + Sync` because neighbouring chunks fill at once on worker threads;
/// the same seed gives the same terrain.
pub trait ModChunkGenerator: Send + Sync {
    /// `ChunkGenerator.fillFromNoise`: the chunk's terrain.
    fn fill_from_noise(&self, chunk: &mut dyn ChunkAccess);
}

/// The installed generators, `modid:void` → generator. Set once, before any
/// world opens (the executable hands the mods' registrations over).
pub type ModChunkGenerators = BTreeMap<String, Arc<dyn ModChunkGenerator>>;

static MOD_GENERATORS: OnceLock<ModChunkGenerators> = OnceLock::new();
static EMPTY: ModChunkGenerators = BTreeMap::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_chunk_generators(
    generators: ModChunkGenerators,
) -> Result<(), ModChunkGenerators> {
    MOD_GENERATORS.set(generators)
}

/// The installed generators: empty until the executable installs the mods'.
pub fn mod_chunk_generators() -> &'static ModChunkGenerators {
    MOD_GENERATORS.get().unwrap_or(&EMPTY)
}
