use super::*;
use std::collections::{BTreeMap, BTreeSet};
use sunrise_core::nbt::Tag;
use sunrise_world::{
    fluid::FluidState,
    heightmap::HeightmapType,
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        structure::{
            Rotation, StateTransforms, StructureResources, StructureTemplate,
            mod_processors::with_test_structure_processor_types,
            processor::{PlaceSettings, ProcessorList, place_in_world},
        },
    },
};

fn test_resources() -> &'static StructureResources {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = serde_json::json!({
        "states": [
            ["air", true, false, "", {"id": "minecraft:air", "properties": {}}],
            ["cave_air", true, false, "", {"id": "minecraft:cave_air", "properties": {}}],
            ["void_air", true, false, "", {"id": "minecraft:void_air", "properties": {}}],
            ["barrier", false, false, "", {"id": "minecraft:barrier", "properties": {}}],
            ["stone", false, false, "", {"id": "minecraft:stone", "properties": {}}],
            ["cobblestone", false, false, "", {"id": "minecraft:cobblestone", "properties": {}}],
            ["dirt", false, false, "", {"id": "minecraft:dirt", "properties": {}}],
            ["bricks", false, false, "", {"id": "minecraft:bricks", "properties": {}}]
        ],
        "defaults": {
            "minecraft:air": 0,
            "minecraft:cave_air": 1,
            "minecraft:void_air": 2,
            "minecraft:barrier": 3,
            "minecraft:stone": 4,
            "minecraft:cobblestone": 5,
            "minecraft:dirt": 6,
            "minecraft:bricks": 7
        }
    });
    let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    Box::leak(Box::new(StructureResources::new(
        Box::leak(Box::new(registry)),
        StateTransforms::default(),
        std::path::Path::new("/nonexistent"),
        BTreeMap::new(),
        &BTreeMap::new(),
    )))
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

/// A mod processor that delegates to a nested processor, replaces `stone` with
/// a configured target block during `process_block`, and writes a marker block
/// beneath the piece offset during `finalize_processing`.
#[derive(Debug)]
struct WeatherAndSupport {
    from: BlockState,
    to: BlockState,
    support: BlockState,
    nested: Option<StructureProcessorRef>,
}
impl StructureProcessor for WeatherAndSupport {
    fn process_block(
        &self,
        context: &mut dyn ProcessorContext,
        original: &StructureBlockInfo,
        mut current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError> {
        if let Some(nested) = &self.nested {
            let Some(next) = context.apply_processor(nested, original, current)? else {
                return Ok(None);
            };
            current = next;
        }
        if current.state == self.from {
            current.state = self.to;
        }
        Ok(Some(current))
    }
    fn finalize_processing(
        &self,
        context: &mut dyn ProcessorFinalizeContext,
        _original: &[StructureBlockInfo],
        processed: Vec<StructureBlockInfo>,
    ) -> Result<Vec<StructureBlockInfo>, StructureError> {
        let [x, y, z] = context.offset();
        context.set_block([x, y - 1, z], self.support, BlockUpdateFlags::CLIENTS);
        Ok(processed)
    }
}

struct WeatherAndSupportType;
impl StructureProcessorType for WeatherAndSupportType {
    fn decode(
        &self,
        context: &dyn ProcessorDecodeContext,
        config: &Tag,
    ) -> Result<Arc<dyn StructureProcessor>, StructureError> {
        let block = |key: &str| -> Result<BlockState, StructureError> {
            let Some(Tag::String(name)) = config.get(key) else {
                return Err(StructureError::new(std::io::Error::other(format!(
                    "missing string field {key}"
                ))));
            };
            let id = mod_api::__private::block_key(&name.to_string_lossy()).ok_or_else(|| {
                StructureError::new(std::io::Error::other(format!("unknown block {name:?}")))
            })?;
            Ok(context.state(id))
        };
        let nested = config
            .get("delegate")
            .map(|tag| context.processor(tag))
            .transpose()?;
        Ok(Arc::new(WeatherAndSupport {
            from: block("from")?,
            to: block("to")?,
            support: block("support")?,
            nested,
        }))
    }
}

/// A mod processor decodes from JSON, transforms blocks alongside a vanilla
/// processor (`minecraft:block_ignore`), delegates to a nested vanilla
/// processor, and writes via `finalize_processing` while leaving vanilla
/// processors unchanged.
#[test]
fn mod_structure_processor_runs_beside_vanilla_and_finalizes() {
    let resources = test_resources();
    let bindings = FeatureBlockBindings::new(resources.registry, BTreeSet::new()).unwrap();
    let cobble = resources
        .registry
        .default_state("minecraft:cobblestone")
        .unwrap();
    let bricks = resources
        .registry
        .default_state("minecraft:bricks")
        .unwrap();
    let air = resources.registry.default_state("minecraft:air").unwrap();

    let mut types = ModStructureProcessorTypes::new();
    types.insert(
        "testmod:weather".to_owned(),
        Arc::new(ProcessorType {
            inner: Arc::new(WeatherAndSupportType),
        }),
    );

    let list_json = serde_json::json!({
        "processors": [
            {
                "processor_type": "minecraft:block_ignore",
                "blocks": [{"id": "minecraft:dirt"}]
            },
            {
                "processor_type": "testmod:weather",
                "from": "minecraft:stone",
                "to": "minecraft:cobblestone",
                "support": "minecraft:bricks",
                "delegate": {
                    "processor_type": "minecraft:nop"
                }
            }
        ]
    });

    let list = with_test_structure_processor_types(&types, || {
        ProcessorList::decode(&list_json.to_string(), resources, &bindings).unwrap()
    });

    let template_tag = sunrise_core::json_nbt::json_to_tag(&serde_json::json!({
        "size": [2, 1, 1],
        "palette": [
            {"Name": "minecraft:stone"},
            {"Name": "minecraft:dirt"}
        ],
        "blocks": [
            {"pos": [0, 0, 0], "state": 0},
            {"pos": [1, 0, 0], "state": 1}
        ]
    }))
    .unwrap();
    let template =
        StructureTemplate::from_nbt(&template_tag, resources.registry, &resources.transforms);
    let mut world = MemoryWorld {
        air,
        blocks: BTreeMap::new(),
        writes: Vec::new(),
    };
    let mut settings = PlaceSettings::new(Rotation::None).with_processors(&list);
    settings.known_shape = true;
    settings.waterlogging = false;

    assert!(
        place_in_world(
            &mut world,
            resources,
            &template,
            [10, 64, 10],
            [10, 64, 10],
            &mut settings,
            None,
            2,
        )
        .unwrap()
    );
    // Mod processor replaced stone with cobblestone.
    assert_eq!(world.blocks.get(&[10, 64, 10]), Some(&cobble));
    // Vanilla minecraft:block_ignore dropped dirt unchanged.
    assert!(!world.blocks.contains_key(&[11, 64, 10]));
    // finalize_processing wrote bricks below the piece offset.
    assert_eq!(world.blocks.get(&[10, 63, 10]), Some(&bricks));
}

/// Unknown processor types and mod decode errors fail processor list decode
/// cleanly with an error.
#[test]
fn unknown_or_invalid_mod_processor_fails_decode_cleanly() {
    let resources = test_resources();
    let bindings = FeatureBlockBindings::new(resources.registry, BTreeSet::new()).unwrap();
    let mut types = ModStructureProcessorTypes::new();
    types.insert(
        "testmod:weather".to_owned(),
        Arc::new(ProcessorType {
            inner: Arc::new(WeatherAndSupportType),
        }),
    );

    with_test_structure_processor_types(&types, || {
        let unknown = serde_json::json!([{"processor_type": "testmod:missing"}]);
        assert!(ProcessorList::decode(&unknown.to_string(), resources, &bindings).is_err());

        let invalid = serde_json::json!([{"processor_type": "testmod:weather"}]);
        assert!(ProcessorList::decode(&invalid.to_string(), resources, &bindings).is_err());
    });
}
