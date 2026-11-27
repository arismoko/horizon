use crate::game::creatures::breeding_tests::Bred;
use crate::game::creatures::breeding_tests::SeededBreed;
use mod_api::ServerHost;
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_sim::mobs::Mob;
use sunrise_sim::mobs::MobEvent;
use sunrise_sim::mobs::MobWorld;
use sunrise_sim::mobs::{
    Species,
    finalize::{Attr, set_base},
    goals::Breed,
};

#[test]
#[ignore = "daily"]
fn breeding_announces_the_orb_before_the_lower_id_child() {
    use mod_api::{self, BabyEntitySpawnEvent, EntityJoinLevelEvent, Flow, Hooks};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Observer {
        joins: Arc<Mutex<Vec<(u32, String)>>>,
        cancel: Arc<AtomicBool>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            _: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            if self.cancel.load(Ordering::SeqCst) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            self.joins
                .lock()
                .unwrap()
                .push((event.subject.id(), event.kind.into()));
            Flow::Continue
        }
    }
    let joins = Arc::new(Mutex::new(Vec::new()));
    let cancel = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "breeding-observer",
        Box::new(Observer {
            joins: joins.clone(),
            cancel: cancel.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
            assert!(game.mob_world_base(&[]).asks_baby_spawn);
            let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let base = game.server.next_entity;
            game.server.next_entity += 2;
            game.mobs.push(Mob::new(
                base + 1,
                cow.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.mobs
                .push(Mob::new(base + 2, cow, Vec3::new(1.0, 64.0, 0.0), 0.0));
            game.native_joins_skip();
            let warm = game.world_variants().cow.lookup("minecraft:warm").unwrap();
            let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
            parent.extra.cow_variant = Some(warm);
            parent.random = sunrise_core::random::LegacyRandom::new(0);
            parent.extra.age = 6000;
            let child = game.breed(
                base + 1,
                base + 2,
                None,
                None,
                None,
                sunrise_sim::mobs::BreedExperience::Deferred,
                true,
            );
            assert_eq!(child, Some(base + 3));
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 3)
                    .unwrap()
                    .extra
                    .age,
                -24000
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .extra
                    .age,
                6000
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 2)
                    .unwrap()
                    .extra
                    .age,
                6000
            );
            assert_eq!(
                *joins.lock().unwrap(),
                vec![
                    (base + 4, "minecraft:experience_orb".into()),
                    (base + 3, "minecraft:cow".into())
                ]
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 3)
                    .unwrap()
                    .extra
                    .cow_variant,
                Some(warm)
            );
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .random
                    .state(),
                234_029_835_660_143
            );
            joins.lock().unwrap().clear();
            let fox = game.res.mob_types().get("minecraft:fox").unwrap().clone();
            let fox_base = game.server.next_entity;
            game.server.next_entity += 2;
            game.mobs.push(Mob::new(
                fox_base + 1,
                fox.clone(),
                Vec3::new(0.0, 64.0, 0.0),
                0.0,
            ));
            game.mobs
                .push(Mob::new(fox_base + 2, fox, Vec3::new(1.0, 64.0, 0.0), 0.0));
            game.native_joins_skip();
            game.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            let feeder = sunrise_sim::lan::playerdata::uuid_ints(game.native_acting_id());
            game.server.love_cause.insert(fox_base + 1, None);
            let fox_child = game.breed(
                fox_base + 1,
                fox_base + 2,
                None,
                None,
                None,
                sunrise_sim::mobs::BreedExperience::Deferred,
                true,
            );
            assert_eq!(fox_child, Some(fox_base + 3));
            let fox_child = game.mobs.iter().find(|mob| mob.id == fox_base + 3).unwrap();
            assert!(sunrise_sim::mobs::fox::trusts(fox_child, feeder));
            assert_eq!(
                *joins.lock().unwrap(),
                vec![
                    (fox_base + 3, "minecraft:fox".into()),
                    (fox_base + 4, "minecraft:experience_orb".into())
                ]
            );
            joins.lock().unwrap().clear();
            let next = game.server.next_entity;
            let parent = game.mobs.iter_mut().find(|mob| mob.id == base + 1).unwrap();
            let mut expected = parent.random.clone();
            let choice = parent.random.next_boolean();
            assert_eq!(choice, expected.next_boolean());
            let random = expected.state();
            cancel.store(true, Ordering::SeqCst);
            let child = game.breed(
                base + 1,
                base + 2,
                None,
                Some(choice),
                None,
                sunrise_sim::mobs::BreedExperience::Deferred,
                true,
            );
            assert_eq!(child, None);
            // The baby is built before the hook now (its draws and entity number are spent), so a
            // veto keeps the number vanilla's constructor took but spawns nothing.
            assert_eq!(game.server.next_entity, next + 1);
            assert_eq!(
                game.mobs
                    .iter()
                    .find(|mob| mob.id == base + 1)
                    .unwrap()
                    .random
                    .state(),
                random
            );
            assert!(joins.lock().unwrap().is_empty());
        });
    });
}

pub(crate) fn tick_breeding(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    parent_kind: &str,
    partner_kind: &str,
    seed: i64,
    mob_drops: bool,
) -> Bred {
    game.mobs.clear();
    game.ui.rules.set(
        "mob_drops",
        sunrise_game_data::gamerules::Value::Bool(mob_drops),
    );
    let base = game.server.next_entity;
    game.server.next_entity += 2;
    let mut parent = Mob::new(
        base + 1,
        game.res.mob_types().get(parent_kind).unwrap().clone(),
        Vec3::new(0.0, 64.0, 0.0),
        0.0,
    );
    let mut partner = Mob::new(
        base + 2,
        game.res.mob_types().get(partner_kind).unwrap().clone(),
        Vec3::new(1.0, 64.0, 0.0),
        0.0,
    );
    parent.persistent = true;
    parent.extra.in_love = 600;
    partner.extra.in_love = 600;
    parent.extra.tame = true;
    partner.extra.tame = true;
    parent.extra.variant = 1;
    partner.extra.variant = 2;
    if parent.species() == Species::Wolf {
        let coats = &game.world_variants().wolf;
        parent.extra.wolf_variant = Some(coats.lookup("minecraft:ashen").unwrap());
        partner.extra.wolf_variant = Some(coats.lookup("minecraft:black").unwrap());
    }
    parent.extra.variant2 = 1;
    partner.extra.variant2 = if sunrise_sim::mobs::llama::is_llama(partner.species()) {
        4
    } else {
        2
    };
    set_base(&mut parent, Attr::MaxHealth, 20.0);
    set_base(&mut partner, Attr::MaxHealth, 28.0);
    set_base(&mut parent, Attr::MovementSpeed, 0.2);
    set_base(&mut partner, Attr::MovementSpeed, 0.3);
    parent.health = 20.0;
    partner.health = 28.0;
    parent.extra.vector[0] = 0.5;
    partner.extra.vector[0] = 0.9;
    // The camel's client animation delay must not add an unrelated draw.
    parent.extra.ints[0] = 1000;
    parent.ai.targets.clear();
    parent.ai.goals.clear();
    parent.ai.goals.add(
        2,
        SeededBreed {
            goal: Breed::new(1.0),
            seed,
        },
    );
    let others = [partner.seen(None, game.items())];
    let mut events = Vec::new();
    let mut test_level = sunrise_sim::health::TestLevel::new(321);
    let mut experience = None;
    {
        let world = MobWorld {
            others: &others,
            mob_drops,
            ..game.mob_world_base(&[])
        };
        assert!(!world.asks_baby_spawn);
        for _ in 0..200 {
            // No floor in this fixture: keep the pair in breeding range.
            parent.position = Vec3::new(0.0, 64.0, 0.0);
            parent.tick(&world, &mut test_level.level(), &mut events);
            experience = events.iter().find_map(|event| match event {
                MobEvent::Breed { experience, .. } => Some(*experience),
                _ => None,
            });
            if experience.is_some() {
                break;
            }
            events.clear();
        }
    }
    let experience = experience.expect("the goal must breed");
    let tick_random = parent.random.state();
    game.mobs.extend([parent, partner]);
    game.handle_mob_events(events).unwrap();
    let parent = game.mobs.iter_mut().find(|m| m.id == base + 1).unwrap();
    assert_eq!(
        parent.random.state(),
        tick_random,
        "Game must not redraw inheritance"
    );
    let next_random = parent.random.next_int();
    let child = game.mobs.iter().find(|m| m.extra.age < 0).unwrap().clone();
    Bred {
        child,
        next_random,
        experience,
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::creatures::ModGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;
