fn before(game: &crate::game::ServerGame<'_>) {
    fnr_beta::note(&format!(
        "BEFORE_CALL next_autosave_interval, rate {}",
        game.common.clock.rate()
    ));
}
