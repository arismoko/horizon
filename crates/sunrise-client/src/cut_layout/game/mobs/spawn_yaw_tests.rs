//! A new mob's yaw as its vanilla spawn path sets it (#12973): `/summon`
//! without NBT loads the mob from `{id:..}` and keeps yaw 0 with no
//! level-random draw (`SummonCommand.createEntity`), and `EntityType.create`
//! (spawn eggs, a dispenser's eggs, mob buckets) wraps its draw with
//! `Mth.wrapDegrees`.
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::creatures::SpawnReason;
use sunrise_gameplay_common::game::creatures::SpawnSite;
use sunrise_level::chunk::Chunk;
use {
    crate::game::*,
    sunrise_gameplay_common::game::{Options, Progress},
};

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
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in [
        "forceload add 0 0",
        "fill 0 99 0 15 99 15 stone",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

fn yaw_of(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> f32 {
    game.mobs.iter().find(|m| m.id == id).expect("spawned").yaw
}

#[test]
#[ignore = "daily"]
fn a_summoned_mob_without_nbt_faces_south() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let before = game.mobs.len();
    assert!(game.run_command("summon pig 4.5 100 8.5"));
    let pig = &game.mobs[before];
    assert_eq!(pig.yaw, 0.0);
}

#[test]
#[ignore = "daily"]
fn a_spawn_eggs_yaw_is_the_wrapped_level_draw() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // Draws until one lands in [180, 360), where wrapping changes the value.
    let mut checked = false;
    for _ in 0..32 {
        let mut level = game.world.random.clone();
        let draw = level.next_float() * 360.0;
        let id = game
            .spawn_mob_spawned(
                "pig",
                [4.5, 100.0, 8.5],
                SpawnSite::Finalizing(SpawnReason::SpawnItemUse),
            )
            .expect("pig");
        let yaw = yaw_of(&game, id);
        assert_eq!(yaw, sunrise_level::camera_math::wrap_degrees(draw));
        assert!((-180.0..180.0).contains(&yaw), "{yaw}");
        if draw >= 180.0 {
            checked = true;
            break;
        }
    }
    assert!(checked, "no draw of 180 or more in 32 spawns");
}

#[test]
#[ignore = "daily"]
fn a_dispensed_mob_takes_the_wrapped_draw_too() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let mut level = game.world.random.clone();
    let draw = level.next_float() * 360.0;
    let id = game.spawn_mob_at("pig", (4, 100, 8), false).expect("pig");
    assert_eq!(
        yaw_of(&game, id),
        sunrise_level::camera_math::wrap_degrees(draw)
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
