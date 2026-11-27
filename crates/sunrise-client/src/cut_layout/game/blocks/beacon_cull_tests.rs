//! `BeaconRenderer.shouldRender`: a beam draws inside the effective render distance, and leaves with its column.
use std::sync::Arc;
use sunrise_client::{audio::Audio, settings::Settings};
use sunrise_level::{
    chunk::Chunk,
    containers::{Beacon, BlockEntity},
};
use {
    crate::game::resource_load::load_resources, sunrise_gameplay_common::game::execution_role,
    sunrise_gameplay_common::game::launch_options::Options,
    sunrise_gameplay_common::game::resource_load::Progress,
};

fn game<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
) -> sunrise_client_gameplay::game::ClientGame<'a> {
    game_as_client(resources, execution_role::ExecutionRole::Client)
}

/// A game loaded from the prepared resources as `role`: a client also loads what it draws with
/// (the GUI and item atlas, baked block models).
pub(crate) fn game_as_client<'a>(
    resources: &'a mut Option<Arc<sunrise_gameplay_common::game::Resources>>,
    role: execution_role::ExecutionRole,
) -> sunrise_client_gameplay::game::ClientGame<'a> {
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
        role,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = role;
    let res = &*resources.insert(loaded.resources);
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        res,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&sunrise_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.build_registries();
    // The beam's sprite joins the GUI atlas with a renderer: any rectangle draws it here.
    let rect = sunrise_client::hud::Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    game.client
        .gui
        .atlas
        .sprites
        .insert("entity/beacon/beacon_beam".into(), rect);
    game
}

/// The vertices `beacon_geometry` writes for a camera at (0.5, 70, 0.5).
fn beam_vertices(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> usize {
    let mut out = Vec::new();
    game.beacon_geometry(glam::DVec3::new(0.5, 70.0, 0.5), 0.0, &mut out);
    out.len()
}

#[test]
#[ignore = "daily"]
fn a_beam_draws_inside_the_render_distance_only_and_leaves_with_its_column() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // The streaming radius keeps a chunk past the render distance, as a server's view distance can.
    game.client.settings.render_distance = 4;
    game.client.streaming.radius = 5;
    game.client.settings.sunrise.lod_distance = 256;
    game.client.lod.0 = Some(sunrise_client::lod::Lod::new(
        game.client.streaming.lod_source(),
        game.worker_context(),
    ));
    let beacon = || {
        BlockEntity::Beacon(Box::new(Beacon {
            levels: 1,
            sections: vec![(0xff_ff_ff, 4)],
            ..Default::default()
        }))
    };
    // The beams' centres are 63 and 64 blocks from the camera.
    for (x, chunk) in [(63, 3), (64, 4)] {
        game.world
            .chunks
            .insert((chunk, 0), Arc::new(Chunk::empty(chunk, 0)));
        game.world.block_entities.insert((x, 60, 0), beacon());
    }
    let both = beam_vertices(&mut game);
    assert!(both > 0, "the beam inside the render distance draws");
    game.world.block_entities.remove(&(64, 60, 0));
    assert_eq!(
        beam_vertices(&mut game),
        both,
        "the one at 64 blocks is culled"
    );
    // Unloaded with its far terrain kept, the beam goes though the camera is in range.
    let mut gone = [(3, 0)].into_iter().collect();
    game.forget_remote_columns(&mut gone, true);
    assert_eq!(
        beam_vertices(&mut game),
        0,
        "an unloaded beacon draws no beam"
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
