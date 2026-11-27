#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_resident_game;
    use mod_api::{EntityRef, MobRef, MoveToPathError, PathNode, WorldEpoch};
    use std::sync::Arc;
    use sunrise_core::geometry::Vec3;
    use sunrise_gameplay::game::server_side::native_mob_navigation::supplied_path_accepts;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt6;
    use sunrise_level::chunk::Chunk;
    use sunrise_sim::mobs::Mob;

    /// `moveTo(Path, speed)`'s boundary never gets stricter than vanilla's:
    /// finite coordinates and speed pass, a node count far above any vanilla
    /// search passes, and coordinates outside the block range are not refused
    /// (they are floored with the saturating cast and read as void air).
    #[test]
    fn a_supplied_path_only_refuses_non_finite_numbers() {
        let node = |x: f64, y: f64, z: f64| PathNode { x, y, z };
        assert!(supplied_path_accepts(&[], 1.0), "an empty path is taken");
        assert!(supplied_path_accepts(
            &[node(1.5, 64.0, -2.5), node(2.0, 64.0, -2.0)],
            0.3
        ));
        assert!(
            supplied_path_accepts(&[node(3.0e7, -3.0e7, 30_000_000.5)], 1.0),
            "past 30,000,000 is not refused"
        );
        assert!(
            supplied_path_accepts(&[node(1.0e300, -1.0e300, 0.0)], 1.0),
            "any finite coordinate is taken, floored by the saturating cast"
        );
        assert!(!supplied_path_accepts(&[node(f64::NAN, 64.0, 0.0)], 1.0));
        assert!(!supplied_path_accepts(
            &[node(0.0, f64::INFINITY, 0.0)],
            1.0
        ));
        assert!(!supplied_path_accepts(
            &[node(0.0, 64.0, f64::NEG_INFINITY)],
            1.0
        ));
        assert!(!supplied_path_accepts(&[node(0.0, 64.0, 0.0)], f64::NAN));
        assert!(
            !supplied_path_accepts(&[node(0.0, 64.0, 0.0)], f64::INFINITY),
            "a non-finite speed is refused as `move_to` refuses it"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn navigation_starts_a_real_path_and_refuses_stale_handles() {
        crate::game::server_side::entity_server::taken_pass_tests::in_both_dimensions(
            |game, balance| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 64, z, stone).unwrap();
                    }
                }
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone();
                let mut zombie = Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0);
                zombie.ground.on_ground = true;
                game.mobs.push(zombie);
                let entity = game.entity(99).unwrap();
                let mob = game.mob(entity).unwrap();
                assert_eq!(mob.is_done(game), Some(true));
                crate::game::server_side::entity_server::taken_pass_tests::assert_mob_spawn_survives(
                game,
                balance,
                |game| {
                    assert!(mob.move_to(game, [10.5, 65.0, 3.5], 1.25));
                },
            );
                assert_eq!(mob.is_done(game), Some(false));
                assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
                assert!(!mob.move_to(game, [f64::NAN, 65.0, 3.5], 1.0));
                assert_eq!(mob.is_done(game), Some(false));
                assert!(mob.stop(game));
                assert_eq!(mob.is_done(game), Some(true));
                assert!(!mob.move_to(game, [1000.0, 65.0, 0.0], 1.0));
                let compare_extreme = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                                       extreme: f64,
                                       boundary: f64| {
                    let result = mob.move_to(game, [10.5, boundary, 3.5], 1.0);
                    let path = game.mobs[0].ai.nav.path.clone();
                    mob.stop(game);
                    assert_eq!(mob.move_to(game, [10.5, extreme, 3.5], 1.0), result);
                    assert_eq!(game.mobs[0].ai.nav.path, path);
                    mob.stop(game);
                };
                let top = f64::from(game.world.top_y() - 1);
                let bottom = f64::from(game.world.bottom_y());
                compare_extreme(game, 2_000_000_000.0, top);
                compare_extreme(game, -2_000_000_000.0, bottom);
                game.world.shape =
                    sunrise_level::chunk::LevelShape::new(0, 128, 128, true, false).unwrap();
                assert_eq!(game.world.shape.max_y(), 128);
                compare_extreme(game, 2_000_000_000.0, 127.0);
                compare_extreme(game, -2_000_000_000.0, 0.0);
                game.world.shape = sunrise_level::chunk::LevelShape::OVERWORLD;
                let stale = MobRef::new(EntityRef::new(
                    WorldEpoch::new(entity.epoch().get() + 1),
                    99,
                ));
                assert!(!stale.move_to(game, [5.0, 65.0, 3.0], 1.0));
                assert!(!stale.stop(game));
                assert_eq!(stale.is_done(game), None);
                let taken = game.mobs.lift();
                assert_eq!(mob.is_done(game), None);
                assert!(!mob.stop(game));
                game.mobs
                    .restore(taken, sunrise_sim::join_seq::RestoreOrder::SurvivorsFirst);
                game.armor_stands
                    .push(sunrise_sim::entities::ArmorStand::new(102, Vec3::ZERO, 0.0));
                let wrong = MobRef::new(game.entity(102).unwrap());
                assert!(!wrong.move_to(game, [5.0, 65.0, 3.0], 1.0));
                assert!(!wrong.stop(game));
                assert_eq!(wrong.is_done(game), None);
                let parrot = game
                    .res
                    .mob_types()
                    .get("minecraft:parrot")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(100, parrot, Vec3::new(3.5, 66.0, 5.5), 0.0));
                let flying = game.mob(game.entity(100).unwrap()).unwrap();
                assert!(flying.move_to(game, [9.5, 66.0, 5.5], 1.0));
                assert_eq!(flying.is_done(game), Some(false));
                let cod = game.res.mob_types().get("minecraft:cod").unwrap().clone();
                game.mobs
                    .push(Mob::new(101, cod, Vec3::new(3.5, 65.0, 7.5), 0.0));
                let fish = game.mob(game.entity(101).unwrap()).unwrap();
                assert!(!fish.move_to(game, [9.5, 65.0, 7.5], 1.0));
                assert_eq!(fish.is_done(game), Some(true));
            },
        );
    }

    /// A refusal names its cause, so a mod tells an invalid number from a mob
    /// that is not there and from a path with nothing left to follow (#4443).
    #[test]
    #[ignore = "daily"]
    fn a_supplied_path_refusal_names_its_cause() {
        with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let entity = game.entity(99).unwrap();
            let mob = game.mob(entity).unwrap();
            let node = |x: f64, y: f64, z: f64| PathNode { x, y, z };
            assert_eq!(
                mob.move_to_path(game, vec![node(f64::NAN, 65.0, 3.5)], 1.0),
                Err(MoveToPathError::NotFinite)
            );
            assert_eq!(
                mob.move_to_path(game, Vec::new(), 1.0),
                Err(MoveToPathError::Empty),
                "an empty path has no node left to follow"
            );
            assert_eq!(
                mob.move_to_path(game, vec![node(3.5, 65.0, 3.5), node(4.5, 65.0, 3.5)], 1.0),
                Ok(()),
                "a path the navigation took answers Ok"
            );
            assert_eq!(mob.is_done(game), Some(false));
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                99,
            ));
            assert_eq!(
                stale.move_to_path(game, vec![node(3.5, 65.0, 3.5)], 1.0),
                Err(MoveToPathError::Unavailable)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn navigation_speed_is_the_paths_speed_modifier() {
        with_resident_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(99, kind, Vec3::new(3.5, 65.0, 3.5), 0.0));
            let entity = game.entity(99).unwrap();
            let mob = game.mob(entity).unwrap();
            // A fresh navigation has no path, so no speed was set for one.
            assert_eq!(mob.navigation_speed(game), Some(0.0));
            assert!(mob.set_navigation_speed(game, 1.25));
            assert_eq!(mob.navigation_speed(game), Some(1.25));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
            assert!(!mob.set_navigation_speed(game, f64::NAN));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
            // Not the movement-speed attribute.
            let attribute = game.mobs[0].movement_speed();
            assert_ne!(attribute, 1.25);
            let stale = MobRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                99,
            ));
            assert_eq!(stale.navigation_speed(game), None);
            assert!(!stale.set_navigation_speed(game, 1.0));
            assert_eq!(game.mobs[0].ai.nav.speed_modifier, 1.25);
        });
    }
}
