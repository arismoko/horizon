use super::*;
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::{nbt::Tag, random::LegacyRandom};
use sunrise_world::{
    heightmap::HeightmapType,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        block_feature::FeatureBlockBindings,
        placement::{
            FeaturePlacementError, FeaturePlacer, PlacementContext, PlacementModifier,
            PlacementQueryError, mod_placements::with_test_placement_modifier_types,
        },
    },
};

fn test_registry() -> &'static BlockRegistry {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = serde_json::json!({
        "states": [
            ["air", true, false, "", {"id": "minecraft:air", "properties": {}}],
            ["cave_air", true, false, "", {"id": "minecraft:cave_air", "properties": {}}],
            ["void_air", true, false, "", {"id": "minecraft:void_air", "properties": {}}],
            ["stone", false, false, "", {"id": "minecraft:stone", "properties": {}}]
        ],
        "defaults": {
            "minecraft:air": 0,
            "minecraft:cave_air": 1,
            "minecraft:void_air": 2,
            "minecraft:stone": 3
        }
    });
    let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    Box::leak(Box::new(registry))
}

struct StubWorld {
    registry: &'static BlockRegistry,
    sea_level: i32,
    blocks: BTreeMap<[i32; 3], BlockStateId>,
}
impl PlacementContext for StubWorld {
    fn block_registry(&self) -> Option<&'static BlockRegistry> {
        Some(self.registry)
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, self.sea_level)
    }
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(self.sea_level)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(-64)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(319)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Err(PlacementQueryError::Unavailable)
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self
            .blocks
            .get(&position)
            .copied()
            .unwrap_or_else(|| self.registry.default_state("minecraft:air").unwrap()))
    }
}

/// A mod placement modifier that filters positions below sea level and
/// delegates surviving positions to a wrapped modifier.
#[derive(Debug)]
struct BelowSeaWrapped {
    inner: PlacementModifierRef,
}
impl ApiPlacementModifier for BelowSeaWrapped {
    fn modify(
        &self,
        context: &mut dyn PlacementModifierContext,
        random: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), FeatureError>,
    ) -> Result<(), FeatureError> {
        let sea = context.sea_level().unwrap_or(63);
        if origin[1] >= sea {
            return Ok(());
        }
        context.apply_modifier(&self.inner, random, origin, output)
    }
}

struct BelowSeaWrappedType;
impl PlacementModifierType for BelowSeaWrappedType {
    fn decode(
        &self,
        context: &dyn PlacementModifierDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn ApiPlacementModifier>, FeatureError> {
        let Some(inner_tag) = config.get("inner") else {
            return Err(FeatureError::new(std::io::Error::other(
                "missing inner placement modifier",
            )));
        };
        let inner = context.modifier(inner_tag)?;
        Ok(Arc::new(BelowSeaWrapped { inner }))
    }
}

/// A mod placement modifier decodes from JSON, runs in a chain beside vanilla
/// modifiers (`minecraft:fixed_placement`, `minecraft:offset`), reads world
/// state (`sea_level`), and delegates to a wrapped vanilla modifier.
#[test]
fn mod_placement_modifier_runs_in_chain_and_wraps_vanilla() {
    let registry = test_registry();
    let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
    let mut types = ModPlacementModifierTypes::new();
    types.insert(
        "testmod:below_sea".to_owned(),
        Arc::new(ModifierType {
            inner: Arc::new(BelowSeaWrappedType),
        }),
    );

    let chain = with_test_placement_modifier_types(&types, || {
        let fixed = PlacementModifier::from_json_with_blocks(
            &serde_json::json!({
                "type": "minecraft:fixed_placement",
                "positions": [[2, 40, 2], [4, 80, 4]]
            })
            .to_string(),
            &bindings,
        )
        .unwrap();
        let below_sea = PlacementModifier::from_json_with_blocks(
            &serde_json::json!({
                "type": "testmod:below_sea",
                "inner": {
                    "type": "minecraft:offset",
                    "x": 1,
                    "y": -2,
                    "z": 3
                }
            })
            .to_string(),
            &bindings,
        )
        .unwrap();
        vec![fixed, below_sea]
    });

    let mut world = StubWorld {
        registry,
        sea_level: 63,
        blocks: BTreeMap::new(),
    };
    let mut random = LegacyRandom::new(7);
    let mut placer = FeaturePlacer::new();
    let mut placed = Vec::new();
    let placed_any = placer
        .place_in(&chain, &mut world, &mut random, [0, 0, 0], |_, pos| {
            placed.push(pos);
            Ok::<bool, std::convert::Infallible>(true)
        })
        .unwrap();
    assert!(placed_any);
    // [2, 40, 2] passes below_sea (40 < 63) and gets shifted by the wrapped
    // vanilla offset (+1, -2, +3); [4, 80, 4] is filtered out.
    assert_eq!(placed, vec![[3, 38, 5]]);
}

/// Unknown placement modifier types and mod decode errors fail decode cleanly.
#[test]
fn unknown_or_invalid_mod_placement_modifier_fails_decode_cleanly() {
    let registry = test_registry();
    let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
    let mut types = ModPlacementModifierTypes::new();
    types.insert(
        "testmod:below_sea".to_owned(),
        Arc::new(ModifierType {
            inner: Arc::new(BelowSeaWrappedType),
        }),
    );

    with_test_placement_modifier_types(&types, || {
        let unknown = serde_json::json!({"type": "testmod:missing"});
        assert!(PlacementModifier::from_json_with_blocks(&unknown.to_string(), &bindings).is_err());

        let invalid = serde_json::json!({"type": "testmod:below_sea"});
        assert!(PlacementModifier::from_json_with_blocks(&invalid.to_string(), &bindings).is_err());
    });
}

/// A mod's own random that answers outside `0..bound`.
struct OutOfRange;
impl RandomSource for OutOfRange {
    fn next_int(&mut self) -> i32 {
        0
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        bound
    }
    fn next_long(&mut self) -> i64 {
        0
    }
    fn next_boolean(&mut self) -> bool {
        false
    }
    fn next_float(&mut self) -> f32 {
        0.
    }
    fn next_double(&mut self) -> f64 {
        0.
    }
    fn next_gaussian(&mut self) -> f64 {
        0.
    }
}

/// Runs its wrapped modifier with [`OutOfRange`] instead of the chain's random.
#[derive(Debug)]
struct OwnRandom {
    inner: PlacementModifierRef,
}
impl ApiPlacementModifier for OwnRandom {
    fn modify(
        &self,
        context: &mut dyn PlacementModifierContext,
        _: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), FeatureError>,
    ) -> Result<(), FeatureError> {
        context.apply_modifier(&self.inner, &mut OutOfRange, origin, output)
    }
}
struct OwnRandomType;
impl PlacementModifierType for OwnRandomType {
    fn decode(
        &self,
        context: &dyn PlacementModifierDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn ApiPlacementModifier>, FeatureError> {
        let Some(inner_tag) = config.get("inner") else {
            return Err(FeatureError::new(std::io::Error::other(
                "missing inner placement modifier",
            )));
        };
        Ok(Arc::new(OwnRandom {
            inner: context.modifier(inner_tag)?,
        }))
    }
}

/// A mod random answering outside `0..bound` to `minecraft:randomly_selected`
/// fails the placement instead of indexing past the list.
#[test]
fn a_mod_random_out_of_range_fails_placement_without_panicking() {
    let registry = test_registry();
    let bindings = FeatureBlockBindings::new(registry, BTreeSet::new()).unwrap();
    let mut types = ModPlacementModifierTypes::new();
    types.insert(
        "testmod:own_random".to_owned(),
        Arc::new(ModifierType {
            inner: Arc::new(OwnRandomType),
        }),
    );
    let chain = with_test_placement_modifier_types(&types, || {
        vec![
            PlacementModifier::from_json_with_blocks(
                &serde_json::json!({
                    "type": "testmod:own_random",
                    "inner": {
                        "type": "minecraft:randomly_selected",
                        "placements": [{"type": "minecraft:in_square"}, {"type": "minecraft:in_square"}]
                    }
                })
                .to_string(),
                &bindings,
            )
            .unwrap(),
        ]
    });
    let mut world = StubWorld {
        registry,
        sea_level: 63,
        blocks: BTreeMap::new(),
    };
    let mut placed = Vec::new();
    let result = FeaturePlacer::new().place_in(
        &chain,
        &mut world,
        &mut LegacyRandom::new(7),
        [0, 0, 0],
        |_, pos| {
            placed.push(pos);
            Ok::<bool, std::convert::Infallible>(true)
        },
    );
    assert!(matches!(
        result,
        Err(FeaturePlacementError::World(PlacementQueryError::Failed(_)))
    ));
    assert!(placed.is_empty());
}
