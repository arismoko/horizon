use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client;
use sunrise_core::geometry::Vec3;
use sunrise_sim::motion::PlayerMotion;

#[allow(unused_imports)]
pub(crate) use crate::game::client_side::remote_client::fall_sound::tests::heard_stone_fall;

#[test]
#[ignore = "daily"]
fn remote_five_block_fall_plays_stone_locally_once() {
    assert_eq!(heard_stone_fall(false), vec![(0.5, 0.75)]);
}

#[test]
#[ignore = "daily"]
fn a_silent_local_player_hears_no_block_fall_sound() {
    assert_eq!(heard_stone_fall(true), Vec::new());
}

/// LocalPlayer.tick runs once the client has loaded, wherever the player is: where no chunk has
/// come yet there are no blocks, as in vanilla's empty chunk, so the player falls (its first move
/// is at rest; gravity then 0.98 leaves -0.0784 for the next).
#[test]
#[ignore = "daily"]
fn a_loaded_client_falls_where_no_chunk_has_come() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.client.remote.as_mut().unwrap().loaded_sent = true;
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap());
            game.tick_remote().unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position.y, 64.0);
            assert!(
                (player.velocity.y - -0.0784).abs() < 1e-6,
                "{}",
                player.velocity.y
            );
            game.tick_remote().unwrap();
            assert!(game.player.as_ref().unwrap().position.y < 64.0);
        })
    });
}
