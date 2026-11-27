//! A registered chunk generator that runs no vanilla terrain stages: just the resolved biome
//! source and the blocks its `build_terrain` writes.
use crate::chunk::{Chunk, LevelShape};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};
use sunrise_core::{
    density::{ConstantSampler, DensitySampler, SamplerContext},
    position::ChunkPos,
};
use sunrise_level::chunk_streamer::ChunkGenerator;
use sunrise_world::{
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        biome_source::ResolvedBiomeSource,
        climate::ClimateSampler,
        mod_generators::{ChunkAccess, ChunkBiomes, ModChunkGenerator, pending_block_entity_pos},
        structure::legacy::block_state,
    },
};

/// `ChunkMap` uses `NoiseRouterData.none()` for a non-noise generator. Its climate and erosion
/// fields are all zero; a biome source does not opt the terrain into using a noise router.
const ZERO: ConstantSampler = ConstantSampler(0.0);
static CLIMATE: LazyLock<ClimateSampler> = LazyLock::new(|| {
    let zero: Arc<dyn DensitySampler> = Arc::new(ZERO);
    ClimateSampler::new(std::array::from_fn(|_| zero.clone()))
});

/// A mod's generator over one level.
pub struct ModTerrain {
    generator: Arc<dyn ModChunkGenerator>,
    registry: &'static BlockRegistry,
    shape: LevelShape,
    source: Arc<ResolvedBiomeSource>,
    /// The world's biomes by name, the source's possible ones and every biome a generator's
    /// `create_biomes` may name.
    biome_ids: HashMap<String, u16>,
    level: Box<str>,
    seed: i64,
}

impl ModTerrain {
    /// A generator paired with a source already bound by `BiomeSourceData::resolve_mod_biome_source`,
    /// and the world's biome table, `biome_ids`. Checks the source's possible holders against it
    /// once, before any chunk fills.
    pub fn new(
        level: &str,
        generator: Arc<dyn ModChunkGenerator>,
        source: Arc<ResolvedBiomeSource>,
        registry: &'static BlockRegistry,
        biome_ids: HashMap<String, u16>,
        shape: LevelShape,
        seed: i64,
    ) -> Result<Self, String> {
        if let Some(name) = source
            .possible_biomes()
            .into_iter()
            .find(|name| !biome_ids.contains_key(*name))
        {
            return Err(format!("{level}: unknown biome {name}"));
        }
        Ok(Self {
            generator,
            registry,
            shape,
            source,
            biome_ids,
            level: level.into(),
            seed,
        })
    }
}

/// The proto chunk as a generator writes it.
struct Filling<'a> {
    chunk: &'a mut Chunk,
    terrain: &'a ModTerrain,
    pos: ChunkPos,
    /// The pending block-entity data, by absolute position, in the order it was set.
    block_entities: &'a mut Vec<([i32; 3], sunrise_core::nbt::Compound)>,
}

/// The proto chunk's biome cells as a generator fills them.
struct Biomes<'a> {
    chunk: &'a mut Chunk,
    terrain: &'a ModTerrain,
    pos: ChunkPos,
    /// The first biome the world does not have.
    error: Option<String>,
}

impl ChunkBiomes for Biomes<'_> {
    fn pos(&self) -> ChunkPos {
        self.pos
    }
    fn level(&self) -> &str {
        &self.terrain.level
    }
    fn seed(&self) -> i64 {
        self.terrain.seed
    }
    fn min_y(&self) -> i32 {
        self.terrain.shape.min_y
    }
    fn height(&self) -> i32 {
        self.terrain.shape.height
    }
    fn fill_biomes_from_noise(
        &mut self,
        resolver: &mut dyn FnMut([i32; 3]) -> sunrise_core::identifier::Identifier,
    ) {
        let bottom = self.terrain.shape.min_y >> 2;
        let (west, north) = (self.pos.x.wrapping_mul(4), self.pos.z.wrapping_mul(4));
        // LevelChunkSection.fillBiomesFromNoise: sections bottom up, then x, y, z.
        for section in 0..self.chunk.sections.len() as i32 {
            for qx in 0..4 {
                for dy in 0..4 {
                    for qz in 0..4 {
                        let qy = section * 4 + dy;
                        let name = resolver([west + qx, bottom + qy, north + qz]);
                        match self.terrain.biome_ids.get(name.as_str()) {
                            Some(&id) => self.chunk.set_biome_cell(qx, qy, qz, id),
                            None => {
                                self.error
                                    .get_or_insert_with(|| format!("unknown biome {name}"));
                            }
                        }
                    }
                }
            }
        }
    }
}

impl ChunkAccess for Filling<'_> {
    fn registry(&self) -> &'static BlockRegistry {
        self.terrain.registry
    }
    fn pos(&self) -> ChunkPos {
        self.pos
    }
    fn level(&self) -> &str {
        &self.terrain.level
    }
    fn seed(&self) -> i64 {
        self.terrain.seed
    }
    fn min_y(&self) -> i32 {
        self.terrain.shape.min_y
    }
    fn height(&self) -> i32 {
        self.terrain.shape.height
    }
    fn state(&self, name: &str) -> BlockStateId {
        block_state(self.terrain.registry, name, &[])
    }
    fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
        block_state(self.terrain.registry, name, properties)
    }
    fn set_block(&mut self, [x, y, z]: [i32; 3], state: BlockStateId) -> bool {
        let inside = x >> 4 == self.pos.x
            && z >> 4 == self.pos.z
            && (self.min_y()..self.min_y() + self.height()).contains(&y);
        if inside {
            self.chunk.set(x & 15, y, z & 15, state.0);
        }
        inside
    }
    fn set_block_entity_nbt(&mut self, nbt: sunrise_core::nbt::Compound) {
        let heights = self.min_y()..self.min_y() + self.height();
        if let Some(pos) = pending_block_entity_pos(self.pos, &nbt, heights) {
            self.block_entities.retain(|(at, _)| *at != pos);
            self.block_entities.push((pos, nbt));
        }
    }
}

impl ChunkGenerator for ModTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(
        &self,
        _: &mut (),
        cx: i32,
        cz: i32,
    ) -> std::result::Result<sunrise_level::chunk_streamer::Proto, String> {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        if let ResolvedBiomeSource::Fixed(source) = self.source.as_ref() {
            let biome = self.biome_ids[&source.biome];
            for s in &mut chunk.sections {
                Arc::make_mut(s).biomes = sunrise_core::paletted::PalettedContainer::new(
                    crate::chunk::BIOME_STRATEGY,
                    u32::from(biome),
                );
            }
        } else {
            // ChunkGenerator.doCreateBiomes: one resolver fills all quart cells of the chunk.
            let mut resolver = self.source.resolver(&CLIMATE, &ZERO);
            let mut context = SamplerContext::default();
            let bottom = self.shape.min_y >> 2;
            let (west, north) = (cx.wrapping_mul(4), cz.wrapping_mul(4));
            for qy in 0..chunk.sections.len() as i32 * 4 {
                for qz in 0..4 {
                    for qx in 0..4 {
                        // The bound source has holders to sample and the non-noise router is constant.
                        // A registered source's failure (its code panicked, or its mod is off)
                        // fails the column: nothing of it is published.
                        let name = resolver
                            .sample(&mut context, [west + qx, bottom + qy, north + qz])
                            .map_err(|error| error.to_string())?;
                        chunk.set_biome_cell(qx, qy, qz, self.biome_ids[name]);
                    }
                }
            }
        }
        let pos = ChunkPos::new(cx, cz);
        // ChunkGenerator.createBiomes, which a generator may answer over the source's biomes.
        let mut biomes = Biomes {
            chunk: &mut chunk,
            terrain: self,
            pos,
            error: None,
        };
        self.generator.create_biomes(&mut biomes);
        if let Some(error) = biomes.error {
            return Err(format!("{}: {error}", self.level));
        }
        let mut block_entities = Vec::new();
        self.generator.build_terrain(&mut Filling {
            chunk: &mut chunk,
            terrain: self,
            pos,
            block_entities: &mut block_entities,
        });
        // LevelChunk's promotion of the pending block entities, as the chunk becomes full.
        if !block_entities.is_empty() {
            let promoted = crate::structure_blocks::promote(&block_entities, |[x, y, z]| {
                self.registry
                    .state(chunk.block(x & 15, y, z & 15))
                    .map_or("minecraft:air", |state| state.name.as_str())
            });
            chunk.block_entities = promoted.modelled;
            if !promoted.unmodelled.is_empty() || !promoted.pending_loot.is_empty() {
                chunk.saved = Some(Arc::new(crate::save::ChunkExtras {
                    block_entities: promoted.unmodelled,
                    pending_loot: promoted.pending_loot,
                    ..Default::default()
                }));
            }
        }
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        Ok(Box::new(chunk))
    }
    fn light_view(&self, proto: &sunrise_level::chunk_streamer::Proto) -> Option<Chunk> {
        proto.downcast_ref::<Chunk>().cloned()
    }
    fn finish(&self, proto: sunrise_level::chunk_streamer::Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("mod terrain proto")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_core::identifier::Identifier;
    use sunrise_world::worldgen::{
        biome_source::BiomeSourceCatalog,
        mod_biome_sources::{LevelBiomeSource, ModBiomeSource, ModBiomeSources},
        mod_generators::ModChunkGenerators,
        overrides::ModDimension,
    };

    const STONE: BlockStateId = BlockStateId(1);
    const BEDROCK: BlockStateId = BlockStateId(2);

    fn registry() -> &'static BlockRegistry {
        Box::leak(Box::new(
            BlockRegistry::from_facts_json(
                &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",true,false,"","minecraft:stone"],["minecraft:bedrock",true,false,"","minecraft:bedrock"]],"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:bedrock":2}}"#[..],
            )
            .unwrap(),
        ))
    }

    /// Stone over the chunk, `seed` blocks above the floor, and bedrock in
    /// the top block of one column; writes past the chunk are skipped.
    struct Slab;
    impl ModChunkGenerator for Slab {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let (pos, floor) = (chunk.pos(), chunk.min_y() + chunk.seed() as i32);
            let top = chunk.min_y() + chunk.height() - 1;
            let (stone, bedrock) = (chunk.state("stone"), chunk.state("minecraft:bedrock"));
            for x in pos.x * 16..pos.x * 16 + 16 {
                for z in pos.z * 16..pos.z * 16 + 16 {
                    assert!(chunk.set_block([x, floor, z], stone));
                }
            }
            let corner = [pos.x * 16, top, pos.z * 16];
            assert!(chunk.set_block(corner, bedrock));
            assert!(!chunk.set_block([corner[0], top + 1, corner[2]], stone));
            assert!(!chunk.set_block([corner[0] - 1, floor, corner[2]], stone));
            assert!(!chunk.set_block([corner[0], floor, corner[2] + 16], stone));
        }
    }

    fn generators() -> ModChunkGenerators {
        ModChunkGenerators::from([(
            Identifier::parse("test:slab").unwrap(),
            Arc::new(Slab) as Arc<dyn ModChunkGenerator>,
        )])
    }

    fn stem(generator: &str) -> serde_json::Value {
        json!({
            "type": "test:pocket",
            "generator": {
                "type": generator,
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:lush_caves"},
            },
        })
    }

    /// Desert at the one quart cell `[13, 5 + seed, -7]`, plains everywhere else.
    struct Oasis(Vec<String>);
    impl ModBiomeSource for Oasis {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, quart: [i32; 3], seed: i64) -> Result<u16, String> {
            Ok(u16::from(quart == [13, 5 + seed as i32, -7]))
        }
    }

    fn sources(biomes: &[&str]) -> ModBiomeSources {
        let oasis = Oasis(biomes.iter().map(|b| (*b).to_owned()).collect());
        ModBiomeSources::from([("test:oasis".to_owned(), Arc::new(oasis) as _)])
    }

    /// Every biome these fixtures name.
    const BIOMES: [&str; 8] = [
        "minecraft:plains",
        "minecraft:desert",
        "minecraft:lush_caves",
        "minecraft:the_end",
        "minecraft:end_highlands",
        "minecraft:end_midlands",
        "minecraft:small_end_islands",
        "minecraft:end_barrens",
    ];

    /// The test world's biome ids: plains 0, desert 3, lush caves 7.
    fn biome_id(name: &str) -> Option<u16> {
        match name {
            "minecraft:plains" => Some(0),
            "minecraft:desert" => Some(3),
            "minecraft:lush_caves" => Some(7),
            "minecraft:the_end" => Some(8),
            "minecraft:end_highlands" => Some(9),
            "minecraft:end_midlands" => Some(10),
            "minecraft:small_end_islands" => Some(11),
            "minecraft:end_barrens" => Some(12),
            _ => None,
        }
    }

    fn for_stem(
        level: &str,
        stem: &serde_json::Value,
        generators: &ModChunkGenerators,
        sources: &ModBiomeSources,
        registry: &'static BlockRegistry,
        biome_id: impl Fn(&str) -> Option<u16>,
        shape: LevelShape,
        seed: i64,
    ) -> Option<Result<ModTerrain, String>> {
        let requested = Identifier::parse_namespaced(stem["generator"]["type"].as_str()?)?;
        let generator = generators.get(&requested)?;
        if generator.vanilla_stages().any() {
            return None;
        }
        let source = &stem["generator"]["biome_source"];
        let resolved = if let Some(name) = source["type"]
            .as_str()
            .filter(|name| name.starts_with("test:"))
        {
            // Only these fixture sources use a local registry. Built-in sources go through the
            // production codec; the registered-source codec has its own integration test.
            LevelBiomeSource::resolve(name, sources, seed, |name| {
                biome_id(name).map(|_| name.to_owned())
            })
            .map(ResolvedBiomeSource::Registered)
            .map_err(|e| e.to_string())
        } else {
            let biomes: BTreeSet<_> = [
                "plains",
                "desert",
                "lush_caves",
                "the_end",
                "end_highlands",
                "end_midlands",
                "small_end_islands",
                "end_barrens",
            ]
            .map(|name| format!("minecraft:{name}"))
            .into();
            let catalog =
                BiomeSourceCatalog::new(&biomes, &BTreeMap::new(), &BTreeSet::new()).unwrap();
            catalog
                .bind_custom_source(&source.to_string(), &BTreeMap::new(), seed)
                .map_err(|e| e.to_string())
        };
        // The world's biome table, as far as these fixtures name biomes.
        let biome_ids = BIOMES
            .iter()
            .filter_map(|name| Some(((*name).to_owned(), biome_id(name)?)))
            .collect();
        Some(resolved.and_then(|source| {
            ModTerrain::new(
                level,
                generator.clone(),
                Arc::new(source),
                registry,
                biome_ids,
                shape,
                seed,
            )
        }))
    }

    fn terrain(stem: &serde_json::Value, shape: LevelShape, seed: i64) -> Option<ModTerrain> {
        let sources = sources(&["minecraft:plains", "minecraft:desert"]);
        for_stem(
            "test:pocket",
            stem,
            &generators(),
            &sources,
            registry(),
            biome_id,
            shape,
            seed,
        )
        .map(Result::unwrap)
    }

    fn oasis_stem() -> serde_json::Value {
        json!({"type": "test:pocket", "generator": {"type": "test:slab", "biome_source": {"type": "test:oasis"}}})
    }

    fn chunk(terrain: &ModTerrain, x: i32, z: i32) -> Chunk {
        terrain.finish(terrain.proto(&mut (), x, z).unwrap())
    }

    /// A registered source whose code fails (as a panic caught under its owner reports).
    struct Broken(Vec<String>);
    impl ModBiomeSource for Broken {
        fn possible_biomes(&self) -> &[String] {
            &self.0
        }
        fn noise_biome(&self, _: [i32; 3], _: i64) -> Result<u16, String> {
            Err("mod broken_pack: panicked".into())
        }
    }

    #[test]
    fn a_failing_biome_source_fails_its_column_while_a_healthy_one_publishes() {
        let mut sources = sources(&["minecraft:plains", "minecraft:desert"]);
        sources.insert(
            "test:broken".to_owned(),
            Arc::new(Broken(vec!["minecraft:plains".to_owned()])) as _,
        );
        let build = |source: &str| {
            let stem = json!({"type": "test:pocket", "generator": {"type": "test:slab", "biome_source": {"type": source}}});
            for_stem(
                "test:pocket",
                &stem,
                &generators(),
                &sources,
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                2,
            )
            .unwrap()
            .unwrap()
        };
        let broken = build("test:broken");
        let error = broken.proto(&mut (), 0, 0).expect_err("the column fails");
        assert!(error.contains("panicked"), "{error}");
        let healthy = build("test:oasis");
        assert!(healthy.proto(&mut (), 0, 0).is_ok());
    }

    #[test]
    fn a_generators_blocks_appear_in_its_levels_chunk() {
        let terrain = terrain(&stem("test:slab"), LevelShape::OVERWORLD, 5).unwrap();
        let chunk = chunk(&terrain, 3, -2);
        assert_eq!(chunk.block(0, -59, 0), STONE);
        assert_eq!(chunk.block(15, -59, 15), STONE);
        assert_eq!(chunk.block(7, -60, 7), BlockStateId(0));
        assert_eq!(chunk.block(7, -58, 7), BlockStateId(0));
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
        assert_eq!(chunk.block(1, 319, 0), BlockStateId(0));
        assert_eq!(chunk.sections[0].biomes.get(0), 7);
    }

    /// One stateless generator serves two levels and tells them apart by `level()`.
    #[test]
    fn a_generator_is_told_the_level_it_fills() {
        struct ByLevel;
        impl ModChunkGenerator for ByLevel {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                let y = match chunk.level() {
                    "test:alpha" => -59,
                    "test:beta" => -58,
                    other => panic!("unexpected level {other}"),
                };
                let stone = chunk.state("stone");
                assert!(chunk.set_block([0, y, 0], stone));
            }
        }
        let generators = ModChunkGenerators::from([(
            Identifier::parse("test:by_level").unwrap(),
            Arc::new(ByLevel) as Arc<dyn ModChunkGenerator>,
        )]);
        let open = |level: &str| {
            let stem = stem("test:by_level");
            for_stem(
                level,
                &stem,
                &generators,
                &sources(&[]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                5,
            )
            .unwrap()
            .unwrap()
        };
        let (alpha, beta) = (
            chunk(&open("test:alpha"), 0, 0),
            chunk(&open("test:beta"), 0, 0),
        );
        assert_eq!(alpha.block(0, -59, 0), STONE);
        assert_eq!(alpha.block(0, -58, 0), BlockStateId(0));
        assert_eq!(beta.block(0, -58, 0), STONE);
        assert_eq!(beta.block(0, -59, 0), BlockStateId(0));
    }

    #[test]
    fn a_levels_height_comes_from_its_dimension_type() {
        let pocket = ModDimension {
            id: "test:pocket".into(),
            stem: stem("test:slab").to_string(),
            dimension_type:
                r#"{"min_y":-128,"height":448,"logical_height":448,"has_skylight":true,"has_ceiling":false}"#
                    .into(),
            sea_level: None,
        };
        let (overrides, _) = crate::datapack::worldgen_overrides_over(
            &[pocket],
            &crate::datapack::DataPacks::default(),
            &sunrise_world::conditions::LoadConditions::vanilla(),
        );
        let height = overrides.level_height("test:pocket").unwrap();
        let shape = LevelShape::new(
            height.min_y,
            height.height,
            height.logical_height,
            height.has_skylight,
            height.has_ceiling,
        )
        .unwrap();
        let terrain = terrain(&stem("test:slab"), shape, 0).unwrap();
        let chunk = chunk(&terrain, 0, 0);
        assert_eq!(chunk.sections.len(), 28);
        assert_eq!(chunk.block(4, -128, 4), STONE);
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
    }

    /// The oasis cell of chunk 3 -2 at seed 2 is blocks 52..56, 28..32 and
    /// -28..-24: chunk columns 4..8, and plains around it.
    #[test]
    fn a_registered_sources_biome_is_what_the_chunk_gets_at_its_quart_cell() {
        let terrain = terrain(&oasis_stem(), LevelShape::OVERWORLD, 2).unwrap();
        let chunk = chunk(&terrain, 3, -2);
        for [x, y, z] in [[4, 28, 4], [7, 31, 7], [5, 30, 6]] {
            assert_eq!(chunk.biome(x, y, z), 3, "{x} {y} {z}");
        }
        for [x, y, z] in [
            [3, 28, 4],
            [8, 28, 4],
            [4, 27, 4],
            [4, 32, 4],
            [4, 28, 8],
            [0, -64, 0],
        ] {
            assert_eq!(chunk.biome(x, y, z), 0, "{x} {y} {z}");
        }
    }

    /// A source naming a biome the world lacks, and a source nobody registered
    /// (its mod removed), refuse the level by name.
    #[test]
    fn a_missing_biome_or_source_refuses_the_level() {
        let refused = |stem: serde_json::Value, biomes: &[&str]| {
            for_stem(
                "test:pocket",
                &stem,
                &generators(),
                &sources(biomes),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                0,
            )
            .unwrap()
            .err()
        };
        assert_eq!(
            refused(oasis_stem(), &["minecraft:plains", "test:mirage"]).as_deref(),
            Some("biome source test:oasis names the unknown biome test:mirage")
        );
        let mut gone = oasis_stem();
        gone["generator"]["biome_source"]["type"] = json!("test:gone");
        assert_eq!(
            refused(gone, &["minecraft:plains"]).as_deref(),
            Some("unknown biome source test:gone")
        );
    }

    /// A chunk saved and read back keeps the biomes the source placed: the
    /// region file stores them by name, through the world's biome table.
    #[test]
    #[ignore = "daily"]
    fn a_registered_sources_biomes_survive_saving_and_loading() {
        let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let assets = crate::assets::Assets::load(&root).unwrap();
        let biomes = crate::biome::Biomes::load(
            &sunrise_assets::resource_source::ResourceRoot::directory(root),
        )
        .unwrap();
        let sources = sources(&["minecraft:plains", "minecraft:desert"]);
        let known = |name: &str| biomes.contains(name).then(|| biomes.id(name));
        let shape = LevelShape::OVERWORLD;
        let terrain = for_stem(
            "test:pocket",
            &oasis_stem(),
            &generators(),
            &sources,
            assets.registry,
            known,
            shape,
            2,
        )
        .unwrap()
        .unwrap();
        let chunk = chunk(&terrain, 3, -2);
        let tables = crate::save::SaveTables::new(&assets);
        let saved = crate::save::encode_with(
            &chunk,
            assets.registry,
            &biomes,
            Some(&tables),
            shape,
            &Default::default(),
        );
        let back = crate::save::decode_with(
            &saved,
            assets.registry,
            &biomes,
            Some(&tables),
            shape.column(),
        )
        .unwrap();
        let name = |x, y, z| biomes.name(back.biome(x, y, z)).to_owned();
        assert_eq!(name(4, 28, 4), "minecraft:desert");
        assert_eq!(name(7, 31, 7), "minecraft:desert");
        assert_eq!(name(8, 28, 4), "minecraft:plains");
        assert_eq!(name(4, 32, 4), "minecraft:plains");
        assert_eq!(name(0, -64, 0), "minecraft:plains");
    }

    /// A generator whose stages are all off makes the chunk it makes today,
    /// its own blocks and nothing else; one that asks for any vanilla stage is
    /// left to the vanilla pipeline.
    #[test]
    fn only_a_generator_without_vanilla_stages_makes_the_mods_terrain_alone() {
        use sunrise_world::worldgen::mod_generators::VanillaStages;
        struct Staged(VanillaStages);
        impl ModChunkGenerator for Staged {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                Slab.build_terrain(chunk);
            }
            fn vanilla_stages(&self) -> VanillaStages {
                self.0
            }
        }
        let for_stem = |stages| {
            let generators = ModChunkGenerators::from([(
                Identifier::parse("test:slab").unwrap(),
                Arc::new(Staged(stages)) as Arc<dyn ModChunkGenerator>,
            )]);
            let stem = stem("test:slab");
            for_stem(
                "test:pocket",
                &stem,
                &generators,
                &sources(&["minecraft:plains"]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                5,
            )
        };
        let off = for_stem(VanillaStages::default()).unwrap().unwrap();
        let chunk = chunk(&off, 3, -2);
        assert_eq!(chunk.block(9, -60, 4), BlockStateId(0));
        assert_eq!(chunk.block(9, -59, 4), STONE);
        assert_eq!(chunk.block(9, -58, 4), BlockStateId(0));
        assert_eq!(chunk.block(9, 63, 4), BlockStateId(0));
        assert_eq!(chunk.block(0, 319, 0), BEDROCK);
        for stages in [
            VanillaStages {
                surface: true,
                ..Default::default()
            },
            VanillaStages {
                carvers: true,
                ..Default::default()
            },
            VanillaStages {
                structures: true,
                ..Default::default()
            },
            VanillaStages {
                features: true,
                ..Default::default()
            },
        ] {
            assert!(for_stem(stages).is_none(), "{stages:?}");
        }
    }

    /// Both stay on the game's own generator: noise as it always was, and a
    /// generator nobody registered to its error.
    #[test]
    fn only_a_registered_generator_makes_mod_terrain() {
        for generator in ["minecraft:noise", "test:gone"] {
            assert!(terrain(&stem(generator), LevelShape::OVERWORLD, 0).is_none());
        }
    }

    /// A custom chunk generator paired with a direct `minecraft:checkerboard` biome source fills
    /// each quart cell from the checkerboard squares directly in `ModTerrain`.
    #[test]
    fn mod_generator_with_direct_checkerboard_fills_biomes() {
        let stem = json!({
            "type": "test:pocket",
            "generator": {
                "type": "test:slab",
                "biome_source": {
                    "type": "minecraft:checkerboard",
                    "biomes": ["plains", "desert"],
                    "scale": 0,
                },
            },
        });
        let terrain = terrain(&stem, LevelShape::OVERWORLD, 5).unwrap();
        let c0 = chunk(&terrain, 0, 0);
        // scale 0 => square width 2^(0 + 2) = 4 quart cells = 1 chunk (16 blocks).
        assert_eq!(c0.biome(0, 0, 0), 0);
        assert_eq!(c0.biome(15, 64, 15), 0);
        let c1 = chunk(&terrain, 1, 0);
        assert_eq!(c1.biome(0, 0, 0), 3);
        let c2 = chunk(&terrain, 1, 1);
        assert_eq!(c2.biome(0, 0, 0), 0);
    }

    /// Refuse an absent or malformed holder instead of substituting plains.
    #[test]
    fn mod_terrain_refuses_invalid_checkerboard_or_fixed_biomes() {
        for bad in [
            json!({"generator": {"type": "test:slab"}}),
            json!({"generator": {"type": "test:slab", "biome_source": {"type": "fixed"}}}),
            json!({"generator": {"type": "test:slab", "biome_source": {"type": "fixed", "biome": 0}}}),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {"type": "minecraft:fixed", "biome": "test:missing"},
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {"type": "minecraft:checkerboard", "biomes": [], "scale": 2},
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {
                        "type": "minecraft:checkerboard",
                        "biomes": ["minecraft:plains"],
                        "scale": 63,
                    },
                },
            }),
            json!({
                "type": "test:pocket",
                "generator": {
                    "type": "test:slab",
                    "biome_source": {
                        "type": "minecraft:checkerboard",
                        "biomes": ["test:missing"],
                        "scale": 2,
                    },
                },
            }),
        ] {
            let result = for_stem(
                "test:pocket",
                &bad,
                &generators(),
                &sources(&["minecraft:plains"]),
                registry(),
                biome_id,
                LevelShape::OVERWORLD,
                0,
            );
            assert!(matches!(result, Some(Err(_))), "{bad}");
        }
    }

    #[test]
    fn non_noise_generation_samples_zero_climate_and_erosion() {
        let point = |biome, temperature| {
            json!({
                "biome": biome,
                "parameters": {"temperature": temperature, "humidity": 0.0,
                    "continentalness": 0.0, "erosion": 0.0, "depth": 0.0,
                    "weirdness": 0.0, "offset": 0.0},
            })
        };
        let mut stem = stem("test:slab");
        // No noise settings in either stem. At zero climate, only plains is an exact match.
        stem["generator"]["biome_source"] = json!({"type": "multi_noise", "biomes": [
            point("desert", 1.0), point("plains", 0.0),
        ]});
        let points = terrain(&stem, LevelShape::OVERWORLD, 3).unwrap();
        for (cx, cz) in [(0, 0), (78, -98)] {
            let chunk = chunk(&points, cx, cz);
            assert_eq!(chunk.biome(0, -64, 0), 0);
            assert_eq!(chunk.biome(15, 319, 15), 0);
        }
        stem["generator"]["biome_source"] = json!({"type": "the_end"});
        let end = terrain(&stem, LevelShape::OVERWORLD, 3).unwrap();
        assert_eq!(chunk(&end, 64, 0).biome(0, 0, 0), 8); // central End
        assert_eq!(chunk(&end, 65, 0).biome(0, 0, 0), 10); // midlands at erosion zero
    }

    /// A chest with a loot table at `[3, 70, 5]` of its chunk (its data set twice, the second
    /// replacing the first), a second chest at `[6, 70, 5]` whose data has no `id`, data for a
    /// block with no block entity (air) beside the first, data whose `x` lies in the next chunk,
    /// and the generator's own biomes: desert where the quart x is even, plains elsewhere. It
    /// records the quarts its resolver is asked for.
    struct Dungeon(std::sync::Mutex<Vec<[i32; 3]>>);
    impl ModChunkGenerator for Dungeon {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let pos = chunk.pos();
            let (x, z) = (pos.x * 16, pos.z * 16);
            let chest = chunk.state("minecraft:chest");
            assert!(chunk.set_block([x + 3, 70, z + 5], chest));
            assert!(chunk.set_block([x + 6, 70, z + 5], chest));
            use sunrise_core::nbt::Tag;
            let data = |dx: i32, id: bool, seed: i64| {
                let mut nbt = sunrise_core::nbt::Compound::new();
                if id {
                    nbt.insert("id".into(), Tag::String("minecraft:chest".into()));
                }
                for (key, value) in [("x", x + dx), ("y", 70), ("z", z + 5)] {
                    nbt.insert(key.into(), Tag::Int(value));
                }
                nbt.insert(
                    "LootTable".into(),
                    Tag::String("minecraft:chests/simple_dungeon".into()),
                );
                nbt.insert("LootTableSeed".into(), Tag::Long(seed));
                nbt
            };
            chunk.set_block_entity_nbt(data(3, true, 7));
            chunk.set_block_entity_nbt(data(3, true, 42));
            chunk.set_block_entity_nbt(data(6, false, 42));
            chunk.set_block_entity_nbt(data(4, true, 42));
            // In the next chunk east: vanilla moves it to x 1 of this one, which is air too.
            chunk.set_block_entity_nbt(data(17, true, 42));
        }
        fn create_biomes(&self, chunk: &mut dyn ChunkBiomes) {
            chunk.fill_biomes_from_noise(&mut |quart| {
                self.0.lock().unwrap().push(quart);
                let name = if quart[0] % 2 == 0 {
                    "minecraft:desert"
                } else {
                    "minecraft:plains"
                };
                sunrise_core::identifier::Identifier::parse(name).unwrap()
            });
        }
    }

    /// Air, stone, bedrock and a chest.
    fn registry_with_chest() -> &'static BlockRegistry {
        Box::leak(Box::new(
            BlockRegistry::from_facts_json(
                &br#"{"states":[["minecraft:air",true,false,"","minecraft:air"],["minecraft:stone",true,false,"","minecraft:stone"],["minecraft:bedrock",true,false,"","minecraft:bedrock"],["minecraft:chest",false,false,"","minecraft:chest"]],"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:bedrock":2,"minecraft:chest":3}}"#[..],
            )
            .unwrap(),
        ))
    }

    fn dungeon(generator: Arc<dyn ModChunkGenerator>) -> ModTerrain {
        let generators =
            ModChunkGenerators::from([(Identifier::parse("test:dungeon").unwrap(), generator)]);
        for_stem(
            "test:pocket",
            &stem("test:dungeon"),
            &generators,
            &sources(&[]),
            registry_with_chest(),
            biome_id,
            LevelShape::OVERWORLD,
            0,
        )
        .unwrap()
        .unwrap()
    }

    /// `setBlockEntityNbt` during generation: the chest's last data becomes its block entity
    /// before the chunk loads, its loot table waiting with its seed as a structure chest's does.
    /// Data with no `id` (`BlockEntity.loadStatic`'s invalid type) and data for blocks with no
    /// block entity are dropped (`LevelChunk.promotePendingBlockEntity`).
    #[test]
    fn a_generators_block_entity_data_becomes_the_chests_when_the_chunk_is_full() {
        let terrain = dungeon(Arc::new(Dungeon(Default::default())));
        let chunk = chunk(&terrain, 2, -3);
        let positions: Vec<_> = chunk.block_entities.iter().map(|(pos, _)| *pos).collect();
        assert_eq!(positions, [(35, 70, -43)]);
        let extras = chunk
            .saved
            .as_deref()
            .expect("the chest's loot table waits");
        assert!(extras.block_entities.is_empty());
        let loot = &extras.pending_loot[&(35, 70, -43)];
        assert_eq!(
            (loot.table_id().as_str(), loot.seed_value()),
            ("minecraft:chests/simple_dungeon", Some(42))
        );
    }

    /// `createBiomes` answered: every cell of the chunk takes the generator's biome over the
    /// level's fixed lush caves, asked once per cell in vanilla's order, sections bottom up and
    /// x, then y, then z within each.
    #[test]
    fn a_generators_resolver_gives_every_cell_its_biome() {
        let generator = Arc::new(Dungeon(Default::default()));
        let terrain = dungeon(generator.clone());
        let chunk = chunk(&terrain, 2, -3);
        // Quart x 8 is even (desert, 3), 9 odd (plains, 0); the source alone gives lush caves.
        for [x, y, z, biome] in [
            [0, -64, 0, 3],
            [4, -64, 0, 0],
            [8, 100, 12, 3],
            [12, 319, 15, 0],
        ] {
            assert_eq!(chunk.biome(x, y, z), biome as u16, "{x} {y} {z}");
        }
        let asked = generator.0.lock().unwrap();
        assert_eq!(asked.len(), 4 * 4 * 4 * 24);
        assert_eq!(
            asked[..5],
            [
                [8, -16, -12],
                [8, -16, -11],
                [8, -16, -10],
                [8, -16, -9],
                [8, -15, -12]
            ]
        );
        assert_eq!(asked[64], [8, -12, -12]);
    }

    /// A resolver naming a biome the world does not have fails the column, naming it.
    #[test]
    fn a_resolver_naming_an_unknown_biome_fails_the_column() {
        struct Unknown;
        impl ModChunkGenerator for Unknown {
            fn build_terrain(&self, _: &mut dyn ChunkAccess) {}
            fn create_biomes(&self, chunk: &mut dyn ChunkBiomes) {
                chunk.fill_biomes_from_noise(&mut |_| {
                    sunrise_core::identifier::Identifier::parse("test:nowhere").unwrap()
                });
            }
        }
        let terrain = dungeon(Arc::new(Unknown));
        let error = terrain.proto(&mut (), 0, 0).err().unwrap();
        assert_eq!(error, "test:pocket: unknown biome test:nowhere");
    }
}
