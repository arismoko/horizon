fn entry(game: &mut ServerGame<'_>) {
    let time = game.common.world.game_time;
    if time % 100 == 0 {
        fnr_loop::note(&format!("HEAD tick_block_entities at game time {time}"));
    }
}
