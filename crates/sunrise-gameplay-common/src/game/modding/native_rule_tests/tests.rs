use super::*;
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::nbt::Tag;
use sunrise_core::random::LegacyRandom;
use sunrise_world::{
    fluid::FluidState,
    heightmap::HeightmapType,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        block_feature::{FeatureBlockBindings, ReplaceBlockFeature},
        feature::FeatureWorld,
        mod_rule_tests::{with_test_pos_rule_test_types, with_test_rule_test_types},
        placement::{PlacementContext, PlacementQueryError},
        structure::{
            Rotation, StateTransforms, StructureResources, StructureTemplate,
            jigsaw::Pools,
            processor::{PlaceSettings, ProcessorList, place_in_world},
        },
    },
};

fn test_env(
    processor_lists: BTreeMap<String, String>,
) -> (&'static StructureResources, FeatureBlockBindings<'static>) {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = serde_json::json!({
        "states": [
            ["air", true, false, "", {"id": "minecraft:air", "properties": {}}],
            ["cave_air", true, false, "", {"id": "minecraft:cave_air", "properties": {}}],
            ["void_air", true, false, "", {"id": "minecraft:void_air", "properties": {}}],
            ["barrier", false, false, "", {"id": "minecraft:barrier", "properties": {}}],
            ["stone", false, false, "", {"id": "minecraft:stone", "properties": {}}],
            ["cobblestone", false, false, "", {"id": "minecraft:cobblestone", "properties": {}}],
            ["bricks", false, false, "", {"id": "minecraft:bricks", "properties": {}}],
            ["water", false, false, "", {"id": "minecraft:water", "properties": {"level": "0"}}],
            ["stairs_n", false, false, "", {"id": "minecraft:oak_stairs", "properties": {"facing": "north"}}],
            ["stairs_e", false, false, "", {"id": "minecraft:oak_stairs", "properties": {"facing": "east"}}]
        ],
        "defaults": {
            "minecraft:air": 0,
            "minecraft:cave_air": 1,
            "minecraft:void_air": 2,
            "minecraft:barrier": 3,
            "minecraft:stone": 4,
            "minecraft:cobblestone": 5,
            "minecraft:bricks": 6,
            "minecraft:water": 7,
            "minecraft:oak_stairs": 8
        }
    });
    let registry = Box::leak(Box::new(
        BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap(),
    ));
    crate::game::api_values::resolve_block_definitions(registry).unwrap();
    let tags = BTreeMap::from([(
        "sample:masonry".to_owned(),
        vec!["minecraft:stone".to_owned()],
    )]);
    let resources = Box::leak(Box::new(StructureResources::new(
        registry,
        StateTransforms::default(),
        std::path::Path::new("/nonexistent"),
        processor_lists,
        &BTreeMap::new(),
    )));
    let bindings = FeatureBlockBindings::with_tags(registry, BTreeSet::new(), &tags).unwrap();
    (resources, bindings)
}

struct MemoryWorld {
    air: BlockStateId,
    blocks: BTreeMap<[i32; 3], BlockStateId>,
    writes: Vec<([i32; 3], BlockStateId, i32)>,
}
impl FeatureWorld for MemoryWorld {
    type Error = ();
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, ()> {
        Ok(self.blocks.get(&position).copied().unwrap_or(self.air))
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, ()> {
        self.blocks.insert(position, state);
        self.writes.push((position, state, flags));
        Ok(true)
    }
}
impl PlacementContext for MemoryWorld {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        Ok(self.blocks.get(&position).copied().unwrap_or(self.air))
    }
    fn fluid_state(&mut self, _: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        Ok(FluidState::EMPTY)
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        Ok(42)
    }
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(63)
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
}

fn rule_type(
    mod_id: &'static str,
    type_id: &str,
    inner: impl RuleTestType,
) -> (String, Arc<dyn ModRuleTestType>) {
    (
        type_id.to_owned(),
        Arc::new(RuleTypeAdapter {
            mod_id,
            type_id: type_id.to_owned(),
            inner: Arc::new(inner),
        }),
    )
}

fn pos_rule_type(
    mod_id: &'static str,
    type_id: &str,
    inner: impl PosRuleTestType,
) -> (String, Arc<dyn ModPosRuleTestType>) {
    (
        type_id.to_owned(),
        Arc::new(PosRuleTypeAdapter {
            mod_id,
            type_id: type_id.to_owned(),
            inner: Arc::new(inner),
        }),
    )
}

/// Binds a tag, an extra block id, and an exact block-state property at decode
/// time; optionally matches liquids too.
#[derive(Debug)]
struct TagBlockOrLiquid {
    matched: Vec<BlockState>,
    include_liquid: bool,
}
impl RuleTest for TagBlockOrLiquid {
    fn test(
        &self,
        context: &dyn RuleTestContext,
        state: BlockState,
        _pos: [i32; 3],
        _random: &mut dyn RandomSource,
    ) -> bool {
        if self.include_liquid && context.is_liquid(state) {
            return true;
        }
        self.matched.contains(&state)
    }
}

struct TagBlockOrLiquidType;
impl RuleTestType for TagBlockOrLiquidType {
    fn decode(
        &self,
        context: &dyn RuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn RuleTest>, StructureError> {
        if !matches!(config, Tag::Compound(_)) {
            return Err(StructureError::new(std::io::Error::other(
                "expected compound config",
            )));
        }
        let mut matched = Vec::new();
        if let Some(tag) = config.get("tag") {
            let Tag::String(tag) = tag else {
                return Err(StructureError::new(std::io::Error::other(
                    "field tag: expected string",
                )));
            };
            matched.extend(context.block_tag(&tag.to_string_lossy()).map_err(|e| {
                StructureError::new(std::io::Error::other(format!("field tag: {e}")))
            })?);
        }
        if let Some(block) = config.get("extra_block") {
            let Tag::String(block) = block else {
                return Err(StructureError::new(std::io::Error::other(
                    "field extra_block: expected string",
                )));
            };
            matched.extend(
                context
                    .block_states(&block.to_string_lossy())
                    .map_err(|e| {
                        StructureError::new(std::io::Error::other(format!(
                            "field extra_block: {e}"
                        )))
                    })?,
            );
        }
        if let Some(state_tag) = config.get("exact_state") {
            let state = context.decode_block_state(state_tag).map_err(|e| {
                StructureError::new(std::io::Error::other(format!("field exact_state: {e}")))
            })?;
            matched.push(state);
        }
        let include_liquid = match config.get("include_liquid") {
            None => false,
            Some(Tag::Byte(b)) => *b != 0,
            Some(_) => {
                return Err(StructureError::new(std::io::Error::other(
                    "field include_liquid: expected boolean",
                )));
            }
        };
        Ok(Arc::new(TagBlockOrLiquid {
            matched,
            include_liquid,
        }))
    }
}

/// Checks `in_template_pos[0]` against a piece-origin bound AND `world_pos[1]`
/// against a `[min_y, max_y]` band, proving both vanilla `inTemplatePos` and
/// `worldPos`/`worldReference` reach mod position rule tests.
#[derive(Debug)]
struct PieceAndYBand {
    min_y: i32,
    max_y: i32,
    max_template_x: i32,
}
impl PosRuleTest for PieceAndYBand {
    fn test(
        &self,
        _context: &dyn PosRuleTestContext,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        _random: &mut dyn RandomSource,
    ) -> bool {
        world_pos[1] >= self.min_y
            && world_pos[1] <= self.max_y
            && in_template_pos[0] <= self.max_template_x
            && world_pos[1] >= world_reference[1]
    }
}

struct PieceAndYBandType;
impl PosRuleTestType for PieceAndYBandType {
    fn decode(
        &self,
        _context: &dyn PosRuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn PosRuleTest>, StructureError> {
        if !matches!(config, Tag::Compound(_)) {
            return Err(StructureError::new(std::io::Error::other(
                "expected compound config",
            )));
        }
        let int_field = |key: &str| -> Result<i32, StructureError> {
            match config.get(key) {
                Some(Tag::Byte(n)) => Ok(i32::from(*n)),
                Some(Tag::Short(n)) => Ok(i32::from(*n)),
                Some(Tag::Int(n)) => Ok(*n),
                Some(Tag::Long(n)) => i32::try_from(*n).map_err(|_| {
                    StructureError::new(std::io::Error::other(format!(
                        "field {key}: out of i32 range"
                    )))
                }),
                _ => Err(StructureError::new(std::io::Error::other(format!(
                    "field {key}: expected integer"
                )))),
            }
        };
        let min_y = int_field("min_y")?;
        let max_y = int_field("max_y")?;
        if min_y > max_y {
            return Err(StructureError::new(std::io::Error::other(format!(
                "field min_y: {min_y} exceeds max_y {max_y}"
            ))));
        }
        let max_template_x = int_field("max_template_x")?;
        Ok(Arc::new(PieceAndYBand {
            min_y,
            max_y,
            max_template_x,
        }))
    }
}

fn three_block_template(resources: &StructureResources) -> StructureTemplate {
    let nbt = sunrise_core::json_nbt::json_to_tag(&serde_json::json!({
        "size": [3, 1, 1],
        "palette": [
            {"Name": "minecraft:stone"},
            {"Name": "minecraft:oak_stairs", "Properties": {"facing": "north"}},
            {"Name": "minecraft:oak_stairs", "Properties": {"facing": "east"}}
        ],
        "blocks": [
            {"pos": [0, 0, 0], "state": 0},
            {"pos": [1, 0, 0], "state": 1},
            {"pos": [2, 0, 0], "state": 2}
        ]
    }))
    .unwrap();
    StructureTemplate::from_nbt(&nbt, resources.registry, &resources.transforms)
}

#[test]
fn mod_rule_and_pos_rule_tests_work_together_in_structure_placement() {
    let list_json = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {
                    "predicate_type": "sample:tag_block_or_liquid",
                    "tag": "#sample:masonry",
                    "exact_state": {
                        "id": "minecraft:oak_stairs",
                        "properties": {"facing": "north"}
                    },
                    "include_liquid": false
                },
                "location_predicate": {
                    "predicate_type": "sample:tag_block_or_liquid",
                    "include_liquid": true
                },
                "position_predicate": {
                    "predicate_type": "sample:piece_and_y_band",
                    "min_y": 50,
                    "max_y": 80,
                    "max_template_x": 1
                },
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();

    let (resources, bindings) = test_env(BTreeMap::from([(
        "sample:mason_rules".to_owned(),
        list_json.clone(),
    )]));
    let rule_types: ModRuleTestTypes = BTreeMap::from([rule_type(
        "sample",
        "sample:tag_block_or_liquid",
        TagBlockOrLiquidType,
    )]);
    let pos_types: ModPosRuleTestTypes = BTreeMap::from([pos_rule_type(
        "sample",
        "sample:piece_and_y_band",
        PieceAndYBandType,
    )]);

    let list = with_test_rule_test_types(&rule_types, || {
        with_test_pos_rule_test_types(&pos_types, || {
            ProcessorList::decode_named("sample:mason_rules", &list_json, resources, &bindings)
                .expect("mod rule test and pos rule test should decode")
        })
    });

    // Pool referencing the named processor list also decodes cleanly.
    let pool_json = serde_json::json!({
        "fallback": "minecraft:empty",
        "elements": [{
            "weight": 1,
            "element": {
                "element_type": "minecraft:single_pool_element",
                "location": "sample:room",
                "processors": "sample:mason_rules",
                "projection": "rigid"
            }
        }]
    })
    .to_string();
    let pools = Pools::new(BTreeMap::from([("sample:good_pool".to_owned(), pool_json)]));
    let pool = with_test_rule_test_types(&rule_types, || {
        with_test_pos_rule_test_types(&pos_types, || {
            pools.get("sample:good_pool", resources, &bindings)
        })
    });
    assert!(pool.is_some());

    // Place template at [10, 60, 10]:
    // - [10,60,10]: input=stone (in #sample:masonry), location=water (liquid), in_template_x=0 <= 1 -> replaced with bricks (5)
    // - [11,60,10]: input=oak_stairs[facing=north] (exact_state), location=water (liquid), in_template_x=1 <= 1 -> replaced with bricks (5)
    // - [12,60,10]: input=oak_stairs[facing=east] (fails input & in_template_x=2 > 1) -> stays oak_stairs[facing=east] (8)
    let water = resources.registry.default_state("minecraft:water").unwrap();
    let bricks = resources
        .registry
        .default_state("minecraft:bricks")
        .unwrap();
    let stairs_east = bindings
        .decode_block_state(r#"{"id":"minecraft:oak_stairs","properties":{"facing":"east"}}"#)
        .unwrap();
    let mut world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::from([
            ([10, 60, 10], water),
            ([11, 60, 10], water),
            ([12, 60, 10], water),
        ]),
        writes: Vec::new(),
    };
    let mut settings = PlaceSettings::new(Rotation::None);
    settings.processors = list.0.iter().collect();
    let mut random = LegacyRandom::new(7);
    let template = three_block_template(resources);
    let placed = place_in_world(
        &mut world,
        resources,
        &template,
        [10, 60, 10],
        [10, 60, 10],
        &mut settings,
        Some(&mut random),
        2,
    )
    .unwrap();
    assert!(placed);
    assert_eq!(world.blocks.get(&[10, 60, 10]), Some(&bricks));
    assert_eq!(world.blocks.get(&[11, 60, 10]), Some(&bricks));
    assert_eq!(world.blocks.get(&[12, 60, 10]), Some(&stairs_east));

    // At Y=40 (below min_y=50), pos rule rejects every block so nothing is replaced.
    let stone = resources.registry.default_state("minecraft:stone").unwrap();
    let mut low_world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::from([([10, 40, 10], water)]),
        writes: Vec::new(),
    };
    let mut low_settings = PlaceSettings::new(Rotation::None);
    low_settings.processors = list.0.iter().collect();
    let mut low_random = LegacyRandom::new(7);
    place_in_world(
        &mut low_world,
        resources,
        &template,
        [10, 40, 10],
        [10, 40, 10],
        &mut low_settings,
        Some(&mut low_random),
        2,
    )
    .unwrap();
    assert_eq!(low_world.blocks.get(&[10, 40, 10]), Some(&stone));
}

#[derive(Debug)]
struct InvertRule(RuleTestRef);
impl RuleTest for InvertRule {
    fn test(
        &self,
        context: &dyn RuleTestContext,
        state: BlockState,
        pos: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> bool {
        !context
            .test_rule(&self.0, state, pos, random)
            .unwrap_or(false)
    }
}
struct InvertRuleType;
impl RuleTestType for InvertRuleType {
    fn decode(
        &self,
        context: &dyn RuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn RuleTest>, StructureError> {
        let inner = config
            .get("inner")
            .ok_or_else(|| StructureError::new(std::io::Error::other("field inner: missing")))?;
        let rule = context
            .rule_test(inner)
            .map_err(|e| StructureError::new(std::io::Error::other(format!("field inner: {e}"))))?;
        Ok(Arc::new(InvertRule(rule)))
    }
}

#[derive(Debug)]
struct InvertPosRule(PosRuleTestRef);
impl PosRuleTest for InvertPosRule {
    fn test(
        &self,
        context: &dyn PosRuleTestContext,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> bool {
        !context
            .test_pos_rule(&self.0, in_template_pos, world_pos, world_reference, random)
            .unwrap_or(false)
    }
}
struct InvertPosRuleType;
impl PosRuleTestType for InvertPosRuleType {
    fn decode(
        &self,
        context: &dyn PosRuleTestDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn PosRuleTest>, StructureError> {
        let inner = config
            .get("inner")
            .ok_or_else(|| StructureError::new(std::io::Error::other("field inner: missing")))?;
        let rule = context
            .pos_rule_test(inner)
            .map_err(|e| StructureError::new(std::io::Error::other(format!("field inner: {e}"))))?;
        Ok(Arc::new(InvertPosRule(rule)))
    }
}

#[test]
fn nested_rule_tests_and_replace_single_block_feature_work() {
    let (resources, bindings) = test_env(BTreeMap::new());
    let rule_types: ModRuleTestTypes = BTreeMap::from([
        rule_type("sample", "sample:invert_rule", InvertRuleType),
        rule_type("sample", "sample:tag_block_or_liquid", TagBlockOrLiquidType),
    ]);
    let pos_types: ModPosRuleTestTypes = BTreeMap::from([
        pos_rule_type("sample", "sample:invert_pos", InvertPosRuleType),
        pos_rule_type("sample", "sample:piece_and_y_band", PieceAndYBandType),
    ]);

    // Mod RuleTest inside ReplaceBlockFeature (nested around a vanilla block_match).
    let feature_json = serde_json::json!({
        "type": "minecraft:replace_single_block",
        "targets": [{
            "target": {
                "predicate_type": "sample:invert_rule",
                "inner": {
                    "predicate_type": "minecraft:block_match",
                    "block": "minecraft:cobblestone"
                }
            },
            "state": {"id": "minecraft:bricks"}
        }]
    })
    .to_string();
    let feature = with_test_rule_test_types(&rule_types, || {
        ReplaceBlockFeature::from_json(&feature_json, &bindings).unwrap()
    });
    let stone = resources.registry.default_state("minecraft:stone").unwrap();
    let bricks = resources
        .registry
        .default_state("minecraft:bricks")
        .unwrap();
    let cobble = resources
        .registry
        .default_state("minecraft:cobblestone")
        .unwrap();
    let mut world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::from([([0, 10, 0], stone), ([1, 10, 0], cobble)]),
        writes: Vec::new(),
    };
    let mut random = LegacyRandom::new(1);
    feature.place(&mut world, &mut random, [0, 10, 0]).unwrap();
    feature.place(&mut world, &mut random, [1, 10, 0]).unwrap();
    assert_eq!(world.blocks.get(&[0, 10, 0]), Some(&bricks));
    assert_eq!(world.blocks.get(&[1, 10, 0]), Some(&cobble));

    // Nested PosRuleTest delegating to another mod PosRuleTest.
    let pos_json = serde_json::json!({
        "predicate_type": "sample:invert_pos",
        "inner": {
            "predicate_type": "sample:piece_and_y_band",
            "min_y": 50,
            "max_y": 80,
            "max_template_x": 0
        }
    });
    let pos_rule = with_test_pos_rule_test_types(&pos_types, || {
        sunrise_world::worldgen::structure::processor::PosRule::decode(Some(&pos_json)).unwrap()
    });
    assert!(
        !pos_rule
            .test([0, 0, 0], [0, 60, 0], [0, 60, 0], &mut random)
            .unwrap()
    );
    assert!(
        pos_rule
            .test([0, 0, 0], [0, 20, 0], [0, 20, 0], &mut random)
            .unwrap()
    );
}

#[test]
fn decode_refusals_name_file_list_type_and_field_and_leave_pool_out() {
    let cases: &[(&str, serde_json::Value, &[&str])] = &[
        (
            "sample:unknown_rule",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {"predicate_type": "sample:missing_rule"},
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/unknown_rule.json",
                "sample:unknown_rule",
                "sample:missing_rule",
                "input_predicate",
                "predicate_type",
            ],
        ),
        (
            "sample:bad_field_type",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {"predicate_type": "minecraft:always_true"},
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "position_predicate": {
                            "predicate_type": "sample:piece_and_y_band",
                            "min_y": "not_an_int",
                            "max_y": 80,
                            "max_template_x": 1
                        },
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/bad_field_type.json",
                "sample:bad_field_type",
                "sample:piece_and_y_band",
                "position_predicate",
                "min_y",
            ],
        ),
        (
            "sample:nested_null_field",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {
                            "predicate_type": "sample:tag_block_or_liquid",
                            "extra_block": [null]
                        },
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/nested_null_field.json",
                "sample:nested_null_field",
                "sample:tag_block_or_liquid",
                "input_predicate",
                "extra_block",
            ],
        ),
        (
            "sample:unknown_block",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {
                            "predicate_type": "sample:tag_block_or_liquid",
                            "extra_block": "minecraft:no_such_block"
                        },
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/unknown_block.json",
                "sample:unknown_block",
                "sample:tag_block_or_liquid",
                "extra_block",
                "minecraft:no_such_block",
            ],
        ),
        (
            "sample:unknown_tag",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {
                            "predicate_type": "sample:tag_block_or_liquid",
                            "tag": "#sample:missing_tag"
                        },
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/unknown_tag.json",
                "sample:unknown_tag",
                "sample:tag_block_or_liquid",
                "tag",
                "sample:missing_tag",
            ],
        ),
        (
            "sample:refused_check",
            serde_json::json!({
                "processors": [{
                    "processor_type": "minecraft:rule",
                    "rules": [{
                        "input_predicate": {"predicate_type": "minecraft:always_true"},
                        "location_predicate": {"predicate_type": "minecraft:always_true"},
                        "position_predicate": {
                            "predicate_type": "sample:piece_and_y_band",
                            "min_y": 90,
                            "max_y": 10,
                            "max_template_x": 1
                        },
                        "output_state": {"id": "minecraft:bricks"}
                    }]
                }]
            }),
            &[
                "data/sample/worldgen/processor_list/refused_check.json",
                "sample:refused_check",
                "sample:piece_and_y_band",
                "position_predicate",
                "min_y",
                "exceeds max_y",
            ],
        ),
    ];

    let lists: BTreeMap<String, String> = cases
        .iter()
        .map(|(name, value, _)| ((*name).to_owned(), value.to_string()))
        .collect();
    let (resources, bindings) = test_env(lists);
    let rule_types: ModRuleTestTypes = BTreeMap::from([rule_type(
        "sample",
        "sample:tag_block_or_liquid",
        TagBlockOrLiquidType,
    )]);
    let pos_types: ModPosRuleTestTypes = BTreeMap::from([pos_rule_type(
        "sample",
        "sample:piece_and_y_band",
        PieceAndYBandType,
    )]);

    for &(list_name, ref list_value, expected) in cases {
        let err = with_test_rule_test_types(&rule_types, || {
            with_test_pos_rule_test_types(&pos_types, || {
                ProcessorList::decode_named(
                    list_name,
                    &list_value.to_string(),
                    resources,
                    &bindings,
                )
                .unwrap_err()
                .to_string()
            })
        });
        for needle in expected {
            assert!(
                err.contains(needle),
                "list {list_name}: expected `{needle}` in `{err}`"
            );
        }

        // Any template pool naming this broken processor list is left out of the load.
        let pool_name = format!("sample:pool_for_{}", list_name.split(':').nth(1).unwrap());
        let pool_json = serde_json::json!({
            "fallback": "minecraft:empty",
            "elements": [{
                "weight": 1,
                "element": {
                    "element_type": "minecraft:single_pool_element",
                    "location": "sample:room",
                    "processors": list_name,
                    "projection": "rigid"
                }
            }]
        })
        .to_string();
        let pools = Pools::new(BTreeMap::from([(pool_name.clone(), pool_json)]));
        let loaded = with_test_rule_test_types(&rule_types, || {
            with_test_pos_rule_test_types(&pos_types, || {
                pools.get(&pool_name, resources, &bindings)
            })
        });
        assert!(
            loaded.is_none(),
            "pool {pool_name} naming broken list {list_name} must be left out"
        );
    }
}

#[derive(Debug)]
struct BoomRule;
impl RuleTest for BoomRule {
    fn test(
        &self,
        _: &dyn RuleTestContext,
        _: BlockState,
        _: [i32; 3],
        _: &mut dyn RandomSource,
    ) -> bool {
        panic!("rule exploded");
    }
}
struct BoomRuleType;
impl RuleTestType for BoomRuleType {
    fn decode(
        &self,
        _: &dyn RuleTestDecodeContext,
        _: &Tag,
    ) -> Result<Arc<dyn RuleTest>, StructureError> {
        Ok(Arc::new(BoomRule))
    }
}

struct BoomDecodeRuleType;
impl RuleTestType for BoomDecodeRuleType {
    fn decode(
        &self,
        _: &dyn RuleTestDecodeContext,
        _: &Tag,
    ) -> Result<Arc<dyn RuleTest>, StructureError> {
        panic!("decode exploded");
    }
}

#[derive(Debug)]
struct BoomPosRule;
impl PosRuleTest for BoomPosRule {
    fn test(
        &self,
        _: &dyn PosRuleTestContext,
        _: [i32; 3],
        _: [i32; 3],
        _: [i32; 3],
        _: &mut dyn RandomSource,
    ) -> bool {
        panic!("pos rule exploded");
    }
}
struct BoomPosRuleType;
impl PosRuleTestType for BoomPosRuleType {
    fn decode(
        &self,
        _: &dyn PosRuleTestDecodeContext,
        _: &Tag,
    ) -> Result<Arc<dyn PosRuleTest>, StructureError> {
        Ok(Arc::new(BoomPosRule))
    }
}

#[test]
fn panicking_mod_rule_or_pos_rule_fails_placement_naming_mod_and_type_with_nothing_substituted() {
    let (resources, bindings) = test_env(BTreeMap::new());
    let rule_types: ModRuleTestTypes = BTreeMap::from([
        rule_type("sample", "sample:boom_rule", BoomRuleType),
        rule_type("sample", "sample:boom_decode", BoomDecodeRuleType),
        rule_type("wrapper", "wrapper:invert_rule", InvertRuleType),
    ]);
    let pos_types: ModPosRuleTestTypes =
        BTreeMap::from([pos_rule_type("sample", "sample:boom_pos", BoomPosRuleType)]);

    // Panicking decode is caught and names mod and type.
    let bad_decode = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {"predicate_type": "sample:boom_decode"},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();
    let decode_err = with_test_rule_test_types(&rule_types, || {
        ProcessorList::decode_named("sample:boom_list", &bad_decode, resources, &bindings)
            .unwrap_err()
            .to_string()
    });
    assert!(decode_err.contains("mod sample"), "{decode_err}");
    assert!(decode_err.contains("sample:boom_decode"), "{decode_err}");

    let template = three_block_template(resources);

    // Panicking RuleTest aborts structure placement with zero blocks written.
    let rule_list_json = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {"predicate_type": "sample:boom_rule"},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();
    let rule_list = with_test_rule_test_types(&rule_types, || {
        ProcessorList::decode(&rule_list_json, resources, &bindings).unwrap()
    });
    let mut world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::new(),
        writes: Vec::new(),
    };
    let mut settings = PlaceSettings::new(Rotation::None);
    settings.processors = rule_list.0.iter().collect();
    let mut random = LegacyRandom::new(0);
    let err = place_in_world(
        &mut world,
        resources,
        &template,
        [0, 64, 0],
        [0, 64, 0],
        &mut settings,
        Some(&mut random),
        2,
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("mod sample"), "{msg}");
    assert!(msg.contains("sample:boom_rule"), "{msg}");
    assert!(
        world.writes.is_empty(),
        "nothing may be substituted on panic"
    );

    // Panicking nested RuleTest cannot be swallowed by an outer mod wrapper.
    let nested_boom_json = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {
                    "predicate_type": "wrapper:invert_rule",
                    "inner": {"predicate_type": "sample:boom_rule"}
                },
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();
    let nested_list = with_test_rule_test_types(&rule_types, || {
        ProcessorList::decode(&nested_boom_json, resources, &bindings).unwrap()
    });
    let mut nested_world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::new(),
        writes: Vec::new(),
    };
    let mut nested_settings = PlaceSettings::new(Rotation::None);
    nested_settings.processors = nested_list.0.iter().collect();
    let nested_err = place_in_world(
        &mut nested_world,
        resources,
        &template,
        [0, 64, 0],
        [0, 64, 0],
        &mut nested_settings,
        Some(&mut random),
        2,
    )
    .unwrap_err()
    .to_string();
    assert!(nested_err.contains("mod sample"), "{nested_err}");
    assert!(nested_err.contains("sample:boom_rule"), "{nested_err}");
    assert!(nested_world.writes.is_empty());

    // Panicking PosRuleTest aborts structure placement with zero blocks written.
    let pos_list_json = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {"predicate_type": "minecraft:always_true"},
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "position_predicate": {"predicate_type": "sample:boom_pos"},
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();
    let pos_list = with_test_pos_rule_test_types(&pos_types, || {
        ProcessorList::decode(&pos_list_json, resources, &bindings).unwrap()
    });
    let mut pos_world = MemoryWorld {
        air: BlockStateId(0),
        blocks: BTreeMap::new(),
        writes: Vec::new(),
    };
    let mut pos_settings = PlaceSettings::new(Rotation::None);
    pos_settings.processors = pos_list.0.iter().collect();
    let pos_err = place_in_world(
        &mut pos_world,
        resources,
        &template,
        [0, 64, 0],
        [0, 64, 0],
        &mut pos_settings,
        Some(&mut random),
        2,
    )
    .unwrap_err()
    .to_string();
    assert!(pos_err.contains("mod sample"), "{pos_err}");
    assert!(pos_err.contains("sample:boom_pos"), "{pos_err}");
    assert!(pos_world.writes.is_empty());
}

#[test]
fn removed_sunrise_rule_tests_fail_processor_list_decode_without_a_mod() {
    let (resources, bindings) = test_env(BTreeMap::new());
    let matter = serde_json::json!({
        "processors": [{
            "processor_type": "minecraft:rule",
            "rules": [{
                "input_predicate": {
                    "predicate_type": "sunrise:matter_phase",
                    "phase_to_test_for": "AIR"
                },
                "location_predicate": {"predicate_type": "minecraft:always_true"},
                "output_state": {"id": "minecraft:bricks"}
            }]
        }]
    })
    .to_string();
    let err = ProcessorList::decode_named("sample:legacy_matter", &matter, resources, &bindings)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("data/sample/worldgen/processor_list/legacy_matter.json"),
        "{err}"
    );
    assert!(err.contains("sunrise:matter_phase"), "{err}");
    assert!(err.contains("predicate_type"), "{err}");

    for pos_type in [
        "sunrise:y_value",
        "sunrise:piece_origin_axis_aligned_linear_pos",
    ] {
        let json = serde_json::json!({
            "processors": [{
                "processor_type": "minecraft:rule",
                "rules": [{
                    "input_predicate": {"predicate_type": "minecraft:always_true"},
                    "location_predicate": {"predicate_type": "minecraft:always_true"},
                    "position_predicate": {"predicate_type": pos_type},
                    "output_state": {"id": "minecraft:bricks"}
                }]
            }]
        })
        .to_string();
        let err = ProcessorList::decode_named("sample:legacy_pos", &json, resources, &bindings)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("data/sample/worldgen/processor_list/legacy_pos.json"),
            "{err}"
        );
        assert!(err.contains(pos_type), "{err}");
        assert!(err.contains("predicate_type"), "{err}");
    }
}
