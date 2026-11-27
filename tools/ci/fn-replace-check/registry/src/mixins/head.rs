fn head(game: &mut crate::game::ServerGame<'_>) {
    fnr_registry::note(
        "HEAD on_tick_rate_changed, ticks_until_autosave",
        game.server.ticks_until_autosave,
    );
}
