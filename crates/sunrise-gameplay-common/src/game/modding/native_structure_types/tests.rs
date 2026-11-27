use super::*;
use mod_api::Tag;
use mod_api::{FoundStructure, StructureError};
use sunrise_world::worldgen::structure::legacy::Result as PieceResult;

/// A type whose piece is a stone box of a size the config names, from the
/// chunk's corner: what the adapter saves, it loads back.
struct Boxes;
#[derive(Debug)]
struct BoxPiece {
    min: [i32; 3],
    max: [i32; 3],
}
impl StructurePiece for BoxPiece {
    fn bounding_box(&self) -> BoundingBox {
        BoundingBox::new(self.min, self.max)
    }
    fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
        Ok(())
    }
    fn add_additional_save_data(&self, tag: &mut Compound) {
        tag.insert("Min".into(), Tag::IntArray(self.min.to_vec()));
        tag.insert("Max".into(), Tag::IntArray(self.max.to_vec()));
    }
}
impl StructureType for Boxes {
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure> {
        let biome = context.noise_biome([0, 16, 0]);
        assert_eq!((biome.namespace(), biome.path()), ("testmod", "meadow"));
        let chunk = context.chunk();
        let height = context.base_height(
            chunk.x * 16 + 8,
            chunk.z * 16 + 8,
            HeightmapType::WorldSurfaceWg,
        );
        let min = [chunk.x * 16, height - 1, chunk.z * 16];
        Some(FoundStructure {
            position: min,
            pool_pieces: Vec::new(),
            pieces: vec![Arc::new(BoxPiece {
                min,
                max: [min[0] + 8, min[1] + 5, min[2] + 8],
            })],
        })
    }
    fn load_piece(
        &self,
        _: &dyn BlockMetadata,
        tag: &Compound,
    ) -> Result<Arc<dyn StructurePiece>, StructureError> {
        let ints = |key: &str| match tag.get(&key.into()) {
            Some(Tag::IntArray(a)) if a.len() == 3 => Ok([a[0], a[1], a[2]]),
            _ => Err(StructureError::new(std::io::Error::other(format!(
                "invalid saved {key}"
            )))),
        };
        Ok(Arc::new(BoxPiece {
            min: ints("Min")?,
            max: ints("Max")?,
        }))
    }
}

/// The same seed gives the same pieces: the type's answer depends on
/// nothing but its context, so two worlds of one seed see one structure.
#[test]
fn the_same_context_gives_the_same_pieces() {
    let adapter = Adapter::new("testmod:boxes", "testmod", Arc::new(Boxes));
    let run = || {
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(12345, 3, 7);
        // A context of the queries Boxes asks; the engine's own tests
        // cover the context itself.
        let (position, pieces) = adapter
            .find_generation_point(
                &mut StubContext,
                &inputs(test_resources()),
                ChunkPos::new(3, 7),
                &Arc::new(Tag::Compound(Compound::new())),
                &mut random,
            )
            .unwrap();
        (position, pieces[0].bounding_box(), pieces.len())
    };
    assert_eq!(run(), run());
}

/// `createTag` + the piece's own data load back into the same piece: a
/// saved tower comes back the same tower, not a rebuilt one.
#[test]
fn a_saved_piece_loads_back_the_same() {
    let resources = test_resources();
    let adapter = Adapter::new("testmod:boxes", "testmod", Arc::new(Boxes));
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(1, 0, 0);
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            &inputs(resources),
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    let pieces = legacy(pieces);
    let tag = pieces[0].save().unwrap();
    assert!(matches!(
        tag.get(&"id".into()),
        Some(Tag::String(s)) if s.to_string_lossy() == "testmod:boxes"
    ));
    assert_eq!(tag.get(&"Min".into()), Some(&Tag::IntArray(vec![0, 64, 0])));
    let loaded = adapter.load_piece(resources.registry, &tag).unwrap();
    assert_eq!(loaded.bounding_box(), pieces[0].bounding_box());
    assert_eq!(loaded.type_name(), "testmod:boxes");
    // And what came back is the saved one, not a fresh generation.
    let again = loaded.save().unwrap();
    assert_eq!(
        again.get(&"Min".into()).unwrap(),
        tag.get(&"Min".into()).unwrap()
    );
}

/// The engine's once-per-chunk `afterPlace`, which it runs over the first
/// piece, reaches the type's `after_place` — the piece has none — with
/// every piece's box.
#[test]
fn the_engines_after_place_call_reaches_the_type() {
    use sunrise_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    struct Counting {
        calls: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    }
    impl StructureType for Counting {
        fn find_generation_point(
            &self,
            context: &mut dyn GenerationContext,
        ) -> Option<FoundStructure> {
            let chunk = context.chunk();
            let height = context.base_height(
                chunk.x * 16 + 8,
                chunk.z * 16 + 8,
                HeightmapType::WorldSurfaceWg,
            );
            let min = [chunk.x * 16, height - 1, chunk.z * 16];
            Some(FoundStructure {
                position: min,
                pool_pieces: Vec::new(),
                pieces: vec![Arc::new(BoxPiece {
                    min,
                    max: [min[0] + 8, min[1] + 5, min[2] + 8],
                })],
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Err(StructureError::new(std::io::Error::other(
                "fixture has no saved pieces",
            )))
        }
        fn after_place(
            &self,
            _: &mut dyn PieceContext,
            pieces: &[BoundingBox],
        ) -> Result<(), StructureError> {
            self.calls.lock().unwrap().push(pieces.len());
            Ok(())
        }
    }
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let adapter = Adapter::new(
        "testmod:counting",
        "testmod",
        Arc::new(Counting {
            calls: calls.clone(),
        }),
    );
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(2, 1, 1);
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            &inputs(test_resources()),
            ChunkPos::new(1, 1),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    let pieces = legacy(pieces);
    let boxes: Vec<BoundingBox> = pieces.iter().map(|p| p.bounding_box()).collect();
    let resources = test_resources();
    struct NoSolid;
    impl SolidRender for NoSolid {
        fn solid_render(&self, _: sunrise_world::registry::BlockStateId) -> bool {
            false
        }
    }
    let air = resources.registry.default_state("minecraft:air").unwrap();
    let mut world = RecordingWorld {
        air,
        writes: Vec::new(),
        states: Vec::new(),
        placed: Vec::new(),
    };
    let mut piece_random = LegacyRandom::new(3);
    let mut engine = EngineContext {
        world: &mut world,
        resources,
        blocks: &NoSolid,
        random: &mut piece_random,
        chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
        chunk: ChunkPos::new(0, 0),
        reference: [8, 64, 8],
    };
    // What the engine does once per chunk: the first piece's call.
    pieces[0].after_place(&mut engine, &boxes).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![boxes.len()]);
}

/// A mod's own structure type lays its start out from the data pack's pools through
/// `assemble_jigsaw`, at a depth `minecraft:jigsaw`'s codec refuses: one link three blocks tall,
/// a jigsaw at its foot facing down and one at its head facing up, stacks 46 deep above the
/// start, 47 jigsaw pieces in all, each on the one before.
#[test]
fn a_mod_type_assembles_from_pools_past_the_jigsaw_types_depth() {
    use sunrise_world::worldgen::structure::start::Piece;
    struct Deep;
    impl StructureType for Deep {
        fn find_generation_point(
            &self,
            context: &mut dyn GenerationContext,
        ) -> Option<FoundStructure> {
            let chunk = context.chunk();
            let mut settings =
                mod_api::JigsawSettings::new(mod_api::Identifier::parse("test:links").ok()?, 46);
            settings.max_distance_from_center = mod_api::JigsawReach {
                horizontal: 16,
                vertical: 400,
            };
            let layout = context.assemble_jigsaw(&settings, [chunk.x * 16, 0, chunk.z * 16])?;
            assert_eq!(layout.pieces.len(), 47);
            Some(FoundStructure {
                position: layout.position,
                pool_pieces: Vec::new(),
                pieces: Vec::new(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Err(StructureError::new(std::io::Error::other(
                "the type saves no piece of its own",
            )))
        }
    }
    let inputs = link_inputs();
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(0, 0, 0);
    let (_, pieces) = Adapter::new("testmod:deep", "testmod", Arc::new(Deep))
        .find_generation_point(
            &mut StubContext,
            &inputs,
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .expect("a start from the pool");
    assert_eq!(pieces.len(), 47);
    assert!(pieces.iter().all(|piece| matches!(piece, Piece::Pool(_))));
    let bottoms: Vec<i32> = pieces.iter().map(|p| p.bounding_box().min[1]).collect();
    assert_eq!(
        bottoms,
        (0..47).map(|i| bottoms[0] + 3 * i).collect::<Vec<_>>()
    );
}

/// The pools of the jigsaw tests: `test:links` holds one link three blocks
/// tall, a jigsaw at its foot facing down (`test:foot`) and one at its head
/// facing up, which draws from `test:links` and targets `test:foot`.
pub(in crate::game) fn link_inputs()
-> sunrise_world::worldgen::structure::mod_types::StructureInputs<'static> {
    use std::collections::BTreeMap;
    use sunrise_world::worldgen::structure::{
        BlockInfo, StateTransforms, StructureResources, jigsaw::Pools, save_nbt,
    };
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let state = |id: &str, properties: serde_json::Value| serde_json::json!([id, id.ends_with("air"), false, "", {"id": id, "properties": properties}]);
    let facts = serde_json::json!({
        "states": [
            state("minecraft:air", serde_json::json!({})),
            state("minecraft:stone", serde_json::json!({})),
            state("minecraft:jigsaw", serde_json::json!({"orientation": "down_south"})),
            state("minecraft:jigsaw", serde_json::json!({"orientation": "up_north"})),
            state("minecraft:cave_air", serde_json::json!({})),
            state("minecraft:void_air", serde_json::json!({})),
        ],
        "defaults": {"minecraft:air": 0, "minecraft:stone": 1, "minecraft:jigsaw": 2,
            "minecraft:cave_air": 4, "minecraft:void_air": 5}
    });
    let registry: &'static _ = Box::leak(Box::new(
        sunrise_world::registry::BlockRegistry::from_facts_json(facts.to_string().as_bytes())
            .unwrap(),
    ));
    crate::game::api_values::resolve_block_definitions(registry).unwrap();
    let transforms = || {
        StateTransforms::from_json(
            r#"{"rotate":[[0,0,0,0],[1,1,1,1],[2,2,2,2],[3,3,3,3],[4,4,4,4],[5,5,5,5]],"mirror":[[0,0],[1,1],[2,2],[3,3],[4,4],[5,5]],"full_block":[false,true,false,false,false,false]}"#,
            6,
        )
        .unwrap()
    };
    let jigsaw = |state: u32, pool: &str, name: &str, target: &str, y: i32| BlockInfo {
        pos: [0, y, 0],
        state: sunrise_world::registry::BlockStateId(state),
        nbt: Some(Box::new(Compound::from_iter([
            ("id".into(), Tag::String("minecraft:jigsaw".into())),
            ("pool".into(), Tag::String(pool.into())),
            ("name".into(), Tag::String(name.into())),
            ("target".into(), Tag::String(target.into())),
            ("final_state".into(), Tag::String("minecraft:air".into())),
        ]))),
    };
    let link = save_nbt(
        registry,
        &transforms(),
        [1, 3, 1],
        vec![
            jigsaw(2, "minecraft:empty", "test:foot", "minecraft:empty", 0),
            BlockInfo {
                pos: [0, 1, 0],
                state: sunrise_world::registry::BlockStateId(1),
                nbt: None,
            },
            jigsaw(3, "test:links", "minecraft:empty", "test:foot", 2),
        ],
        &[],
        sunrise_world::DATA_VERSION,
    );
    let raw = sunrise_core::nbt::write_named(
        &sunrise_core::nbt::NamedTag {
            name: "".into(),
            tag: link,
        },
        sunrise_core::nbt::Limits::templates(),
    )
    .unwrap();
    let file = sunrise_world::compression::Compression::Gzip
        .encode(&raw)
        .unwrap();
    let resources: &'static _ = Box::leak(Box::new(
        StructureResources::new(
            registry,
            transforms(),
            std::path::Path::new("/nonexistent"),
            BTreeMap::new(),
            &BTreeMap::new(),
        )
        .with_pack_templates([("test:link".to_owned(), file.into())].into()),
    ));
    let pools: &'static _ = Box::leak(Box::new(Pools::new(BTreeMap::from([
        (
            "minecraft:empty".to_owned(),
            serde_json::json!({"fallback": "minecraft:empty", "elements": []}).to_string(),
        ),
        (
            "test:links".to_owned(),
            serde_json::json!({"fallback": "minecraft:empty", "elements": [{"weight": 1,
                "element": {"element_type": "minecraft:single_pool_element",
                    "location": "test:link", "projection": "rigid",
                    "processors": {"processors": []}}}]})
            .to_string(),
        ),
    ]))));
    let bindings: &'static _ = Box::leak(Box::new(
        sunrise_world::worldgen::block_feature::FeatureBlockBindings::new(
            registry,
            Default::default(),
        )
        .unwrap(),
    ));
    sunrise_world::worldgen::structure::mod_types::StructureInputs::new(resources, pools, bindings)
}

/// The start context the stub answers: heights from a flat 64.
pub(in crate::game) struct StubContext;
impl sunrise_world::worldgen::structure::jigsaw::JigsawContext for StubContext {
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        320
    }
    fn base_height(&mut self, _: i32, _: i32, _: sunrise_world::heightmap::HeightmapType) -> i32 {
        65
    }
    fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }
}
impl sunrise_world::worldgen::structure::start::StartContext for StubContext {
    fn seed(&self) -> i64 {
        1
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn base_column(
        &mut self,
        _: i32,
        _: i32,
        _: sunrise_world::heightmap::HeightmapType,
    ) -> Vec<bool> {
        Vec::new()
    }
    fn base_columns_by(
        &mut self,
        _: i32,
        _: i32,
        count: usize,
        kinds: &dyn Fn(usize) -> sunrise_world::heightmap::HeightmapType,
    ) -> Vec<Vec<bool>> {
        use sunrise_world::heightmap::HeightmapType::*;
        let all = [
            WorldSurfaceWg,
            WorldSurface,
            OceanFloorWg,
            OceanFloor,
            MotionBlocking,
            MotionBlockingNoLeaves,
        ];
        (0..count)
            .map(|index| {
                let selected = kinds(index);
                all.iter().map(|kind| *kind == selected).collect()
            })
            .collect()
    }
    fn noise_column_into(
        &mut self,
        _: i32,
        _: i32,
        column: &mut dyn sunrise_world::worldgen::structure::start::NoiseColumnTarget,
    ) {
        use sunrise_world::worldgen::structure::start::NoiseBlock::*;
        column.resize(-2, 5);
        for (index, block) in [Air, Water, Lava, Solid, Other].into_iter().enumerate() {
            column.set(index, block);
        }
    }
    fn cold_enough_to_snow(
        &mut self,
        _: sunrise_world::worldgen::biome::BiomeId,
        _: [i32; 3],
    ) -> bool {
        false
    }
    fn generation(&self) -> sunrise_world::worldgen::anchor::WorldGenerationContext {
        sunrise_world::worldgen::anchor::WorldGenerationContext::new(-64, 384, 63)
    }
    fn noise_biome(&mut self, _: [i32; 3]) -> sunrise_world::worldgen::biome::BiomeId {
        sunrise_world::worldgen::biome::BiomeId(0)
    }
    fn biome_key(&self, biome: sunrise_world::worldgen::biome::BiomeId) -> BiomeKey {
        assert_eq!(biome, sunrise_world::worldgen::biome::BiomeId(0));
        BiomeKey::from_parts("testmod", "meadow").unwrap()
    }
    fn noise_biomes(
        &mut self,
        quarts: &[[i32; 3]],
    ) -> Vec<sunrise_world::worldgen::biome::BiomeId> {
        vec![sunrise_world::worldgen::biome::BiomeId(0); quarts.len()]
    }
    fn set_valid_biomes(
        &mut self,
        _: Option<&std::collections::BTreeSet<sunrise_world::worldgen::biome::BiomeId>>,
    ) {
    }
}

/// The piece world of the clipping test: every write recorded, and a
/// write outside the test's expectations fails it.
pub(in crate::game) struct RecordingWorld {
    pub(in crate::game) air: sunrise_world::registry::BlockStateId,
    pub(in crate::game) writes: Vec<[i32; 3]>,
    pub(in crate::game) states: Vec<sunrise_world::registry::BlockStateId>,
    /// The features and elements placed, with where, in order.
    pub(in crate::game) placed: Vec<String>,
}
impl sunrise_world::worldgen::structure::legacy::PieceWorld for RecordingWorld {
    fn block(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::registry::BlockStateId> {
        Ok(self.air)
    }
    fn set(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
        _: i32,
    ) -> PieceResult<bool> {
        self.writes.push(pos);
        self.states.push(state);
        Ok(true)
    }
    fn fluid(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::fluid::FluidState> {
        Ok(sunrise_world::fluid::FluidState::EMPTY)
    }
    fn height(
        &mut self,
        _: sunrise_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> PieceResult<i32> {
        Ok(64)
    }
    fn min_y(&mut self) -> PieceResult<i32> {
        Ok(-64)
    }
    fn max_y(&mut self) -> PieceResult<i32> {
        Ok(320)
    }
    fn block_entity(&mut self, _: [i32; 3], _: Compound) -> PieceResult<()> {
        Ok(())
    }
    fn entity(&mut self, _: Compound) -> PieceResult<()> {
        Ok(())
    }
    fn post_process(&mut self, _: [i32; 3]) -> PieceResult<()> {
        Ok(())
    }
    fn fluid_tick(
        &mut self,
        _: [i32; 3],
        _: sunrise_world::fluid::FluidKind,
        _: i32,
    ) -> PieceResult<()> {
        Ok(())
    }
    fn seed(&mut self) -> PieceResult<i64> {
        Ok(0)
    }
    fn level_boolean(&mut self) -> PieceResult<bool> {
        Ok(false)
    }
    fn level_float(&mut self) -> PieceResult<f32> {
        Ok(0.0)
    }
    fn level_int(&mut self, _: i32) -> PieceResult<i32> {
        Ok(0)
    }
    fn sea_level(&mut self) -> PieceResult<i32> {
        Ok(63)
    }
    fn face_full(
        &mut self,
        _: [i32; 3],
        _: sunrise_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn face_sturdy(
        &mut self,
        _: [i32; 3],
        _: sunrise_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn center_sturdy(
        &mut self,
        _: [i32; 3],
        _: sunrise_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn biome(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::worldgen::biome::BiomeId> {
        Err(PlacementQueryError::Unavailable)
    }
    fn place_template(
        &mut self,
        _: &sunrise_world::worldgen::structure::StructureResources,
        _: &sunrise_world::worldgen::structure::StructureTemplate,
        _: [i32; 3],
        _: [i32; 3],
        _: &mut sunrise_world::worldgen::structure::processor::PlaceSettings<'_>,
        _: &mut dyn sunrise_world::worldgen::structure::processor::ProcessorRandom,
        _: i32,
    ) -> PieceResult<bool> {
        Ok(true)
    }
    fn place_feature(
        &mut self,
        _: &mut dyn sunrise_world::worldgen::structure::legacy::PieceRandom,
        feature: sunrise_world::worldgen::structure::jigsaw::FeatureKey<'_>,
        pos: [i32; 3],
    ) -> PieceResult<bool> {
        self.placed.push(format!("{feature:?} at {pos:?}"));
        Ok(true)
    }
    fn place_element(
        &mut self,
        _: &mut dyn sunrise_world::worldgen::structure::legacy::PieceRandom,
        element: &sunrise_world::worldgen::structure::jigsaw::PoolElement,
        at: sunrise_world::worldgen::structure::jigsaw::ElementAt,
    ) -> PieceResult<bool> {
        let kind = match element {
            sunrise_world::worldgen::structure::jigsaw::PoolElement::Single {
                template, ..
            } => template.location.to_string(),
            _ => "other".into(),
        };
        self.placed.push(format!("{kind} at {at:?}"));
        Ok(true)
    }
}

/// The engine's own placement tests build their registry this way
/// (`legacy::tests::registry`): two facts rows, held once for the
/// `&'static` the engine's resources take — no game resources needed.
/// What a start builds from over `resources`: no template pools, as these types lay out none.
fn inputs(
    resources: &'static sunrise_world::worldgen::structure::StructureResources,
) -> sunrise_world::worldgen::structure::mod_types::StructureInputs<'static> {
    let pools: &'static _ = Box::leak(Box::new(
        sunrise_world::worldgen::structure::jigsaw::Pools::new(Default::default()),
    ));
    let bindings: &'static _ = Box::leak(Box::new(
        sunrise_world::worldgen::block_feature::FeatureBlockBindings::new(
            resources.registry,
            Default::default(),
        )
        .unwrap(),
    ));
    sunrise_world::worldgen::structure::mod_types::StructureInputs::new(resources, pools, bindings)
}

/// The adapter's pieces, which are all its own legacy pieces.
fn legacy(
    pieces: Vec<sunrise_world::worldgen::structure::start::Piece>,
) -> Vec<Box<dyn sunrise_world::worldgen::structure::legacy::LegacyPiece>> {
    pieces
        .into_iter()
        .map(|piece| match piece {
            sunrise_world::worldgen::structure::start::Piece::Legacy(piece) => piece,
            sunrise_world::worldgen::structure::start::Piece::Pool(_) => {
                unreachable!("the adapter lays out no jigsaw")
            }
        })
        .collect()
}

pub(in crate::game) fn test_resources()
-> &'static sunrise_world::worldgen::structure::StructureResources {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = serde_json::json!({
        "states": [
            ["air", true, false, "", {"id":"minecraft:air","properties":{}}],
            ["stone", false, false, "", {"id":"minecraft:stone","properties":{}}],
            ["cave_air", true, false, "", {"id":"minecraft:cave_air","properties":{}}],
            ["void_air", true, false, "", {"id":"minecraft:void_air","properties":{}}]
        ],
        "defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:cave_air":2,"minecraft:void_air":3}
    });
    let registry =
        sunrise_world::registry::BlockRegistry::from_facts_json(facts.to_string().as_bytes())
            .unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    resources_for_registry(registry)
}

fn resources_for_registry(
    registry: sunrise_world::registry::BlockRegistry,
) -> &'static sunrise_world::worldgen::structure::StructureResources {
    Box::leak(Box::new(
        sunrise_world::worldgen::structure::StructureResources::new(
            Box::leak(Box::new(registry)),
            sunrise_world::worldgen::structure::StateTransforms::default(),
            std::path::Path::new("/nonexistent"),
            std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
        ),
    ))
}

/// `placeBlock`'s clipping: a write outside `chunk_box` answers `false`
/// and touches nothing, silently, as vanilla's `placeBlock` skips it; one
/// inside writes.
#[test]
fn a_write_outside_the_chunk_box_is_dropped_silently() {
    use sunrise_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    let resources = test_resources();
    struct Air;
    impl SolidRender for Air {
        fn solid_render(&self, _: sunrise_world::registry::BlockStateId) -> bool {
            false
        }
    }
    let air = resources.registry.default_state("minecraft:air").unwrap();
    let mut world = RecordingWorld {
        air,
        writes: Vec::new(),
        states: Vec::new(),
        placed: Vec::new(),
    };
    let mut random = sunrise_core::random::LegacyRandom::new(7);
    {
        let mut engine = EngineContext {
            world: &mut world,
            resources,
            blocks: &Air,
            random: &mut random,
            // Chunk (0, 0)'s writable box.
            chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
            chunk: ChunkPos::new(0, 0),
            reference: [8, 64, 8],
        };
        let mut context = Placing::new(&mut engine, "testmod", "testmod:boxes").unwrap();
        let stone = context.state(mod_api::__private::block_key("minecraft:stone").unwrap());
        // Outside on every axis: dropped, silently, nothing recorded.
        for outside in [[16, 64, 0], [0, 64, 16], [-1, 64, 0], [0, 64, -1]] {
            assert!(!context.set_block(outside, stone));
        }
        // Inside: written.
        assert!(context.set_block([15, 64, 15], stone));
    }
    assert_eq!(world.writes, [[15, 64, 15]]);
}

#[test]
fn structure_random_adapter_preserves_vanilla_scalar_draws() {
    assert_eq!(Draws(&mut LegacyRandom::new(0)).next_int_bound(100), 60);
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_long(),
        -4962768465676381896
    );
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_float().to_bits(),
        0.73096776_f32.to_bits()
    );
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_double().to_bits(),
        0.730967787376657_f64.to_bits()
    );
    assert!(Draws(&mut LegacyRandom::new(0)).next_boolean());
    // The unbounded int and the gaussian are the engine stream's own, cache and all.
    let mut engine = LegacyRandom::new(0);
    let expected = (
        sunrise_core::random::RandomSource::next_int(&mut engine),
        sunrise_core::random::RandomSource::next_gaussian(&mut engine),
        sunrise_core::random::RandomSource::next_gaussian(&mut engine),
    );
    let mut random = LegacyRandom::new(0);
    let mut draws = Draws(&mut random);
    let random: &mut dyn mod_api::RandomSource = &mut draws;
    assert_eq!(
        (
            random.next_int(),
            random.next_gaussian(),
            random.next_gaussian()
        ),
        expected
    );
    let mut random = LegacyRandom::new(0);
    let mut draws = Draws(&mut random);
    assert_eq!(draws.next_int_bound(0), 0);
    assert_eq!(draws.next_long(), -4962768465676381896);
}

#[test]
fn structure_column_projection_keeps_classes_and_resets_reused_output() {
    use sunrise_world::worldgen::structure::start::{NoiseBlock as Engine, NoiseColumnTarget};
    let mut column = Column(NoiseColumn::default());
    column.resize(-2, 5);
    for (index, block) in [
        Engine::Air,
        Engine::Water,
        Engine::Lava,
        Engine::Solid,
        Engine::Other,
    ]
    .into_iter()
    .enumerate()
    {
        column.set(index, block);
    }
    assert_eq!(
        column.0.blocks,
        [
            NoiseBlock::Air,
            NoiseBlock::Water,
            NoiseBlock::Lava,
            NoiseBlock::Solid,
            NoiseBlock::Other
        ]
    );
    column.resize(7, 2);
    assert_eq!(column.0.min, 7);
    assert_eq!(column.0.blocks, [NoiseBlock::Air, NoiseBlock::Air]);
}

struct ConfigObserver(std::sync::Arc<std::sync::Mutex<Vec<Arc<Tag>>>>);
impl StructureType for ConfigObserver {
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure> {
        self.0.lock().unwrap().push(context.config());
        None
    }
    fn load_piece(
        &self,
        _: &dyn BlockMetadata,
        _: &Compound,
    ) -> Result<Arc<dyn StructurePiece>, StructureError> {
        Err(StructureError::new(std::io::Error::other(
            "fixture has no saved pieces",
        )))
    }
}

#[test]
fn structure_config_is_converted_at_load_and_shared_by_starts() {
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_world::worldgen::structure::{
        mod_types::ModStructureTypes,
        start::{StructureKind, StructureRegistry},
    };
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let types: ModStructureTypes = BTreeMap::from([(
        "probe:kind".to_owned(),
        Adapter::new(
            "probe:kind",
            "probe",
            Arc::new(ConfigObserver(observed.clone())),
        )
        .into_engine_type(),
    )]);
    let mut definitions = BTreeMap::from([(
        "probe:tower".to_owned(),
        serde_json::json!({
            "type":"probe:kind", "biomes":[], "size":32768, "wide":2147483648_i64,
            "list":[true,128], "nested":{"enabled":true}
        })
        .to_string(),
    )]);
    let mut registry = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &types,
    )
    .unwrap();
    let StructureKind::Mod(kind) = &mut registry.structures[0].kind else {
        panic!("registered kind");
    };
    let Tag::Compound(config) = kind.config.as_ref() else {
        panic!("compound config");
    };
    assert_eq!(config.get(&"size".into()), Some(&Tag::Int(32768)));
    assert_eq!(config.get(&"wide".into()), Some(&Tag::Long(2147483648)));
    assert_eq!(
        config.get(&"list".into()),
        Some(&Tag::List(vec![Tag::Byte(1), Tag::Short(128)]))
    );
    assert_eq!(
        config.get(&"nested".into()),
        Some(&Tag::compound([("enabled", Tag::Byte(1))]))
    );
    // Discard the source text: starts must use the retained prepared value, not a config map.
    definitions.clear();
    let resources = test_resources();
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    for _ in 0..2 {
        assert!(
            kind.ty
                .find_generation_point(
                    &mut StubContext,
                    &inputs(resources),
                    ChunkPos::new(0, 0),
                    &kind.config,
                    &mut random
                )
                .is_none()
        );
    }
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(
        observed
            .iter()
            .all(|config| Arc::ptr_eq(config, &kind.config))
    );
}

#[test]
#[allow(clippy::err_expect)] // TODO(#2273): `expect_err` once the Ok type is Debug
fn structure_bad_config_refuses_at_load_but_unknown_types_do_not_convert() {
    use std::collections::{BTreeMap, BTreeSet};
    use sunrise_world::worldgen::structure::{
        mod_types::ModStructureTypes,
        start::{StructureKind, StructureRegistry},
    };
    let types: ModStructureTypes = BTreeMap::from([(
        "probe:kind".to_owned(),
        Adapter::new(
            "probe:kind",
            "probe",
            Arc::new(ConfigObserver(Default::default())),
        )
        .into_engine_type(),
    )]);
    let definitions = BTreeMap::from([(
        "probe:tower".to_owned(),
        serde_json::json!({"type":"probe:kind","biomes":[],"bad":[null]}).to_string(),
    )]);
    let error = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &types,
    )
    .err()
    .expect("refused during definition load");
    assert_eq!(
        error.to_string(),
        "structure probe:tower: probe:kind: JSON-to-NBT does not accept nested null"
    );
    let registry = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        matches!(&registry.structures[0].kind, StructureKind::Unsupported(id) if id == "probe:kind")
    );
}

#[test]
fn compiled_book_piece_resolves_on_worker_load_and_refuses_bad_saved_values() {
    use sunrise_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
        "mymod",
        Box::new(book_example::structures::StructureMod),
    )])
    .unwrap();
    let registry = sunrise_world::registry::BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:air":0,"minecraft:oak_log":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"y"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"z"}}]]}"#.as_slice()).unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    let resources = resources_for_registry(registry);
    let (id, mod_id, implementation) = mod_api::__private::structure_type_declarations()
        .into_iter()
        .next()
        .unwrap();
    assert_eq!(id, "mymod:tower");
    assert_eq!(mod_id, "mymod");
    std::thread::spawn(move || {
        assert!(crate::game::api_values::block_state::current().is_err());
        let metadata = WorkerResources::get(resources.registry).unwrap().metadata;
        let adapter = Adapter::new(&id, mod_id, implementation.clone());
        let tag = Compound::from([
            ("block".into(), Tag::String("minecraft:oak_log".into())),
            ("axis".into(), Tag::String("x".into())),
            ("min".into(), Tag::IntArray(vec![1, 64, 2])),
            ("max".into(), Tag::IntArray(vec![1, 66, 2])),
        ]);
        for (key, value) in [
            ("block", Tag::String("minecraft:missing".into())),
            ("axis", Tag::String("diagonal".into())),
            ("min", Tag::IntArray(vec![1, 64])),
        ] {
            let mut invalid = tag.clone();
            invalid.insert(key.into(), value);
            assert!(implementation.load_piece(metadata, &invalid).is_err());
            assert!(adapter.load_piece(resources.registry, &invalid).is_none());
        }
        let piece = adapter.load_piece(resources.registry, &tag).unwrap();
        assert_eq!(
            piece.bounding_box(),
            BoundingBox::new([1, 64, 2], [1, 66, 2])
        );
        let saved = piece.save().unwrap();
        for key in ["block", "axis", "min", "max"] {
            assert_eq!(saved.get(&key.into()), tag.get(&key.into()));
        }
        struct Opaque;
        impl SolidRender for Opaque {
            fn solid_render(&self, _: sunrise_world::registry::BlockStateId) -> bool {
                true
            }
        }
        let mut world = RecordingWorld {
            air: sunrise_world::registry::BlockStateId(0),
            writes: vec![],
            states: vec![],
            placed: vec![],
        };
        let mut random = LegacyRandom::new(9);
        piece
            .post_process(&mut EngineContext {
                world: &mut world,
                resources,
                blocks: &Opaque,
                random: &mut random,
                chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
                chunk: ChunkPos::new(0, 0),
                reference: [8, 64, 8],
            })
            .unwrap();
        assert_eq!(world.writes, [[1, 64, 2], [1, 65, 2], [1, 66, 2]]);
        assert_eq!(world.states, [sunrise_world::registry::BlockStateId(2); 3]);
    })
    .join()
    .unwrap();
}

/// Exercise the actual terrain consumer with generated and saved mod pieces.
#[test]
fn piece_beardifier_shapes_terrain_and_is_resolved_once() {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunrise_core::density::{DensitySampler, SamplerContext, TerrainAdjustment as Engine};
    use sunrise_world::worldgen::structure::start::{
        Piece, StructureRegistry, StructureStart, beardifier,
    };

    struct TerrainType(Arc<AtomicUsize>);
    #[derive(Debug)]
    struct TerrainPiece {
        mode: i32,
        calls: Arc<AtomicUsize>,
    }
    impl StructurePiece for TerrainPiece {
        fn bounding_box(&self) -> BoundingBox {
            let x = self.mode * 40;
            BoundingBox::new([x, 64, 0], [x + 8, 73, 8])
        }
        fn beardifier(&self) -> Option<mod_api::PieceBeardifier> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                0 => Some(mod_api::PieceBeardifier::new(
                    BoundingBox::new([0, 67, 0], [8, 73, 8]),
                    mod_api::TerrainAdjustment::BeardThin,
                    2,
                )),
                1 => None,
                _ => Some(mod_api::PieceBeardifier::new(
                    self.bounding_box(),
                    mod_api::TerrainAdjustment::None,
                    0,
                )),
            }
        }
        fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
            Ok(())
        }
        fn add_additional_save_data(&self, tag: &mut Compound) {
            tag.insert("Mode".into(), Tag::Int(self.mode));
        }
    }
    impl StructureType for TerrainType {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            Some(FoundStructure {
                position: [0, 64, 0],
                pool_pieces: Vec::new(),
                pieces: (0..3)
                    .map(|mode| {
                        Arc::new(TerrainPiece {
                            mode,
                            calls: self.0.clone(),
                        }) as Arc<dyn StructurePiece>
                    })
                    .collect(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            tag: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            let Some(Tag::Int(mode)) = tag.get(&"Mode".into()) else {
                return Err(StructureError::new(std::io::Error::other(
                    "missing fixture mode",
                )));
            };
            Ok(Arc::new(TerrainPiece {
                mode: *mode,
                calls: self.0.clone(),
            }))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = Adapter::new(
        "probe:terrain",
        "probe",
        Arc::new(TerrainType(calls.clone())),
    );
    let resources = test_resources();
    let (_, generated) = adapter
        .find_generation_point(
            &mut StubContext,
            &inputs(resources),
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut WorldgenRandom::new(LegacyRandom::new(0)),
        )
        .unwrap();
    let generated = legacy(generated);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let loaded: Vec<_> = generated
        .iter()
        .map(|piece| {
            adapter
                .load_piece(resources.registry, &piece.save().unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    let mut registry = StructureRegistry::new(
        &BTreeMap::from([(
            "probe:terrain".into(),
            serde_json::json!({"type":"probe:terrain", "biomes":[],
                "terrain_adaptation":"none"})
            .to_string(),
        )]),
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &BTreeMap::new(),
    )
    .unwrap();
    for pieces in [generated, loaded] {
        let start = Arc::new(StructureStart::new(
            0,
            &registry.structures[0],
            ChunkPos::new(0, 0),
            pieces.into_iter().map(Piece::Legacy).collect(),
        ));
        assert!(start.bounding_box.is_inside([-12, 69, 0]));
        for default in [Engine::None, Engine::Bury] {
            registry.structures[0].terrain = default;
            for _ in 0..2 {
                let density =
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(0, 0))
                        .unwrap();
                let sample = |position| {
                    density
                        .sample_value(&mut SamplerContext::default(), position)
                        .unwrap()
                };
                // Vanilla Beardifier: dy = y - (67 + 2), kernel exp(-r²/16),
                // -dyOffset / sqrt(r²/2) / 2 * kernel * .8. Allow the
                // vanilla fast inverse-square-root approximation, not a second
                // call to Sunrise's density helpers.
                for y in [64, 68, 69, 73] {
                    let offset = f64::from(y - 69) + 0.5;
                    let expected = -offset / (offset * offset / 2.0).sqrt() / 2.0
                        * (-offset * offset / 16.0).exp()
                        * 0.8;
                    assert!((f64::from(sample([4, y, 4])) - expected).abs() < 0.002);
                }
                assert_eq!(sample([4, 81, 4]), 0.0);
                let second =
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(2, 0));
                let second = second.map_or(0.0, |d| {
                    d.sample_value(&mut SamplerContext::default(), [44, 64, 4])
                        .unwrap()
                });
                assert_eq!(second, if default == Engine::None { 0.0 } else { 1.0 });
                assert!(
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(5, 0))
                        .is_none()
                );
            }
        }
        registry.structures[0].terrain = Engine::None;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        6,
        "terrain chunks must use cached answers"
    );
}

#[test]
fn unordered_beardifier_bounds_add_no_terrain_adjustment() {
    let unordered = mod_api::PieceBeardifier::new(
        BoundingBox::new([1, 0, 0], [0, 0, 0]),
        mod_api::TerrainAdjustment::BeardThin,
        0,
    );
    assert!(engine_beardifier(unordered).is_none());
    let ordered = mod_api::PieceBeardifier::new(
        BoundingBox::new([0, 0, 0], [1, 1, 1]),
        mod_api::TerrainAdjustment::BeardThin,
        2,
    );
    assert_eq!(engine_beardifier(ordered).unwrap().ground_level_delta, 2);
}

#[test]
fn repeated_returned_load_piece_errors_keep_the_loader_enabled() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug)]
    struct InnerCause;
    impl std::fmt::Display for InnerCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("corrupt coordinate array")
        }
    }
    impl std::error::Error for InnerCause {}

    #[derive(Debug)]
    struct OuterCause(InnerCause);
    impl std::fmt::Display for OuterCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("invalid saved box")
        }
    }
    impl std::error::Error for OuterCause {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    struct FailingThenOk(Arc<AtomicUsize>);
    impl StructureType for FailingThenOk {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            None
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            tag: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let ints = |key: &str| match tag.get(&key.into()) {
                Some(Tag::IntArray(a)) if a.len() == 3 => Ok([a[0], a[1], a[2]]),
                _ => Err(StructureError::new(OuterCause(InnerCause))),
            };
            Ok(Arc::new(BoxPiece {
                min: ints("Min")?,
                max: ints("Max")?,
            }))
        }
    }

    let resources = test_resources();
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = Adapter::new(
        "testmod:boxes",
        "testmod",
        Arc::new(FailingThenOk(calls.clone())),
    );
    let bad = Compound::from([("Min".into(), Tag::IntArray(vec![1, 64]))]);
    assert!(adapter.load_piece(resources.registry, &bad).is_none());
    assert!(adapter.load_piece(resources.registry, &bad).is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let good = Compound::from([
        ("Min".into(), Tag::IntArray(vec![1, 64, 2])),
        ("Max".into(), Tag::IntArray(vec![3, 66, 4])),
    ]);
    let loaded = adapter.load_piece(resources.registry, &good).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(
        loaded.bounding_box(),
        BoundingBox::new([1, 64, 2], [3, 66, 4])
    );
    assert_eq!(loaded.type_name(), "testmod:boxes");
}

#[test]
fn start_panic_skips_only_that_callback_without_disabling_independent_type() {
    use mod_api::{
        Hooks, ModEntry, PackageInfo, PackageKind, Registrar, RegistrationContext, Side,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    const OWNER: &str = "test_structures";
    struct StartProbe {
        panic_on_start: bool,
        start_calls: Arc<AtomicUsize>,
        load_calls: Arc<AtomicUsize>,
    }
    impl StructureType for StartProbe {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            self.start_calls.fetch_add(1, Ordering::SeqCst);
            assert!(!self.panic_on_start, "start callback panicked");
            Some(FoundStructure {
                position: [0, 64, 0],
                pieces: vec![Arc::new(BoxPiece {
                    min: [0, 64, 0],
                    max: [2, 66, 2],
                })],
                pool_pieces: Vec::new(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            self.load_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(BoxPiece {
                min: [4, 64, 4],
                max: [6, 66, 6],
            }))
        }
    }

    struct StructureMod {
        broken_starts: Arc<AtomicUsize>,
        broken_loads: Arc<AtomicUsize>,
        healthy_starts: Arc<AtomicUsize>,
        healthy_loads: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for StructureMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            r.register_structure_type(
                "extra:broken",
                StartProbe {
                    panic_on_start: true,
                    start_calls: self.broken_starts.clone(),
                    load_calls: self.broken_loads.clone(),
                },
            );
            r.register_structure_type(
                "healthy",
                StartProbe {
                    panic_on_start: false,
                    start_calls: self.healthy_starts.clone(),
                    load_calls: self.healthy_loads.clone(),
                },
            );
        }
    }

    let broken_starts = Arc::new(AtomicUsize::new(0));
    let broken_loads = Arc::new(AtomicUsize::new(0));
    let healthy_starts = Arc::new(AtomicUsize::new(0));
    let healthy_loads = Arc::new(AtomicUsize::new(0));
    let package = PackageInfo::new(
        OWNER.into(),
        "1.0.0".into(),
        PackageKind::Code,
        Side::Both,
        vec![OWNER.into(), "extra".into()],
        vec![],
    );
    let mods: mod_api::Mods = vec![ModEntry::new(
        OWNER,
        Box::new(StructureMod {
            broken_starts: broken_starts.clone(),
            broken_loads: broken_loads.clone(),
            healthy_starts: healthy_starts.clone(),
            healthy_loads: healthy_loads.clone(),
        }),
    )]
    .into();
    mod_platform::__private::with_test_package_inventory(vec![package], || {
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = serde_json::json!({
                "states": [
                    ["air", true, false, "", {"id":"minecraft:air","properties":{}}],
                    ["stone", false, false, "", {"id":"minecraft:stone","properties":{}}],
                    ["cave_air", true, false, "", {"id":"minecraft:cave_air","properties":{}}],
                    ["void_air", true, false, "", {"id":"minecraft:void_air","properties":{}}]
                ],
                "defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:cave_air":2,"minecraft:void_air":3}
            });
            let registry = sunrise_world::registry::BlockRegistry::from_facts_json(
                facts.to_string().as_bytes(),
            )
            .unwrap();
            crate::game::api_values::resolve_block_definitions(&registry).unwrap();
            let resources = resources_for_registry(registry);
            let declarations = mod_api::__private::structure_type_declarations();
            assert!(
                declarations
                    .iter()
                    .any(|(id, mod_id, _)| id == "extra:broken" && *mod_id == OWNER)
            );
            let types = installed_structure_types();
            let broken = &types["extra:broken"];
            let healthy = &types["test_structures:healthy"];
            let empty = Arc::new(Tag::Compound(Compound::new()));
            let mut random = WorldgenRandom::new(LegacyRandom::new(0));
            assert!(
                broken
                    .find_generation_point(
                        &mut StubContext,
                        &inputs(resources),
                        ChunkPos::new(0, 0),
                        &empty,
                        &mut random,
                    )
                    .is_none()
            );
            assert!(
                broken
                    .find_generation_point(
                        &mut StubContext,
                        &inputs(resources),
                        ChunkPos::new(1, 0),
                        &empty,
                        &mut random,
                    )
                    .is_none()
            );
            assert_eq!(broken_starts.load(Ordering::SeqCst), 1);
            let loaded = broken
                .load_piece(resources.registry, &Compound::new())
                .unwrap();
            assert_eq!(broken_loads.load(Ordering::SeqCst), 1);
            assert_eq!(
                loaded.bounding_box(),
                BoundingBox::new([4, 64, 4], [6, 66, 6])
            );
            let (position, pieces) = healthy
                .find_generation_point(
                    &mut StubContext,
                    &inputs(resources),
                    ChunkPos::new(0, 0),
                    &empty,
                    &mut random,
                )
                .unwrap();
            assert_eq!(position, [0, 64, 0]);
            assert_eq!(pieces.len(), 1);
            assert_eq!(healthy_starts.load(Ordering::SeqCst), 1);
            assert_eq!(healthy_loads.load(Ordering::SeqCst), 0);
        });
    });
}

#[test]
fn piece_metadata_and_save_panics_stay_local_and_debug_skips_mod_fmt() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct HostilePiece {
        panic_beardifier: bool,
        panic_save: bool,
        save_calls: Arc<AtomicUsize>,
    }
    impl std::fmt::Debug for HostilePiece {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("mod Debug must not run outside the callback boundary")
        }
    }
    impl StructurePiece for HostilePiece {
        fn bounding_box(&self) -> BoundingBox {
            BoundingBox::new([1, 64, 2], [3, 66, 4])
        }
        fn beardifier(&self) -> Option<mod_api::PieceBeardifier> {
            assert!(!self.panic_beardifier, "beardifier panicked");
            None
        }
        fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
            Ok(())
        }
        fn add_additional_save_data(&self, tag: &mut Compound) {
            self.save_calls.fetch_add(1, Ordering::SeqCst);
            tag.insert("partial".into(), Tag::Int(1));
            assert!(!self.panic_save, "save panicked after partial write");
        }
    }

    struct PairType(Arc<AtomicUsize>);
    impl StructureType for PairType {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            Some(FoundStructure {
                position: [1, 64, 2],
                pieces: vec![
                    Arc::new(HostilePiece {
                        panic_beardifier: false,
                        panic_save: true,
                        save_calls: self.0.clone(),
                    }),
                    Arc::new(HostilePiece {
                        panic_beardifier: false,
                        panic_save: false,
                        save_calls: self.0.clone(),
                    }),
                    Arc::new(HostilePiece {
                        panic_beardifier: true,
                        panic_save: false,
                        save_calls: self.0.clone(),
                    }),
                ],
                pool_pieces: Vec::new(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Err(StructureError::new(std::io::Error::other(
                "fixture has no saved pieces",
            )))
        }
    }

    let resources = test_resources();
    let save_calls = Arc::new(AtomicUsize::new(0));
    let adapter = Adapter::new(
        "testmod:hostile",
        "testmod",
        Arc::new(PairType(save_calls.clone())),
    );
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            &inputs(resources),
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    assert_eq!(pieces.len(), 2);
    let debug = format!("{:?}", pieces[0]);
    assert!(debug.contains("ModPiece") && debug.contains("testmod:hostile"));
    assert!(pieces[0].save().is_none());
    assert!(pieces[1].save().is_none());
    assert_eq!(save_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        pieces[0].bounding_box(),
        BoundingBox::new([1, 64, 2], [3, 66, 4])
    );
    let other = Adapter::new("testmod:boxes", "testmod", Arc::new(Boxes));
    let (_, other_pieces) = other
        .find_generation_point(
            &mut StubContext,
            &inputs(resources),
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    assert!(other_pieces[0].save().is_some());
}

struct NoSolid;
impl sunrise_world::worldgen::structure::legacy::SolidRender for NoSolid {
    fn solid_render(&self, _: sunrise_world::registry::BlockStateId) -> bool {
        false
    }
}

struct ChunkWorld {
    air: sunrise_world::registry::BlockStateId,
    writes: Vec<([i32; 3], sunrise_world::registry::BlockStateId)>,
}
impl sunrise_world::worldgen::feature::FeatureWorld for ChunkWorld {
    type Error = PlacementQueryError;
    fn block(&mut self, _: [i32; 3]) -> Result<sunrise_world::registry::BlockStateId, Self::Error> {
        Ok(self.air)
    }
    fn set_block(
        &mut self,
        pos: [i32; 3],
        state: sunrise_world::registry::BlockStateId,
        _: i32,
    ) -> Result<bool, Self::Error> {
        self.writes.push((pos, state));
        Ok(true)
    }
}
impl sunrise_world::worldgen::placement::PlacementContext for ChunkWorld {
    fn generation(&self) -> sunrise_world::worldgen::anchor::WorldGenerationContext {
        sunrise_world::worldgen::anchor::WorldGenerationContext::new(-64, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(-64)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(319)
    }
    fn height(
        &mut self,
        _: sunrise_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> Result<i32, PlacementQueryError> {
        Ok(64)
    }
    fn block_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<sunrise_world::registry::BlockStateId, PlacementQueryError> {
        Ok(self.air)
    }
    fn fluid_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<sunrise_world::fluid::FluidState, PlacementQueryError> {
        Ok(sunrise_world::fluid::FluidState::EMPTY)
    }
    fn mark_postprocessing(&mut self, _: [i32; 3]) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    fn schedule_fluid_tick(
        &mut self,
        _: [i32; 3],
        _: sunrise_world::fluid::FluidKind,
        _: i32,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
}

fn test_structure_registry(
    name: &str,
) -> sunrise_world::worldgen::structure::start::StructureRegistry {
    use std::collections::{BTreeMap, BTreeSet};
    sunrise_world::worldgen::structure::start::StructureRegistry::new(
        &BTreeMap::from([(
            name.to_owned(),
            serde_json::json!({
                "type": name,
                "biomes": [],
                "terrain_adaptation": "none"
            })
            .to_string(),
        )]),
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &BTreeMap::new(),
    )
    .unwrap()
}

#[test]
fn failing_mod_piece_preserves_writes_draws_and_continues_via_place_in_chunk() {
    use sunrise_world::worldgen::structure::start::{StructureStart, place_in_chunk};

    #[derive(Debug)]
    struct InnerCause;
    impl std::fmt::Display for InnerCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("corrupt piece payload")
        }
    }
    impl std::error::Error for InnerCause {}

    #[derive(Debug)]
    struct OuterCause(InnerCause);
    impl std::fmt::Display for OuterCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("piece placement failed")
        }
    }
    impl std::error::Error for OuterCause {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    type Log = Arc<std::sync::Mutex<Vec<(&'static str, ChunkPos, i32)>>>;

    #[derive(Debug)]
    struct OrderedPiece {
        label: &'static str,
        x_offset: i32,
        fail: bool,
        stone: BlockKey,
        log: Log,
    }
    impl StructurePiece for OrderedPiece {
        fn bounding_box(&self) -> BoundingBox {
            BoundingBox::new([0, 64, 0], [24, 66, 4])
        }
        fn post_process(&self, context: &mut dyn PieceContext) -> Result<(), StructureError> {
            let chunk = context.chunk();
            let stone = context.state(self.stone);
            assert!(!context.set_block([chunk.x * 16 - 1, 64, 1], stone));
            assert!(context.set_block([chunk.x * 16 + self.x_offset, 64, 1], stone));
            let draw = context.random().next_int();
            self.log.lock().unwrap().push((self.label, chunk, draw));
            if self.fail {
                return Err(StructureError::new(OuterCause(InnerCause)));
            }
            Ok(())
        }
        fn add_additional_save_data(&self, _: &mut Compound) {}
    }

    struct OrderedType {
        stone: BlockKey,
        log: Log,
    }
    impl StructureType for OrderedType {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            Some(FoundStructure {
                position: [0, 64, 0],
                pieces: vec![
                    Arc::new(OrderedPiece {
                        label: "piece0",
                        x_offset: 1,
                        fail: true,
                        stone: self.stone,
                        log: self.log.clone(),
                    }),
                    Arc::new(OrderedPiece {
                        label: "piece1",
                        x_offset: 2,
                        fail: false,
                        stone: self.stone,
                        log: self.log.clone(),
                    }),
                ],
                pool_pieces: Vec::new(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Err(StructureError::new(std::io::Error::other(
                "fixture has no saved pieces",
            )))
        }
        fn after_place(
            &self,
            context: &mut dyn PieceContext,
            pieces: &[BoundingBox],
        ) -> Result<(), StructureError> {
            assert_eq!(pieces.len(), 2);
            let chunk = context.chunk();
            let stone = context.state(self.stone);
            assert!(context.set_block([chunk.x * 16 + 3, 64, 1], stone));
            let draw = context.random().next_int();
            self.log.lock().unwrap().push(("after_place", chunk, draw));
            if chunk == ChunkPos::new(0, 0) {
                return Err(StructureError::new(std::io::Error::other(
                    "after_place failed on chunk 0",
                )));
            }
            Ok(())
        }
    }

    let resources = test_resources();
    let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();
    let stone_id = resources.registry.default_state("minecraft:stone").unwrap();
    let air_id = resources.registry.default_state("minecraft:air").unwrap();
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let adapter = Adapter::new(
        "testmod:ordered",
        "testmod",
        Arc::new(OrderedType {
            stone: stone_key,
            log: log.clone(),
        }),
    );
    let mut start_random = WorldgenRandom::new(LegacyRandom::new(0));
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            &inputs(resources),
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut start_random,
        )
        .unwrap();
    let registry = test_structure_registry("testmod:ordered");
    let start = StructureStart::new(0, &registry.structures[0], ChunkPos::new(0, 0), pieces);

    let mut world = ChunkWorld {
        air: air_id,
        writes: Vec::new(),
    };
    let mut piece_random = LegacyRandom::new(0);
    let mut place_feature = |_: &mut ChunkWorld,
                             _: &mut LegacyRandom,
                             _: sunrise_world::worldgen::structure::jigsaw::FeatureKey<'_>,
                             _: [i32; 3]| Ok(false);
    for cx in [0, 1] {
        let chunk_box = BoundingBox::new([cx * 16, -64, 0], [cx * 16 + 15, 319, 15]);
        place_in_chunk(
            &start,
            &mut world,
            resources,
            &NoSolid,
            &mut piece_random,
            chunk_box,
            &mut place_feature,
        )
        .unwrap();
    }

    let mut expected_rng = LegacyRandom::new(0);
    let expected_draws: Vec<i32> = (0..6)
        .map(|_| sunrise_core::random::RandomSource::next_int(&mut expected_rng))
        .collect();
    assert_eq!(
        *log.lock().unwrap(),
        vec![
            ("piece0", ChunkPos::new(0, 0), expected_draws[0]),
            ("piece1", ChunkPos::new(0, 0), expected_draws[1]),
            ("after_place", ChunkPos::new(0, 0), expected_draws[2]),
            ("piece0", ChunkPos::new(1, 0), expected_draws[3]),
            ("piece1", ChunkPos::new(1, 0), expected_draws[4]),
            ("after_place", ChunkPos::new(1, 0), expected_draws[5]),
        ]
    );
    assert_eq!(
        world.writes,
        vec![
            ([1, 64, 1], stone_id),
            ([2, 64, 1], stone_id),
            ([3, 64, 1], stone_id),
            ([17, 64, 1], stone_id),
            ([18, 64, 1], stone_id),
            ([19, 64, 1], stone_id),
        ]
    );
    assert_eq!(
        sunrise_core::random::RandomSource::next_int(&mut piece_random),
        sunrise_core::random::RandomSource::next_int(&mut expected_rng),
    );

    // Real infrastructure failures still surface from place_in_chunk.
    let uninitialized_registry = sunrise_world::registry::BlockRegistry::from_facts_json(
        br#"{"defaults":{"minecraft:air":0,"minecraft:stone":1},"states":[["air",true,false,"",{"id":"minecraft:air","properties":{}}],["stone",false,false,"",{"id":"minecraft:stone","properties":{}}]]}"#
            .as_slice(),
    )
    .unwrap();
    let bare_resources = resources_for_registry(uninitialized_registry);
    let err = place_in_chunk(
        &start,
        &mut world,
        bare_resources,
        &NoSolid,
        &mut piece_random,
        BoundingBox::new([0, -64, 0], [15, 319, 15]),
        &mut place_feature,
    )
    .unwrap_err();
    assert_eq!(err, PlacementQueryError::Unavailable);
}

#[test]
fn placing_propagates_unwritten_and_contains_real_world_errors() {
    use sunrise_world::worldgen::structure::legacy::{PieceContext as EngineContext, PieceWorld};

    struct FaultyWorld {
        air: sunrise_world::registry::BlockStateId,
        set_result: PieceResult<bool>,
        be_error: bool,
        entity_error: bool,
    }
    impl PieceWorld for FaultyWorld {
        fn block(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::registry::BlockStateId> {
            Ok(self.air)
        }
        fn set(
            &mut self,
            _: [i32; 3],
            _: sunrise_world::registry::BlockStateId,
            _: i32,
        ) -> PieceResult<bool> {
            self.set_result.clone()
        }
        fn fluid(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::fluid::FluidState> {
            Ok(sunrise_world::fluid::FluidState::EMPTY)
        }
        fn height(
            &mut self,
            _: sunrise_world::heightmap::HeightmapType,
            _: i32,
            _: i32,
        ) -> PieceResult<i32> {
            Ok(64)
        }
        fn min_y(&mut self) -> PieceResult<i32> {
            Ok(-64)
        }
        fn max_y(&mut self) -> PieceResult<i32> {
            Ok(320)
        }
        fn block_entity(&mut self, _: [i32; 3], _: Compound) -> PieceResult<()> {
            if self.be_error {
                Err(PlacementQueryError::Failed(
                    "block entity storage failed".into(),
                ))
            } else {
                Ok(())
            }
        }
        fn entity(&mut self, _: Compound) -> PieceResult<()> {
            if self.entity_error {
                Err(PlacementQueryError::Failed("entity storage failed".into()))
            } else {
                Ok(())
            }
        }
        fn post_process(&mut self, _: [i32; 3]) -> PieceResult<()> {
            Ok(())
        }
        fn fluid_tick(
            &mut self,
            _: [i32; 3],
            _: sunrise_world::fluid::FluidKind,
            _: i32,
        ) -> PieceResult<()> {
            Ok(())
        }
        fn seed(&mut self) -> PieceResult<i64> {
            Ok(0)
        }
        fn level_boolean(&mut self) -> PieceResult<bool> {
            Ok(false)
        }
        fn level_float(&mut self) -> PieceResult<f32> {
            Ok(0.0)
        }
        fn level_int(&mut self, _: i32) -> PieceResult<i32> {
            Ok(0)
        }
        fn sea_level(&mut self) -> PieceResult<i32> {
            Ok(63)
        }
        fn face_full(
            &mut self,
            _: [i32; 3],
            _: sunrise_core::geometry::Direction,
        ) -> PieceResult<bool> {
            Ok(false)
        }
        fn face_sturdy(
            &mut self,
            _: [i32; 3],
            _: sunrise_core::geometry::Direction,
        ) -> PieceResult<bool> {
            Ok(false)
        }
        fn center_sturdy(
            &mut self,
            _: [i32; 3],
            _: sunrise_core::geometry::Direction,
        ) -> PieceResult<bool> {
            Ok(false)
        }
        fn biome(&mut self, _: [i32; 3]) -> PieceResult<sunrise_world::worldgen::biome::BiomeId> {
            Err(PlacementQueryError::Unavailable)
        }
        fn place_template(
            &mut self,
            _: &sunrise_world::worldgen::structure::StructureResources,
            _: &sunrise_world::worldgen::structure::StructureTemplate,
            _: [i32; 3],
            _: [i32; 3],
            _: &mut sunrise_world::worldgen::structure::processor::PlaceSettings<'_>,
            _: &mut dyn sunrise_world::worldgen::structure::processor::ProcessorRandom,
            _: i32,
        ) -> PieceResult<bool> {
            Ok(false)
        }
    }

    let resources = test_resources();
    let air = resources.registry.default_state("minecraft:air").unwrap();
    let stone_key = mod_api::__private::block_key("minecraft:stone").unwrap();

    // Same-state / vanilla clip (Ok(false)) returns false without failing.
    let mut same_state = FaultyWorld {
        air,
        set_result: Ok(false),
        be_error: false,
        entity_error: false,
    };
    let mut random = LegacyRandom::new(7);
    {
        let mut engine = EngineContext {
            world: &mut same_state,
            resources,
            blocks: &NoSolid,
            random: &mut random,
            chunk_box: BoundingBox::new([0, -64, 0], [15, 319, 15]),
            chunk: ChunkPos::new(0, 0),
            reference: [8, 64, 8],
        };
        let mut context = Placing::new(&mut engine, "testmod", "testmod:boxes").unwrap();
        let stone = context.state(stone_key);
        assert!(!context.set_block([4, 64, 4], stone));
    }

    // Real errors on set_block, block_entity, and entity are reported and contained.
    let mut broken = FaultyWorld {
        air,
        set_result: Err(PlacementQueryError::Failed(
            "piece write: region fault".into(),
        )),
        be_error: true,
        entity_error: true,
    };
    {
        let mut engine = EngineContext {
            world: &mut broken,
            resources,
            blocks: &NoSolid,
            random: &mut random,
            chunk_box: BoundingBox::new([0, -64, 0], [15, 319, 15]),
            chunk: ChunkPos::new(0, 0),
            reference: [8, 64, 8],
        };
        let mut context = Placing::new(&mut engine, "testmod", "testmod:boxes").unwrap();
        let stone = context.state(stone_key);
        assert!(!context.set_block([4, 64, 4], stone));
        context.block_entity([4, 64, 4], Compound::new());
        let with_pos = Compound::from([(
            "Pos".into(),
            Tag::List(vec![
                Tag::Double(4.5_f64.to_bits()),
                Tag::Double(64.0_f64.to_bits()),
                Tag::Double(4.5_f64.to_bits()),
            ]),
        )]);
        context.entity(with_pos);
        context.entity(Compound::new());
    }
}

#[test]
#[ignore = "daily"]
fn piece_and_after_place_panics_poison_only_their_operation_with_toast_notices() {
    use mod_api::__private::{HookPanicReport, take_hook_panics};
    use mod_api::{
        Hooks, ModEntry, PackageInfo, PackageKind, Registrar, RegistrationContext, Side,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunrise_world::worldgen::structure::start::{StructureStart, place_in_chunk};

    // Run the worldgen fixture in its own process.
    const CHILD: &str = "SUNRISE_STRUCTURE_PANIC_TEST_CHILD";
    if crate::game::test_fixtures::run_fresh_cases(
        "game::native_structure_types::tests::piece_and_after_place_panics_poison_only_their_operation_with_toast_notices",
        CHILD,
        &["1"],
    ) {
        return;
    }

    const OWNER: &str = "test_structure_placer";
    const PIECE_PANIC: &str = "test post_process panicked after writing";
    const AFTER_PANIC: &str = "test after_place panicked after writing";

    #[derive(Debug)]
    struct TestPiece {
        x_offset: i32,
        panic_on_place: bool,
        stone: BlockKey,
        calls: Arc<AtomicUsize>,
    }
    impl StructurePiece for TestPiece {
        fn bounding_box(&self) -> BoundingBox {
            BoundingBox::new([0, 64, 0], [24, 66, 4])
        }
        fn post_process(&self, context: &mut dyn PieceContext) -> Result<(), StructureError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let chunk = context.chunk();
            let stone = context.state(self.stone);
            assert!(context.set_block([chunk.x * 16 + self.x_offset, 64, 1], stone));
            context.random().next_int();
            assert!(!self.panic_on_place, "{PIECE_PANIC}");
            Ok(())
        }
        fn add_additional_save_data(&self, _: &mut Compound) {}
    }

    struct ProbeType {
        piece_x: i32,
        second_piece_x: Option<i32>,
        after_x: i32,
        panic_first_piece: bool,
        panic_after: bool,
        error_after_chunk0: bool,
        stone: BlockKey,
        piece_calls: Arc<AtomicUsize>,
        after_calls: Arc<AtomicUsize>,
    }
    impl StructureType for ProbeType {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            let mut pieces: Vec<Arc<dyn StructurePiece>> = vec![Arc::new(TestPiece {
                x_offset: self.piece_x,
                panic_on_place: self.panic_first_piece,
                stone: self.stone,
                calls: self.piece_calls.clone(),
            })];
            if let Some(x_offset) = self.second_piece_x {
                pieces.push(Arc::new(TestPiece {
                    x_offset,
                    panic_on_place: false,
                    stone: self.stone,
                    calls: self.piece_calls.clone(),
                }));
            }
            Some(FoundStructure {
                position: [0, 64, 0],
                pieces,
                pool_pieces: Vec::new(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Ok(Arc::new(TestPiece {
                x_offset: self.piece_x,
                panic_on_place: false,
                stone: self.stone,
                calls: self.piece_calls.clone(),
            }))
        }
        fn after_place(
            &self,
            context: &mut dyn PieceContext,
            _: &[BoundingBox],
        ) -> Result<(), StructureError> {
            self.after_calls.fetch_add(1, Ordering::SeqCst);
            let chunk = context.chunk();
            let stone = context.state(self.stone);
            assert!(context.set_block([chunk.x * 16 + self.after_x, 64, 1], stone));
            context.random().next_int();
            assert!(!self.panic_after, "{AFTER_PANIC}");
            if self.error_after_chunk0 && chunk == ChunkPos::new(0, 0) {
                return Err(StructureError::new(std::io::Error::other(
                    "after_place returned error on chunk 0",
                )));
            }
            Ok(())
        }
    }

    struct StructureMod {
        broken_piece_calls: Arc<AtomicUsize>,
        broken_piece_after_calls: Arc<AtomicUsize>,
        broken_after_piece_calls: Arc<AtomicUsize>,
        broken_after_calls: Arc<AtomicUsize>,
        healthy_piece_calls: Arc<AtomicUsize>,
        healthy_after_calls: Arc<AtomicUsize>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for StructureMod {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let stone = r.block_key("minecraft:stone");
            r.register_structure_type(
                "extra:broken_piece",
                ProbeType {
                    piece_x: 1,
                    second_piece_x: Some(2),
                    after_x: 3,
                    panic_first_piece: true,
                    panic_after: false,
                    error_after_chunk0: true,
                    stone,
                    piece_calls: self.broken_piece_calls.clone(),
                    after_calls: self.broken_piece_after_calls.clone(),
                },
            );
            r.register_structure_type(
                "extra:broken_after",
                ProbeType {
                    piece_x: 4,
                    second_piece_x: None,
                    after_x: 5,
                    panic_first_piece: false,
                    panic_after: true,
                    error_after_chunk0: false,
                    stone,
                    piece_calls: self.broken_after_piece_calls.clone(),
                    after_calls: self.broken_after_calls.clone(),
                },
            );
            r.register_structure_type(
                "healthy",
                ProbeType {
                    piece_x: 6,
                    second_piece_x: None,
                    after_x: 7,
                    panic_first_piece: false,
                    panic_after: false,
                    error_after_chunk0: false,
                    stone,
                    piece_calls: self.healthy_piece_calls.clone(),
                    after_calls: self.healthy_after_calls.clone(),
                },
            );
        }
    }

    let broken_piece_calls = Arc::new(AtomicUsize::new(0));
    let broken_piece_after_calls = Arc::new(AtomicUsize::new(0));
    let broken_after_piece_calls = Arc::new(AtomicUsize::new(0));
    let broken_after_calls = Arc::new(AtomicUsize::new(0));
    let healthy_piece_calls = Arc::new(AtomicUsize::new(0));
    let healthy_after_calls = Arc::new(AtomicUsize::new(0));

    let package = PackageInfo::new(
        OWNER.into(),
        "1.0.0".into(),
        PackageKind::Code,
        Side::Both,
        vec![OWNER.into(), "extra".into()],
        vec![],
    );
    let mods: mod_api::Mods = vec![ModEntry::new(
        OWNER,
        Box::new(StructureMod {
            broken_piece_calls: broken_piece_calls.clone(),
            broken_piece_after_calls: broken_piece_after_calls.clone(),
            broken_after_piece_calls: broken_after_piece_calls.clone(),
            broken_after_calls: broken_after_calls.clone(),
            healthy_piece_calls: healthy_piece_calls.clone(),
            healthy_after_calls: healthy_after_calls.clone(),
        }),
    )]
    .into();

    mod_platform::__private::with_test_package_inventory(vec![package], || {
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            let facts = serde_json::json!({
                "states": [
                    ["air", true, false, "", {"id":"minecraft:air","properties":{}}],
                    ["stone", false, false, "", {"id":"minecraft:stone","properties":{}}],
                    ["cave_air", true, false, "", {"id":"minecraft:cave_air","properties":{}}],
                    ["void_air", true, false, "", {"id":"minecraft:void_air","properties":{}}]
                ],
                "defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:cave_air":2,"minecraft:void_air":3}
            });
            let block_registry = sunrise_world::registry::BlockRegistry::from_facts_json(
                facts.to_string().as_bytes(),
            )
            .unwrap();
            crate::game::api_values::resolve_block_definitions(&block_registry).unwrap();
            let resources = resources_for_registry(block_registry);
            let air_id = resources.registry.default_state("minecraft:air").unwrap();
            let stone_id = resources.registry.default_state("minecraft:stone").unwrap();

            let types = installed_structure_types();
            let empty = Arc::new(Tag::Compound(Compound::new()));
            let mut start_random = WorldgenRandom::new(LegacyRandom::new(0));
            let build_start = |id: &str, start_random: &mut WorldgenRandom<LegacyRandom>| {
                let (_, pieces) = types[id]
                    .find_generation_point(
                        &mut StubContext,
                        &inputs(resources),
                        ChunkPos::new(0, 0),
                        &empty,
                        start_random,
                    )
                    .unwrap();
                let structure_registry = test_structure_registry(id);
                StructureStart::new(
                    0,
                    &structure_registry.structures[0],
                    ChunkPos::new(0, 0),
                    pieces,
                )
            };

            let broken_piece_start = build_start("extra:broken_piece", &mut start_random);
            let broken_after_start = build_start("extra:broken_after", &mut start_random);
            let healthy_start = build_start("test_structure_placer:healthy", &mut start_random);

            let mut world = ChunkWorld {
                air: air_id,
                writes: Vec::new(),
            };
            let mut piece_random = LegacyRandom::new(0);
            let mut place_feature =
                |_: &mut ChunkWorld,
                 _: &mut LegacyRandom,
                 _: sunrise_world::worldgen::structure::jigsaw::FeatureKey<'_>,
                 _: [i32; 3]| Ok(false);
            let chunk0 = BoundingBox::new([0, -64, 0], [15, 319, 15]);
            let chunk1 = BoundingBox::new([16, -64, 0], [31, 319, 15]);

            // 1. Piece panic disables only post_process on extra:broken_piece; sibling piece is
            // skipped, while after_place still runs and returns an error without latching.
            place_in_chunk(
                &broken_piece_start,
                &mut world,
                resources,
                &NoSolid,
                &mut piece_random,
                chunk0,
                &mut place_feature,
            )
            .unwrap();
            assert_eq!(broken_piece_calls.load(Ordering::SeqCst), 1);
            assert_eq!(broken_piece_after_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: OWNER,
                    hook: "StructurePiece::post_process",
                }]
            );

            // Next chunk: post_process stays disabled with no second toast; after_place runs again.
            place_in_chunk(
                &broken_piece_start,
                &mut world,
                resources,
                &NoSolid,
                &mut piece_random,
                chunk1,
                &mut place_feature,
            )
            .unwrap();
            assert_eq!(broken_piece_calls.load(Ordering::SeqCst), 1);
            assert_eq!(broken_piece_after_calls.load(Ordering::SeqCst), 2);
            assert!(take_hook_panics().is_empty());
            // Other operations on the same registration (load_piece) remain active.
            assert!(
                types["extra:broken_piece"]
                    .load_piece(resources.registry, &Compound::new())
                    .is_some()
            );

            // 2. after_place panic disables only after_place on extra:broken_after; post_process
            // still runs on subsequent chunks.
            place_in_chunk(
                &broken_after_start,
                &mut world,
                resources,
                &NoSolid,
                &mut piece_random,
                chunk0,
                &mut place_feature,
            )
            .unwrap();
            assert_eq!(broken_after_piece_calls.load(Ordering::SeqCst), 1);
            assert_eq!(broken_after_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                take_hook_panics(),
                [HookPanicReport {
                    mod_id: OWNER,
                    hook: "StructureType::after_place",
                }]
            );

            place_in_chunk(
                &broken_after_start,
                &mut world,
                resources,
                &NoSolid,
                &mut piece_random,
                chunk1,
                &mut place_feature,
            )
            .unwrap();
            assert_eq!(broken_after_piece_calls.load(Ordering::SeqCst), 2);
            assert_eq!(broken_after_calls.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());

            // 3. Another structure type registered by the same mod remains unaffected.
            place_in_chunk(
                &healthy_start,
                &mut world,
                resources,
                &NoSolid,
                &mut piece_random,
                chunk0,
                &mut place_feature,
            )
            .unwrap();
            assert_eq!(healthy_piece_calls.load(Ordering::SeqCst), 1);
            assert_eq!(healthy_after_calls.load(Ordering::SeqCst), 1);
            assert!(take_hook_panics().is_empty());

            assert_eq!(
                world.writes,
                vec![
                    ([1, 64, 1], stone_id),
                    ([3, 64, 1], stone_id),
                    ([19, 64, 1], stone_id),
                    ([4, 64, 1], stone_id),
                    ([5, 64, 1], stone_id),
                    ([20, 64, 1], stone_id),
                    ([6, 64, 1], stone_id),
                    ([7, 64, 1], stone_id),
                ]
            );
            let mut expected_rng = LegacyRandom::new(0);
            for _ in 0..8 {
                sunrise_core::random::RandomSource::next_int(&mut expected_rng);
            }
            assert_eq!(
                sunrise_core::random::RandomSource::next_int(&mut piece_random),
                sunrise_core::random::RandomSource::next_int(&mut expected_rng),
            );
        });
    });
}
