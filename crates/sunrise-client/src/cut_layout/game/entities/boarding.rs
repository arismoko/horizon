#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1;
    use sunrise_gameplay_common::game::mounts_common::MountsCommonGameExt1;

    #[test]
    #[ignore = "daily"]
    fn retained_passengers_occupy_seats_in_boarding_order() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            use sunrise_core::geometry::Vec3;
            use sunrise_sim::mobs::Mob;
            game.build_registries();
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            for (id, order) in [(74, 2), (72, 0)] {
                let mut m = Mob::new(id, pig.clone(), Vec3::ZERO, 0.0);
                m.extra.mount = Some(71);
                m.extra.vehicle_order = order;
                game.persist.off_level.insert(
                    id,
                    sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(m),
                );
            }
            let mut resident = Mob::new(73, pig, Vec3::ZERO, 0.0);
            resident.extra.mount = Some(71);
            resident.extra.vehicle_order = 1;
            game.mobs.push(resident);
            assert_eq!(
                game.seated_passengers(71),
                vec![(0, 72, false), (1, 73, false), (2, 74, false)]
            );
            assert_eq!(game.boarding_order(71, 75, false), 3);
            assert_eq!(game.boarding_order(71, 75, true), -1);
            game.seat_mob(72);
            assert_eq!(game.mob_object(72).unwrap().extra.vehicle_order, 3);
        });
    }
    /// `Entity.addPassenger` appends a player when the first passenger is already one: this
    /// game's own player boarding after a guest and a pig sits third, not first.
    #[test]
    #[ignore = "daily"]
    fn the_own_player_boards_behind_a_guest_and_a_pig() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            use sunrise_core::geometry::Vec3;
            use sunrise_sim::mobs::Mob;
            game.build_registries();
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let (vehicle, rider) = (90, 91);
            game.mobs
                .push(Mob::new(vehicle, pig.clone(), Vec3::ZERO, 0.0));
            let order = game.boarding_order(vehicle, 12, true);
            let dimension = game.dim.id().to_owned();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.dimension = dimension;
                    g.vehicle = Some(vehicle);
                    g.vehicle_order = order;
                })
                .unwrap();
            let mut pig = Mob::new(rider, pig, Vec3::ZERO, 0.0);
            pig.extra.mount = Some(vehicle);
            game.mobs.push(pig);
            game.seat_mob(rider);
            game.start_riding(vehicle);
            let own = game.player.as_ref().unwrap().entity_id as u32;
            let aboard: Vec<u32> = game
                .seated_passengers(vehicle)
                .into_iter()
                .map(|(_, id, _)| id)
                .collect();
            assert_eq!(aboard, vec![12, rider, own]);
        });
    }
    /// `Entity.positionRider` picks the attachment by the passenger's index in `getPassengers`: the
    /// own player behind a guest and a pig sits in the third seat of a happy ghast, 1.7 south of it.
    #[test]
    #[ignore = "daily"]
    fn the_own_player_in_the_third_seat_of_a_happy_ghast_sits_there() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            use sunrise_core::geometry::Vec3;
            use sunrise_sim::mobs::Mob;
            game.build_registries();
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            let ghast = game
                .res
                .mob_types()
                .get("minecraft:happy_ghast")
                .unwrap()
                .clone();
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let (vehicle, rider) = (90, 91);
            game.mobs
                .push(Mob::new(vehicle, ghast, Vec3::new(8.5, 64.0, 8.5), 0.0));
            let order = game.boarding_order(vehicle, 12, true);
            let dimension = game.dim.id().to_owned();
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |g| {
                    g.dimension = dimension;
                    g.vehicle = Some(vehicle);
                    g.vehicle_order = order;
                })
                .unwrap();
            let mut pig = Mob::new(rider, pig, Vec3::ZERO, 0.0);
            pig.extra.mount = Some(vehicle);
            game.mobs.push(pig);
            game.seat_mob(rider);
            game.start_riding(vehicle);
            game.position_rider();
            let at = game.player.as_ref().unwrap().position;
            assert_eq!(
                (at.x, at.z),
                (8.5, 8.5 - 1.7),
                "the third seat, not the first"
            );
        });
    }
}
