//! What a chunk generator's palette and a region render read of blocks: `BlockState.isSolid`,
//! the cached full collision test, the liquid container interfaces, and the owned metadata.
use crate::game::native_entity_refs::tests::with_game;
use sunrise_gameplay_common::game::native_block_tables::BlockTables;
use sunrise_world::registry::BlockStateId;

#[test]
#[ignore = "daily"]
fn solidity_and_full_collision_are_vanillas_cached_answers() {
    with_game(|game| {
        let tables = BlockTables::of(game);
        let registry = &game.common.pack.assets.registry;
        // `isSolid()` and `isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)`.
        for (block, solid, full) in [
            ("minecraft:stone", Some(true), Some(true)),
            // Solid and a full cube to collide with, though not to render.
            ("minecraft:oak_leaves", Some(true), Some(true)),
            // A collision shape a block and a half high.
            ("minecraft:oak_fence", Some(true), Some(false)),
            // No collision shape.
            ("minecraft:torch", Some(false), Some(false)),
            // `forceSolidOn`, though it has no collision shape.
            ("minecraft:oak_sign", Some(true), Some(false)),
            // `forceSolidOn`, though it has no collision shape.
            ("minecraft:cobweb", Some(true), Some(false)),
            // `forceSolidOff`, though its collision shape is a block high.
            ("minecraft:ladder", Some(false), Some(false)),
            // A bottom slab: half a block high, its bounds' average size 0.8333.
            ("minecraft:oak_slab", Some(true), Some(false)),
            // `dynamicShape`: no cache, so not solid, and no cached full-block answer.
            ("minecraft:scaffolding", Some(false), None),
            // `dynamicShape` too, but `forceSolidOn` comes first.
            ("minecraft:shulker_box", Some(true), None),
        ] {
            let state = registry
                .default_state(block)
                .unwrap_or_else(|| panic!("missing default state for {block}"));
            assert_eq!(
                tables.block_state_is_solid(state),
                solid,
                "{block} is solid"
            );
            assert_eq!(
                tables.block_state_collision_is_full_block(state),
                full,
                "{block} has a full collision shape"
            );
        }
        let unknown = BlockStateId(u32::MAX as _);
        assert_eq!(tables.block_state_is_solid(unknown), None);
        assert_eq!(tables.block_state_collision_is_full_block(unknown), None);
    });
}

#[test]
#[ignore = "daily"]
fn liquid_containers_and_waterloggable_blocks_are_their_interfaces() {
    with_game(|game| {
        let tables = BlockTables::of(game);
        let registry = &game.common.pack.assets.registry;
        // (`LiquidBlockContainer`, `SimpleWaterloggedBlock`), Java's `instanceof`.
        for (block, container, waterlogged) in [
            ("minecraft:oak_slab", true, true),
            ("minecraft:chest", true, true),
            // Holds water without a `waterlogged` property.
            ("minecraft:kelp", true, false),
            ("minecraft:seagrass", true, false),
            // `LiquidBlock` is a `BucketPickup`, not a container.
            ("minecraft:water", false, false),
            ("minecraft:stone", false, false),
        ] {
            let state = registry
                .default_state(block)
                .unwrap_or_else(|| panic!("missing default state for {block}"));
            assert_eq!(
                tables.block_state_is_liquid_block_container(state),
                Some(container),
                "{block} is a liquid container"
            );
            assert_eq!(
                tables.block_state_is_simple_waterlogged(state),
                Some(waterlogged),
                "{block} is waterloggable"
            );
        }
        let unknown = BlockStateId(u32::MAX as _);
        assert_eq!(tables.block_state_is_liquid_block_container(unknown), None);
        assert_eq!(tables.block_state_is_simple_waterlogged(unknown), None);
    });
}

#[test]
#[ignore = "daily"]
fn the_owned_block_metadata_is_read_on_a_spawned_thread() {
    with_game(|game| {
        let metadata = BlockTables::of(game)
            .block_metadata_snapshot()
            .expect("the block registry is complete");
        std::thread::spawn(move || {
            let stone = metadata.key("minecraft:stone").expect("stone has a key");
            assert_eq!(metadata.name(stone), Ok("minecraft:stone"));
            assert_eq!(metadata.key("minecraft:no_such_block"), None);
            let slab = metadata
                .key("minecraft:oak_slab")
                .expect("oak slab has a key");
            let default = metadata.default_state(slab).unwrap();
            assert_eq!(metadata.property(default, "type"), Ok(Some("bottom")));
            assert_eq!(metadata.property(default, "waterlogged"), Ok(Some("false")));
            let properties: Vec<_> = metadata
                .state_definition(slab)
                .unwrap()
                .iter()
                .map(|property| (property.name.to_owned(), property.values.to_vec()))
                .collect();
            assert_eq!(
                properties,
                [
                    (
                        "type".to_owned(),
                        vec!["top".to_owned(), "bottom".into(), "double".into()]
                    ),
                    (
                        "waterlogged".to_owned(),
                        vec!["true".to_owned(), "false".into()]
                    ),
                ]
            );
        })
        .join()
        .unwrap();
    });
}
