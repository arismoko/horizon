/// Runs `command`: whether it succeeded and what it said first.
pub fn run(game: &mut crate::game::ServerGame<'_>, command: &str) -> (bool, Option<String>) {
    game.server.chat_capture = Some(Vec::new());
    let ok = game.run_command(command);
    let said = game.server.chat_capture.take().unwrap_or_default();
    (ok, said.first().map(sunrise_game_data::text::Text::plain))
}
