//! `BlockStates::block_class`: vanilla's class of a block, subclasses included, not a tag.
use mod_api::BlockClass::{
    self, Bonemealable, Entity, Fence, IronBars, Slab, StainedGlassPane, Stair, TrapDoor, Wall,
};
use sunrise_gameplay_common::game::native_block_tables::BlockTables;

const ALL: [BlockClass; 9] = [
    Stair,
    Entity,
    IronBars,
    StainedGlassPane,
    TrapDoor,
    Bonemealable,
    Slab,
    Wall,
    Fence,
];

#[test]
#[ignore = "daily"]
fn a_block_is_of_the_vanilla_classes_its_chain_names() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let tables = BlockTables::of(game);
        let registry = &game.pack.assets.registry;
        // Each block's classes, as Java's instanceof gives them: the classes it is, no others.
        for (block, classes) in [
            ("minecraft:oak_stairs", &[Stair][..]),
            // WeatheringCopperStairBlock extends StairBlock.
            ("minecraft:waxed_cut_copper_stairs", &[Stair][..]),
            ("minecraft:exposed_cut_copper_stairs", &[Stair][..]),
            ("minecraft:iron_bars", &[IronBars][..]),
            ("minecraft:glass_pane", &[IronBars][..]),
            (
                "minecraft:red_stained_glass_pane",
                &[IronBars, StainedGlassPane][..],
            ),
            ("minecraft:oak_trapdoor", &[TrapDoor][..]),
            ("minecraft:waxed_copper_trapdoor", &[TrapDoor][..]),
            ("minecraft:chest", &[Entity][..]),
            ("minecraft:furnace", &[Entity][..]),
            // BonemealableBlock: the blocks that take bone meal, a valid target here or not.
            ("minecraft:wheat", &[Bonemealable][..]),
            ("minecraft:oak_sapling", &[Bonemealable][..]),
            ("minecraft:grass_block", &[Bonemealable][..]),
            ("minecraft:rooted_dirt", &[Bonemealable][..]),
            ("minecraft:oak_slab", &[Slab][..]),
            // WeatheringCopperSlabBlock extends SlabBlock.
            ("minecraft:cut_copper_slab", &[Slab][..]),
            ("minecraft:cobblestone_wall", &[Wall][..]),
            ("minecraft:oak_fence", &[Fence][..]),
            ("minecraft:nether_brick_fence", &[Fence][..]),
            // FenceGateBlock is not a FenceBlock.
            ("minecraft:oak_fence_gate", &[][..]),
            ("minecraft:stone", &[][..]),
            ("minecraft:oak_planks", &[][..]),
        ] {
            let state = registry
                .default_state(block)
                .unwrap_or_else(|| panic!("missing default state for {block}"));
            for class in ALL {
                assert_eq!(
                    tables.block_state_class(state, class),
                    Ok(Some(classes.contains(&class))),
                    "{block} as {class:?}"
                );
            }
        }
        // A state no block has is unknown, not "not of the class".
        let unknown = sunrise_world::registry::BlockStateId(u32::MAX as _);
        assert_eq!(tables.block_state_class(unknown, Stair), Ok(None));
    });
}
