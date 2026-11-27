#![cfg(test)]
use sunrise_core::geometry::Aabb;
use sunrise_core::geometry::Vec3;
use sunrise_gameplay_common::game::boats_common::Boat;
use sunrise_sim::join_seq::ListStamp;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::riders;
use sunrise_sim::mobs::seen_grid::SeenGrid;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_gameplay_common::game::boats_common::BoatsCommonGameExt1;
    use sunrise_gameplay_common::game::collision_source::CollisionSourceGameExt1;

    #[test]
    #[ignore = "daily"]
    fn rejected_boat_passengers_fill_actual_capacity_without_becoming_resident() {
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    71,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            for id in [72, 73] {
                let mut rider = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                rider.extra.mount = Some(71);
                game.persist.off_level.insert(
                    id,
                    sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(rider),
                );
            }
            assert!(game.boat_mobs(71).is_empty());
            // An ordinary oak boat has two seats; rejected passengers still own both.
            assert_eq!(game.seated_passenger_count(71), 2);
            assert!(!game.boat_has_room(0));
            game.mob_object_mut(73).unwrap().extra.mount = None;
            assert_eq!(game.seated_passenger_count(71), 1);
            assert!(game.boat_has_room(0));
        });
    }
    /// A mob pushes the cart its box touches while a mob far from every cart pushes none and
    /// is not pushed, whichever grid finds them: the carts' (four carts in four of its columns,
    /// asked once per mob while the mobs are the fewer) or, in a crowd, the mobs' (asked once
    /// per cart).
    #[test]
    #[ignore = "daily"]
    fn only_the_mob_touching_a_cart_pushes_it() {
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        let pushed = [0, 12].map(|crowd: u32| {
            let mut resources = None;
            let mut game = super::super::game_tests::tests::server_game(&mut resources);
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for (i, (x, z)) in [(0.5, 0.5), (20.5, 0.5), (0.5, 20.5), (20.5, 20.5)]
                .into_iter()
                .enumerate()
            {
                let at = Vec3::new(x, 64.0, z);
                game.carts
                    .push(Minecart::new(100 + i as u32, CartKind::Chest, at));
            }
            // 0.5 west of the second cart (boxes 0.9 and 0.98 wide), in the middle of them
            // all, and the crowd well away.
            let spots = [(20.0, 0.5), (10.5, 10.5)].into_iter();
            let crowd = (0..crowd).map(|i| (40.5 + 3.0 * f64::from(i), 40.5));
            for (i, (x, z)) in spots.chain(crowd).enumerate() {
                let at = Vec3::new(x, 64.0, z);
                game.mobs.push(Mob::new(1 + i as u32, pig.clone(), at, 0.0));
            }
            game.tick_carts();
            let velocity = |id| game.mobs.iter().find(|m| m.id == id).unwrap().velocity;
            assert!(velocity(1).x < 0.0, "pushed back west: {:?}", velocity(1));
            let idle = game.mobs.iter().filter(|m| m.velocity == Vec3::ZERO);
            assert_eq!(idle.count(), game.mobs.len() - 1);
            velocity(1)
        });
        assert_eq!(pushed[0], pushed[1]);
    }
    /// A vehicle's box query during a stretch of vehicle ticks names the mobs a walk over
    /// every box names, in the same order, also right after a mob moved into the box (a
    /// push or a pick-up writes the mob list in the middle of a stretch).
    #[test]
    #[ignore = "daily"]
    fn a_vehicles_box_query_matches_a_walk_over_every_mob() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        // A small LCG keeps the scene the same on every run.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = |range: f64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * range
        };
        for id in 1..=400 {
            let at = Vec3::new(next(48.0), 64.0 + next(3.0), next(48.0));
            game.mobs.push(Mob::new(id, pig.clone(), at, 0.0));
        }
        let mut met = 0;
        for round in 0..300 {
            // Three stretches, each with two writes to the mob list.
            if round % 100 == 0 {
                game.vehicles_ticking(true);
            }
            let corner = Vec3::new(next(46.0), 63.0 + next(3.0), next(46.0));
            let size = Vec3::new(0.5 + next(2.5), 0.5 + next(1.5), 0.5 + next(2.5));
            let area = Aabb::new(corner, corner.add_vector(size));
            if round % 40 == 20 {
                let j = round % game.mobs.len();
                game.mobs[j].position = area.center();
            }
            let walked: Vec<Aabb> = game
                .mobs
                .iter()
                .map(Mob::bounds)
                .filter(|b| b.intersects(area))
                .collect();
            met += walked.len();
            assert_eq!(
                game.entity_collisions(crate::game::collision_source::Mover::Boat {
                    id: u32::MAX,
                    area
                }),
                walked,
                "round {round}"
            );
            if round % 100 == 99 {
                // The last comparisons ran against the grid: it stands, for the list as it is.
                let fresh = |(stamp, _): &(ListStamp, SeenGrid)| *stamp == game.mobs.stamp();
                assert!(
                    game.server
                        .vehicle_mobs
                        .grid
                        .borrow()
                        .built
                        .as_ref()
                        .is_some_and(fresh)
                );
                game.vehicles_ticking(false);
            }
        }
        assert!(met > 100, "the boxes met {met} mobs");
    }
    /// Who rides a vehicle follows the mob list through a stretch of vehicle ticks: a mob that
    /// boards counts from then on, one removed no longer does, and a dead one still makes
    /// its cart a vehicle but is no passenger of a boat.
    #[test]
    #[ignore = "daily"]
    fn a_vehicle_knows_its_mob_riders_as_soon_as_they_board() {
        let mut resources = None;
        let mut game = super::super::game_tests::tests::server_game(&mut resources);
        let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        for id in 1..=4 {
            game.mobs.push(Mob::new(id, pig.clone(), Vec3::ZERO, 0.0));
        }
        let riders = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            [100, 101, 102].map(|id| (game.has_seated_passengers(id), game.boat_mobs(id)))
        };
        game.mobs[0].extra.mount = Some(100);
        game.vehicles_ticking(true);
        let none = (false, vec![]);
        assert_eq!(riders(&game), [(true, vec![0]), none.clone(), none.clone()]);
        game.mobs[0].removed = true;
        game.mobs[1].extra.mount = Some(101);
        game.mobs[3].extra.mount = Some(102);
        game.mobs[3].health = 0.0;
        let after = [none, (true, vec![1]), (true, vec![])];
        assert_eq!(riders(&game), after);
        game.vehicles_ticking(false);
        assert_eq!(riders(&game), after);
    }
}

#[cfg(test)]
mod off_level_tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn retained_boat_passenger_occupies_a_seat_without_being_positioned() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                &mut game.world,
                (0, 0),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let boat = Boat::new(71, "oak".into(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            let mut ghost = sunrise_sim::mobs::Mob::new(72, pig.clone(), Vec3::ZERO, 0.0);
            ghost.extra.mount = Some(71);
            ghost.extra.vehicle_order = 0;
            let mut resident = sunrise_sim::mobs::Mob::new(73, pig, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(71);
            resident.extra.vehicle_order = 1;
            let offset = riders::vehicle_attachment(&resident, 1.375);
            let expected = boat.seat(1, 2, true, offset, false);
            game.boats.push(boat);
            game.mobs.push(resident);
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(ghost),
            );
            game.position_riders();
            assert_eq!(game.mob_object(73).unwrap().position, expected);
            assert_eq!(game.mob_object(72).unwrap().position, Vec3::ZERO);
        });
    }
}

#[cfg(test)]
mod boat_callback_review_tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use sunrise_sim::mobs::Mob;

    struct MutateRider {
        discard: bool,
        heard: Arc<Mutex<Vec<(u32, bool)>>>,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for MutateRider {
        fn entity_mount(
            &self,
            host: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::EntityMountEvent<'_>,
        ) -> mod_api::Flow {
            self.heard
                .lock()
                .unwrap()
                .push((event.entity, event.mounting));
            if !event.mounting && event.entity == 72 {
                let rider = host.entity(72).unwrap();
                assert_eq!(event.vehicle, 71);
                assert_eq!(rider.is_passenger(host), Some(true));
                assert_eq!(rider.vehicle(host), None);
                assert!(
                    host.entity(71).is_none(),
                    "boat is already logically removed"
                );
                if self.discard {
                    assert!(rider.discard(host));
                } else {
                    let later = host.entity(73).unwrap();
                    let replacement = host.entity(74).unwrap();
                    assert!(later.start_riding(host, replacement, true).is_ok());
                }
            }
            mod_api::Flow::Continue
        }
    }
    #[test]
    #[ignore = "daily"]
    fn boat_ejection_preserves_a_later_passengers_callback_reparenting() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: false,
                heard: heard.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                    &mut game.world,
                    (0, 0),
                );
                game.options.dedicated = Some(Vec::new());
                game.boats
                    .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                        71,
                        "oak".into(),
                        Vec3::ZERO,
                        0.0,
                    ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for id in [72, 73, 74] {
                    let mut mob = Mob::new(id, kind.clone(), Vec3::ZERO, 0.0);
                    mob.extra.mount = (id != 74).then_some(71);
                    mob.extra.vehicle_order = i64::from(id);
                    game.mobs.push(mob);
                }
                assert!(game.kill_vehicle_object(
                    sunrise_gameplay_common::game::commands_common::Target::Boat(71)
                ));
                assert_eq!(game.mob_object(73).unwrap().extra.mount, Some(74));
                assert_eq!(
                    heard
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|&&(id, mounting)| id == 73 && !mounting)
                        .count(),
                    1,
                    "only the callback's deliberate reparenting dismounts this rider"
                );
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn boat_ejection_does_not_snap_a_rider_discarded_by_its_callback() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "mutate_rider",
            Box::new(MutateRider {
                discard: true,
                heard: Arc::default(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                sunrise_gameplay_common::game::test_fixtures::fixture_resident_column(
                    &mut game.world,
                    (0, 0),
                );
                game.options.dedicated = Some(Vec::new());
                game.boats
                    .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                        71,
                        "oak".into(),
                        Vec3::ZERO,
                        0.0,
                    ));
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let original = Vec3::new(2.0, 60.0, 3.0);
                let mut rider = Mob::new(72, kind.clone(), original, 0.0);
                rider.extra.mount = Some(71);
                game.mobs.push(rider);
                let mut child = Mob::new(73, kind, original, 0.0);
                child.extra.mount = Some(72);
                game.mobs.push(child);
                let mut boat = game.boats[0].clone();
                boat.position = Vec3::new(40.0, 70.0, 40.0);
                game.boats.store_back(0, boat);
                assert!(game.kill_vehicle_object(
                    sunrise_gameplay_common::game::commands_common::Target::Boat(71)
                ));
                assert!(game.mob_object(72).unwrap().removed);
                assert_eq!(game.mob_object(72).unwrap().position, original);
                assert_eq!(game.mob_object(73).unwrap().position, original);
            });
        });
    }
}

use sunrise_gameplay_common::game::passengers_common::PassengersCommonGameExt1 as _;
