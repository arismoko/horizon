//! `NetherPortalBlock.randomTick`'s spawn, the rest of it after the won roll:
//! `isValidSpawn` refuses the block under the column when vanilla's table says
//! so (glowstone's 15 light, bedrock's `never`), and `EntityType.spawn` plays
//! the new piglin's ambient sound once it is in the level, before the spawn
//! sets its portal cooldown.
use crate::game::resource_load::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_core::position::ChunkPos;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::launch_options::Options;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

pub(crate) fn game<'a>(
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
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_gameplay::game::ServerGame::new(
        res,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game
}

/// A portal block at (0, 64, 0) standing on `floor`.
pub(crate) fn portal_on(game: &mut sunrise_gameplay::game::ServerGame<'_>, floor: &str) {
    assert!(game.run_command(&format!("setblock 0 63 0 minecraft:{floor}")));
    assert!(game.run_command("setblock 0 64 0 minecraft:nether_portal"));
    let state = game.world.block(0, 64, 0);
    assert_eq!(
        &*game.res.facts().get(state).name,
        "minecraft:nether_portal",
        "the portal block stands"
    );
}

pub(crate) fn spawned(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<String> {
    game.mobs.iter().map(|m| m.kind.id.clone()).collect()
}

/// `BlockState.isValidSpawn` is "sturdy top face and `getLightEmission() < 14`"
/// plus the blocks that override it: a portal on glowstone (15 light) or on
/// bedrock (`isValidSpawn(never)`) spawns nothing.
#[test]
#[ignore = "daily"]
fn a_portal_over_glowstone_or_bedrock_spawns_no_piglin() {
    let mut resources = None;
    let mut game = game(&mut resources);
    for floor in ["glowstone", "bedrock"] {
        portal_on(&mut game, floor);
        game.portal_spawn([0, 64, 0]);
        assert_eq!(
            spawned(&game),
            Vec::<String>::new(),
            "a portal over {floor} spawns no piglin"
        );
    }
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
