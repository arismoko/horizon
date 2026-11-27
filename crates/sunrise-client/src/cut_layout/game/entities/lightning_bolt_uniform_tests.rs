//! A summoned bolt reaches a shader pack's `lightningBoltPosition` through the frame's bolts
//! (`bolt_instances`, Sunrise's draw list): the last in the list, camera-relative, w = 1.
#![cfg(feature = "client")]
use crate::game::server_side::native_entity_refs::tests::with_client_game;
use sunrise_client::gfx::pack_uniforms::lightning_bolt_position;

#[test]
#[ignore = "daily"]
fn a_summoned_bolt_reaches_the_pack_in_player_space() {
    with_client_game(|game| {
        let camera = glam::DVec3::new(0.25, 70.5, 0.75);
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [0.0; 4]
        );
        game.spawn_remote_bolt(1, [10.5, 64.0, -5.25]);
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [10.25, -6.5, -6.0, 1.0]
        );
        // A bolt past every draw distance (`shouldRenderAtSqrDistance`) isn't in the draw
        // list, so the listed one still counts; one summoned in range goes last and takes over.
        game.spawn_remote_bolt(2, [1000.5, 64.0, 0.5]);
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [10.25, -6.5, -6.0, 1.0]
        );
        game.spawn_remote_bolt(3, [-3.5, 66.0, 2.75]);
        assert_eq!(
            lightning_bolt_position(&game.bolt_instances(camera), camera),
            [-3.75, -4.5, 2.0, 1.0]
        );
    });
}
