#[cfg(test)]
use sunrise_core::geometry::Vec3;
#[cfg(test)]
use sunrise_gameplay_common::game::boats_common::Boat;
#[cfg(test)]
pub(crate) mod friction_tests;
#[cfg(test)]
pub(crate) mod lifetime_tests;
#[cfg(test)]
pub(crate) mod paddle_tests;

#[cfg(test)]
pub(crate) mod tests {

    use super::*;

    use crate::game::boats_common::Status;

    /// A boat the client takes over while it eases toward the server's position goes on
    /// easing there and reports that spot (the handler is the entity's, not the driver's).
    #[test]
    #[ignore = "daily"]
    fn a_boat_taken_over_mid_way_goes_on_easing() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut b = Boat::new(9, "oak".into(), Vec3::new(0.5, 64.0, 2.5), 0.0);
            b.go_on_easing(Vec3::new(0.5, 64.0, 3.5), (0.0, 0.0), 2);
            assert_eq!(b.client_position().0, Vec3::new(0.5, 64.0, 3.5));
            game.interpolate_driven_boat(&mut b);
            assert!((b.position.z - 3.0).abs() < 1e-9, "{}", b.position.z);
            game.interpolate_driven_boat(&mut b);
            assert!((b.position.z - 3.5).abs() < 1e-9, "{}", b.position.z);
            assert!(b.lerp.is_none());
        });
    }
    /// The boat this client drives eases to a server teleport over three ticks
    /// (LinearInterpolationHandler), and until it gets there it reports the spot it is
    /// going to (getClientPositionAndRotation).
    #[test]
    #[ignore = "daily"]
    fn a_driven_boat_eases_to_a_teleport_and_reports_where_it_is_going() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let mut b = Boat::new(9, "oak".into(), Vec3::new(0.5, 64.0, 2.5), 0.0);
            b.interpolate_to(Vec3::new(0.5, 64.0, 6.5), 90.0, 0.0);
            let mut steps = Vec::new();
            for _ in 0..3 {
                game.interpolate_driven_boat(&mut b);
                steps.push((b.position.z, b.yaw, b.client_position()));
            }
            let expected = [
                (2.5 + 4.0 / 3.0, 30.0),
                (2.5 + 8.0 / 3.0, 60.0),
                (6.5, 90.0),
            ];
            for (i, ((z, yaw, _), (want_z, want_yaw))) in steps.iter().zip(expected).enumerate() {
                assert!((z - want_z).abs() < 1e-9, "step {i}: z {z}");
                assert!((yaw - want_yaw).abs() < 1e-4, "step {i}: yaw {yaw}");
            }
            assert_eq!(steps[0].2, (Vec3::new(0.5, 64.0, 6.5), 90.0, 0.0));
            assert_eq!(steps[1].2, (Vec3::new(0.5, 64.0, 6.5), 90.0, 0.0));
            assert_eq!(steps[2].2, (b.position, b.yaw, b.pitch));
            assert!(b.lerp.is_none());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn destruction_and_underwater_ejection_snap_rejected_passenger_descendants() {
        for broken in [false, true] {
            super::super::native_entity_refs::tests::with_game(|game| {
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut boat = Boat::new(71, "oak".into(), Vec3::new(10.0, 200.0, 20.0), 0.0);
                boat.status = Status::UnderWater;
                boat.out_of_control = 60.0;
                game.boats.push(boat);
                let mut rider =
                    sunrise_sim::mobs::Mob::new(72, kind.clone(), Vec3::new(5.0, 190.0, 6.0), 0.0);
                rider.extra.mount = Some(71);
                rider.extra.passenger = true;
                game.persist.off_level.insert(
                    72,
                    sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(rider),
                );
                let mut child = sunrise_sim::mobs::Mob::new(73, kind, Vec3::ZERO, 0.0);
                child.extra.mount = Some(72);
                child.age = 19;
                game.mobs.push(child);
                if broken {
                    assert!(game.hurt_boat(0, 5.0, true));
                    assert!(game.boats.is_empty());
                } else {
                    // The actual ejection entry used by AbstractBoat.tick at 60 submerged ticks.
                    game.eject_boat_riders(0);
                    assert_eq!(game.boats.len(), 1);
                }
                let rider = game.mob_object(72).unwrap();
                assert_eq!(rider.extra.mount, None);
                assert!(!rider.extra.passenger);
                assert_eq!(rider.extra.boarding_cooldown, 60);
                // Removed vehicle: retain x/z and clamp y to the boat's y.
                // Live underwater boat over air: getDismountLocation's box-top fallback.
                let expected = if broken {
                    Vec3::new(5.0, 200.0, 6.0)
                } else {
                    Vec3::new(10.0, 200.5625, 20.0)
                };
                assert_eq!(rider.position, expected);
                let child = game.mob_object(73).unwrap();
                assert_eq!(child.position.x, expected.x);
                assert_eq!(child.position.z, expected.z);
                assert!((child.position.y - expected.y - 0.86875).abs() < 1e-6);
                assert_eq!(child.previous, child.position);
                assert_eq!(child.age, 19);
                assert_eq!(child.extra.mount, Some(72));
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod cobweb_tests {
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_gameplay_common::game::boats_common::{Boat, BoatsCommonGameExt1};
    use sunrise_level::chunk::Chunk;

    /// `AbstractBoat.tick`'s `applyEffectsFromBlocks` calls `WebBlock.entityInside`, which
    /// makes the boat stuck: its next move goes a quarter of the speed across and a
    /// twentieth down, its velocity is spent and its fall is reset. A boat outside the web
    /// moves as it did and keeps falling.
    #[test]
    #[ignore = "daily"]
    fn a_cobweb_slows_a_boat_and_resets_its_fall() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let web = game
            .pack
            .assets
            .registry
            .default_state("minecraft:cobweb")
            .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.set(8, 64, 8, web).unwrap();
        let boat = |x: f64| {
            let mut b = Boat::new(1, Arc::from("oak"), Vec3::new(x, 64.0, 8.5), 0.0);
            b.velocity = Vec3::new(1.0, 0.0, 0.0);
            b.fall_distance = 5.0;
            b
        };
        let (mut in_web, mut outside) = (boat(8.5), boat(12.5));
        // The tick in the web sets the multiplier and resets the fall.
        in_web = game.tick_boat(in_web, false);
        outside = game.tick_boat(outside, false);
        let (web_fall, free_fall) = (in_web.fall_distance, outside.fall_distance);
        // The next move spends the multiplier.
        let (web_from, free_from) = (in_web.position, outside.position);
        in_web = game.tick_boat(in_web, false);
        outside = game.tick_boat(outside, false);
        let (web_moved, free_moved) = (
            in_web.position.subtract(web_from),
            outside.position.subtract(free_from),
        );
        assert!(
            free_moved.x > 0.5,
            "the boat outside the web moves: {free_moved:?}"
        );
        assert_eq!(
            web_moved.x,
            free_moved.x * 0.25,
            "a quarter of the speed across"
        );
        assert_eq!(web_moved.z, 0.0);
        assert!(
            (web_moved.y - free_moved.y * f64::from(0.05_f32)).abs() < 1e-12,
            "a twentieth down: {} against {}",
            web_moved.y,
            free_moved.y
        );
        assert_eq!(
            in_web.velocity,
            Vec3::ZERO,
            "the multiplier and velocity are spent"
        );
        assert_eq!(web_fall, 0.0, "makeStuckInBlock resets the fall");
        assert!(
            free_fall > 5.0,
            "a boat outside the web keeps falling: {free_fall}"
        );
    }

    /// A boat whose box reaches a cobweb and, after the move, powder snow takes
    /// the multiplier of the block vanilla's traversal reaches last: the one
    /// after the move, not the one it leaves.
    #[test]
    #[ignore = "daily"]
    fn a_boat_leaving_a_cobweb_for_powder_snow_takes_the_powder_snow() {
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let (web, snow) = {
            let registry = &game.pack.assets.registry;
            (
                registry.default_state("minecraft:cobweb").unwrap(),
                registry.default_state("minecraft:powder_snow").unwrap(),
            )
        };
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.world.set(8, 64, 8, web).unwrap();
        game.world.set(10, 64, 8, snow).unwrap();
        let mut boat = Boat::new(1, Arc::from("oak"), Vec3::new(8.5, 64.0, 8.5), 0.0);
        // Far enough in one move to leave the cobweb's cell and reach the snow's.
        boat.velocity = Vec3::new(2.0, 0.0, 0.0);
        boat = game.tick_boat(boat, false);
        // The same boat without the stored multiplier: the next move is the full one.
        let mut free = boat.clone();
        free.stuck = Vec3::ZERO;
        let (from, free_from) = (boat.position, free.position);
        boat = game.tick_boat(boat, false);
        free = game.tick_boat(free, false);
        let (moved, free_moved) = (
            boat.position.subtract(from),
            free.position.subtract(free_from),
        );
        assert!(
            free_moved.x > 1.0,
            "the boat outside the blocks moves: {free_moved:?}"
        );
        assert!(
            (moved.x - free_moved.x * f64::from(0.9_f32)).abs() < 1e-12,
            "powder snow's across multiplier: {} against {}",
            moved.x,
            free_moved.x
        );
        assert!(
            (moved.y - free_moved.y * 1.5).abs() < 1e-12,
            "powder snow's down multiplier: {} against {}",
            moved.y,
            free_moved.y
        );
    }
}
#[cfg(test)]
use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1 as _;

pub(crate) mod below_world;
