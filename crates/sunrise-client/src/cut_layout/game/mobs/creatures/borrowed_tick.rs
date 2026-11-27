#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;
#[cfg(test)]
pub(crate) mod off_level_tests {
    use crate::game::creatures::borrowed_tick::*;

    #[test]
    #[ignore = "daily"]
    fn borrowed_tick_resolves_the_retained_controller() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            use sunrise_sim::mobs::goals::{JUMP, LOOK, MOVE};
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let spider = game
                .res
                .mob_types()
                .get("minecraft:spider")
                .unwrap()
                .clone();
            let skeleton = game
                .res
                .mob_types()
                .get("minecraft:skeleton")
                .unwrap()
                .clone();
            let at = Vec3::new(1.0, 80.0, 0.5);
            let mut vehicle = Mob::new(71, spider, at, 0.0);
            vehicle.extra.rider = Some(72);
            let mut rider = Mob::new(72, skeleton, at, 0.0);
            rider.extra.mount = Some(71);
            rider.age = 19;
            game.mobs.push(vehicle);
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(rider),
            );
            game.tick_creature_entities().unwrap();
            assert_eq!(
                game.mob_object(71).unwrap().ai.goals.disabled & (MOVE | JUMP | LOOK),
                MOVE | JUMP | LOOK
            );
            assert_eq!(game.mob_object(72).unwrap().age, 19);
            assert_eq!(game.mobs.len(), 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn resident_descendant_does_not_tick_until_detached() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut parent = Mob::new(72, pig.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            parent.extra.mount = Some(71);
            let mut child = Mob::new(73, pig, parent.position, 0.0);
            child.extra.mount = Some(72);
            child.no_ai = true;
            child.age = 19;
            child.fire_ticks = 20;
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(parent),
            );
            game.mobs.push(child);
            game.tick_creature_entities().unwrap();
            let child = game.mob_object(73).unwrap();
            assert_eq!(child.age, 19);
            assert_eq!(child.fire_ticks, 20);
            game.mob_object_mut(73).unwrap().extra.mount = None;
            game.tick_creature_entities().unwrap();
            assert_eq!(game.mob_object(73).unwrap().age, 20);
        });
    }
}

#[cfg(test)]
#[path = "mob_tick_access_tests.rs"]
pub(crate) mod mob_tick_access_tests;
