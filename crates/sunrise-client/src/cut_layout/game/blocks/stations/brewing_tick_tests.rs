use sunrise_game_data::gamerules::Value;
use sunrise_gameplay::game::game_tests::tests::server_game;
use sunrise_items::items::ItemStack;
use sunrise_level::{brewing::BrewingStand, containers::BlockEntity};

/// A stand whose block state already shows the bottles it holds, so a tick
/// only reports what it changes.
fn stand(items: [Option<ItemStack>; 5], brew_time: i32, fuel: i32) -> BlockEntity {
    let bottles = [items[0].is_some(), items[1].is_some(), items[2].is_some()];
    BlockEntity::Brewing(Box::new(BrewingStand {
        items,
        brew_time,
        total_brew_time: 400,
        fuel,
        total_fuel: 20,
        speed: 1.0,
        ingredient: None,
        last_bottles: Some(bottles),
    }))
}

fn place(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
    entity: BlockEntity,
) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:brewing_stand")
        .unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
    game.world.block_entities.insert(pos, entity);
}

/// `BrewingStandBlockEntity.doBrew` drops the ingredient's remainder with
/// `Containers.dropItemStack`: no `block_drops` test (unlike `Block.popResource`),
/// and the item leaves with no pickup delay.
#[test]
#[ignore = "daily"]
fn a_brewed_remainder_ignores_the_block_drops_rule() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    assert!(game.ui.rules.set("block_drops", Value::Bool(false)));
    let pos = (8, 64, 8);
    place(
        &mut game,
        pos,
        stand(
            [
                Some(ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:awkward")),
                None,
                None,
                // Two: the second one leaves the glass bottle behind.
                Some(ItemStack::new("minecraft:dragon_breath", 2)),
                None,
            ],
            1,
            5,
        ),
    );

    game.tick_brewing_stand(pos).expect("the stand ticks");

    let bottle = game
        .dropped
        .iter()
        .find(|i| &*i.stack.item == "minecraft:glass_bottle")
        .expect("the dragon's breath left its bottle");
    assert_eq!(bottle.pickup_delay, 0, "Containers.dropItemStack has none");
    let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
        panic!("no brewing stand");
    };
    let brewed = b.items[0]
        .as_ref()
        .expect("the potion slot still holds one");
    assert_eq!(&*brewed.item, "minecraft:lingering_potion");
    assert_eq!(brewed.potion(), Some("minecraft:awkward"));
    assert_eq!(b.items[3].as_ref().unwrap().count, 1, "one breath is left");
}

/// `BrewingStandBlockEntity.serverTick` calls `setChanged` whenever it takes a
/// fuel item, even when the item's `brewing_fuel` gives 0 uses: the eaten item
/// must not come back from the next save.
#[test]
#[ignore = "daily"]
fn a_fuel_item_with_no_uses_marks_the_chunk() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let pos = (8, 64, 8);
    let mut fuel = ItemStack::new("minecraft:blaze_powder", 1);
    fuel.data_mut().set_loot_component(
        "minecraft:brewing_fuel",
        &Some(sunrise_game_data::scoreboard::json_to_tag(
            &serde_json::json!({"uses": 0, "speed_multiplier": 1.0}),
        )),
    );
    place(
        &mut game,
        pos,
        stand([None, None, None, None, Some(fuel)], 0, 0),
    );
    game.world.modified.clear();

    game.tick_brewing_stand(pos).expect("the stand ticks");

    let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
        panic!("no brewing stand");
    };
    assert!(b.items[4].is_none(), "the fuel item was taken");
    assert_eq!((b.fuel, b.brew_time), (0, 0), "0 uses brew nothing");
    assert!(
        game.world.modified.contains(&(0, 0)),
        "the taken fuel left the chunk unmarked"
    );
}

/// `BrewingStandBlockEntity.serverTick` reads the fuel's `speed_multiplier`
/// provider from the server's reloadable registries: a world's data pack that
/// defines it decides the brew time (`ceil(400 / speed)`), as the stand's own
/// ticks count it down.
#[test]
#[ignore = "daily"]
fn a_world_pack_provider_sets_the_brew_time() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    let dir = std::env::temp_dir().join(format!(
        "sunrise-brewing-world-providers-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let pack = dir.join("datapacks/bp/data/parity");
    for (registry, name, value) in [
        ("context_int_provider", "uses", "5"),
        ("context_float_provider", "speed", "8.0"),
    ] {
        std::fs::create_dir_all(pack.join(registry)).unwrap();
        std::fs::write(pack.join(registry).join(format!("{name}.json")), value).unwrap();
    }
    std::fs::write(
        dir.join("datapacks/bp/pack.mcmeta"),
        r#"{"pack":{"description":"fixture"}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("datapacks.json"),
        r#"{"enabled":["file/bp"],"disabled":[]}"#,
    )
    .unwrap();
    game.server.world_root = dir.clone();
    let root = game.options.resources.clone();
    let stack = sunrise_gameplay::game::functions::pack_stack(
        &game.server.world_root,
        &root,
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
        &Default::default(),
        false,
        &[],
        &[],
    );
    game.server.world_tables.0 = game
        .stage_world_tables(&stack, false, &root)
        .unwrap()
        .tables;
    let _ = std::fs::remove_dir_all(&dir);

    let mut fuel = ItemStack::new("minecraft:blaze_powder", 1);
    fuel.data_mut().set_loot_component(
        "minecraft:brewing_fuel",
        &Some(sunrise_game_data::scoreboard::json_to_tag(
            &serde_json::json!({"uses": "parity:uses", "speed_multiplier": "parity:speed"}),
        )),
    );
    let pos = (8, 64, 8);
    place(
        &mut game,
        pos,
        stand(
            [
                Some(ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward")),
                None,
                None,
                Some(ItemStack::new("minecraft:golden_carrot", 1)),
                Some(fuel),
            ],
            0,
            0,
        ),
    );

    // The tick that takes the fuel starts the brew: ceil(400 / 8) ticks.
    game.tick_brewing_stand(pos).expect("the stand ticks");
    let ticks = 400 / 8;
    for _ in 0..ticks - 1 {
        game.tick_brewing_stand(pos).expect("the stand ticks");
        let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
            panic!("no brewing stand");
        };
        assert_eq!(&*b.items[0].as_ref().unwrap().item, "minecraft:potion");
    }
    game.tick_brewing_stand(pos).expect("the stand ticks");
    let Some(BlockEntity::Brewing(b)) = game.world.block_entities.get(&pos) else {
        panic!("no brewing stand");
    };
    assert_eq!(
        b.items[0].as_ref().unwrap().potion(),
        Some("minecraft:night_vision")
    );
}
