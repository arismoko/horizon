/// Runs `command`: whether it succeeded and what the issuer was told.
pub fn run(game: &mut crate::game::ServerGame<'_>, command: &str) -> (bool, Vec<String>) {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.server.chat_capture.take().unwrap_or_default();
    (
        ok,
        said.iter()
            .map(sunrise_game_data::text::Text::plain)
            .collect(),
    )
}
