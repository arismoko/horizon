fn stop_message_probe(game: &mut Game<'_>) {
    let players = game.native_server_players();
    let mut results = Vec::new();
    for player in players {
        let result = game.native_net_send_to_player(player.uuid, "stop_probe:stop", b"farewell");
        results.push(format!("{}:{result:?}", player.name));
    }
    if let Ok(path) = std::env::var("STOP_SENT") {
        let _ = std::fs::write(path, results.join("\n"));
    }
}
