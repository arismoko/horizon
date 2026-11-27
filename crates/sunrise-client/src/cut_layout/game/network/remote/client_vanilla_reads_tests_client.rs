use sunrise_gameplay_common::game::Screen;
use sunrise_level::chunk::Chunk;
use sunrise_protocol::packets::play::{self, Clientbound as C};

/// The server's teleport that makes the local player, on a stone floor, with the level loaded.
pub(in crate::game) fn stand(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    teleport: i32,
) {
    game.client
        .streaming
        .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    for x in 0..16 {
        for z in 0..16 {
            game.world.set(x, 63, z, stone).unwrap();
        }
    }
    game.handle_packet(C::PlayerPosition(play::PlayerPosition {
        teleport,
        position: [0.5, 64.0, 0.5],
        delta: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        relatives: 0,
    }))
    .unwrap();
    let remote = game.client.remote.as_mut().unwrap();
    remote.loaded_sent = true;
    remote.level_load.finish();
    game.client.screen = Screen::Playing;
}
