//! A loaded sulfur cube's body primes it one tick after it is loaded (H249):
//! `SulfurCube.tick` checks the power (`primeWhenOnPoweredPosition`, which
//! needs the body's `explosionData`) before `super.tick()` adopts the body
//! (`detectEquipmentUpdates` → `collectEquipmentChanges`), so a cube loaded
//! with a TNT body on a redstone block primes on its second tick
//! (tools/test_sulfur_prime_phase_parity.py measures the same on vanilla 26.3).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::{Options, Progress, execution_role};
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

fn fuse(game: &sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    let cube = game
        .mobs
        .iter()
        .find(|m| m.species() == sunrise_sim::mobs::Species::SulfurCube)
        .expect("the cube");
    sunrise_sim::mobs::sulfur_cube::fuse(cube)
}

fn step(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    assert!(game.run_command("tick step 1"));
    game.tick_server(false).unwrap();
}

/// A NoAI cube with a TNT body summoned at 4.5 100 8.5, on `under`.
fn cube_on(game: &mut sunrise_gameplay::game::ServerGame<'_>, under: &str) {
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 98 0 15 99 15 stone"));
    if under != "stone" {
        assert!(game.run_command(&format!("setblock 4 99 8 {under}")));
    }
    for command in [
        "tick freeze".to_owned(),
        "summon sulfur_cube 4.5 100 8.5 {NoAI:1b,Silent:1b,UUID:[I;45641,0,0,1],\
         equipment:{body:{id:\"minecraft:tnt\",count:1}}}"
            .to_owned(),
    ] {
        assert!(game.run_command(&command), "{command}");
    }
    assert_eq!(fuse(game), -1, "not primed when loaded");
}

#[test]
#[ignore = "daily"]
fn a_cube_loaded_with_a_tnt_body_on_power_primes_on_its_second_tick() {
    let mut resources = None;
    let mut game = game(&mut resources);
    cube_on(&mut game, "redstone_block");
    step(&mut game);
    assert_eq!(fuse(&game), -1, "its body is adopted after the power check");
    step(&mut game);
    assert_eq!(fuse(&game), 120);
    step(&mut game);
    assert_eq!(fuse(&game), 119);
}

#[test]
#[ignore = "daily"]
fn a_warmed_cube_primes_on_the_first_powered_tick() {
    let mut resources = None;
    let mut game = game(&mut resources);
    cube_on(&mut game, "stone");
    for _ in 0..5 {
        step(&mut game);
    }
    assert_eq!(fuse(&game), -1, "no power");
    assert!(game.run_command("setblock 5 100 8 redstone_block"));
    step(&mut game);
    assert_eq!(fuse(&game), 120);
}

// Controls added after the baseline: the old pass's gates still hold for the
// cubes whose body was adopted when the tick started.

#[test]
#[ignore = "daily"]
fn control_an_adopted_cube_without_power_stays_unprimed() {
    let mut resources = None;
    let mut game = game(&mut resources);
    cube_on(&mut game, "stone");
    for _ in 0..10 {
        step(&mut game);
    }
    assert_eq!(fuse(&game), -1);
}

#[test]
#[ignore = "daily"]
fn control_a_primed_cube_on_power_is_not_primed_again() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command("forceload add 0 0"));
    assert!(game.run_command("fill 0 98 0 15 99 15 stone"));
    assert!(game.run_command("setblock 4 99 8 redstone_block"));
    assert!(game.run_command("tick freeze"));
    assert!(game.run_command(
        "summon sulfur_cube 4.5 100 8.5 {NoAI:1b,Silent:1b,fuse:50,UUID:[I;45641,0,0,1],\
         equipment:{body:{id:\"minecraft:tnt\",count:1}}}"
    ));
    assert_eq!(fuse(&game), 50);
    for expected in [49, 48, 47] {
        step(&mut game);
        assert_eq!(fuse(&game), expected, "the countdown goes on, no new fuse");
    }
}

// `canExplode` and `primeTime` read the cached `explosionData`, the body
// `collectEquipmentChanges` adopted, not the live body: a TNT body taken off
// since the last tick still primes the cube on power, with TNT's fuse.

fn replace_body(game: &mut sunrise_gameplay::game::ServerGame<'_>, with: &str) {
    assert!(game.run_command(&format!(
        "item replace entity @e[type=sulfur_cube,limit=1] armor.body with {with}"
    )));
}

#[test]
#[ignore = "daily"]
fn a_tnt_body_taken_off_this_tick_still_primes_with_its_cached_fuse() {
    let mut resources = None;
    let mut game = game(&mut resources);
    cube_on(&mut game, "stone");
    for _ in 0..5 {
        step(&mut game);
    }
    replace_body(&mut game, "minecraft:air");
    assert!(game.run_command("setblock 5 100 8 redstone_block"));
    step(&mut game);
    assert_eq!(fuse(&game), 120, "primed from the cached explosion data");
    step(&mut game);
    assert_eq!(fuse(&game), 119);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
