//! A vibration keeps its source's UUID: `VibrationInfo(gameEvent, distance,
//! pos, entity)` takes `entity.getUUID()`, and `VibrationInfo.CODEC` saves it
//! as `source` (and a projectile owner's as `projectile_owner`), so a sculk
//! sensor's saved listener names its source, even one that is not loaded
//! (tools/test_vibration_source_parity.py measures the same on vanilla 26.3).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::Options;
use sunrise_gameplay_common::game::Resources;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;

fn game(
    resources: &mut Option<std::sync::Arc<Resources>>,
) -> sunrise_gameplay::game::ServerGame<'_> {
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
    assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
    // The sensor joins the listeners through the level's own index.
    game.sculk_prepare();
    assert!(game.run_command("setblock 8 100 8 sculk_sensor"));
    game.sculk_sync_index();
    assert!(game.run_command(
        "summon pig 5.5 100 8.5 {NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Health:10f,UUID:[I;100,0,0,1]}"
    ));
    game
}

/// What `data get block 8 100 8 <path>` says, if anything.
fn read(game: &mut sunrise_gameplay::game::ServerGame<'_>, path: &str) -> Option<String> {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(&format!("data get block 8 100 8 {path}"));
    let said = game.server.chat_capture.take().unwrap_or_default();
    ok.then(|| {
        said.iter()
            .map(sunrise_game_data::text::Text::plain)
            .collect()
    })
}

#[test]
#[ignore = "daily"]
fn a_saved_vibration_names_its_source() {
    let mut game_resources = None;
    let mut game = game(&mut game_resources);
    assert!(game.run_command("damage 00000064-0000-0000-0000-000000000001 1"));
    let game_event = read(&mut game, "listener.selector.event.game_event").unwrap();
    assert!(
        game_event.ends_with("\"minecraft:entity_damage\""),
        "{game_event}"
    );
    let source = read(&mut game, "listener.selector.event.source");
    assert!(
        source
            .as_deref()
            .is_some_and(|s| s.ends_with("[I; 100, 0, 0, 1]")),
        "the damaged pig is the source: {source:?}"
    );
    assert!(read(&mut game, "listener.selector.event.projectile_owner").is_none());
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
