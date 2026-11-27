#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Tag;
use sunrise_items::items::ItemStack;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::beehive::Beehive;
#[cfg(test)]
pub(crate) mod honey_overflow_tests {
    use super::*;

    /// `BeehiveBlock.useItemOn`: a honey bottle that no longer fits is
    /// `player.drop(stack, false, PREDICTED)`: from 0.3 below the eyes,
    /// thrown along the view, with no thrower (#11377).
    #[test]
    #[ignore = "daily"]
    fn a_honey_bottle_that_does_not_fit_is_thrown_ahead() {
        super::super::native_entity_refs::tests::with_game(|game| {
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let registry = &game.pack.assets.registry;
            let hive = registry.default_state("minecraft:beehive").unwrap();
            let mut full = registry.state(hive).unwrap().clone();
            full.properties.insert("honey_level".into(), "5".into());
            let full = registry.id(&full).unwrap();
            game.world.set(8, 64, 10, full).unwrap();
            game.player = Some(
                sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 64.0, 8.5))
                    .unwrap(),
            );
            // Facing south (+z), towards the hive.
            game.yaw = 0.0;
            game.pitch = 0.0;
            for slot in game.inventory.iter_mut() {
                *slot = Some(ItemStack::new("minecraft:stone", 64));
            }
            game.selected = 0;
            let bottles = ItemStack::new("minecraft:glass_bottle", 2);
            game.inventory[0] = Some(bottles.clone());
            game.dropped.clear();

            assert!(game.use_beehive([8, 64, 10], Some(&bottles)).unwrap());
            let eye = game.eye_position().unwrap();
            let honey: Vec<_> = game
                .dropped
                .iter()
                .filter(|e| &*e.stack.item == "minecraft:honey_bottle")
                .collect();
            assert_eq!(honey.len(), 1);
            let e = honey[0];
            assert!((e.position.y - (eye[1] - f64::from(0.3f32))).abs() < 1e-9);
            assert!(e.velocity.z > 0.2, "thrown ahead: {:?}", e.velocity);
            assert_eq!(e.thrower, None);
            assert_eq!(e.pickup_delay, 40);
        });
    }
}

#[cfg(test)]
pub(crate) mod stay_tests {
    use super::*;
    use sunrise_core::nbt::Compound;
    use sunrise_sim::beehive::Occupant;

    /// `BeehiveBlockEntity.tickOccupants`: every bee whose time is up asks
    /// `releaseOccupant`, which keeps it in at night (`BEES_STAY_IN_HIVE`)
    /// and lets it out by day. All of a hive's bees get the same answer in
    /// one turn, however many ask.
    #[test]
    #[ignore = "daily"]
    fn every_ready_bee_of_a_hive_turn_gets_the_same_answer() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let state = game
                .pack
                .assets
                .registry
                .default_state("minecraft:beehive")
                .unwrap();
            let key = (8, 64, 8);
            game.world.set(key.0, key.1, key.2, state).unwrap();
            let bee = || {
                let mut tag = Compound::new();
                tag.insert("id".into(), Tag::String("minecraft:bee".into()));
                Occupant {
                    entity_data: Tag::Compound(tag),
                    ticks_in_hive: 601,
                    min_ticks_in_hive: 600,
                }
            };
            let hive = Beehive {
                bees: vec![bee(), bee(), bee()],
                flower: None,
            };
            game.world
                .block_entities
                .insert(key, BlockEntity::Beehive(Box::new(hive)));
            let inside = |game: &sunrise_gameplay::game::ServerGame<'_>| match game
                .world
                .block_entities
                .get(&key)
            {
                Some(BlockEntity::Beehive(h)) => h.bees.len(),
                _ => panic!("the hive is gone"),
            };
            game.world.day_time = 13_000;
            game.tick_beehive(key);
            assert_eq!((inside(game), game.mobs.len()), (3, 0), "night");
            game.world.day_time = 1_000;
            game.tick_beehive(key);
            assert_eq!((inside(game), game.mobs.len()), (0, 3), "day");
        });
    }
}

#[cfg(test)]
pub(crate) mod changed_tests {
    use sunrise_core::geometry::Vec3;
    use sunrise_core::nbt::Tag;
    use sunrise_gameplay::game::game_tests::tests::server_game;
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::{
        beehive::{Beehive, Occupant},
        mobs::Mob,
    };

    fn hive_at(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        key: (i32, i32, i32),
        hive: Beehive,
    ) {
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:beehive")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        game.world
            .block_entities
            .insert(key, BlockEntity::Beehive(Box::new(hive)));
    }

    /// `BeehiveBlockEntity.serverTick` -> `tickOccupants` calls `setChanged`
    /// when a bee leaves. The hive's saved `bees` list lost an entry, and
    /// nothing else in the tick changed a block state.
    #[test]
    #[ignore = "daily"]
    fn a_released_bee_marks_the_chunk() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let key = (8, 64, 8);
        hive_at(
            &mut game,
            key,
            Beehive {
                bees: vec![Occupant {
                    entity_data: Tag::compound([("id", Tag::String("minecraft:bee".into()))]),
                    ticks_in_hive: 601,
                    min_ticks_in_hive: 600,
                }],
                flower: None,
            },
        );
        game.world.day_time = 1_000;
        game.world.modified.clear();

        game.tick_beehive(key);

        let Some(BlockEntity::Beehive(h)) = game.world.block_entities.get(&key) else {
            panic!("no hive");
        };
        assert!(h.bees.is_empty(), "the ready bee was let out");
        assert_eq!(game.mobs.len(), 1, "the bee reached the level");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a released bee left the chunk unmarked"
        );
    }

    /// `BeehiveBlockEntity.addOccupant` ends with `setChanged`: the hive
    /// stored the bee and the mob is gone, which no block state shows.
    #[test]
    #[ignore = "daily"]
    fn a_bee_going_in_marks_the_chunk() {
        let mut game_resources = None;
        let mut game = server_game(&mut game_resources);
        let key = (8, 64, 8);
        hive_at(&mut game, key, Beehive::default());
        let kind = game.res.mob_types().get("minecraft:bee").unwrap().clone();
        game.mobs
            .push(Mob::new(42, kind, Vec3::new(8.0, 64.0, 8.0), 0.0));
        game.world.modified.clear();

        game.bee_enter_hive(42, [8, 64, 8]);

        let Some(BlockEntity::Beehive(h)) = game.world.block_entities.get(&key) else {
            panic!("no hive");
        };
        assert_eq!(h.bees.len(), 1, "the bee went in");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "a stored bee left the chunk unmarked"
        );
    }
}
