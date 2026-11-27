/// Generic living camera only (oracle drives a deathTime stand-in, not a real creeper); the camera player's own fov modifier is open.
#[test]
#[ignore = "run by tools/test_camera_parity.py"]
fn a_spectated_dying_living_entity_squeezes_the_fov_26_3() {
    run_a_spectated_dying_living_entity_squeezes_the_fov_26_3();
}

#[allow(unused_imports)]
pub(in crate::game) use crate::game::client_side::remote_client::tests::camera_death_tests::run_a_spectated_dying_living_entity_squeezes_the_fov_26_3;
