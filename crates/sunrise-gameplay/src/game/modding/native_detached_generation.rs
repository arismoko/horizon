//! Detached off-world chunk generation (`DetachedChunkGenerator`, #5236).
//!
//! Hands mods a `Send + Sync + 'static` handle backed by the level's `VanillaTerrain` generator
//! and a bounded `GeneratorWorker` pool (`Mutex<Vec<GeneratorWorker>>`). Executes purely in
//! memory on the caller's thread without mutating `World`, `Streaming`, tickets, or `SaveDir`. On
//! level close or `DetachedGenerators` drop, the open flag flips to `false` and every pooled
//! worker is immediately dropped.
use crate::game::ServerGame;
use mod_api::__private::DetachedGeneratorBackend;
use mod_api::{
    ChunkSnapshot, DetachedChunkGenerator, DimensionKey, GenerationError, GenerationStage,
    LevelRef, TerrainColumn,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use sunrise_gameplay_common::game::native_chunk_snapshot::EngineChunkSnapshot;
use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
use sunrise_gameplay_common::game::{Dim, api_values};
use sunrise_gameplay_common::world_biomes;
use sunrise_level::assets::Properties;
use sunrise_level::biome::Biomes;
use sunrise_level::chunk::LevelShape;
use sunrise_level::chunk_streamer::chunk_pos_is_valid;
use sunrise_level::light::LightTables;
use sunrise_sim::vanillagen::VanillaTerrain;
use sunrise_world::worldgen::generator::GeneratorWorker;

/// Matches vanilla 26.3 `net.minecraft.util.Util.maxAllowedExecutorThreads()` (`Util.java:235-237`):
/// `(Runtime.getRuntime().availableProcessors() - 1)` clamped to `1..=255`.
fn max_pooled_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).clamp(1, 255))
        .unwrap_or(1)
}

fn dim_of(key: DimensionKey) -> Dim {
    match key {
        DimensionKey::OVERWORLD => Dim::Overworld,
        DimensionKey::NETHER => Dim::Nether,
        DimensionKey::END => Dim::End,
        other => Dim::Extra(other),
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "generator panicked".to_owned()
    }
}

/// Open detached chunk generators handed out by `LevelRef::detached_generator`, keyed by level.
/// Dropping `ServerGame` closes every remaining entry and releases its pooled `GeneratorWorker`s.
#[derive(Default)]
pub(in crate::game) struct DetachedGenerators(
    std::cell::RefCell<std::collections::HashMap<Dim, Arc<DetachedGeneratorState>>>,
);

impl DetachedGenerators {
    fn get_or_insert(&self, dim: Dim) -> Arc<DetachedGeneratorState> {
        self.0
            .borrow_mut()
            .entry(dim)
            .or_insert_with(|| Arc::new(DetachedGeneratorState::new()))
            .clone()
    }

    fn close_dim(&self, dim: Dim) {
        if let Some(state) = self.0.borrow_mut().remove(&dim) {
            state.close();
        }
    }

    fn close_all(&self) {
        for (_, state) in self.0.borrow_mut().drain() {
            state.close();
        }
    }
}

impl Drop for DetachedGenerators {
    fn drop(&mut self) {
        self.close_all();
    }
}

/// Shared lifecycle + bounded idle-worker pool for one dimension's detached generator handles.
pub(in crate::game) struct DetachedGeneratorState {
    open: AtomicBool,
    workers: Mutex<Vec<GeneratorWorker>>,
}

impl DetachedGeneratorState {
    fn new() -> Self {
        Self {
            open: AtomicBool::new(true),
            workers: Mutex::new(Vec::new()),
        }
    }

    fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Flip the open flag and immediately drain all idle workers so closed levels keep zero state.
    pub(in crate::game) fn close(&self) {
        self.open.store(false, Ordering::Release);
        let drained = {
            let mut guard = self.workers.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *guard)
        };
        drop(drained);
    }

    fn checkout(&self, terrain: &VanillaTerrain) -> GeneratorWorker {
        let reused = {
            let mut guard = self.workers.lock().unwrap_or_else(|e| e.into_inner());
            guard.pop()
        };
        reused.unwrap_or_else(|| terrain.new_worker())
    }

    fn checkin(&self, mut worker: GeneratorWorker) {
        if !self.is_open() {
            return;
        }
        worker.release();
        let cap = max_pooled_workers();
        let mut guard = self.workers.lock().unwrap_or_else(|e| e.into_inner());
        if !self.is_open() {
            return;
        }
        if guard.len() < cap {
            guard.push(worker);
        }
    }
}

struct EngineDetachedGenerator {
    terrain: Weak<VanillaTerrain>,
    state: Arc<DetachedGeneratorState>,
    shape: LevelShape,
    states: Arc<api_values::block_state::Index>,
    biomes: Arc<Biomes>,
    properties: Properties,
    light_tables: Arc<LightTables>,
}

impl EngineDetachedGenerator {
    fn active_terrain(&self) -> Result<Arc<VanillaTerrain>, GenerationError> {
        if !self.state.is_open() {
            return Err(GenerationError::Unavailable);
        }
        match self.terrain.upgrade() {
            Some(terrain) => Ok(terrain),
            None => {
                self.state.close();
                Err(GenerationError::Unavailable)
            }
        }
    }
}

impl DetachedGeneratorBackend for EngineDetachedGenerator {
    fn generate_chunk(
        &self,
        pos: [i32; 2],
        stage: GenerationStage,
    ) -> Result<ChunkSnapshot, GenerationError> {
        let terrain = self.active_terrain()?;
        if !chunk_pos_is_valid((pos[0], pos[1])) {
            return Err(GenerationError::InvalidPos);
        }
        let mut worker = self.state.checkout(&terrain);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            terrain.detached_chunk(
                &mut worker,
                pos,
                stage,
                &self.light_tables,
                self.shape.sky_light,
            )
        }));
        match outcome {
            Ok(Ok((chunk, has_light))) => {
                self.state.checkin(worker);
                if !self.state.is_open() {
                    return Err(GenerationError::Unavailable);
                }
                Ok(EngineChunkSnapshot::from_chunk(
                    &chunk,
                    self.shape,
                    self.states.clone(),
                    self.biomes.clone(),
                    self.properties.clone(),
                    has_light,
                ))
            }
            Ok(Err(message)) => {
                self.state.checkin(worker);
                Err(GenerationError::Failed(message))
            }
            Err(payload) => {
                // Do not return a worker that unwound mid-generation to the pool.
                drop(worker);
                Err(GenerationError::Failed(panic_message(payload)))
            }
        }
    }

    fn sample_surface_columns(
        &self,
        columns: &[(i32, i32)],
    ) -> Result<Vec<Option<TerrainColumn>>, GenerationError> {
        let terrain = self.active_terrain()?;
        for &(x, z) in columns {
            if !chunk_pos_is_valid((x >> 4, z >> 4)) {
                return Err(GenerationError::InvalidPos);
            }
        }
        if columns.is_empty() {
            return Ok(Vec::new());
        }
        let mut worker = self.state.checkout(&terrain);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut out = Vec::with_capacity(columns.len());
            for &(x, z) in columns {
                let sample = match terrain.detached_surface(&mut worker, x, z) {
                    None => None,
                    Some(surface) => {
                        let state = self.states.to_api(surface.state).map_err(|error| {
                            format!("detached surface block conversion: {error:?}")
                        })?;
                        Some(
                            mod_api::__private::built::TerrainColumn {
                                height: surface.height,
                                block: state.block(),
                                water: surface.water.map(|(height, _)| height),
                                biome: self.biomes.name(surface.biome).to_owned(),
                            }
                            .into(),
                        )
                    }
                };
                out.push(sample);
            }
            Ok::<_, String>(out)
        }));
        match outcome {
            Ok(Ok(result)) => {
                self.state.checkin(worker);
                if !self.state.is_open() {
                    return Err(GenerationError::Unavailable);
                }
                Ok(result)
            }
            Ok(Err(message)) => {
                self.state.checkin(worker);
                Err(GenerationError::Failed(message))
            }
            Err(payload) => {
                drop(worker);
                Err(GenerationError::Failed(panic_message(payload)))
            }
        }
    }
}

impl ServerGame<'_> {
    /// Server-side backend for `LevelRef::detached_generator`.
    pub(in crate::game) fn level_detached_generator(
        &self,
        level: LevelRef,
    ) -> Option<DetachedChunkGenerator> {
        if !self.common.is_logical_server() {
            return None;
        }
        let world = self.native_level_world(level)?;
        let dim = dim_of(level.dimension());
        let terrain = if dim == self.common.dim {
            self.common.end.locator()
        } else {
            self.server.levels.0.parked_locator_of(level.dimension_id())
        }?;
        let states = api_values::block_state::current().ok()?;
        let biomes = world_biomes!(self).clone();
        let properties = self.common.pack.assets.properties.clone();
        let light_tables = self.common.pack.chunk_context.light.clone();
        let shape = world.shape;
        let state = self.server.detached_generators.get_or_insert(dim);
        Some(DetachedChunkGenerator::from_backend(Arc::new(
            EngineDetachedGenerator {
                terrain: Arc::downgrade(&terrain),
                state,
                shape,
                states,
                biomes,
                properties,
                light_tables,
            },
        )))
    }

    /// Close all `DetachedChunkGenerator` handles for `dim` and drop their pooled workers.
    pub(in crate::game) fn close_detached_generators_for_dim(&mut self, dim: Dim) {
        self.server.detached_generators.close_dim(dim);
    }

    /// Close all `DetachedChunkGenerator` handles across every dimension and drop pooled workers.
    pub(in crate::game) fn close_all_detached_generators(&mut self) {
        self.server.detached_generators.close_all();
    }
}

#[cfg(test)]
mod tests {
    use crate::game::ServerGame;
    use crate::game::native_entity_refs::tests::with_game;
    use mod_api::{GenerationError, GenerationStage};
    use std::sync::Arc;
    use sunrise_core::position::ChunkPos;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::{Dim, api_values};
    use sunrise_gameplay_common::world_biomes;
    use sunrise_sim::vanillagen::{TerrainResources, VanillaTerrain};
    use sunrise_world::worldgen::generator::FlatSettings;

    fn make_flat_terrain(game: &ServerGame<'_>) -> Arc<VanillaTerrain> {
        let directory = game.common.options.resources.join("worldgen");
        Arc::new(
            VanillaTerrain::for_flat(
                TerrainResources {
                    data: Some(&game.common.pack.data_changes),
                    dir: &directory,
                    registry: game.common.pack.assets.registry,
                    biomes: world_biomes!(game),
                    item_data: &game.common.res.core.item_data,
                },
                &Default::default(),
                12345,
                &FlatSettings {
                    layers: vec![
                        ("minecraft:bedrock".into(), 1),
                        ("minecraft:dirt".into(), 2),
                        ("minecraft:grass_block".into(), 1),
                    ],
                    biome: "minecraft:plains".into(),
                    structure_overrides: Some(Vec::new()),
                    features: false,
                    lakes: false,
                },
                false,
            )
            .expect("flat terrain"),
        )
    }

    #[test]
    #[ignore = "daily"]
    fn detached_generator_stages_lighting_surface_columns_and_close_cleanup() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            // No generator installed yet -> returns None.
            game.common.end.set_locator(None);
            assert!(level.detached_generator(game).is_none());

            let terrain = make_flat_terrain(game);
            game.common.end.set_locator(Some(terrain.clone()));
            let generator = level
                .detached_generator(game)
                .expect("detached generator available");

            let before = (
                game.common.world.chunks.len(),
                game.server.streaming.pending(),
            );
            let bedrock_key = mod_api::__private::block_key("minecraft:bedrock").unwrap();
            let dirt_key = mod_api::__private::block_key("minecraft:dirt").unwrap();
            let grass_key = mod_api::__private::block_key("minecraft:grass_block").unwrap();

            // Run stages off-thread across worker threads.
            let worker_gen = generator.clone();
            let handle = std::thread::spawn(move || {
                // 1. Biomes stage: biomes populated, blocks empty (all air), unlit.
                let biomes_snap = worker_gen
                    .generate_chunk([4, -3], GenerationStage::Biomes)
                    .expect("biomes stage");
                assert!(!biomes_snap.has_light());
                let plains = mod_api::BiomeKey::parse("minecraft:plains").unwrap();
                assert_eq!(biomes_snap.biome(0, 0, 0), Some(&plains));
                assert!(biomes_snap.section(0).unwrap().is_empty());

                // 2. Noise/Surface/Carvers/Features stages: layers present, unlit (no fake light).
                for stage in [
                    GenerationStage::Noise,
                    GenerationStage::Surface,
                    GenerationStage::Carvers,
                    GenerationStage::Features,
                ] {
                    let snap = worker_gen.generate_chunk([4, -3], stage).expect("stage ok");
                    assert!(!snap.has_light(), "{stage:?} must not fabricate light");
                    assert_eq!(snap.block_state(0, -64, 0).unwrap().block(), bedrock_key);
                    assert_eq!(snap.block_state(0, -63, 0).unwrap().block(), dirt_key);
                    assert_eq!(snap.block_state(0, -61, 0).unwrap().block(), grass_key);
                    assert_eq!(snap.biome(0, 0, 0), Some(&plains));
                }

                // 3. Light stage: runs Sunrise's real light engine -> has_light() == true and
                // open sky above grass_block has full skylight 15.
                let lit_snap = worker_gen
                    .generate_chunk([4, -3], GenerationStage::Light)
                    .expect("light stage");
                assert!(lit_snap.has_light());
                assert_eq!(lit_snap.light(0, -60, 0), Some((15, 0)));
                assert_eq!(lit_snap.light(0, -64, 0), Some((0, 0)));

                // 4. sample_surface_columns off-thread.
                let columns = [(100000, -100000), (0, 0), (-1, 31)];
                let surfaces = worker_gen
                    .sample_surface_columns(&columns)
                    .expect("surface sample ok");
                assert_eq!(surfaces.len(), columns.len());
                for col in surfaces {
                    let col = col.unwrap();
                    assert_eq!(col.height, -60);
                    assert_eq!(col.block, grass_key);
                    assert_eq!(col.water, None);
                    assert_eq!(col.biome, "minecraft:plains");
                }
                assert!(worker_gen.sample_surface_columns(&[]).unwrap().is_empty());

                // 5. Out-of-bounds coordinates return InvalidPos without panicking.
                assert_eq!(
                    worker_gen
                        .generate_chunk([i32::MAX, 0], GenerationStage::Carvers)
                        .err(),
                    Some(GenerationError::InvalidPos)
                );
                assert_eq!(
                    worker_gen.sample_surface_columns(&[(i32::MAX, 0)]),
                    Err(GenerationError::InvalidPos)
                );
            });
            handle.join().expect("detached worker succeeded");

            // Purely in-memory: live World and Streaming were untouched.
            assert_eq!(
                before,
                (
                    game.common.world.chunks.len(),
                    game.server.streaming.pending()
                )
            );

            // Closing the level immediately invalidates handles and drops all pooled workers.
            let dim = game.common.dim;
            game.close_detached_generators_for_dim(dim);
            assert_eq!(
                generator
                    .generate_chunk([4, -3], GenerationStage::Biomes)
                    .err(),
                Some(GenerationError::Unavailable)
            );
            assert_eq!(
                generator.sample_surface_columns(&[(0, 0)]),
                Err(GenerationError::Unavailable)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn detached_carvers_matches_live_worldgen_bit_for_bit() {
        with_game(|game| {
            let directory = game.common.options.resources.join("worldgen");
            let terrain = Arc::new(
                VanillaTerrain::for_world(
                    TerrainResources {
                        data: Some(&game.common.pack.data_changes),
                        dir: &directory,
                        registry: game.common.pack.assets.registry,
                        biomes: world_biomes!(game),
                        item_data: &game.common.res.core.item_data,
                    },
                    424242,
                    "minecraft:normal",
                    "minecraft:overworld",
                    false,
                )
                .expect("overworld terrain"),
            );
            game.common.end.set_locator(Some(terrain.clone()));
            let level = game.level("minecraft:overworld").unwrap();
            let generator = level.detached_generator(game).unwrap();

            // Compare detached Carvers snapshot against live GeneratorWorker::generate_proto
            // on the exact same seed & chunk coordinate.
            let pos = [3, -5];
            let detached = generator
                .generate_chunk(pos, GenerationStage::Carvers)
                .expect("detached carvers");
            assert!(!detached.has_light());

            let mut live_worker = terrain.new_worker();
            let live_proto = live_worker
                .generate_proto(ChunkPos::new(pos[0], pos[1]))
                .expect("live proto");
            let states = api_values::block_state::current().unwrap();
            let (bx, bz) = (pos[0] * 16, pos[1] * 16);
            for y in detached.min_y()..(detached.min_y() + detached.height()) {
                for z in 0..16 {
                    for x in 0..16 {
                        let expected = states.to_api(live_proto.block(bx + x, y, bz + z)).unwrap();
                        let actual = detached.block_state(x as u8, y, z as u8).unwrap();
                        assert_eq!(actual, expected, "block mismatch at local ({x}, {y}, {z})");
                    }
                }
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn detached_generator_covers_overworld_noise_surface_features_light_nether_and_parked_level() {
        with_game(|game| {
            let directory = game.common.options.resources.join("worldgen");
            let overworld_terrain = Arc::new(
                VanillaTerrain::for_world(
                    TerrainResources {
                        data: Some(&game.common.pack.data_changes),
                        dir: &directory,
                        registry: game.common.pack.assets.registry,
                        biomes: world_biomes!(game),
                        item_data: &game.common.res.core.item_data,
                    },
                    424242,
                    "minecraft:normal",
                    "minecraft:overworld",
                    false,
                )
                .expect("overworld terrain"),
            );
            game.common.end.set_locator(Some(overworld_terrain.clone()));
            let overworld = game.level("minecraft:overworld").unwrap();
            let generator = overworld
                .detached_generator(game)
                .expect("overworld detached generator");

            let pos = [3, -5];
            let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
            let deepslate_key = mod_api::__private::block_key("minecraft:deepslate").unwrap();
            let bedrock_key = mod_api::__private::block_key("minecraft:bedrock").unwrap();
            let air_key = mod_api::__private::block_key("minecraft:air").unwrap();

            // 1. Non-superflat Overworld Biomes: all air, biomes present, unlit.
            let biomes_snap = generator
                .generate_chunk(pos, GenerationStage::Biomes)
                .expect("overworld biomes");
            assert!(!biomes_snap.has_light());
            assert_eq!(biomes_snap.min_y(), -64);
            assert_eq!(biomes_snap.section_count(), 24);
            assert!(biomes_snap.biome(0, 64, 0).is_some());
            assert_eq!(biomes_snap.block_state(0, -64, 0).unwrap().block(), air_key);

            // 2. Non-superflat Overworld Noise: stone/water/air generated, NO bedrock/surface yet
            // (bedrock is placed by surface rules, not noise).
            let noise_snap = generator
                .generate_chunk(pos, GenerationStage::Noise)
                .expect("overworld noise");
            assert!(!noise_snap.has_light());
            let bottom_noise = noise_snap.block_state(0, -64, 0).unwrap().block();
            assert_eq!(
                bottom_noise, stone_key,
                "Noise stage stops before surface rules paint bedrock/deepslate"
            );

            // 3. Non-superflat Overworld Surface: applies surface rules (bedrock at y=-64,
            // deepslate below y=0), and restores `worker.surface_rule` so a second Surface call
            // on the same pooled worker produces bit-identical output.
            let surface_1 = generator
                .generate_chunk(pos, GenerationStage::Surface)
                .expect("overworld surface #1");
            let surface_2 = generator
                .generate_chunk(pos, GenerationStage::Surface)
                .expect("overworld surface #2 (reused worker)");
            assert!(!surface_1.has_light());
            assert_eq!(
                surface_1.block_state(0, -64, 0).unwrap().block(),
                bedrock_key,
                "Surface stage paints bedrock at y=-64"
            );
            assert!(
                (0..16).any(|x| (0..16).any(|z| {
                    surface_1.block_state(x, -32, z).unwrap().block() == deepslate_key
                })),
                "Surface stage paints deepslate underground"
            );
            for y in -64..320 {
                for z in 0..16u8 {
                    for x in 0..16u8 {
                        assert_eq!(
                            surface_1.block_state(x, y, z),
                            surface_2.block_state(x, y, z),
                            "reused worker surface_rule restoration mismatch at ({x}, {y}, {z})"
                        );
                    }
                }
            }

            // 4. Non-superflat Overworld Features and Light stages.
            let features_snap = generator
                .generate_chunk(pos, GenerationStage::Features)
                .expect("overworld features");
            assert!(!features_snap.has_light());
            assert_eq!(
                features_snap.block_state(0, -64, 0).unwrap().block(),
                bedrock_key
            );

            let light_snap = generator
                .generate_chunk(pos, GenerationStage::Light)
                .expect("overworld light");
            assert!(light_snap.has_light());
            assert_eq!(
                light_snap.light(8, 318, 8).map(|(sky, _)| sky),
                Some(15),
                "open sky at top of Overworld has skylight 15"
            );
            assert_eq!(
                light_snap.light(8, -60, 8).map(|(sky, _)| sky),
                Some(0),
                "deep underground has skylight 0"
            );

            // 5. Nether parked level (`sky_light == false`, `LevelShape::NETHER` 0..256):
            // open Nether, attach its VanillaTerrain, park it behind Overworld, and obtain its
            // `DetachedChunkGenerator` via `levels.parked_locator_of`.
            let nether_terrain = Arc::new(
                VanillaTerrain::for_world(
                    TerrainResources {
                        data: Some(&game.common.pack.data_changes),
                        dir: &directory,
                        registry: game.common.pack.assets.registry,
                        biomes: world_biomes!(game),
                        item_data: &game.common.res.core.item_data,
                    },
                    424242,
                    "minecraft:normal",
                    "minecraft:the_nether",
                    false,
                )
                .expect("nether terrain"),
            );
            let back = game.common.dim;
            game.enter_level(Dim::Nether).unwrap();
            game.common.end.set_locator(Some(nether_terrain));
            game.enter_level(back).unwrap();
            assert_eq!(game.common.dim, Dim::Overworld);

            let nether = game.level("minecraft:the_nether").unwrap();
            let nether_gen = nether
                .detached_generator(game)
                .expect("parked Nether detached generator available");
            let nether_light = nether_gen
                .generate_chunk([1, 1], GenerationStage::Light)
                .expect("nether light stage");
            assert!(nether_light.has_light());
            assert_eq!(nether_light.min_y(), 0);
            assert_eq!(nether_light.height(), 256);
            assert_eq!(nether_light.section_count(), 16);
            assert_eq!(
                nether_light.block_state(0, 0, 0).unwrap().block(),
                bedrock_key,
                "Nether floor bedrock at y=0"
            );
            assert_eq!(
                nether_light.block_state(0, 127, 0).unwrap().block(),
                bedrock_key,
                "Nether roof bedrock at y=127"
            );
            // Nether has no sky light (`sky_light == false`), so even above the roof skylight is 0.
            assert_eq!(nether_light.light(8, 200, 8).map(|(sky, _)| sky), Some(0));

            game.close_detached_generators_for_dim(Dim::Nether);
            game.close_detached_generators_for_dim(Dim::Overworld);
        });
    }
}
