use sunrise_gameplay::game::game_tests::tests::server_game;

/// CommandSourceStack.getDisplayName: a command block names itself, the
/// console says "Server".
#[test]
#[ignore = "daily"]
fn a_blocks_roll_names_the_block_and_the_console_says_server() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    game.set_allow_commands(true);
    assert!(game.run_command(
        r#"setblock 0 200 0 minecraft:command_block{Command:"random roll 1..2",CustomName:{text:"Dice"}}"#
    ));
    let roll = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                source: sunrise_gameplay::game::commands::execute::Source| {
        let outer = game.server.command_source.replace(source);
        game.server.chat_capture = Some(Vec::new());
        assert!(game.run_command("random roll 1..2"));
        game.server.command_source = outer;
        let lines = game.server.chat_capture.take().unwrap();
        assert_eq!(lines.len(), 1, "{:?}", lines[0].plain());
        lines[0].plain()
    };
    let block = sunrise_gameplay::game::commands::execute::Source {
        entity: None,
        block: Some((0, 200, 0)),
        ..game.server_source()
    };
    let named = roll(&mut game, block);
    assert!(named.starts_with("Dice rolled "), "{named}");
    assert!(named.ends_with(" (from 1 to 2)"), "{named}");
    let console = game.server_source();
    let named = roll(&mut game, console);
    assert!(named.starts_with("Server rolled "), "{named}");
    assert!(named.ends_with(" (from 1 to 2)"), "{named}");
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
