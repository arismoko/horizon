//! A frame whose only particles are a pack's GPU particle records (wiki "GPU Particles"): the
//! records aren't in the native particle count, and the frame still builds their quads when no
//! renderer draws the type's shader; rain that dies under the heightmap reads heights taken
//! before those quads, from its first frame.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use sunrise_assets::resource_source::{Resource, ResourceSource};
use sunrise_client::hud::Rect;
use sunrise_client::particles::ParticleEngine;
use sunrise_client::particles::polytone::Registry;
use sunrise_level::particle_options::ParticleOptions;

/// A game over the prepared resources that registers no mod tables of the process (the
/// particles here are the pack's): a direct `ClientGame`, not the mod fixture's `with_client_game`.
fn with_game(test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>)) {
    use crate::game::{Audio, Settings, load_resources};
    use {
        sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
        sunrise_gameplay_common::game::execution_role,
    };
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
    let mut game = sunrise_client_gameplay::game::ClientGame::new(
        &loaded.resources,
        loaded.client_res.unwrap(),
        loaded.pack,
        loaded.client_pack.unwrap(),
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&ResourceSource::directory(root)),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    test(&mut game);
}

/// A pack with one GPU particle type, `test:rain`, drawn with a vanilla particle sprite and
/// no shader of its own; `kill_below_heightmap` when `dies_under`.
fn rain_pack(dies_under: bool) -> ResourceSource {
    let rain = serde_json::json!({
        "type": "gpu", "texture": "minecraft:textures/particle/generic_0.png",
        "render_type": "translucent", "kill_below_heightmap": dies_under,
        "initializer": {"lifetime": "40", "size": "0.5"}
    });
    ResourceSource::indexed(
        [(
            "assets/test/polytone/custom_particles/rain.json".to_owned(),
            Resource::bytes(rain.to_string().into_bytes()),
        )]
        .into_iter()
        .collect(),
    )
}

/// `game`'s particles reading `pack`, with the rain's sprite in their atlas (as a resource load
/// puts it there) and their camera at `camera`.
fn rain_particles(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    pack: &ResourceSource,
    camera: [f64; 3],
) {
    let registry = Registry::read(pack, Default::default(), game.pack.assets.registry);
    let rect = Rect {
        u0: 0.0,
        v0: 0.0,
        u1: 1.0,
        v1: 1.0,
        w: 16,
        h: 16,
    };
    game.client.particles.0 = ParticleEngine::new(
        pack,
        &HashMap::from([("particle/generic_0".to_owned(), rect)]),
        &HashSet::new(),
    );
    game.client
        .particles
        .0
        .set_polytone(Some(Arc::new(registry)));
    game.client.particles.0.camera = camera;
}

#[test]
#[ignore = "daily"]
fn a_frame_of_gpu_records_only_draws_their_quads() {
    with_game(|game| {
        let at = [0.5, 80.0, 0.5];
        rain_particles(game, &rain_pack(false), at);
        game.client
            .particles
            .0
            .add(&ParticleOptions::named("test:rain"), at, [0.0; 3]);
        assert_eq!(game.client_particle_count(), 0, "not in the native count");
        assert_eq!(game.client.particles.0.polytone_gpu_records(), 1);
        let camera = glam::DVec3::new(at[0], at[1] + 2.0, at[2]);
        game.polytone_gpu_decide(camera);
        let mut out = Vec::new();
        game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
        assert_eq!(out.len(), 4, "the record's quad");
    });
}

/// In the first frame of a rain-only scene, the heights are taken before the quads: of a drop
/// above the world's surface and one under it, only the first draws.
#[test]
#[ignore = "daily"]
fn rain_under_the_surface_is_hidden_from_its_first_frame() {
    with_game(|game| {
        // The fixture's world has nothing in these columns: the surface is its bottom.
        let bottom = f64::from(game.world.bottom_y());
        let camera = [0.5, bottom + 4.0, 0.5];
        rain_particles(game, &rain_pack(true), camera);
        let rain = ParticleOptions::named("test:rain");
        game.client
            .particles
            .0
            .add(&rain, [0.5, bottom + 2.0, 0.5], [0.0; 3]);
        game.client
            .particles
            .0
            .add(&rain, [0.5, bottom - 2.0, 0.5], [0.0; 3]);
        assert_eq!(game.client.particles.0.polytone_gpu_records(), 2);
        let camera = glam::DVec3::from_array(camera);
        game.polytone_gpu_decide(camera);
        let mut out = Vec::new();
        game.client_particle_geometry(camera, 0.0, 70.0, None, &mut out);
        assert_eq!(out.len(), 4, "the drop above the surface only");
    });
}

/// A mod's clear asked before the frame is applied before the heightmap's readers are counted,
/// in the same call: the cleared rain is gone when they are.
#[test]
#[ignore = "daily"]
fn a_pending_clear_goes_before_the_heightmap_is_read() {
    with_game(|game| {
        let bottom = f64::from(game.world.bottom_y());
        let camera = [0.5, bottom + 4.0, 0.5];
        rain_particles(game, &rain_pack(true), camera);
        game.client.particles.0.add(
            &ParticleOptions::named("test:rain"),
            [0.5, bottom + 2.0, 0.5],
            [0.0; 3],
        );
        game.client.mod_clear_particles.set(true);
        game.polytone_gpu_decide(glam::DVec3::from_array(camera));
        assert_eq!(
            game.client.particles.0.polytone_gpu_records(),
            0,
            "cleared first"
        );
    });
}
