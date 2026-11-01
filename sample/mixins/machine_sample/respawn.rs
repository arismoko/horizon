fn sample_respawn(game: &Game<'_>, player: u128, name: &str) {
    let players = game.native_server_players();
    let result = game.native_tell(player, Text::literal("Respawn hook"));
    eprintln!("RESPAWN_TEST uuid={player} name={name} serving={} known={} tell={result}", game.serving, players.len());
}
