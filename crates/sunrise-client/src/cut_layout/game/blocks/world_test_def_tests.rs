//! Test instances come from the data packs the world enables
//! (`Registries.TEST_INSTANCE` is loaded from them), not only from the
//! game's own data: a definition only a world pack has is found and runs
//! (tools/test_world_test_def_parity.py measures the same on vanilla 26.3).
use crate::game::load_resources;
use std::sync::Arc;
use sunrise_client::settings::Settings;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::launch_options::Options;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;

/// A world folder with one data pack holding `h275:broken`.
fn world_with_pack(tag: &str) -> std::path::PathBuf {
    let world = std::env::temp_dir().join(format!("h275-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let pack = world.join("datapacks/fixture");
    let tests = pack.join("data/h275/test_instance");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"H275 fixture","pack_format":100}}"#,
    )
    .unwrap();
    std::fs::write(
        tests.join("broken.json"),
        r#"{"type":"minecraft:block_based","structure":"minecraft:empty","environment":"minecraft:default","max_ticks":20,"required":true}"#,
    )
    .unwrap();
    world
}

fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
    world: &std::path::Path,
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
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role =
        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
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
    game.server.world_root = world.to_path_buf();
    game.build_registries();
    game.set_allow_commands(true);
    game
}

#[test]
#[ignore = "daily"]
fn a_test_instance_from_an_enabled_world_pack_is_found_and_runs() {
    let world = world_with_pack("found");
    let mut resources = None;
    let mut game = game(&mut resources, &world);
    // The world opens: its state, registries and test instances.
    game.load_world_state().unwrap();
    game.ensure_datapacks();
    let enabled = &game.ui.registries.datapacks_enabled;
    assert!(
        enabled.iter().any(|id| id == "file/fixture"),
        "the world's pack is enabled: {enabled:?}"
    );
    let found = game.test_def("h275:broken").expect("the world pack's test");
    assert!(found.block_based && found.required, "its own settings");
    assert!(
        game.test_def("minecraft:always_pass").is_some(),
        "the built-in one"
    );
    assert!(game.test_def("h275:missing").is_none(), "no pack has it");
    // `/test run` places and starts it.
    for cx in -1..=2 {
        for cz in -1..=2 {
            game.world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
        }
    }
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(24.5, 100.0, 8.5)).unwrap(),
    );
    game.server.chat_capture = Some(Vec::new());
    assert!(game.run_command("test run h275:broken"));
    let said: Vec<String> = game
        .server
        .chat_capture
        .take()
        .unwrap()
        .iter()
        .map(|t| t.plain())
        .collect();
    assert_eq!(
        said.first().map(String::as_str),
        Some("Running 1 test(s)..."),
        "{said:?}"
    );
    // TestBatchSummaryDisplayer tells the command's source as the batch starts.
    assert_eq!(
        said.get(1).map(String::as_str),
        Some("Starting environment minecraft:default batch 0"),
        "{said:?}"
    );
    let _ = std::fs::remove_dir_all(&world);
}

/// A world folder with one pack holding these `test_instance` files.
fn world_with(tag: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let world = std::env::temp_dir().join(format!("h275-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let pack = world.join("datapacks/fixture");
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"H275 fixture","pack_format":100}}"#,
    )
    .unwrap();
    for (id, body) in files {
        let (ns, path) = id.split_once(':').unwrap();
        let file = pack.join(format!("data/{ns}/test_instance/{path}.json"));
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, body).unwrap();
    }
    world
}

const FAILING: &str = r#"{"type":"minecraft:block_based","structure":"minecraft:empty","environment":"minecraft:default","max_ticks":20,"required":true}"#;

/// Added controls (after the baseline): the world opens with its test
/// instances and keeps them. Files changed after it opened, and packs
/// disabled and reloaded, change nothing. The next world replaces them,
/// cached misses and selector ids included. A pack overrides a built-in id.
/// A game with no world opened reads the game's own data as before.
#[test]
#[ignore = "daily"]
fn a_world_keeps_the_test_instances_it_opened_with() {
    let a = world_with(
        "a",
        &[("h275:broken", FAILING), ("minecraft:always_pass", FAILING)],
    );
    let b = world_with("b", &[("h275:other", FAILING)]);
    let mut resources = None;
    let mut game = game(&mut resources, &a);
    // No world opened yet: the game's own data, as before.
    let builtin = game.test_def("minecraft:always_pass").expect("built in");
    assert!(!builtin.block_based, "the built-in function test");
    // World opening publishes its data packs before snapshotting test instances.
    game.reload_datapacks(false, &game.level.enabled_features.clone())
        .unwrap();
    game.load_world_state().unwrap();
    let ids = |game: &sunrise_gameplay::game::ServerGame<'_>| {
        game.ui.registries.ids("minecraft:test_instance").to_vec()
    };
    assert_eq!(ids(&game), ["h275:broken", "minecraft:always_pass"]);
    assert!(
        game.test_def("minecraft:always_pass").unwrap().block_based,
        "the pack's override"
    );
    assert!(game.test_def("h275:other").is_none(), "a miss in world A");
    // A file changed after the world opened.
    let file = a.join("datapacks/fixture/data/h275/test_instance/broken.json");
    std::fs::write(
        &file,
        FAILING.replace("\"required\":true", "\"required\":false"),
    )
    .unwrap();
    assert!(
        game.test_def("h275:broken").unwrap().required,
        "the opened definition"
    );
    // The pack disabled and the data reloaded.
    assert!(game.run_command("datapack disable \"file/fixture\""));
    assert!(game.run_command("reload"));
    assert!(
        game.test_def("h275:broken").is_some(),
        "kept over /datapack and /reload"
    );
    assert_eq!(ids(&game), ["h275:broken", "minecraft:always_pass"]);
    // World B.
    game.server.world_root = b.clone();
    game.reload_datapacks(false, &game.level.enabled_features.clone())
        .unwrap();
    game.load_world_state().unwrap();
    assert_eq!(ids(&game), ["h275:other", "minecraft:always_pass"]);
    assert!(
        game.test_def("h275:broken").is_none(),
        "nothing from world A"
    );
    assert!(
        game.test_def("h275:other").is_some(),
        "world A's miss is gone"
    );
    assert!(
        !game.test_def("minecraft:always_pass").unwrap().block_based,
        "built in again"
    );
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
}

/// A pack test instance `test:smoke` runs with `/test run test:smoke`, applies the
/// pack test environment it names, and both entries appear in the synced registry data.
#[test]
#[ignore = "daily"]
fn a_pack_test_instance_runs_and_applies_its_pack_test_environment() {
    let world = std::env::temp_dir().join(format!("game-test-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    let pack = world.join("datapacks/fixture");
    let envs = pack.join("data/test/test_environment");
    let instances = pack.join("data/test/test_instance");
    std::fs::create_dir_all(&envs).unwrap();
    std::fs::create_dir_all(&instances).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"description":"Game test fixture","pack_format":100}}"#,
    )
    .unwrap();
    std::fs::write(
        envs.join("storm.json"),
        r#"{"type":"minecraft:all_of","definitions":[{"type":"minecraft:weather","weather":"thunder"},{"type":"minecraft:game_rules","rules":{"minecraft:advance_time":false}}]}"#,
    )
    .unwrap();
    std::fs::write(
        instances.join("smoke.json"),
        r#"{"type":"minecraft:block_based","structure":"minecraft:empty","environment":"test:storm","max_ticks":20,"required":true}"#,
    )
    .unwrap();
    let mut resources = None;
    let mut game = game(&mut resources, &world);
    game.reload_datapacks(false, &game.level.enabled_features.clone())
        .unwrap();
    game.load_world_state().unwrap();
    assert!(game.test_environment("test:storm").is_some());
    assert!(game.test_def("test:smoke").is_some());
    for cx in -1..=2 {
        for cz in -1..=2 {
            game.world
                .chunks
                .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
        }
    }
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(24.5, 100.0, 8.5)).unwrap(),
    );
    game.world.weather.raining = false;
    game.world.weather.thundering = false;
    game.ui.rules.set(
        "minecraft:advance_time",
        sunrise_game_data::gamerules::Value::Bool(true),
    );
    game.server.chat_capture = Some(Vec::new());
    assert!(game.run_command("test run test:smoke"));
    let said: Vec<String> = game
        .server
        .chat_capture
        .take()
        .unwrap()
        .iter()
        .map(|t| t.plain())
        .collect();
    assert_eq!(
        said.first().map(String::as_str),
        Some("Running 1 test(s)..."),
        "{said:?}"
    );
    assert_eq!(
        said.get(1).map(String::as_str),
        Some("Starting environment test:storm batch 0"),
        "{said:?}"
    );
    assert!(
        game.world.weather.raining && game.world.weather.thundering,
        "pack test environment weather must be applied"
    );
    assert_eq!(
        game.ui.rules.get("minecraft:advance_time"),
        Some(sunrise_game_data::gamerules::Value::Bool(false)),
        "pack test environment game rule must be applied"
    );
    let tables = game.server.world_tables.0.as_ref();
    let overlay = sunrise_gameplay_common::game::pack_files(&game.server.world_tables.0);
    let sync = sunrise_sim::lan::data::SyncData::load_with_frozen_damage(
        &game.options.resources,
        &overlay,
        &sunrise_sim::lan::data::FrozenDamage::new(&game.health_resources.damage, &overlay),
        tables.map(|t| t.biomes.table.as_ref()),
        tables.map(|t| &t.packed_registries),
        game.res.item_defaults(),
        &sunrise_sim::crafting::ResolvedSources::default(),
        None,
        None,
    )
    .unwrap();
    let packets = sync.registry_packets(true, &game.server.level_shapes);
    for (registry, id) in [
        ("minecraft:test_environment", "test:storm"),
        ("minecraft:test_instance", "test:smoke"),
    ] {
        let entry = packets
            .iter()
            .find(|packet| packet.registry == registry)
            .unwrap_or_else(|| panic!("missing synced registry {registry}"))
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .unwrap_or_else(|| panic!("missing synced entry {id} in {registry}"));
        assert!(
            entry.data.is_some(),
            "pack entry {id} in {registry} must carry its synced data"
        );
    }
    let _ = std::fs::remove_dir_all(&world);
}

/// A malformed test instance refuses the world open and names the offending file
/// (`RegistryDataLoader.load/logErrors`).
#[test]
#[ignore = "daily"]
fn a_malformed_test_instance_refuses_world_open_naming_the_file() {
    let world = world_with(
        "malformed",
        &[(
            "test:broken",
            r#"{"type":"minecraft:block_based","structure":"minecraft:empty","environment":"minecraft:default","max_ticks":0}"#,
        )],
    );
    let mut resources = None;
    let mut game = game(&mut resources, &world);
    let error = game
        .reload_datapacks(false, &game.level.enabled_features.clone())
        .err()
        .expect("malformed test_instance must refuse world open")
        .to_string();
    assert!(
        error.contains("data/test/test_instance/broken.json"),
        "error must name the malformed file: {error}"
    );
    let _ = std::fs::remove_dir_all(&world);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
