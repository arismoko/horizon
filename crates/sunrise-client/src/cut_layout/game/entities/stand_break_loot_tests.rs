//! A burnt-out armor stand rolls its entity loot table: `ArmorStand.causeDamage`
//! at 0.5 health or below calls `brokenByAnything`, whose `dropAllDeathLoot`
//! rolls `minecraft:entities/armor_stand` (a stand that is not small, with
//! `mob_drops` on) before the equipment pops
//! (tools/test_stand_break_loot_parity.py measures the same on vanilla 26.3).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::launch_options::Options;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;

fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_gameplay::game::ServerGame<'a> {
    let root = std::path::PathBuf::from(
        std::env::var_os("SUNRISE_TEST_RESOURCES")
            .expect("set SUNRISE_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game.load_ticking_neighbourhood(0, 0);
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    game
}

/// The world pack's table for armor stands: one stick.
fn with_stick_table(game: &mut sunrise_gameplay::game::ServerGame<'_>, name: &str) {
    let dir =
        std::env::temp_dir().join(format!("sunrise-stand-loot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let tables = dir.join("datapacks/stand/data/minecraft/loot_table/entities");
    std::fs::create_dir_all(&tables).unwrap();
    std::fs::write(
        dir.join("datapacks/stand/pack.mcmeta"),
        r#"{"pack":{"description":"fixture"}}"#,
    )
    .unwrap();
    std::fs::write(
        tables.join("armor_stand.json"),
        r#"{"type":"minecraft:entity","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("datapacks.json"),
        r#"{"enabled":["file/stand"],"disabled":[]}"#,
    )
    .unwrap();
    game.server.world_root = dir;
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
    // Publish the world's snapshot exactly as world open does. Do not prime
    // the process or thread-local structure-generation cache: entity drops
    // must read the server's pack-aware snapshot instead.
    game.load_server_loot();
    // The table the pack installed: a direct roll, far from the stand.
    assert!(game.run_command("loot spawn 12.5 100 12.5 loot minecraft:entities/armor_stand"));
    assert!(
        game.run_command(
            "execute if entity @e[type=item,x=12.5,y=100,z=12.5,distance=..1,nbt={Item:{id:\"minecraft:stick\"}}]"
        ),
        "the pack's table is the stand's"
    );
    assert!(game.run_command("kill @e[type=item]"));
}

fn burn(game: &mut sunrise_gameplay::game::ServerGame<'_>, small: bool) {
    let small = if small { ",Small:1b" } else { "" };
    assert!(game.run_command(&format!(
        "summon armor_stand 2.5 100 8.5 {{NoGravity:1b,Fire:200s,Health:0.6f,UUID:[I;23326,0,0,1]{small},\
         equipment:{{head:{{id:\"minecraft:diamond_helmet\",count:1}}}}}}"
    )));
    // Stands tick only in entity-ticking chunks.
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("tick freeze"));
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
    assert!(
        !game.run_command("execute if entity 00005b1e-0000-0000-0000-000000000001"),
        "the fire breaks the stand"
    );
    assert!(
        game.run_command(
            "execute if entity @e[type=item,nbt={Item:{id:\"minecraft:diamond_helmet\"}}]"
        ),
        "its helmet pops"
    );
}

const STICK: &str = "execute if entity @e[type=item,x=2.5,y=100,z=8.5,distance=..3,nbt={Item:{id:\"minecraft:stick\"}}]";

#[test]
#[ignore = "daily"]
fn a_burnt_out_stand_drops_its_loot_table() {
    let mut resources = None;
    let mut game = game(&mut resources);
    with_stick_table(&mut game, "normal");
    burn(&mut game, false);
    assert!(game.run_command(STICK), "the table's stick drops");
}

#[test]
#[ignore = "daily"]
fn a_small_burnt_out_stand_drops_no_loot() {
    // ArmorStand.isBaby is isSmall: LivingEntity.shouldDropLoot is false.
    let mut resources = None;
    let mut game = game(&mut resources);
    with_stick_table(&mut game, "small");
    burn(&mut game, true);
    assert!(!game.run_command(STICK));
}

// Controls added after the baseline: the roll is gated by mob_drops, and one
// break rolls the table once.

fn sticks(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> String {
    game.server.chat_capture = Some(Vec::new());
    game.run_command(STICK);
    let said = game.server.chat_capture.take().unwrap_or_default();
    said.iter()
        .map(sunrise_game_data::text::Text::plain)
        .collect()
}

#[test]
#[ignore = "daily"]
fn control_mob_drops_off_rolls_nothing() {
    let mut resources = None;
    let mut game = game(&mut resources);
    with_stick_table(&mut game, "nodrops");
    assert!(game.run_command("gamerule mob_drops false"));
    burn(&mut game, false);
    assert!(!game.run_command(STICK));
}

#[test]
#[ignore = "daily"]
fn control_one_break_rolls_the_table_once() {
    let mut resources = None;
    let mut game = game(&mut resources);
    with_stick_table(&mut game, "once");
    burn(&mut game, false);
    assert_eq!(sticks(&mut game), "Test passed. Count: 1");
    for _ in 0..20 {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
    assert_eq!(sticks(&mut game), "Test passed. Count: 1");
}

#[test]
#[ignore = "daily"]
fn a_broken_stand_item_keeps_its_styled_custom_name() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command(r#"summon armor_stand 2.5 100 8.5 {NoGravity:1b,CustomName:{text:"Named",bold:true},UUID:[I;23326,0,0,9]}"#));
    assert!(game.run_command("damage 00005b1e-0000-0000-0000-000000000009 1 minecraft:arrow"));
    assert!(game.run_command(r#"execute if entity @e[type=item,nbt={Item:{id:"minecraft:armor_stand",count:1,components:{"minecraft:custom_name":{text:"Named",bold:1b}}}}]"#));
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
