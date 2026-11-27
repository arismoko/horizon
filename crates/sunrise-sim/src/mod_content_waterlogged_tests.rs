use super::*;
use crate::mobs::{
    data::{MobData, PathComputationType, VALID_EMPTY_SPAWN},
    path::PathType,
};
use std::collections::HashMap;
use sunrise_assets::resource_source::ResourceRoot;
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use sunrise_world::{
    fluid::{FluidKind, FluidState},
    level::{
        Behaviours, BlockEntityNeighbors, BucketContent, LevelAccess, LevelEvent, PlaceContext,
        PlaceOutcome, PlacementContext, Ticks,
    },
    registry::BlockStateId,
};

struct Level {
    air: BlockStateId,
    blocks: HashMap<BlockPos, BlockStateId>,
    random: LegacyRandom,
    placement_answer: Option<BlockStateId>,
    shape_answer: bool,
    client: bool,
    time: i64,
}
impl BlockEntityNeighbors for Level {}
impl LevelAccess for Level {
    fn block_state(&self, p: BlockPos) -> BlockStateId {
        self.blocks.get(&p).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, p: BlockPos, s: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(p, s).unwrap_or(self.air))
    }
    fn is_loaded(&self, _: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn emit(&mut self, _: LevelEvent) {}
    fn is_client_side(&self) -> bool {
        self.client
    }
    fn before_state_for_placement(
        &self,
        _: &Ticks,
        _: Option<&sunrise_world::level::PreviewBlocks>,
        _: BlockStateId,
        _: &PlacementContext,
        _: FluidState,
        answer: &mut Option<Option<BlockStateId>>,
    ) {
        *answer = self.placement_answer.map(Some);
    }
    fn before_update_shape(
        &self,
        _: &mut Ticks,
        _: BlockPos,
        state: BlockStateId,
        _: Direction,
        _: BlockPos,
        _: BlockStateId,
        answer: &mut Option<BlockStateId>,
    ) {
        if self.shape_answer {
            *answer = Some(state);
        }
    }
}

#[test]
#[ignore = "daily"]
fn declared_waterlogged_blocks_keep_their_class_and_gain_liquid_defaults() {
    let Some(base) = prepared() else { return };
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-declared-waterlogged-{}",
        std::process::id()
    ));
    for (template, reason) in [
        ("seagrass", "already holds water"),
        ("tall_seagrass", "already holds water"),
        ("kelp", "already holds water"),
        ("kelp_plant", "already holds water"),
        ("bubble_column", "already holds water"),
        ("lava", "standalone liquid block"),
        ("oak_stairs", "already on"),
    ] {
        let json = format!(
            r#"{{"template":"minecraft:{template}","item":false,"state_properties":{{"waterlogged":{{"type":"boolean"}}}},"default_state":{{"waterlogged":"true"}}}}"#
        );
        let file = "data/wet/sunrise/block/aquatic/plant.json";
        let rejected = folder_mod(
            &scratch.join(format!("refused-{template}")),
            &[("sunrise-mod.toml", &manifest("wet", "")), (file, &json)],
        );
        let error = resource_root_fixture(
            &base,
            Path::new(""),
            &[rejected],
            &scratch.join("cache-refused"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.starts_with(&format!("{file}:"))
                && error.contains("wet:aquatic/plant")
                && error.contains("state_properties.waterlogged")
                && error.contains(&format!("minecraft:{template}"))
                && error.contains(reason),
            "{error}"
        );
    }
    let m = folder_mod(
        &scratch.join("mod"),
        &[
            ("sunrise-mod.toml", &manifest("wet", "")),
            (
                "data/wet/sunrise/block/plant.json",
                r#"{"template":"minecraft:tall_grass","state_properties":{"waterlogged":{"type":"boolean"}},"default_state":{"waterlogged":"true"}}"#,
            ),
            (
                "data/wet/sunrise/block/stone.json",
                r#"{"template":"minecraft:stone","state_properties":{"waterlogged":{"type":"boolean"}},"collision_shapes":{"default":[]}}"#,
            ),
            (
                "data/wet/sunrise/block/stairs.json",
                r#"{"template":"minecraft:oak_stairs","default_state":{"waterlogged":"true"}}"#,
            ),
            (
                "data/wet/sunrise/item/tools/chisel.json",
                r#"{"template":"minecraft:stick"}"#,
            ),
        ],
    );
    let root = resource_root_fixture(&base, Path::new(""), &[m], &scratch.join("cache")).unwrap();
    let registry = sunrise_world::level::data::load_registry(&root).unwrap();
    let b = Behaviours::load(&registry, &root).unwrap();
    let air = registry.default_state("minecraft:air").unwrap();
    let water = registry.default_state("minecraft:water").unwrap();
    let stone = registry.default_state("wet:stone").unwrap();
    let plant = registry.default_state("wet:plant").unwrap();
    let dry_plant = b.table.set_bool(plant, b.props.waterlogged, false);
    let stairs = registry.default_state("wet:stairs").unwrap();
    let wet_stone = b.table.set_bool(stone, b.props.waterlogged, true);
    let block_flags: Vec<(bool, bool)> =
        serde_json::from_slice(&fs::read(root.join("worldgen/block_flags.json")).unwrap()).unwrap();
    assert_eq!(block_flags[plant.0 as usize], (false, true));
    assert_eq!(block_flags[dry_plant.0 as usize], (false, false));

    let facts = BlockFacts::load(&root, &registry).unwrap();
    let mobs = MobData::load(&ResourceRoot::directory(root.clone()), facts.states.len()).unwrap();
    assert_eq!(mobs.path_type(plant), PathType::Water);
    assert_eq!(mobs.path_type(dry_plant), PathType::Open);
    assert!(mobs.pathfindable(plant, PathComputationType::Water));
    assert!(!mobs.pathfindable(dry_plant, PathComputationType::Water));
    assert_eq!(mobs.state(plant).flags & VALID_EMPTY_SPAWN, 0);
    assert_ne!(mobs.state(dry_plant).flags & VALID_EMPTY_SPAWN, 0);
    assert_eq!(mobs.path_type(wet_stone), PathType::Blocked);
    assert_eq!(b.fluid(stairs), FluidState::WATER);
    // A mod block on the stair template is of the stair class, as `BlockStates::block_class` reads it.
    assert!(
        b.data(stairs)
            .is(sunrise_world::level::classes::Class::Stair)
    );
    assert!(
        !b.data(stone)
            .is(sunrise_world::level::classes::Class::Stair)
    );
    // A mod block's item, whose prototype carries no `item_name`, is named by its block's key.
    let blocks: &'static sunrise_world::registry::BlockRegistry = Box::leak(Box::new(
        sunrise_world::level::data::load_registry(&root).unwrap(),
    ));
    let names = crate::item_names::Names::default().with_item_names(&Default::default(), blocks);
    for (item, key) in [
        ("wet:stone", "block.wet.stone"),
        ("wet:plain_item", "item.wet.plain_item"),
        ("wet:tools/chisel", "item.wet.tools.chisel"),
    ] {
        let name = crate::stations::hover_name_text(&names, &crate::items::ItemStack::new(item, 1));
        assert_eq!(name.component()["translate"], key, "{item}");
    }
    assert!(!mobs.pathfindable(stairs, PathComputationType::Water));

    let is_waterlogged = |state| b.table.bool(state, b.props.waterlogged);
    let pos = BlockPos::new(4, 64, 4);
    let above = BlockPos::new(4, 65, 4);
    let beside = BlockPos::new(5, 64, 4);
    let context = |item: &str| PlaceContext {
        clicked_pos: pos,
        face: Direction::Up,
        hit: Vec3::new(4.5, 64.0, 4.5),
        inside: false,
        source: sunrise_world::level::PlacementSource::Player {
            yaw: 0.0,
            pitch: 0.0,
            sneaking: false,
        },
        item: item.into(),
        can_use_game_master_blocks: false,
    };
    for client in [false, true] {
        let mut level = Level {
            air,
            blocks: HashMap::new(),
            random: LegacyRandom::new(1),
            placement_answer: None,
            shape_answer: false,
            client,
            time: 0,
        };
        let mut ticks = Ticks::default();
        ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
        level.blocks.insert(pos, water);
        let wet = b
            .state_for_placement(
                &mut level,
                &mut ticks,
                b.block(stone),
                &context("wet:stone"),
            )
            .unwrap()
            .unwrap();
        assert!(is_waterlogged(wet));
        assert_eq!(b.fluid(wet), FluidState::WATER);
        level
            .blocks
            .insert(pos, b.table.set_int(water, b.props.level, 1));
        let flowing = b
            .state_for_placement(
                &mut level,
                &mut ticks,
                b.block(stone),
                &context("wet:stone"),
            )
            .unwrap()
            .unwrap();
        assert!(
            !is_waterlogged(flowing),
            "only source water waterlogs the placed half"
        );
        level.blocks.insert(pos, air);
        let dry = b
            .state_for_placement(
                &mut level,
                &mut ticks,
                b.block(stone),
                &context("wet:stone"),
            )
            .unwrap()
            .unwrap();
        assert!(!is_waterlogged(dry));
        assert_eq!(b.fluid(dry).kind(), FluidKind::Empty);
        level.blocks.insert(pos, water);
        level.placement_answer = Some(dry);
        assert_eq!(
            b.state_for_placement(
                &mut level,
                &mut ticks,
                b.block(stone),
                &context("wet:stone")
            ),
            Ok(Some(dry)),
            "Part answer overrides the water default"
        );
        level.placement_answer = None;
        level.blocks.insert(pos, dry);
        assert!(b.is_liquid_container(dry));
        if !client {
            assert_eq!(
                b.bucket_empty(
                    &mut level,
                    &mut ticks,
                    pos,
                    None,
                    BucketContent::Fluid(FluidKind::Water),
                    "minecraft:item.bucket.empty",
                    false,
                    false
                ),
                Some(pos)
            );
            assert!(is_waterlogged(level.block_state(pos)));
            assert_eq!(
                b.bucket_pickup(&mut level, &mut ticks, pos, false),
                Some("minecraft:water_bucket")
            );
            assert_eq!(level.block_state(pos), dry);
        }
        // Each half copies its own cell: submerged lower, dry upper.
        level.blocks.insert(pos, water);
        level.blocks.insert(above, air);
        level.blocks.insert(
            BlockPos::new(4, 63, 4),
            registry.default_state("minecraft:dirt").unwrap(),
        );
        assert!(matches!(
            b.place_block(&mut level, &mut ticks, &context("wet:plant")),
            PlaceOutcome::Placed { .. }
        ));
        assert!(is_waterlogged(level.block_state(pos)));
        assert!(!is_waterlogged(level.block_state(above)));
        assert_eq!(
            registry.state(level.block_state(above)).unwrap().properties["half"],
            "upper"
        );
        assert_eq!(b.block(level.block_state(above)), b.block(plant));
        if !client {
            level.blocks.insert(BlockPos::new(4, 63, 4), air);
            assert_eq!(
                b.bucket_pickup(&mut level, &mut ticks, pos, false),
                Some("minecraft:water_bucket")
            );
            assert_eq!(
                level.block_state(pos),
                air,
                "a plant that cannot survive drained breaks"
            );
            // A shape Part suppresses the default tick; without it, a neighbour update schedules it.
            let mut shape_ticks = Ticks::default();
            shape_ticks.add_empty_chunk(ChunkPos::new(0, 0)).unwrap();
            level.blocks.insert(pos, wet);
            level.shape_answer = true;
            b.set_block(
                &mut level,
                &mut shape_ticks,
                beside,
                registry.default_state("minecraft:stone").unwrap(),
                3,
            );
            assert!(!shape_ticks.fluids.has_scheduled(
                pos,
                sunrise_core::ticks::TickTypeId(FluidKind::Water.tick_id())
            ));
            level.shape_answer = false;
            level.blocks.insert(
                BlockPos::new(4, 63, 4),
                registry.default_state("minecraft:stone").unwrap(),
            );
            b.set_block(&mut level, &mut shape_ticks, beside, air, 3);
            assert!(shape_ticks.fluids.has_scheduled(
                pos,
                sunrise_core::ticks::TickTypeId(FluidKind::Water.tick_id())
            ));
            assert_eq!(level.block_state(beside), air);
            level.time = 5;
            b.tick_scheduled(&mut level, &mut shape_ticks);
            assert_eq!(
                b.fluid(level.block_state(beside)).kind(),
                FluidKind::FlowingWater
            );
        }
    }
    fs::remove_dir_all(scratch).unwrap();
}
