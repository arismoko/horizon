//! `/clone` counts what it wrote and needs every column of both boxes loaded
//! (`CloneCommands.clone`: `Level.setBlock`'s result is the count, and
//! `Level.hasChunksAt` covers the whole box, not just its corners).
use crate::game::load_resources;
use crate::settings::Settings;
use std::sync::Arc;
use sunrise_core::position::ChunkPos;
use sunrise_gameplay_common::game::Dim;
use sunrise_gameplay_common::game::Options;
#[cfg(test)]
use sunrise_gameplay_common::game::execution_role;
use sunrise_gameplay_common::game::resource_load::Progress;
use sunrise_level::chunk::Chunk;

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
    game.build_registries();
    game.set_allow_commands(true);
    game
}

/// Gives the current level the empty chunk at `x, z`, as an opened world has it.
pub(crate) fn load_chunk(game: &mut sunrise_gameplay::game::ServerGame<'_>, x: i32, z: i32) {
    game.world
        .chunks
        .insert((x, z), Arc::new(Chunk::empty(x, z)));
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(x, z));
}

/// Opens `dim`'s level through the same adapter the clone uses, gives it the
/// empty chunk at the origin, and returns to `back`.
pub(crate) fn load_chunk_in(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    dim: Dim,
    back: Dim,
) {
    game.enter_command_level(dim);
    assert_eq!(game.dim, dim, "the level is in place");
    load_chunk(game, 0, 0);
    game.enter_command_level(back);
    assert_eq!(game.dim, back, "back in the first level");
}

#[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::commands_server::clone_writes_tests::run;

/// The value `/clone` returned, through `execute store result`.
pub(crate) fn cloned(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    command: &str,
) -> (bool, Option<String>, i32) {
    assert!(game.run_command("scoreboard players set r h236 -1"));
    let said = run(
        game,
        &format!("execute store result score r h236 run {command}"),
    );
    let score = game
        .score
        .board
        .score("r", "h236")
        .map_or(i32::MIN, |s| s.value);
    (said.0, said.1, score)
}

/// The destination reaches one block above the build height, so that write
/// fails: vanilla reports the one block it did write, not the two it copied.
#[test]
#[ignore = "daily"]
fn a_clone_whose_destination_leaves_the_world_counts_only_the_writes_that_landed() {
    let mut resources = None;
    let mut game = game(&mut resources);
    load_chunk(&mut game, 0, 0);
    assert!(game.run_command("scoreboard objectives add h236 dummy"));
    assert!(game.run_command("setblock 0 300 0 stone"));
    assert!(game.run_command("setblock 0 301 0 stone"));
    assert_eq!(
        cloned(&mut game, "clone 0 300 0 0 301 0 0 319 0"),
        (true, Some("Successfully cloned 1 block(s)".to_owned()), 1),
        "the write at y=320 is outside the build height"
    );
    assert!(
        run(&mut game, "execute if block 0 319 0 stone").0,
        "the block that fits landed"
    );
}

/// The nether's build height ends at y=255 although its chunks hold a taller
/// column: a destination at y=255 writes one block and the y=256 write is
/// refused, so vanilla counts one.
#[test]
#[ignore = "daily"]
fn a_clone_into_the_nether_counts_by_the_nether_build_height() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let here = game.dim;
    load_chunk(&mut game, 0, 0);
    load_chunk_in(&mut game, Dim::Nether, here);
    assert!(game.run_command("scoreboard objectives add h236 dummy"));
    assert!(game.run_command("setblock 0 300 0 stone"));
    assert!(game.run_command("setblock 0 301 0 stone"));
    assert_eq!(
        cloned(
            &mut game,
            "clone 0 300 0 0 301 0 to minecraft:the_nether 0 255 0"
        ),
        (true, Some("Successfully cloned 1 block(s)".to_owned()), 1),
        "the nether's highest block is y=255"
    );
    assert_eq!(game.dim, here, "the command's level is back");
    assert!(
        run(
            &mut game,
            "execute in minecraft:the_nether if block 0 255 0 stone"
        )
        .0,
        "the block that fits landed in the nether"
    );
    // A corner above the nether's build height is out of the world there:
    // `BlockPosArgument.getLoadedBlockPos` checks `Level.isInWorldBounds`.
    assert_eq!(
        run(
            &mut game,
            "clone 0 300 0 0 301 0 to minecraft:the_nether 0 300 0"
        ),
        (
            false,
            Some("That position is out of this world!".to_owned())
        ),
        "the nether's build height ends at y=255"
    );
    assert_eq!(game.dim, here, "the command's level is back");
}

/// The corners' columns are loaded, a column a box spans is not: vanilla
/// refuses the clone (`Level.hasChunksAt` walks every column of both boxes).
#[test]
#[ignore = "daily"]
fn a_clone_refuses_a_box_that_spans_an_unloaded_column() {
    let mut resources = None;
    let mut game = game(&mut resources);
    load_chunk(&mut game, 0, 0);
    load_chunk(&mut game, 2, 0);
    assert!(game.run_command("scoreboard objectives add h236 dummy"));
    for (command, what) in [
        ("clone 0 100 0 32 100 0 0 200 0", "the source box"),
        ("clone 0 100 0 1 100 0 15 200 0", "the destination box"),
    ] {
        assert_eq!(
            run(&mut game, command),
            (false, Some("That position is not loaded".to_owned())),
            "{what} spans column 1, which is not loaded"
        );
        assert!(
            run(&mut game, "execute if block 0 200 0 air").0,
            "{what}: nothing was cloned"
        );
    }
    // The same boxes with the column they span loaded clone every block.
    load_chunk(&mut game, 1, 0);
    assert!(game.run_command("fill 0 100 0 32 100 0 stone"));
    assert_eq!(
        cloned(&mut game, "clone 0 100 0 32 100 0 0 200 0"),
        (true, Some("Successfully cloned 33 block(s)".to_owned()), 33)
    );
    assert!(run(&mut game, "execute if block 32 200 0 stone").0);
    assert_eq!(
        cloned(&mut game, "clone 0 100 0 1 100 0 15 200 0"),
        (true, Some("Successfully cloned 2 block(s)".to_owned()), 2)
    );
    assert!(run(&mut game, "execute if block 16 200 0 stone").0);
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
