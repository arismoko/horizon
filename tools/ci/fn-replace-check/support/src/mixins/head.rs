fn head(game: &mut crate::game::ServerGame<'_>) {
    let ticks = game.server.__horizon_mixins_fnr_support_ticks();
    *ticks = crate::game::server_side::autosave_server::fnr_runtime::bumped(*ticks);
    fnr_support::note(&format!(
        "module and bridge: ticks_until_autosave {}",
        *ticks
    ));
}
