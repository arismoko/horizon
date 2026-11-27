use crate::game::chat_common::end_clock_tests::game;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_sim::save::Level;

/// `/time <line>`; the feedback it printed.
pub(crate) fn run(game: &mut sunrise_gameplay::game::ServerGame<'_>, line: &str) -> String {
    game.server.chat_capture = Some(Vec::new());
    assert!(game.run_command(&format!("time {line}")), "/time {line}");
    let lines = game.server.chat_capture.take().unwrap();
    lines.last().expect("feedback").plain()
}

#[test]
#[ignore = "daily"]
fn time_moves_the_clock_the_end_runs_on() {
    let mut resources = None;
    let mut game = game(&mut resources);
    let world = std::env::temp_dir().join(format!("sunrise-end-clock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    game.server.world_root = world.clone();
    // `DimensionTypes`: the End's dimension type's default clock.
    assert_eq!(
        game.dimension_type().default_clock.as_deref(),
        Some("minecraft:the_end")
    );
    assert_eq!(
        run(&mut game, "set 5000"),
        "Set minecraft:the_end to 5000 tick(s)"
    );
    // `/time` and the End's own clock are one instance, as in vanilla.
    assert_eq!(game.world.end_clock, 5_000);
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 5000 tick(s)"
    );
    // It grows each tick the End's clock advances.
    game.advance_clock();
    game.advance_clock();
    assert_eq!(game.world.end_clock, 5_002);
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 5002 tick(s)"
    );
    assert_eq!(
        run(&mut game, "add 1200"),
        "Set minecraft:the_end to 6202 tick(s)"
    );
    // `/time pause` stops that same clock, and `resume` starts it again.
    assert_eq!(run(&mut game, "pause"), "Paused clock minecraft:the_end");
    for _ in 0..5 {
        game.advance_clock();
    }
    assert_eq!(game.world.end_clock, 6_202);
    assert_eq!(run(&mut game, "resume"), "Resumed clock minecraft:the_end");
    game.advance_clock();
    assert_eq!(game.world.end_clock, 6_203);
    // The clock and its pause survive a save: vanilla packs every clock into
    // world_clocks.dat, and the world opens with them.
    assert_eq!(
        run(&mut game, "set 400"),
        "Set minecraft:the_end to 400 tick(s)"
    );
    assert_eq!(run(&mut game, "pause"), "Paused clock minecraft:the_end");
    game.sync_level();
    game.level.write(&world, &Default::default()).unwrap();
    game.save_world_state();
    let level = Level::read(&world).unwrap();
    assert_eq!(level.end_clock, Some(400));
    game.level = level;
    game.world.end_clock = game.level.end_clock.unwrap_or(game.level.time);
    game.load_world_state().unwrap();
    assert_eq!(
        run(&mut game, "query time"),
        "Clock minecraft:the_end is at 400 tick(s)"
    );
    game.advance_clock();
    assert_eq!(game.world.end_clock, 400, "the pause came back too");
    assert_eq!(run(&mut game, "resume"), "Resumed clock minecraft:the_end");
    game.advance_clock();
    assert_eq!(game.world.end_clock, 401);
    let _ = std::fs::remove_dir_all(&world);
}
