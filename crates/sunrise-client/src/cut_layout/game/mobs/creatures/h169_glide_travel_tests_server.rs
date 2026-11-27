use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::{geometry::Vec3, position::ChunkPos, random::RandomSource};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress};

pub(crate) fn with_role_game(
    role: sunrise_gameplay_common::game::execution_role::ExecutionRole,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let root = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
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
    options.game_dir = Some(
        std::path::PathBuf::from(std::env::var_os("SUNRISE_HOME").unwrap())
            .join(format!("h169-{:?}", std::thread::current().id())),
    );
    options.execution_role = role;
    let mut game = sunrise_gameplay::game::ServerGame::new(
        &loaded.resources,
        loaded.pack,
        options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);

    let mut center = Chunk::empty(0, 0);
    center.lit = true;
    game.world.chunks.insert((0, 0), Arc::new(center));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("difficulty normal"));
    assert!(game.run_command("forceload add 0 0"));
    for x in -1..=1 {
        for z in -1..=1 {
            if x == 0 && z == 0 {
                continue;
            }
            let mut chunk = Chunk::empty(x, z);
            chunk.lit = true;
            game.world.chunks.insert((x, z), Arc::new(chunk));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    game.load_ticking_neighbourhood(0, 0);
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(2.5, 200.0, 2.5)).unwrap());
    game.creative = true;
    assert!(game.run_command("fill 0 199 0 31 199 31 stone"));
    game.world.random.set_seed(169);
    game.update_chunk_tickets();
    assert!(crate::game::ticking_chunk(&game.world, 0, 0));
    assert!(game.server.tickets.0.levels().block_ticking(0, 0));
    test(&mut game);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
