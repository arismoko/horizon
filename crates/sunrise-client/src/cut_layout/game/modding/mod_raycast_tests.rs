//! A client mod's `raycast` and `view_hit` hit loaded blocks only, far terrain on or off.
use std::sync::Arc;
use sunrise_client::{audio::Audio, settings::Settings};
use sunrise_client_gameplay::game::ClientGame;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::ServerGame;
use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;
use {
    crate::game::resource_load::load_resources, sunrise_gameplay_common::game::execution_role,
    sunrise_gameplay_common::game::launch_options::Options,
    sunrise_gameplay_common::game::resource_load::Progress,
};

fn game<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
) -> (ClientGame<'a>, ServerGame<'a>) {
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
        execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
    let res = &*resources.insert(loaded.resources);
    let mut server_options = options.clone();
    server_options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut server = ServerGame::new(
        res,
        loaded.pack.clone(),
        server_options,
        settings.sim_settings(),
        None,
        crate::game::profile_resolver,
    )
    .unwrap();
    server.build_registries();
    server.set_allow_commands(true);
    let mut game = ClientGame::new(
        res,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
        .unwrap();
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..=15 {
        for z in 0..=15 {
            game.world.set(x, 99, z, stone).unwrap();
        }
    }
    (game, server)
}

fn run_command(client: &mut ClientGame<'_>, server: &mut ServerGame<'_>, command: &str) -> bool {
    std::mem::swap(&mut client.common.world, &mut server.common.world);
    std::mem::swap(
        &mut client.common.block_ticks,
        &mut server.common.block_ticks,
    );
    server.common.player.clone_from(&client.common.player);
    server.common.yaw = client.common.yaw;
    server.common.pitch = client.common.pitch;
    let succeeded = server.run_command(command);
    std::mem::swap(&mut client.common.world, &mut server.common.world);
    std::mem::swap(
        &mut client.common.block_ticks,
        &mut server.common.block_ticks,
    );
    succeeded
}

#[test]
#[ignore = "daily"]
fn mod_raycasts_hit_loaded_blocks_only_with_far_terrain_on_or_off() {
    let mut resources = None;
    let (mut game, mut server) = game(&mut resources);
    // Looking straight down at the stone under the player.
    game.pitch = 90.0;
    let at = |game: &mut ClientGame<'_>, x: f64| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(x, 100.0, 0.5)).unwrap());
    };
    at(&mut game, 0.5);
    let without = game.view_hit().expect("the stone below");
    assert_eq!(
        (without.1.block_x, without.1.block_y, without.1.block_z),
        (0, 99, 0)
    );
    assert_eq!(game.raycast(8.0), Some(without.1.clone()));
    assert_eq!(game.raycast(1.0), None, "past the distance asked for");
    game.client.settings.sunrise.lod_distance = 256;
    game.client.lod.0 = Some(sunrise_client::lod::Lod::new(
        game.client.streaming.lod_source(),
        game.worker_context(),
    ));
    assert_eq!(
        game.view_hit(),
        Some(without.clone()),
        "far terrain changes nothing"
    );
    assert_eq!(game.raycast(8.0), Some(without.1));
    // Over a chunk that isn't loaded, far terrain or not, there is nothing to hit.
    at(&mut game, 40.5);
    assert_eq!(game.view_hit(), None);
    assert_eq!(game.raycast(1024.0), None);

    // Loaded chunks past 1024 blocks are reachable with no fixed distance ceiling,
    // and nonpositive or non-finite distances miss.
    game.world
        .chunks
        .insert((0, 100), Arc::new(Chunk::empty(0, 100)));
    game.block_ticks
        .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 100))
        .unwrap();
    assert!(run_command(
        &mut game,
        &mut server,
        "fill 0 100 1600 15 103 1600 stone"
    ));
    game.yaw = 0.0;
    game.pitch = 0.0;
    at(&mut game, 0.5);
    let (far_distance, far_hit) = game.view_hit().expect("loaded stone past 1024 blocks");
    assert_eq!(
        (far_hit.block_x, far_hit.block_y, far_hit.block_z),
        (0, 101, 1600)
    );
    assert_eq!(far_distance, 1599.5);
    assert_eq!(game.raycast(2000.0), Some(far_hit.clone()));
    assert_eq!(game.raycast(f32::MAX), Some(far_hit));
    assert_eq!(game.raycast(1500.0), None);
    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(game.raycast(bad), None);
    }

    // Starting inside a block, neither query's shape clip depends on distant chunks.
    let distant = game.world.chunks.remove(&(0, 100)).unwrap();
    assert!(run_command(
        &mut game,
        &mut server,
        "setblock 0 101 0 stone"
    ));
    let inside_view = game.view_hit().expect("the stone containing the eye");
    assert_eq!(
        (
            inside_view.1.block_x,
            inside_view.1.block_y,
            inside_view.1.block_z
        ),
        (0, 101, 0)
    );
    assert!(inside_view.1.z > 0.5 && inside_view.1.z < 1.0);
    // The bounded query uses the caller's endpoints for VoxelShape.clip's nudge
    // and short-sweep rejection, even though traversal stops with the loaded chunks.
    let inside_raycast = game.raycast(100.0).expect("inside stone");
    assert_eq!(
        (
            inside_raycast.block_x,
            inside_raycast.block_y,
            inside_raycast.block_z
        ),
        (0, 101, 0)
    );
    assert_eq!(inside_raycast.z, 0.6);
    assert_eq!(
        game.raycast(0.0001),
        None,
        "sub-1e-7 squared sweep inside a block misses as in VoxelShape.clip"
    );
    game.world.chunks.insert((0, 100), distant);
    assert_eq!(game.view_hit(), Some(inside_view));
    assert_eq!(game.raycast(100.0), Some(inside_raycast));
}
