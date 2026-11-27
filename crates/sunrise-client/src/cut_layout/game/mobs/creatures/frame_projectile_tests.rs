//! A projectile that strikes an item frame hurts it (#11383): `ItemFrame.hurtServer`
//! gives the item up, an empty frame breaks, and a fixed one refuses the hit.
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_level::chunk::Chunk;
use {
    sunrise_gameplay_common::game::Options, sunrise_gameplay_common::game::Progress,
    sunrise_gameplay_common::game::execution_role,
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
        "fill 0 63 0 15 63 15 stone",
        "fill 4 64 0 4 66 15 stone",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// An arrow from `(0.5, 64.5, z)` flying east into the wall at x = 4.
fn shoot(game: &mut sunrise_gameplay::game::ServerGame<'_>, z: f64) {
    assert!(game.run_command(&format!(
        "summon minecraft:arrow 0.5 64.5 {z} {{Motion:[1.0d,0.0d,0.0d]}}"
    )));
    for _ in 0..10 {
        game.tick_projectiles().unwrap();
    }
}

fn dropped(game: &sunrise_gameplay::game::ServerGame<'_>, item: &str) -> usize {
    game.dropped
        .iter()
        .filter(|d| &*d.stack.item == item)
        .count()
}

#[test]
#[ignore = "daily"]
fn an_arrow_knocks_the_item_out_then_breaks_the_frame() {
    let mut resources = None;
    let mut game = game(&mut resources);
    // On the wall's west face (Facing 4: west).
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 64.5 6.5 {Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    assert_eq!(game.item_frames.len(), 1);
    shoot(&mut game, 6.5);
    assert_eq!(game.item_frames.len(), 1, "the first hit leaves the frame");
    assert!(game.item_frames[0].item.is_none(), "the item came out");
    assert_eq!(dropped(&game, "minecraft:diamond"), 1);
    // The empty frame breaks and drops itself.
    shoot(&mut game, 6.5);
    assert!(game.item_frames.is_empty());
    assert_eq!(dropped(&game, "minecraft:item_frame"), 1);
}

#[test]
#[ignore = "daily"]
fn a_fixed_frame_refuses_the_arrow() {
    let mut resources = None;
    let mut game = game(&mut resources);
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 64.5 9.5 {Facing:4b,Fixed:1b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    shoot(&mut game, 9.5);
    assert_eq!(game.item_frames.len(), 1);
    assert!(
        game.item_frames[0].item.is_some(),
        "a fixed frame keeps its item"
    );
    assert_eq!(dropped(&game, "minecraft:diamond"), 0);
    // The frame took the arrow's hit and refused it, so the arrow turned back
    // (`AbstractArrow.onHitEntity`'s deflection) instead of reaching the wall.
    assert!(
        game.creatures
            .projectiles
            .iter()
            .all(|p| p.position.x < 3.9),
        "the arrow went through the frame"
    );
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
