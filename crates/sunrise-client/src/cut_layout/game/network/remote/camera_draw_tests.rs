/// The local player and a villager drawn over camera entity, camera type and
/// sleeping, against what the real 26.3 `LevelExtractor` drew.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn local_player_and_camera_entity_draw_like_the_real_client() {
    run_local_player_and_camera_entity_draw_like_the_real_client();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_draw_tests::run_local_player_and_camera_entity_draw_like_the_real_client;
