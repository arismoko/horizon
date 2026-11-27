use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
use sunrise_core::geometry::Vec3;
use sunrise_level::chunk::Chunk;
use sunrise_sim::motion::PlayerMotion;

/// The `minecraft:block.stone.fall` sounds the landing player's own client plays
/// for a 5-block fall onto stone, as (volume, pitch).
pub(crate) fn heard_stone_fall(silent: bool) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            game.client.remote.as_mut().unwrap().local_entity_silent = silent;
            game.client
                .streaming
                .insert_remote(&mut game.common.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(1, 63, 1, stone).unwrap();
            let mut player = PlayerMotion::new(game.motion, Vec3::new(1.5, 64.1, 1.5)).unwrap();
            player.fall_distance = 5.0;
            player.velocity = Vec3::new(0.0, -0.2, 0.0);
            game.player = Some(player);
            game.client.audio.0.capture = Some(Vec::new());
            game.tick_remote().unwrap();
            out = game
                .client
                .audio
                .0
                .capture
                .as_ref()
                .unwrap()
                .iter()
                .filter(|s| s.event == "minecraft:block.stone.fall")
                .map(|s| (s.volume, s.pitch))
                .collect();
        })
    });
    out
}
