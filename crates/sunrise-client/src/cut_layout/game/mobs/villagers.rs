#![cfg(test)]
use sunrise_core::geometry::Vec3;
use sunrise_sim::mobs::Mob;
#[cfg(test)]
pub(crate) mod off_level_tests {
    use super::*;
    use sunrise_sim::mobs::villager_brain;

    #[test]
    #[ignore = "daily"]
    fn retained_ancestor_does_not_consume_a_due_villager_sensor() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let kind = game
                .res
                .mob_types()
                .get("minecraft:villager")
                .unwrap()
                .clone();
            let mut m = Mob::new(73, kind.clone(), Vec3::new(1.0, 80.0, 0.5), 0.0);
            villager_brain::init(&mut m, 0, 0, 0);
            let mut health = sunrise_sim::health::TestLevel::new(0);
            for _ in 0..40 {
                villager_brain::tick(
                    &mut m,
                    &game.mob_world_base(&[]),
                    &mut health.level(),
                    &mut Vec::new(),
                );
            }
            m.extra.mount = Some(72);
            game.mobs.push(m);
            let ghost = Mob::new(72, kind, Vec3::ZERO, 0.0);
            game.persist.off_level.insert(
                72,
                sunrise_gameplay_common::game::entity_memory::OffLevel::Mob(ghost),
            );
            game.tick_creature_entities().unwrap();
            assert!(
                villager_brain::take_item_scan(game.mob_object_mut(73).unwrap()),
                "a frozen resident's due sensor was consumed by independent item sensing"
            );
        });
    }
}

#[cfg(test)]
#[path = "tests/villager_witness_tests.rs"]
pub(crate) mod witness_tests;

#[cfg(test)]
pub(crate) mod birth_tests {
    use super::*;
    use mod_api::{
        BabyEntitySpawnEvent, EntityJoinLevelEvent, FinalizeSpawnEvent, Flow, Hooks, Host,
        ServerHost,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sunrise_sim::mobs::MobEvent;

    const BED: [i32; 3] = [3, 64, 1];

    fn parents(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> (u32, u32) {
        game.mobs.clear();
        game.world
            .chunks
            .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
        let bed = game
            .pack
            .assets
            .registry
            .resolve(&sunrise_world::chunk::BlockState {
                name: "minecraft:red_bed".into(),
                properties: [
                    ("facing".into(), "east".into()),
                    ("occupied".into(), "false".into()),
                    ("part".into(), "head".into()),
                ]
                .into_iter()
                .collect(),
            })
            .unwrap();
        game.world.set(BED[0], BED[1], BED[2], bed).unwrap();
        let kind = game
            .res
            .mob_types()
            .get("minecraft:villager")
            .unwrap()
            .clone();
        let parent = game.server.next_entity + 1;
        let partner = parent + 1;
        game.server.next_entity = partner;
        for id in [parent, partner] {
            game.mobs.push(Mob::with_random(
                id,
                kind.clone(),
                Vec3::new(1.5, 64.0, 1.5),
                0.0,
                sunrise_core::random::LegacyRandom::new(i64::from(id)),
            ));
        }
        // The brain has already eaten, selected BED and aged the initiating
        // parent before emitting VillagerBirth; the Game owns the other half.
        game.mobs[0].extra.age = 6000;
        game.native_joins_skip();
        game.start_audio_capture();
        game.server.lan_entity_events.clear();
        (parent, partner)
    }

    fn birth(game: &mut sunrise_gameplay::game::ServerGame<'_>, parent: u32, partner: u32) {
        game.handle_mob_events(vec![MobEvent::VillagerBirth {
            parent,
            partner,
            child: Some((BED, Some(2))),
        }])
        .unwrap();
        for id in [parent, partner] {
            assert_eq!(
                game.mobs.iter().find(|m| m.id == id).unwrap().extra.age,
                6000
            );
        }
    }

    fn bed_owners(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<u32> {
        // The same Seen.bed projection supplies the POI ticket searches and
        // take_bed_for_child; no independent reservation exists in the Game.
        game.mobs
            .iter()
            .filter(|m| m.seen(None, game.items()).bed == Some(BED))
            .map(|m| m.id)
            .collect()
    }

    struct RejectJoin(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for RejectJoin {
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            assert_eq!(event.kind, "minecraft:villager");
            self.0.fetch_add(1, Ordering::SeqCst);
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn rejected_birth_leaves_bed_free_and_parents_resting() {
        let joins = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reject-villager-birth",
            Box::new(RejectJoin(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                assert!(bed_owners(game).is_empty());
                birth(game, parent, partner);
                assert_eq!(joins.load(Ordering::SeqCst), 1);
                assert_eq!(game.mobs.len(), 2);
                assert!(bed_owners(game).is_empty());
                assert!(game.server.lan_entity_events.is_empty());
            });
        });
    }

    fn assert_baby(game: &sunrise_gameplay::game::ServerGame<'_>, parent: u32, partner: u32) {
        assert_eq!(game.mobs.len(), 3);
        let baby = game
            .mobs
            .iter()
            .find(|m| m.id != parent && m.id != partner)
            .unwrap();
        assert_eq!(baby.extra.age, -24000);
        assert_eq!(baby.extra.variant, 2);
        assert_eq!(baby.extra.bed, Some(BED));
        assert_eq!(bed_owners(game), vec![baby.id]);
        assert_eq!(game.server.lan_entity_events, vec![(baby.id as i32, 12)]);
    }

    #[test]
    #[ignore = "daily"]
    fn untouched_birth_claims_one_bed_and_announces_one_baby() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                assert!(bed_owners(game).is_empty());
                birth(game, parent, partner);
                assert_baby(game, parent, partner);
            });
        });
    }

    struct RejectOtherBirthPaths(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for RejectOtherBirthPaths {
        fn finalize_spawn(&self, _: &dyn Host, event: &mut FinalizeSpawnEvent<'_>) -> Flow {
            self.0.fetch_add(1, Ordering::SeqCst);
            event.spawn_cancelled = true;
            Flow::Continue
        }

        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            _: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            self.0.fetch_add(1, Ordering::SeqCst);
            Flow::Cancel
        }
    }

    #[test]
    #[ignore = "daily"]
    fn villager_birth_is_raw_and_does_not_post_the_animal_hook() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "reject-non-villager-birth-paths",
            Box::new(RejectOtherBirthPaths(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let (parent, partner) = parents(game);
                birth(game, parent, partner);
                assert_baby(game, parent, partner);
                assert_eq!(calls.load(Ordering::SeqCst), 0);
            });
        });
    }
}
use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;
