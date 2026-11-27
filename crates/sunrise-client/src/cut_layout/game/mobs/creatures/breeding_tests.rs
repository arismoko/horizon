use sunrise_gameplay_common::game::creatures::{Mob, MobWorld, Species, Vec3, breeding_credit};

use sunrise_core::random::LegacyRandom;
use sunrise_sim::mobs::{
    BreedExperience,
    goals::{Breed, Cx, Goal},
};

/// Isolate the goal's stream from earlier tick work, but leave the actual
/// Breed, subsequent aiStep and queued Game event handling in the route.
#[derive(Clone, Debug)]
pub(in crate::game) struct SeededBreed {
    pub(in crate::game) goal: Breed,
    pub(in crate::game) seed: i64,
}
impl Goal for SeededBreed {
    fn box_clone(&self) -> Box<dyn Goal> {
        Box::new(self.clone())
    }
    fn flags(&self) -> u8 {
        self.goal.flags()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.goal.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        self.goal.can_continue(m, cx)
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        m.random = LegacyRandom::new(self.seed);
        self.goal.tick(m, cx);
    }
}
pub(crate) struct Bred {
    pub(crate) child: Mob,
    pub(crate) next_random: i32,
    pub(crate) experience: BreedExperience,
}
#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::breeding_tests::tick_breeding;

#[test]
#[ignore = "daily"]
fn equine_breeding_inheritance_precedes_same_tick_healing_and_grazing() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        // java.util.Random(42), Horse/Donkey/Llama.getBreedOffspring,
        // then XP, then AbstractHorse.aiStep's nextInt(900)/nextInt(300)
        // (llamas only heal). The attributes use stocks (20,.5,.2) and (28,.9,.3).
        for (parent, partner) in [
            ("minecraft:horse", "minecraft:horse"),
            ("minecraft:donkey", "minecraft:donkey"),
            ("minecraft:horse", "minecraft:donkey"),
            ("minecraft:donkey", "minecraft:horse"),
            ("minecraft:llama", "minecraft:llama"),
            ("minecraft:trader_llama", "minecraft:llama"),
        ] {
            let result = tick_breeding(game, parent, partner, 42, true);
            let horse = parent == "minecraft:horse" && partner == parent;
            let llama = sunrise_sim::mobs::llama::is_llama(result.child.species());
            assert_eq!(
                result.child.extra.variant,
                if horse {
                    5
                } else if llama {
                    1
                } else {
                    0
                }
            );
            assert_eq!(
                result.child.extra.variant2,
                if horse {
                    2
                } else if llama {
                    4
                } else {
                    0
                }
            );
            let expected = if horse {
                [24.8238468170166, 0.5886834847817017, 0.3030804447968077]
            } else {
                [24.91461181640625, 0.7668932765979051, 0.22812363785863735]
            };
            let actual = [
                f64::from(result.child.attrs.max_health),
                result.child.extra.vector[0],
                result.child.attrs.speed,
            ];
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 1e-12,
                    "{parent}/{partner}: {actual} != {expected}"
                );
            }
            assert_eq!(
                result.experience,
                BreedExperience::Drawn(Some(if horse || llama { 2 } else { 1 }))
            );
            assert_eq!(
                result.next_random,
                if horse {
                    -1_074_144_810
                } else if llama {
                    1_888_925_827
                } else {
                    652_655_727
                },
                "{parent}/{partner}"
            );
            if parent != partner && !llama {
                assert_eq!(result.child.species(), Species::Mule);
            }
            if parent == "minecraft:trader_llama" {
                assert!(result.child.persistent);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn wolf_breeding_selects_either_coat_from_the_parent_stream() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        // Computed with an independent implementation of the documented java.util.Random
        // algorithm, not from the game's output.
        // Java LCG: s0 = (seed ^ 0x5DEECE66D) & (2^48 - 1),
        // s' = (25214903917*s + 11) mod 2^48. Draw 1 selects the coat;
        // draw 2 selects one of 7 voices; draw 3 yields 1 + (s >> 17) % 7 XP;
        // draw 4 is signed(s >> 16). Neither bounded draw rejects these seeds.
        for (seed, coat, xp, next) in [
            (42, "minecraft:ashen", 7, 205_897_768),
            (4096, "minecraft:black", 3, 1_387_017_903),
        ] {
            let result = tick_breeding(game, "minecraft:wolf", "minecraft:wolf", seed, true);
            let variant = result.child.extra.wolf_variant.unwrap();
            assert_eq!(game.world_variants().wolf.id(variant), Some(coat));
            assert_eq!(result.experience, BreedExperience::Drawn(Some(xp)));
            assert_eq!(result.next_random, next);
        }
    });
}

#[test]
#[ignore = "daily"]
fn camel_breeding_leaves_the_parent_stream_unchanged() {
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let kind = "minecraft:camel";
        let result = tick_breeding(game, kind, kind, 42, false);
        assert_eq!(result.next_random, -1_170_105_035); // Random(42).nextInt(), no inheritance.
        assert_eq!(result.child.attrs.max_health, result.child.kind.max_health);
        assert_eq!(result.child.attrs.speed, result.child.kind.speed);
    });
}

#[test]
#[ignore = "daily"]
fn first_chicken_breed_matches_without_mods_and_with_an_empty_mod() {
    use mod_api::{self, Hooks};
    use sunrise_core::random::LegacyRandom;
    use sunrise_sim::mobs::{BreedExperience, MobEvent, goals::Breed};
    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:chicken")
            .unwrap()
            .clone();
        let mut mate = Mob::new(2, kind.clone(), Vec3::new(0.5, 64.0, 0.0), 0.0);
        mate.extra.in_love = 600;
        let others = [mate.seen(None, game.items())];
        let run = || {
            let world = MobWorld {
                others: &others,
                ..game.mob_world_base(&[])
            };
            assert!(!world.asks_baby_spawn);
            let mut parent = Mob::new(1, kind.clone(), Vec3::new(0.0, 64.0, 0.0), 0.0);
            parent.random = LegacyRandom::new(123);
            parent.persistent = true;
            parent.extra.in_love = 600;
            parent.extra.egg_time = 10_000;
            parent.ai.targets.clear();
            parent.ai.goals.clear();
            parent.ai.goals.add(2, Breed::new(1.0));
            let mut events = Vec::new();
            let mut test_level = sunrise_sim::health::TestLevel::new(321);
            let level = &mut test_level.level();
            for _ in 0..200 {
                // The fixture has no floor; keep the pair within breeding range.
                parent.position = Vec3::new(0.0, 64.0, 0.0);
                parent.tick(&world, level, &mut events);
                if let Some(xp) = events.iter().find_map(|event| match event {
                    MobEvent::Breed {
                        experience: BreedExperience::Drawn(Some(xp)),
                        ..
                    } => Some(*xp),
                    _ => None,
                }) {
                    return (parent.random.state(), xp);
                }
                events.clear();
            }
            panic!("chickens did not breed");
        };
        let no_mods = mod_platform::testing::with_mods(Vec::new().into(), run);
        let empty = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
        let unrelated = mod_platform::testing::with_mods(empty, run);
        assert_eq!(no_mods, unrelated);
    });
}

#[test]
#[ignore = "daily"]
fn empty_mod_never_defers_breeding_xp() {
    use mod_api::{self, Hooks};
    use sunrise_gameplay::game::server_side::native_pending_children::{
        BabyChildSlot, ChildConstraint,
    };

    struct Empty;
    #[mod_api::__subscriptions]
    impl Hooks for Empty {}
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("empty", Box::new(Empty))].into();
    mod_platform::testing::with_mods(mods, || {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            assert!(!game.mob_world_base(&[]).asks_baby_spawn);
            let mut slot = None;
            let mut cancel = false;
            let mut editor = BabyChildSlot {
                action: 0,
                slot: &mut slot,
                resources: game.common.res.clone(),
                attributes: game.common.motion.attributes.clone(),
                dimension: game.common.dim,
                constraint: ChildConstraint::BreedingAgeable,
            };
            game.native_baby_entity_spawn(mod_api::__private::BabyEntitySpawnCall {
                dimension: "minecraft:overworld",
                parent: 1,
                partner: 2,
                player: None,
                player_entity_id: None,
                child: mod_api::BabyEntitySpawnChild::new(&mut editor),
                cancel: &mut cancel,
            });
            assert!(slot.is_none());
            assert!(!game.mob_world_base(&[]).asks_baby_spawn);
        });
    });
}

#[test]
fn the_player_who_fed_either_animal_is_credited() {
    let mut love = std::collections::HashMap::from([(1, Some(7)), (2, Some(9))]);
    // The parent's feeder first, and both animals are spent.
    assert_eq!(breeding_credit(&mut love, 1, 2), Some(Some(7)));
    assert!(love.is_empty());
    // Only the partner was fed: still credited.
    let mut love = std::collections::HashMap::from([(2, None)]);
    assert_eq!(breeding_credit(&mut love, 1, 2), Some(None));
    // Neither was fed by a player (a bred pair loaded from a save).
    assert_eq!(breeding_credit(&mut love, 1, 2), None);
}

use sunrise_gameplay_common::game::tables::GameItems as _;
