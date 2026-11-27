//! Native terrain adapter for the API's worker context.
use super::native_worldgen::WorkerResources;
use mod_api::{
    BiomeKey, BlockKey, BlockMetadata, BlockState, ChunkAccess, ChunkBiomes, ChunkGenerator,
    ChunkPos, Compound, Identifier,
};
use std::sync::Arc;
use sunrise_world::worldgen::mod_generators::{
    ChunkAccess as EngineChunk, ChunkBiomes as EngineBiomes, ModChunkGenerator, ModChunkGenerators,
    VanillaStages, install_mod_chunk_generators,
};

/// A mod's generator with the vanilla stages it asked for at registration.
struct Adapter {
    stages: VanillaStages,
    generator: Arc<dyn ChunkGenerator>,
}
impl ModChunkGenerator for Adapter {
    fn build_terrain(&self, chunk: &mut dyn EngineChunk) {
        let Ok(worker) = WorkerResources::get(chunk.registry()) else {
            log::error!("Sunrise: mod terrain has no resolved worker metadata");
            return;
        };
        self.generator.build_terrain(&mut Converting {
            inner: chunk,
            worker,
        });
    }
    fn vanilla_stages(&self) -> VanillaStages {
        self.stages
    }
    fn create_biomes(&self, chunk: &mut dyn EngineBiomes) {
        self.generator.create_biomes(&mut Biomes(chunk));
    }
}

/// The engine's biome cells as the API's [`ChunkBiomes`].
struct Biomes<'a>(&'a mut dyn EngineBiomes);
impl ChunkBiomes for Biomes<'_> {
    fn pos(&self) -> ChunkPos {
        self.0.pos()
    }
    fn level(&self) -> &str {
        self.0.level()
    }
    fn seed(&self) -> i64 {
        self.0.seed()
    }
    fn min_y(&self) -> i32 {
        self.0.min_y()
    }
    fn height(&self) -> i32 {
        self.0.height()
    }
    fn fill_biomes_from_noise(&mut self, resolver: &mut dyn FnMut([i32; 3]) -> BiomeKey) {
        self.0.fill_biomes_from_noise(&mut |quart| -> Identifier {
            resolver(quart).into_identifier()
        });
    }
}

struct Converting<'a> {
    inner: &'a mut dyn EngineChunk,
    worker: &'a WorkerResources,
}
impl ChunkAccess for Converting<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn pos(&self) -> ChunkPos {
        self.inner.pos()
    }
    fn level(&self) -> &str {
        self.inner.level()
    }
    fn seed(&self) -> i64 {
        self.inner.seed()
    }
    fn min_y(&self) -> i32 {
        self.inner.min_y()
    }
    fn height(&self) -> i32 {
        self.inner.height()
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.inner.registry(), block, properties)
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockState) -> bool {
        self.worker
            .index
            .to_engine(state)
            .is_ok_and(|state| self.inner.set_block(pos, state))
    }
    fn set_block_entity_nbt(&mut self, nbt: Compound) {
        self.inner.set_block_entity_nbt(nbt);
    }
}

pub(in crate::game) fn installed_chunk_generators() -> ModChunkGenerators {
    mod_api::__private::chunk_generator_declarations()
        .into_iter()
        .map(|(id, stages, generator)| {
            let mod_api::VanillaStages {
                surface,
                carvers,
                structures,
                features,
            } = stages;
            let stages = VanillaStages {
                surface,
                carvers,
                structures,
                features,
            };
            (
                id,
                Arc::new(Adapter { stages, generator }) as Arc<dyn ModChunkGenerator>,
            )
        })
        .collect()
}

/// Hand the chunk generators the mods registered to the engine, before any world opens.
pub fn install() {
    if install_mod_chunk_generators(installed_chunk_generators()).is_err() {
        log::warn!("Mod chunk generators already installed; keeping the first table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::install_parts;
    use mod_api::{Hooks, ModEntry, Registrar, RegistrationContext};

    /// Terrain alone, with the surface and the features after it.
    struct Decorated;
    impl ChunkGenerator for Decorated {
        fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
        fn vanilla_stages(&self) -> mod_api::VanillaStages {
            mod_api::VanillaStages {
                surface: true,
                features: true,
                ..Default::default()
            }
        }
    }

    /// The engine sees the stages the mod's generator asked for, read once, at registration.
    #[test]
    fn a_generators_vanilla_stages_reach_the_engine_read_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ASKED: AtomicUsize = AtomicUsize::new(0);
        struct Counted;
        impl ChunkGenerator for Counted {
            fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
            fn vanilla_stages(&self) -> mod_api::VanillaStages {
                ASKED.fetch_add(1, Ordering::Relaxed);
                Decorated.vanilla_stages()
            }
        }
        struct Registers;
        #[mod_api::__subscriptions]
        impl Hooks for Registers {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                r.register_chunk_generator("chaos", Counted);
            }
        }
        install_parts(&[ModEntry::new("test", Box::new(Registers))]).unwrap();
        let generators = installed_chunk_generators();
        for _ in 0..2 {
            assert_eq!(
                generators[&Identifier::parse("test:chaos").unwrap()].vanilla_stages(),
                VanillaStages {
                    surface: true,
                    carvers: false,
                    structures: false,
                    features: true,
                }
            );
        }
        assert_eq!(ASKED.load(Ordering::Relaxed), 1);
    }
}
