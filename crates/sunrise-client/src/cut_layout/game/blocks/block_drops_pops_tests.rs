//! `Block.popResource` refuses its item while `block_drops` is off
//! (`Block.java:434-440`): it draws its three offsets, then the private overload
//! every behaviour-level pop goes through reads the rule. Tall-flower and
//! flower-bed bone meal (`TallFlowerBlock.java:29`, `FlowerBedBlock.java:95`)
//! and the cave-vine and sweet-berry harvests (`CaveVines.java:34`,
//! `SweetBerryBushBlock.java:122`) all pop through it, so they drop nothing with
//! the rule off and their usual item with it on.
use sunrise_core::position::ChunkPos;
use sunrise_core::random::RandomSource;
use sunrise_gameplay_common::game::Hit;
use sunrise_items::items::ItemStack;
use {
    crate::game::game_tests::tests::isolated_server_game,
    sunrise_gameplay::game::game_tests::tests::server_game,
};

fn game(
    resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'_> {
    let mut game = server_game(resources);
    set_up(&mut game);
    game
}

fn set_up(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
}

/// `ServerPlayerGameMode.useItemOn` on the block's top face.
fn hit(pos: [i32; 3]) -> Hit {
    Hit {
        pos,
        inside: false,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
    }
}

/// Clicks the block with `stack` in the main hand: the block's own use, then
/// the item's (`BoneMealItem.useOn`). True when something was used.
fn use_block(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: [i32; 3],
    stack: Option<ItemStack>,
) -> bool {
    game.dropped.clear();
    let hit = hit(pos);
    game.server.forced_hit = Some(hit);
    *game.held_mut() = stack;
    let used = game.use_hand(Some(&hit)).unwrap();
    game.server.forced_hit = None;
    *game.held_mut() = None;
    used
}

fn place(game: &mut sunrise_gameplay::game::ServerGame<'_>, pos: [i32; 3], block: &str) {
    assert!(game.run_command(&format!(
        "setblock {} {} {} {block}",
        pos[0], pos[1], pos[2]
    )));
}

/// (block, item it pops, how many stacks, is bone meal its trigger,
/// rule-off spot, rule-on spot)
const CASES: [(&str, &str, usize, bool, [i32; 3], [i32; 3]); 4] = [
    (
        "minecraft:peony",
        "minecraft:peony",
        1,
        true,
        [2, 100, 2],
        [2, 100, 6],
    ),
    (
        "minecraft:pink_petals[flower_amount=4]",
        "minecraft:pink_petals",
        1,
        true,
        [6, 100, 2],
        [6, 100, 6],
    ),
    (
        "minecraft:cave_vines[berries=true]",
        "minecraft:glow_berries",
        1,
        false,
        [10, 100, 2],
        [10, 100, 6],
    ),
    (
        // `harvest/sweet_berry_bush` has two pools at age 3.
        "minecraft:sweet_berry_bush[age=3]",
        "minecraft:sweet_berries",
        2,
        false,
        [14, 100, 2],
        [14, 100, 6],
    ),
];

fn stack(meal: bool) -> Option<ItemStack> {
    meal.then(|| ItemStack::new("minecraft:bone_meal", 1))
}

#[test]
#[ignore = "daily"]
fn block_drops_off_stops_the_pop_of_bone_meal_and_harvests() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command("gamerule block_drops false"));
    for (block, _, _, meal, off, _) in CASES {
        place(&mut game, off, block);
        let before = game.world.random.clone();
        assert!(use_block(&mut game, off, stack(meal)), "{block} was used");
        assert!(
            game.dropped.is_empty(),
            "{block} drops nothing with block_drops off"
        );
        if meal {
            // `Block.popResource` draws its three offsets before the private
            // overload reads the rule; bone meal rolls no loot in between.
            let mut expected = before;
            for _ in 0..3 {
                expected.next_double();
            }
            assert_eq!(
                game.world.random.next_double(),
                expected.next_double(),
                "{block}: the gated pop still draws its offsets"
            );
        }
    }
    // The harvests still take the berries and leave the bush picked.
    assert!(game.run_command("execute if block 10 100 2 minecraft:cave_vines[berries=false]"));
    assert!(game.run_command("execute if block 14 100 2 minecraft:sweet_berry_bush[age=1]"));
    // Turning the rule back on restores the pop in the same game: the rule
    // is read per use, not taken once when the level is built.
    assert!(game.run_command("gamerule block_drops true"));
    let (block, item, _, meal, _, on) = CASES[0];
    place(&mut game, on, block);
    assert!(use_block(&mut game, on, stack(meal)));
    assert_eq!(game.dropped.len(), 1, "the rule on pops {item} again");
    assert_eq!(&*game.dropped[0].stack.item, item);
}

#[test]
#[ignore = "daily"]
fn block_drops_on_keeps_the_pop() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(
        game.ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::BLOCK_DROPS)
    );
    for (block, item, drops, meal, _, on) in CASES {
        place(&mut game, on, block);
        let mut expected = game.world.random.clone();
        assert!(use_block(&mut game, on, stack(meal)), "{block} was used");
        assert_eq!(game.dropped.len(), drops, "{block} pops its stacks");
        for dropped in &game.dropped {
            assert_eq!(&*dropped.stack.item, item);
        }
        if meal {
            // `Block.popResource`'s own offsets from the level random.
            let at = sunrise_gameplay_common::game::spill_common::pop_position(&mut expected, on);
            let p = game.dropped[0].position;
            assert_eq!([p.x, p.y, p.z], at, "{block} pops where vanilla pops it");
        }
    }
}

/// A game over the isolated fixture with the clicking player beside the
/// bush at `[14, 100, 2]`, the loot data the entity matcher reads
/// (`server_loot`) and `table` as the sweet berry harvest table.
fn harvest_game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    table: serde_json::Value,
) -> (
    crate::game::game_tests::tests::Published,
    sunrise_gameplay::game::ServerGame<'a>,
) {
    let (published, mut game) = isolated_server_game(resources);
    set_up(&mut game);
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(12.5, 100.0, 2.5),
        )
        .unwrap(),
    );
    game.server.server_loot.0.set(Some(std::sync::Arc::new(
        sunrise_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap(),
    )));
    let mut items = game.items().clone();
    items.set_loot_tables(std::sync::Arc::new(
        [("minecraft:harvest/sweet_berry_bush".to_owned(), table)]
            .into_iter()
            .collect(),
    ));
    game.server_items = Some(std::sync::Arc::new(items));
    (published, game)
}

/// Clicks a ripe bush at `[14, 100, 2]` and returns the sorted item ids it dropped.
fn harvest(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> Vec<String> {
    let at = [14, 100, 2];
    place(game, at, "minecraft:sweet_berry_bush[age=3]");
    assert!(use_block(game, at, None));
    let mut dropped: Vec<String> = game
        .dropped
        .iter()
        .map(|d| d.stack.item.to_string())
        .collect();
    dropped.sort();
    dropped
}

/// `SweetBerryBushBlock.useWithoutItem` drops through `BLOCK_INTERACT`: the
/// clicking player is `interacting_entity` with the fire it is in, nothing is
/// `this`, and `location_check` reads around the block (`ORIGIN`). A table of
/// those conditions, run through the real click, drops what each one holds for.
#[test]
#[ignore = "daily"]
fn a_harvest_rolls_with_its_interacting_player_and_block_origin() {
    use serde_json::json;
    let mut resources = None;
    let item = |name: &str| json!({"type": "minecraft:item", "name": name});
    let pool = |condition: serde_json::Value, name: &str| json!({"rolls": 1, "condition": condition, "entries": [item(name)]});
    let who = |who: &str, predicate: serde_json::Value| json!({"type": "minecraft:entity_properties", "entity": who, "predicate": predicate});
    let below = |block: &str| {
        json!({"type": "minecraft:location_check", "offsetY": -1,
            "predicate": {"block": {"blocks": block}}})
    };
    let table = json!({"type": "minecraft:block_interact", "pools": [
        pool(who("interacting_entity", json!({"minecraft:flags": {"is_on_fire": true}})),
            "minecraft:apple"),
        pool(who("interacting_entity", json!({"minecraft:entity_type": "minecraft:player"})),
            "minecraft:stick"),
        pool(who("this", json!({"minecraft:entity_type": "minecraft:player"})), "minecraft:bone"),
        pool(below("minecraft:stone"), "minecraft:diamond"),
        pool(below("minecraft:dirt"), "minecraft:coal"),
    ]});
    let (_published, mut game) = harvest_game(&mut resources, table);
    assert_eq!(
        harvest(&mut game),
        ["minecraft:diamond", "minecraft:stick"],
        "the player is the interacting entity and nothing is `this`; the stone below holds"
    );
    game.player.as_mut().unwrap().ignite_for_ticks(100).unwrap();
    assert_eq!(
        harvest(&mut game),
        ["minecraft:apple", "minecraft:diamond", "minecraft:stick"],
        "the burning player matches `is_on_fire`"
    );
}

/// `LocationPredicate.matches` tests the level's dimension: a harvest table
/// whose `location_check` asks for another dimension drops nothing here, and
/// one that asks for this dimension (and combines it with a block test, which
/// the old block-only reader could not read) drops.
#[test]
#[ignore = "daily"]
fn a_harvest_location_check_reads_the_dimension() {
    use serde_json::json;
    let mut resources = None;
    let check = |predicate: serde_json::Value| json!({"type": "minecraft:location_check", "predicate": predicate});
    let pool = |condition: serde_json::Value, name: &str| {
        json!({"rolls": 1, "condition": condition,
            "entries": [{"type": "minecraft:item", "name": name}]})
    };
    let table = json!({"type": "minecraft:block_interact", "pools": [
        pool(check(json!({"dimension": "minecraft:the_nether"})), "minecraft:diamond"),
        pool(check(json!({"dimension": "minecraft:overworld"})), "minecraft:coal"),
        pool(check(json!({"dimension": "minecraft:the_nether",
            "block": {"blocks": "minecraft:sweet_berry_bush"}})), "minecraft:emerald"),
        pool(check(json!({"dimension": "minecraft:overworld",
            "block": {"blocks": "minecraft:sweet_berry_bush"}})), "minecraft:stick"),
    ]});
    let (_published, mut game) = harvest_game(&mut resources, table);
    assert_eq!(harvest(&mut game), ["minecraft:coal", "minecraft:stick"]);
}

/// A player riding a mob is `interacting_entity` with that mob as its
/// `vehicle`: a `vehicle` predicate on its type holds, one on another type
/// does not.
/// A harvest's `time_check` reads the level's world clock (`TimeCheck.test` on
/// `LootContext.getLevel().clockManager()`), as the advancement and command loot paths do: a
/// range holding the clock's ticks drops, one that misses does not.
#[test]
#[ignore = "daily"]
fn a_harvest_time_check_reads_the_levels_clock() {
    use serde_json::json;
    let mut resources = None;
    let pool = |min: i64, max: i64, name: &str| {
        json!({"rolls": 1,
            "condition": {"type": "minecraft:time_check", "clock": "minecraft:overworld",
                "value": {"min": min, "max": max}},
            "entries": [{"type": "minecraft:item", "name": name}]})
    };
    let table = json!({"type": "minecraft:block_interact", "pools": [
        pool(1234, 1234, "minecraft:diamond"),
        pool(0, 1233, "minecraft:coal"),
    ]});
    let (_published, mut game) = harvest_game(&mut resources, table);
    game.set_command_clock_time("minecraft:overworld", 1234);
    assert_eq!(harvest(&mut game), ["minecraft:diamond"]);
}

#[test]
#[ignore = "daily"]
fn a_harvest_reads_the_vehicle_the_player_rides() {
    use serde_json::json;
    let mut resources = None;
    let vehicle = |kind: &str| {
        json!({"type": "minecraft:entity_properties", "entity": "interacting_entity",
            "predicate": {"minecraft:vehicle": {"minecraft:entity_type": kind}}})
    };
    let pool = |condition: serde_json::Value, name: &str| {
        json!({"rolls": 1, "condition": condition,
            "entries": [{"type": "minecraft:item", "name": name}]})
    };
    let table = json!({"type": "minecraft:block_interact", "pools": [
        pool(vehicle("minecraft:pig"), "minecraft:apple"),
        pool(vehicle("minecraft:cow"), "minecraft:bone"),
    ]});
    let (_published, mut game) = harvest_game(&mut resources, table);
    assert!(harvest(&mut game).is_empty(), "no vehicle, no drop");
    assert!(game.run_command("summon minecraft:pig 12.5 100 2.5 {NoAI:1b}"));
    game.riding = Some(game.mobs.last().unwrap().id);
    assert_eq!(harvest(&mut game), ["minecraft:apple"]);
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
