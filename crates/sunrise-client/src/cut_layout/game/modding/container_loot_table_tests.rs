//! A mod container's loot table and seed (`RandomizableContainer.setLootTable`,
//! `unpackLootTable`, `LootTable.fill`), rolled by its first access. The fixture installs the
//! process's block-entity type table, so each test runs in a fresh process.
use super::quiet_write_tests::{KEY, POS, SLOTS, with_crate};
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::{ServerGame, commands::data::DataRef};
use sunrise_gameplay_common::game::test_fixtures::run_fresh_cases;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;

const TABLE: &str = "minecraft:chests/simple_dungeon";

/// Vanilla 26.3's `LootTable.fill` of `TABLE` with seed 42 into an empty 27-slot container:
/// the non-empty slots `VanillaLootFillOracle` prints from the server jar
/// (`mods/sunrise/src/test/java/dev/sunriselab/parity/VanillaLootFillOracle.java`, arguments
/// `out.txt minecraft:chests/simple_dungeon 42`).
const VANILLA_42: &[(usize, &str, u32)] = &[
    (2, "minecraft:leather", 1),
    (3, "minecraft:gunpowder", 1),
    (6, "minecraft:string", 1),
    (7, "minecraft:leather", 1),
    (8, "minecraft:leather", 1),
    (9, "minecraft:gunpowder", 1),
    (10, "minecraft:rotten_flesh", 2),
    (11, "minecraft:coal", 2),
    (12, "minecraft:gunpowder", 1),
    (13, "minecraft:coal", 2),
    (14, "minecraft:leather", 1),
    (15, "minecraft:wheat", 1),
    (16, "minecraft:leather", 2),
    (19, "minecraft:string", 3),
    (22, "minecraft:name_tag", 1),
    (24, "minecraft:wheat", 1),
    (25, "minecraft:wheat", 1),
    (26, "minecraft:rotten_flesh", 1),
];

/// The same fill into the 27 slots with a stone already in slots 0 and 13: the same oracle
/// with those two stacks set before `getAvailableSlots`, which lists and shuffles only the
/// empty slots.
const VANILLA_42_AROUND_STONE: &[(usize, &str, u32)] = &[
    (0, "minecraft:stone", 1),
    (1, "minecraft:leather", 1),
    (2, "minecraft:name_tag", 1),
    (5, "minecraft:coal", 1),
    (6, "minecraft:rotten_flesh", 1),
    (7, "minecraft:wheat", 1),
    (8, "minecraft:gunpowder", 2),
    (11, "minecraft:leather", 1),
    (12, "minecraft:wheat", 1),
    (13, "minecraft:stone", 1),
    (15, "minecraft:string", 2),
    (16, "minecraft:coal", 1),
    (17, "minecraft:rotten_flesh", 2),
    (18, "minecraft:leather", 2),
    (19, "minecraft:wheat", 1),
    (21, "minecraft:coal", 2),
    (22, "minecraft:string", 2),
    (23, "minecraft:leather", 1),
    (25, "minecraft:leather", 1),
    (26, "minecraft:gunpowder", 1),
];

fn expected(rows: &[(usize, &str, u32)]) -> Vec<Option<(String, u32)>> {
    let mut slots = vec![None; SLOTS];
    for (slot, item, count) in rows {
        slots[*slot] = Some(((*item).to_owned(), *count));
    }
    slots
}

fn summary(slots: &[Option<ItemStack>]) -> Vec<Option<(String, u32)>> {
    slots
        .iter()
        .map(|slot| slot.as_ref().map(|s| (s.item.to_string(), s.count)))
        .collect()
}

fn slots(game: &ServerGame<'_>) -> Vec<Option<(String, u32)>> {
    summary(game.world.block_entities[&KEY].slots())
}

fn saved(game: &mut ServerGame<'_>) -> Tag {
    game.data_of(&DataRef::Block(POS)).unwrap()
}

#[test]
#[ignore = "daily"]
fn a_mod_containers_loot_table_rolls_once_with_fills_placement() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_mod_containers_loot_table_rolls_once_with_fills_placement"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_CONTAINER_LOOT_TABLE_CHILD",
        &["loot"],
    ) {
        return;
    }
    loot_case();
}

/// The table and seed wait beside the container, saved as vanilla's keys; the first look
/// into it rolls them into its slots as vanilla's `LootTable.fill` does, and only once.
/// Setting the table again with its unqualified name or the same seed changes nothing.
fn loot_case() {
    with_crate(|game, handle| {
        let empty = vec![None; SLOTS];
        assert_eq!(handle.set_loot_table(game, TABLE, 42), Ok(true));
        assert_eq!(handle.set_loot_table(game, TABLE, 42), Ok(false));
        assert_eq!(
            handle.set_loot_table(game, "chests/simple_dungeon", 42),
            Ok(false),
            "the table it already waits with"
        );
        assert_eq!(handle.set_loot_table(game, "Not An Id", 42), Ok(false));
        let saved_data = saved(game);
        assert_eq!(
            saved_data.get("LootTable"),
            Some(&Tag::String(TABLE.into()))
        );
        assert_eq!(saved_data.get("LootTableSeed"), Some(&Tag::Long(42)));
        assert_eq!(saved_data.get("Items"), None);
        assert_eq!(slots(game), empty, "nothing rolls before an access");

        game.loot_inventory_item(KEY, 0);
        assert_eq!(slots(game), expected(VANILLA_42));
        assert_eq!(saved(game).get("LootTable"), None, "the table is consumed");
        game.loot_inventory_item(KEY, 0);
        assert_eq!(
            slots(game),
            expected(VANILLA_42),
            "a second access rolls nothing"
        );

        // A seed of 0 is saved as no seed; asking for it again is no change either.
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&KEY) {
            entity.items = vec![None; SLOTS];
        }
        assert_eq!(handle.set_loot_table(game, TABLE, 0), Ok(true));
        assert_eq!(saved(game).get("LootTableSeed"), None);
        assert_eq!(
            handle.set_loot_table(game, "chests/simple_dungeon", 0),
            Ok(false)
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_filled_mod_container_keeps_its_stacks_and_fills_the_empty_slots() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_filled_mod_container_keeps_its_stacks_and_fills_the_empty_slots"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_CONTAINER_LOOT_FILLED_CHILD",
        &["filled"],
    ) {
        return;
    }
    // `setLootTable` on a container with stacks, then `unpackLootTable` through the public
    // handle: `LootTable.fill` fills the empty slots around them.
    with_crate(|game, handle| {
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&KEY) {
            for slot in [0, 13] {
                entity.items[slot] = Some(ItemStack::new("minecraft:stone", 1));
            }
        }
        assert_eq!(handle.set_loot_table(game, TABLE, 42), Ok(true));
        assert_eq!(handle.unpack_loot_table(game), Ok(true));
        assert_eq!(slots(game), expected(VANILLA_42_AROUND_STONE));
        assert_eq!(handle.unpack_loot_table(game), Ok(false), "rolled once");
    });
}

#[test]
#[ignore = "daily"]
fn a_data_merge_keeps_a_mod_containers_waiting_table() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_data_merge_keeps_a_mod_containers_waiting_table"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_CONTAINER_LOOT_MERGE_CHILD",
        &["merge"],
    ) {
        return;
    }
    // `BlockDataAccessor.setData` loads the merged data, its `LootTable` with it: the table
    // still waits after a merge of another field, and the first access rolls it.
    with_crate(|game, handle| {
        assert_eq!(handle.set_loot_table(game, TABLE, 42), Ok(true));
        assert!(game.as_console(|g| g.run_command("data merge block 2 64 2 {fuel:5}")));
        let saved_data = saved(game);
        assert_eq!(saved_data.get("fuel"), Some(&Tag::Int(5)));
        assert_eq!(
            saved_data.get("LootTable"),
            Some(&Tag::String(TABLE.into()))
        );
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&KEY) else {
            panic!("the crate's block entity")
        };
        assert_eq!(
            entity.nbt.get(&"LootTable".into()),
            None,
            "the table waits beside it"
        );
        game.loot_inventory_item(KEY, 0);
        assert_eq!(slots(game), expected(VANILLA_42));
    });
}

#[test]
#[ignore = "daily"]
fn a_saved_mod_container_loads_its_table_before_its_items() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_saved_mod_container_loads_its_table_before_its_items"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_CONTAINER_LOOT_LOAD_CHILD",
        &["load"],
    ) {
        return;
    }
    // ChestBlockEntity.loadAdditional skips saved items when there is a loot table.
    with_crate(|game, handle| {
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&KEY) {
            entity.items[0] = Some(ItemStack::new("minecraft:stone", 1));
        }
        assert_eq!(handle.set_loot_table(game, TABLE, 42), Ok(true));
        let registry = game.pack.assets.registry;
        let mut batch = sunrise_sim::save::chunks_with_block_entities(
            game.world.chunks.values(),
            &game.world.block_entities,
            &game.world.block_names,
        );
        game.attach_save_data(&mut batch[0]);
        let encoded =
            sunrise_sim::save::encode(&batch[0], registry, game.res.biomes(), &Default::default());
        let loaded = sunrise_sim::save::decode(&encoded, registry, game.res.biomes()).unwrap();
        let pending = loaded
            .saved
            .as_ref()
            .unwrap()
            .pending_loot
            .get(&KEY)
            .cloned();
        assert_eq!(pending.map(|p| p.table_id()), Some(TABLE.to_owned()));
        let (_, entity) = loaded
            .block_entities
            .iter()
            .find(|(p, _)| *p == KEY)
            .unwrap();
        assert_eq!(
            sunrise_sim::save::block_mod_item_slots("fixture:crate"),
            SLOTS
        );
        assert_eq!(entity.slots(), &[], "from a chunk");
        let BlockEntity::Mod(loaded_entity) = entity else {
            panic!("the crate's block entity")
        };
        assert_eq!(loaded_entity.nbt.get(&"LootTable".into()), None);

        let structure = sunrise_core::nbt::parse_snbt(
            r#"{fuel:4,Items:[{Slot:0b,id:"minecraft:stone",count:1}],LootTable:"minecraft:chests/simple_dungeon",LootTableSeed:42L}"#,
        )
        .unwrap()
        .0;
        let Tag::Compound(structure) = structure else {
            unreachable!()
        };
        let (entity, pending) =
            sunrise_sim::structure_blocks::generated_container("fixture:crate", &structure)
                .unwrap();
        assert_eq!(pending.map(|p| p.table_id()), Some(TABLE.to_owned()));
        assert_eq!(entity.slots(), &[], "from a structure");
    });
}

#[test]
#[ignore = "daily"]
fn a_slotless_mod_block_entitys_loot_fields_stay_in_a_captured_structure() {
    if run_fresh_cases(
        concat!(
            module_path!(),
            "::a_slotless_mod_block_entitys_loot_fields_stay_in_a_captured_structure"
        )
        .split_once("::")
        .unwrap()
        .1,
        "SUNRISE_CONTAINER_LOOT_SLOTLESS_CHILD",
        &["slotless"],
    ) {
        return;
    }
    // A mod block entity at a block with no container slots owns its `LootTable` and
    // `LootTableSeed` fields: the structure capture writes them as they are.
    with_crate(|game, _| {
        let at = (3, 64, 2);
        let saved = sunrise_core::nbt::parse_snbt(
            r#"{id:"fixture:crate_entity",x:3,y:64,z:2,LootTable:"fixture:own",LootTableSeed:5L}"#,
        )
        .unwrap()
        .0;
        let (_, entity) = sunrise_sim::save::decode_block_entity(&saved).unwrap();
        game.world.block_entities.insert(at, entity);
        let mut tag = saved.clone();
        ServerGame::write_pending_loot(&game.world, at, "minecraft:stone", &mut tag);
        assert_eq!(tag, saved);
    });
}
