//! `Entity.isInvisibleTo`: a spectator sees an invisible body as the translucent
//! one (`LivingEntityRenderer.submit`'s `forceTransparent`), anyone else not at all.
#![cfg(feature = "client")]
use crate::game::{Audio, Settings, load_resources};
use std::sync::Arc;
use sunrise_command::args::GameMode;
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::{
    connection::Pipe,
    packets::play::{self, Clientbound as C},
};
use sunrise_sim::mobs::{Mob, MobTypes};
use {
    sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    sunrise_gameplay_common::game::execution_role,
};

/// The prepared vanilla resources.
fn root() -> std::path::PathBuf {
    std::env::var_os("SUNRISE_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .expect("set SUNRISE_TEST_RESOURCES to prepared resources")
}

fn game<'a>(
    resources: &'a mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (sunrise_client_gameplay::game::ClientGame<'a>, Pipe) {
    let root = root();
    let settings = Settings::default();
    let loaded = load_resources(
        sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        sunrise_gameplay_common::game::execution_role::ExecutionRole::Client,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::Client;
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
            root.clone(),
        )),
        None,
        sunrise_gameplay::game::server_side::integrated::serve,
        sunrise_gameplay::game::server_side::integrated::run_component_harness_on_server,
        crate::game::session_verifier(),
        crate::game::profile_resolver,
    )
    .unwrap();
    let server =
        sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(
            &mut game,
        );
    // The atlas the game bakes at startup: `mob_geometry` finds a body's texture in it.
    game.client.gui.atlas = sunrise_client::hud::HudAtlas::build(
        &sunrise_assets::resource_source::ResourceSource::directory(root.clone()),
    )
    .unwrap();
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    (game, server)
}

/// `/effect give @e[type=minecraft:pig,limit=1] minecraft:invisibility 100 0`
/// sets the shared invisible flag a client reads (`LivingEntity.updateInvisibilityStatus`).
/// `Entity.isInvisibleTo` is false for a spectator, so `LivingEntityRenderer.submit`
/// draws the body it cannot show as visible as `entityTranslucentCull` at
/// `ARGB.multiply(654311423, getModelTint)`: alpha 38, in the blending pass.
#[test]
#[ignore = "daily"]
fn a_spectator_sees_the_invisible_body() {
    let mut resources = None;
    let (mut game, _server) = game(&mut resources);
    let profile = game.client.remote.as_ref().unwrap().profile();
    let types = MobTypes::load(&root()).unwrap();
    let kind = Arc::new(types.types["minecraft:pig"].clone());
    let mut pig = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
    pig.invisible = true;
    game.mobs.push(pig);
    let camera = glam::DVec3::new(6.5, 65.0, 6.5);

    // Anyone but a spectator sees no body at all.
    game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
        actions: 1 | 4,
        entries: vec![play::PlayerInfo {
            uuid: profile,
            name: Some("SpectatorViewTest".into()),
            game_mode: Some(GameMode::Survival.id()),
            ..Default::default()
        }],
    }))
    .unwrap();
    game.set_local_mode(GameMode::Survival);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(out.is_empty(), "an invisible body is hidden from a player");

    // A spectator sees it forced translucent.
    game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
        actions: 4,
        entries: vec![play::PlayerInfo {
            uuid: profile,
            game_mode: Some(GameMode::Spectator.id()),
            ..Default::default()
        }],
    }))
    .unwrap();
    game.set_local_mode(GameMode::Spectator);
    let mut out = Vec::new();
    game.mob_geometry(camera, 0.0, &mut out);
    assert!(!out.is_empty(), "a spectator sees the invisible body");
    assert!(
        out.iter().all(|v| v.color >> 24 == 0x26),
        "the body is forced translucent (alpha 38)"
    );
    assert!(
        out.iter()
            .all(|v| v.layer & sunrise_client::gfx::OIT_ENTITY != 0),
        "the translucent body blends"
    );
}
