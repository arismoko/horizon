use std::sync::Arc;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::boats_common::Boat;

/// `AbstractBoat.controlBoat` pushes along Mth's sine table at the float angle, in the order
/// vanilla evaluates it. The expected bits are the table's entries for yaw 31 (the paddle turned
/// the boat from 30 this tick) times 0.04f32, widened: they differ from `f32::sin` at 1e-6.
#[test]
#[ignore = "daily"]
fn a_paddled_boat_accelerates_by_the_sine_table_at_its_turned_yaw() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let mut boat = Boat::new(12, "oak".into(), Vec3::new(8.0, 70.0, 8.0), 30.0);
        // Right and forward: the oar turns the boat one degree, then it speeds up.
        boat.input = [false, true, true, false];
        let boat = game.tick_boat(boat, true);
        assert_eq!(boat.yaw, 31.0);
        assert_eq!(boat.velocity.x, f64::from_bits(0xbf95_183d_6000_0000));
        assert_eq!(boat.velocity.z, f64::from_bits(0x3fa1_8e1f_8000_0000));
    });
}

use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;
