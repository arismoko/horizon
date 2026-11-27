#![cfg(test)]
#[cfg(test)]
pub(crate) mod tests {

    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    use sunrise_core::geometry::Vec3;

    #[test]
    #[ignore = "daily"]
    fn reset_fall_distance_forgets_a_fall_for_mobs_carts_and_players() {
        use sunrise_gameplay_common::game::minecarts::{CartKind, Minecart};
        use sunrise_sim::mobs::Mob;
        fn fake_fall<'a>(game: &'a mut sunrise_gameplay::game::ServerGame<'_>) -> &'a mut f64 {
            let host = game.server.lan.as_mut().unwrap();
            let conn = host.hub.guests.iter().find(|g| g.uuid == 2).unwrap().conn;
            &mut host
                .hub
                .guests
                .motion_mut(conn)
                .expect("a fake has a motion")
                .fall_distance
        }
        super::super::native_entity_refs::tests::with_resident_game(|game| {
            sunrise_gameplay::game::server_side::bossbars::audience_tests::add_real_and_fake(game);
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut zombie = Mob::new(40, kind, Vec3::ZERO, 0.0);
            zombie.fall_distance = 12.0;
            game.mobs.push(zombie);
            let mut cart = Minecart::new(41, CartKind::Rideable, Vec3::ZERO);
            cart.fall_distance = 7.0;
            game.carts.push(cart);
            let (zombie, cart) = (game.entity(40).unwrap(), game.entity(41).unwrap());
            assert!(game.entity_reset_fall_distance(zombie));
            assert!(game.entity_reset_fall_distance(cart));
            assert_eq!(game.mobs[0].fall_distance, 0.0);
            assert_eq!(game.carts[0].fall_distance, 0.0);
            // A player: the fake guest's own motion.
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 2, "fake").unwrap().entity();
            *fake_fall(game) = 9.0;
            assert!(game.entity_reset_fall_distance(fake));
            assert_eq!(*fake_fall(game), 0.0);
            // An entity that is gone: nothing.
            game.mobs.clear();
            assert!(!game.entity_reset_fall_distance(zombie));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn only_a_mount_its_rider_controls_takes_the_riders_speed() {
        use sunrise_gameplay_common::game::commands_common::Target;
        use sunrise_sim::mobs::{Mob, equine::Rider};
        super::super::native_entity_refs::tests::with_game(|game| {
            for (id, name) in [(30, "minecraft:horse"), (31, "minecraft:pig")] {
                let kind = game.res.mob_types().get(name).unwrap().clone();
                let mut m = Mob::new(id, kind, Vec3::ZERO, 0.0);
                m.rider = Some(Rider::default());
                game.mobs.push(m);
            }
            // A horse being broken in, unsaddled: its own speed (AbstractHorse.java:962).
            assert!(!game.native_steered(Target::Mob(30)));
            game.mobs[0].extra.set_saddled(true);
            assert!(game.native_steered(Target::Mob(30)));
            // A saddled pig whose rider holds no stick (Pig.java:97-100).
            game.mobs[1].extra.set_saddled(true);
            assert!(!game.native_steered(Target::Mob(31)));
            game.mobs[1].rider.as_mut().unwrap().steer = true;
            assert!(game.native_steered(Target::Mob(31)));
        });
    }
}
