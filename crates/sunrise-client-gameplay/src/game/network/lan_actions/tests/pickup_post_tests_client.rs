#[allow(dead_code)]
pub fn host_animations(game: &crate::game::ClientGame<'_>) -> u64 {
    game.client
        .pickups
        .0
        .made()
        .get("item")
        .copied()
        .unwrap_or(0)
}
