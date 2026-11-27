#![cfg(test)]
use mod_api::{ArrowPickup, Deflection, EntityRef, PierceLevelError, ProjectileRef};
use sunrise_core::geometry::Vec3;
use sunrise_sim::entities::arrows::Pickup;
use sunrise_sim::mobs::Who;
use sunrise_sim::projectiles::Projectile;
use sunrise_sim::projectiles::ProjectileKind;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::WorldEpoch;
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay_common::game::fireworks::Rocket;
    use sunrise_gameplay_common::game::test_fixtures::fixture_resident_column;
    use sunrise_sim::thrown_trident::ThrownTrident;
    use sunrise_sim::{mobs::Mob, motion::PlayerMotion};

    #[test]
    #[ignore = "daily"]
    fn a_boat_owned_snowball_excludes_its_owner_until_it_leaves() {
        use sunrise_core::geometry::Aabb;
        use sunrise_sim::projectiles::{Hit, ProjectileWorld};
        struct Air(Vec<(Who, Aabb)>);
        impl ProjectileWorld for Air {
            fn clip(&self, _: Vec3, _: Vec3) -> Option<(Vec3, [i32; 3], [i32; 3])> {
                None
            }
            fn in_water(&self, _: Aabb) -> bool {
                false
            }
            fn targets(&self) -> &[(Who, Aabb)] {
                &self.0
            }
        }
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            fixture_resident_column(&mut game.world, (-1, 0));
            game.options.dedicated = Some(Vec::new());
            let boat_id = game.take_entity_id();
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            let boat = game.entity(boat_id).unwrap();
            let id = game.take_entity_id();
            let mut p = Projectile::new(ProjectileKind::Snowball, Vec3::new(-1.1, 0.2, 0.0), None);
            p.id = id;
            p.velocity = Vec3::new(1.0, 0.0, 0.0);
            game.creatures.projectiles.push(p);
            let entity = game.entity(id).unwrap();
            let projectile = game.projectile(entity).unwrap();
            assert!(game.projectile_set_owner(projectile, Some(boat)));
            assert_eq!(game.projectile_owner(projectile), Some(boat));
            let scene = Air(game.pickable_object_targets());
            let mut p = game.creatures.projectiles[0].clone();
            assert!(!p.left_owner);
            assert_eq!(p.owner, None, "a vehicle must not credit a living attacker");
            assert_eq!(p.owner_uuid, Some(game.uuid_for(boat_id)));
            let mut control = p.clone();
            control.owner = Some(Who::Mob(boat_id));
            assert_eq!(control.tick(&scene).1, Hit::None);
            let mut returned = p.clone();
            returned.left_owner = true;
            assert!(
                matches!(returned.tick(&scene).1, Hit::Entity { who: Who::Mob(id), .. } if id == boat_id)
            );
            assert_eq!(p.tick(&scene).1, Hit::None);
            // Reload mid-flight, before leftOwner: the UUID restores exclusion too.
            let keys = std::collections::HashSet::from([(-1, 0)]);
            let (_, tag) = game.save_projectile(0, &keys).unwrap();
            game.creatures.projectiles.clear();
            game.load_listed("minecraft:snowball", Some([1, 2, 3, 4]), None, tag)
                .unwrap();
            let mut loaded = game.creatures.projectiles[0].clone();
            assert!(!loaded.left_owner);
            assert_eq!(loaded.owner, None);
            assert_eq!(loaded.tick(&scene).1, Hit::None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_vehicle_owned_trident_does_not_scale_damage_as_a_living_attacker() {
        use sunrise_sim::projectiles::Hit;
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Easy;
            let boat_id = game.take_entity_id();
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            let boat = game.entity(boat_id).unwrap();
            game.add_projectile(Projectile::new(
                ProjectileKind::Trident,
                Vec3::new(3.0, 0.0, 0.0),
                None,
            ));
            let id = game.creatures.projectiles[0].id;
            let projectile = game.projectile(game.entity(id).unwrap()).unwrap();
            assert!(game.projectile_set_owner(projectile, Some(boat)));
            let p = game.creatures.projectiles[0].clone();
            game.projectile_hit(
                &p,
                Hit::Entity {
                    who: Who::Player,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            // Vanilla's 8 damage is not Easy's living-attacker reduction to 5.
            assert_eq!(game.player.as_ref().unwrap().health.health(), 12.0);
        });
    }

    fn arrow(owner: Who) -> Projectile {
        let kind = ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        };
        let mut p = Projectile::new(kind, Vec3::new(0.0, 64.0, 0.0), Some(owner));
        p.velocity = Vec3::new(1.0, 2.0, 3.0);
        p
    }

    #[test]
    #[ignore = "daily"]
    fn projectile_handles_read_and_write_owner_deflection_and_arrow_state() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();
            game.add_projectile(arrow(Who::Mob(99)));
            let id = game.creatures.projectiles[0].id;
            let entity = game.entity(id).unwrap();
            assert!(game.projectile(mob).is_none());
            let handle = game.projectile(entity).unwrap();
            assert_eq!(handle.entity(), entity);

            assert_eq!(handle.owner(game), Some(mob));
            assert!(handle.set_owner(game, None));
            assert_eq!(handle.owner(game), None);
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert!(handle.set_owner(game, Some(mob)));
            assert_eq!(handle.owner(game), Some(mob));
            assert_eq!(game.creatures.projectiles[0].owner, Some(Who::Mob(99)));
            // A non-living owner has collision identity but no living hit credit.
            let boat_id = game.take_entity_id();
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            let boat = game.entity(boat_id).unwrap();
            assert!(handle.set_owner(game, Some(boat)));
            assert_eq!(handle.owner(game), Some(boat));
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert_eq!(game.creatures.projectiles[0].owner_vehicle, Some(boat_id));
            let saved = game.uuid_for(boat_id);
            assert_eq!(game.creatures.projectiles[0].owner_uuid, Some(saved));
            assert!(handle.set_owner(game, Some(mob)));

            assert_eq!(handle.pierce_level(game), Ok(0));
            let role = game.options.execution_role;
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            assert_eq!(handle.pierce_level(game), Err(PierceLevelError::WrongSide));
            assert_eq!(
                handle.set_pierce_level(game, 1),
                Err(PierceLevelError::WrongSide)
            );
            game.options.execution_role = role;
            for level in [i8::MIN, -1, 0, 1, i8::MAX] {
                assert_eq!(handle.set_pierce_level(game, level), Ok(()));
                assert_eq!(handle.pierce_level(game), Ok(level));
            }
            assert_eq!(handle.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(handle.set_pickup(game, ArrowPickup::CreativeOnly));
            assert_eq!(
                game.creatures.projectiles[0].arrow.as_ref().unwrap().pickup,
                Pickup::CreativeOnly
            );
            assert_eq!(handle.base_damage(game), Some(2.0));
            assert!(handle.set_base_damage(game, 7.5));
            assert_eq!(handle.base_damage(game), Some(7.5));
            assert!(handle.set_base_damage(game, f64::NAN));
            assert!(handle.base_damage(game).unwrap().is_nan());
            assert_eq!(handle.critical(game), Some(false));
            assert!(handle.set_critical(game, true));
            assert!(game.creatures.projectiles[0].critical());

            // REVERSE: one float from the projectile's `Entity.random`, then
            // the scaled velocity.
            let draw = game.creatures.projectiles[0].random.0.clone().next_float();
            let yaw = game.creatures.projectiles[0].yaw;
            assert!(handle.deflect(game, Deflection::Reverse, None, None, false, [0.2; 3]));
            let p = &game.creatures.projectiles[0];
            assert_eq!(
                p.velocity,
                Vec3::new(1.0 * (-0.2 * 0.5), 2.0 * (-0.2 * 0.5), 3.0 * (-0.2 * 0.5))
            );
            assert_eq!(p.yaw, yaw + (170.0 + draw * 20.0));
            assert_eq!(p.owner, None);
            assert!(p.needs_sync);

            // AIM_DEFLECT along the deflector's look, scaled by the power.
            let look = game.entity_view_vector(mob).unwrap();
            assert!(handle.deflect(
                game,
                Deflection::AimDeflect,
                Some(mob),
                Some(mob),
                true,
                [2.0; 3]
            ));
            let p = &game.creatures.projectiles[0];
            assert_eq!(
                p.velocity,
                Vec3::new(look[0] * 2.0, look[1] * 2.0, look[2] * 2.0)
            );
            assert_eq!(p.owner, Some(Who::Mob(99)));

            // MOMENTUM_DEFLECT of a mob that has not moved has no direction.
            assert!(handle.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(mob),
                None,
                false,
                [1.0; 3]
            ));
            assert_eq!(game.creatures.projectiles[0].velocity, Vec3::ZERO);

            // A hurting projectile's acceleration: 0.1 by an attack, else halved.
            let mut ball = Projectile::fireball(
                ProjectileKind::LargeFireball { power: 1 },
                Vec3::ZERO,
                None,
                [1.0, 0.0, 0.0],
            );
            ball.acceleration = 0.1;
            game.add_projectile(ball);
            let ball_id = game.creatures.projectiles[1].id;
            let ball = game.projectile(game.entity(ball_id).unwrap()).unwrap();
            assert_eq!(ball.pierce_level(game), Err(PierceLevelError::NotArrow));
            assert_eq!(
                ball.set_pierce_level(game, 1),
                Err(PierceLevelError::NotArrow)
            );
            assert_eq!(ball.pickup(game), None);
            assert_eq!(ball.base_damage(game), None);
            assert!(!ball.set_critical(game, true));
            assert!(ball.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[1].acceleration, 0.05);
            assert!(ball.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[1].acceleration, 0.1);

            let stale = ProjectileRef::new(EntityRef::new(
                WorldEpoch::new(entity.epoch().get() + 1),
                entity.id(),
            ));
            assert_eq!(stale.pierce_level(game), Err(PierceLevelError::Stale));
            assert_eq!(
                stale.set_pierce_level(game, 1),
                Err(PierceLevelError::Stale)
            );
            assert_eq!(stale.owner(game), None);
            assert!(!stale.set_pickup(game, ArrowPickup::Allowed));
            assert!(!stale.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            let taken = game.creatures.projectiles.lift();
            assert!(handle.base_damage(game).is_none());
            game.creatures.projectiles = taken;
        });
    }

    /// A player entity for the tests: the host's player standing in for a guest.
    fn player(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> EntityRef {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        game.entity(20).unwrap()
    }

    #[test]
    #[ignore = "daily"]
    fn trident_and_rocket_handles_narrow_and_turn_with_their_own_random() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            let player = player(game);
            let id = game.take_entity_id();
            let mut trident = ThrownTrident::remote(id, Vec3::new(0.0, 64.0, 0.0), 10.0, 0.0);
            trident.velocity = Vec3::new(1.0, 2.0, 3.0);
            let mut expect = trident.random.clone();
            game.tridents.push(trident);
            let id = game.take_entity_id();
            let mut rocket =
                Rocket::remote(id, Vec3::new(0.0, 70.0, 0.0), Vec3::new(0.0, 1.0, 0.0));
            let mut rocket_random = rocket.random.clone();
            rocket.velocity = Vec3::new(2.0, 4.0, 6.0);
            game.rockets.push(rocket);

            let trident = game
                .projectile(game.entity(game.tridents[0].id).unwrap())
                .unwrap();
            let rocket = game
                .projectile(game.entity(game.rockets[0].id).unwrap())
                .unwrap();
            // A trident or rocket has no arrow state: those reads are None.
            assert_eq!(trident.pickup(game), None);
            assert_eq!(rocket.base_damage(game), None);
            assert!(!rocket.set_critical(game, true));

            // A trident a player threw keeps a player owner.
            assert!(!trident.set_owner(game, None));
            assert!(trident.set_owner(game, Some(player)));
            assert_eq!(game.tridents[0].guest, None);
            let zombie = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, zombie, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();
            assert!(!trident.set_owner(game, Some(mob)));

            // REVERSE: one float from the trident's own random, the level's untouched.
            let level = game.world.random.clone().next_float();
            let draw = expect.next_float();
            assert!(trident.deflect(
                game,
                Deflection::Reverse,
                None,
                Some(player),
                false,
                [0.02, 0.2, 0.02]
            ));
            assert_eq!(
                game.tridents[0].velocity,
                Vec3::new(1.0 * (-0.02 * 0.5), 2.0 * (-0.2 * 0.5), 3.0 * (-0.02 * 0.5))
            );
            assert_eq!(game.tridents[0].yaw, 10.0 + (170.0 + draw * 20.0));
            assert_eq!(
                game.tridents[0].random.clone().next_float(),
                expect.next_float()
            );
            assert_eq!(game.world.random.clone().next_float(), level);
            // No owner for a trident a player threw: refused, nothing changed.
            let velocity = game.tridents[0].velocity;
            assert!(!trident.deflect(game, Deflection::Reverse, None, None, false, [1.0; 3]));
            assert_eq!(game.tridents[0].velocity, velocity);

            // A rocket: owner, then REVERSE, then an owner of none.
            assert_eq!(rocket.owner(game), None);
            assert!(rocket.set_owner(game, Some(mob)));
            assert_eq!(rocket.owner(game), Some(mob));
            assert_eq!(game.rockets[0].owner, Some(Who::Mob(99)));
            let _ = rocket_random.next_float();
            assert!(rocket.deflect(game, Deflection::Reverse, None, None, false, [0.5; 3]));
            assert_eq!(game.rockets[0].velocity, Vec3::new(-0.5, -1.0, -1.5));
            assert_eq!(
                game.rockets[0].random.clone().next_float(),
                rocket_random.next_float()
            );
            assert_eq!(rocket.owner(game), None);

            // A rocket keeps an owner that is neither player nor mob, across a
            // save too, but a player's trident still refuses one.
            let boat_id = game.take_entity_id();
            game.boats
                .push(sunrise_gameplay_common::game::boats_common::Boat::new(
                    boat_id,
                    "oak".into(),
                    Vec3::ZERO,
                    0.0,
                ));
            let boat = game.entity(boat_id).unwrap();
            assert!(!trident.set_owner(game, Some(boat)));
            assert!(rocket.set_owner(game, Some(boat)));
            assert_eq!(rocket.owner(game), Some(boat));
            assert_eq!(game.rockets[0].owner, None);
            let saved = game.uuid_for(boat_id);
            let tag = game.rockets[0].to_nbt([9; 4], None, None, game.res.items());
            let back = Rocket::from_nbt(
                sunrise_sim::entity_nbt::firework_rocket_from_nbt(&tag, None).unwrap(),
            );
            assert_eq!(back.owner_uuid, Some(saved));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn deflection_follows_vanilla_for_wind_charges_owners_random_and_known_speed() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            let player = player(game);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.mobs.push(Mob::new(99, kind, Vec3::ZERO, 0.0));
            let mob = game.entity(99).unwrap();

            // A wind charge refuses for its first five ticks, with no change and no draw.
            let mut charge =
                Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, Some(Who::Mob(99)));
            charge.velocity = Vec3::new(0.0, 1.0, 0.0);
            game.add_projectile(charge);
            let charge = game
                .projectile(game.entity(game.creatures.projectiles[0].id).unwrap())
                .unwrap();
            let random = game.creatures.projectiles[0].random.0.clone().next_float();
            for left in 1..=5 {
                game.creatures
                    .projectiles
                    .update(0, |p| p.no_deflect_ticks = left);
                assert!(!charge.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
                let p = &game.creatures.projectiles[0];
                assert_eq!(p.velocity, Vec3::new(0.0, 1.0, 0.0));
                assert_eq!(p.owner, Some(Who::Mob(99)));
                assert_eq!(p.random.0.clone().next_float(), random);
            }
            game.creatures
                .projectiles
                .update(0, |p| p.no_deflect_ticks = 0);

            // REVERSE of a non-arrow takes one float from its own random, not the level's.
            let level = game.world.random.clone().next_float();
            let mut expect = game.creatures.projectiles[0].random.0.clone();
            let _ = expect.next_float();
            assert!(charge.deflect(game, Deflection::Reverse, None, None, true, [1.0; 3]));
            assert_eq!(game.creatures.projectiles[0].owner, None);
            assert_eq!(game.world.random.clone().next_float(), level);
            assert_eq!(
                game.creatures.projectiles[0].random.0.clone().next_float(),
                expect.next_float()
            );

            // Setting a disallowed arrow's owner to a player allows pickup; `deflect`
            // takes the other overload and does not.
            let mut shot = arrow(Who::Mob(99));
            shot.velocity = Vec3::new(0.0, 0.0, 1.0);
            game.add_projectile(shot);
            let arrow_ref = game
                .projectile(game.entity(game.creatures.projectiles[1].id).unwrap())
                .unwrap();
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(arrow_ref.deflect(
                game,
                Deflection::Reverse,
                None,
                Some(player),
                false,
                [1.0; 3]
            ));
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Disallowed));
            assert!(arrow_ref.set_owner(game, Some(player)));
            assert_eq!(arrow_ref.pickup(game), Some(ArrowPickup::Allowed));

            // MOMENTUM_DEFLECT normalizes the known speed (the last move), not the velocity.
            game.mobs[0].previous = Vec3::new(0.0, 0.0, -3.0);
            game.mobs[0].position = Vec3::ZERO;
            game.mobs[0].velocity = Vec3::new(5.0, 0.0, 0.0);
            assert!(arrow_ref.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(mob),
                None,
                true,
                [2.0; 3]
            ));
            assert_eq!(
                game.creatures.projectiles[1].velocity,
                Vec3::new(0.0, 0.0, 2.0)
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn momentum_follows_a_boats_displacement_and_a_refused_blow_still_attacks() {
        with_game(|game| {
            fixture_resident_column(&mut game.world, (0, 0));
            game.options.dedicated = Some(Vec::new());
            // A boat that moved along +Z while its velocity points along +X.
            let id = game.take_entity_id();
            let mut boat = sunrise_gameplay_common::game::boats_common::Boat::new(
                id,
                "oak".into(),
                Vec3::ZERO,
                0.0,
            );
            boat.previous = Vec3::new(0.0, 0.0, -2.0);
            boat.position = Vec3::ZERO;
            boat.velocity = Vec3::new(5.0, 0.0, 0.0);
            game.boats.push(boat);
            let boat = game.entity(id).unwrap();
            assert_eq!(
                game.native_entity_known_speed(boat),
                Some(Vec3::new(0.0, 0.0, 2.0))
            );
            let mut shot = arrow(Who::Mob(99));
            shot.velocity = Vec3::new(1.0, 0.0, 0.0);
            game.add_projectile(shot);
            let arrow_ref = game
                .projectile(game.entity(game.creatures.projectiles[0].id).unwrap())
                .unwrap();
            assert!(arrow_ref.deflect(
                game,
                Deflection::MomentumDeflect,
                Some(boat),
                None,
                true,
                [3.0; 3]
            ));
            assert_eq!(
                game.creatures.projectiles[0].velocity,
                Vec3::new(0.0, 0.0, 3.0)
            );

            // A player's blow at a new wind charge: refused, yet the attack
            // charge resets and nothing about the charge changes.
            let mut charge =
                Projectile::new(ProjectileKind::WindCharge, Vec3::ZERO, Some(Who::Mob(99)));
            charge.velocity = Vec3::new(0.0, 1.0, 0.0);
            game.add_projectile(charge);
            let i = game.creatures.projectiles.len() - 1;
            game.server.attack_ticks = 50;
            assert!(!game.deflect(i));
            assert_eq!(game.server.attack_ticks, 0);
            assert_eq!(
                game.creatures.projectiles[i].velocity,
                Vec3::new(0.0, 1.0, 0.0)
            );
            assert_eq!(game.creatures.projectiles[i].owner, Some(Who::Mob(99)));
            game.creatures
                .projectiles
                .update(i, |p| p.no_deflect_ticks = 0);
            game.server.attack_ticks = 50;
            assert!(game.deflect(i));
            assert_eq!(game.server.attack_ticks, 0);
            assert_eq!(game.creatures.projectiles[i].owner, Some(Who::Player));
        });
    }
}
