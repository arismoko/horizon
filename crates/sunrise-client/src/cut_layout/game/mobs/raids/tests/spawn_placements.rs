use mod_api::{
    BlockPos, EntityTypeKey, GeneralEntityCategory, GeneralSavePolicy, Heightmap, Hooks, Host,
    LegacyRandomSource, MobSpawnPlacementCheckEvent, ModEntry, Registrar, RegistrationContext,
    SpawnCheck, SpawnPlacementContext, SpawnPlacementOperation, SpawnPlacementPredicate,
    SpawnPlacementType, SpawnReason,
};
use std::sync::{Arc, Mutex};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::random::RandomSource;
use sunrise_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom},
};
use sunrise_gameplay::village_siege::VillageSiege;
use sunrise_level::chunk::{Chunk, Light, block_entity::Next};
use sunrise_sim::mobs::Species;
use sunrise_sim::{
    mobs::{
        data::Placement,
        spawn_rules::check_spawn_rules,
        spawn_selection::NaturalLevel,
        spawner::{CustomSpawnRules, SpawnData, SpawnDataExt, Spawner},
        trial_spawner::{TrialSpawner, TrialState},
    },
    motion::PlayerMotion,
};
use {
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::creatures::SpawnSite,
};

#[derive(Debug, PartialEq)]
enum Draw {
    Predicate(i32),
    Listener {
        default_result: bool,
        result: SpawnCheck,
        value: i32,
    },
}

struct SpawnProbe {
    predicate_result: bool,
    draws: Arc<Mutex<Vec<Draw>>>,
}

#[mod_api::__subscriptions]
impl Hooks for SpawnProbe {
    fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
        let draws = self.draws.clone();
        let answer = self.predicate_result;
        let predicate: Arc<dyn SpawnPlacementPredicate> = Arc::new(
            move |_: EntityTypeKey,
                  ctx: &mut dyn SpawnPlacementContext,
                  _: SpawnReason,
                  _: BlockPos| {
                let value = ctx.random().next_int();
                draws.lock().unwrap().push(Draw::Predicate(value));
                answer
            },
        );
        for id in ["minecraft:pillager", "minecraft:zombie"] {
            let kind = registrar.entity_type_key(id);
            registrar
                .register_spawn_placement(
                    kind,
                    None,
                    None,
                    predicate.clone(),
                    SpawnPlacementOperation::Replace,
                )
                .unwrap();
        }
    }

    fn mob_spawn_placement_check(&self, _: &dyn Host, event: &mut MobSpawnPlacementCheckEvent<'_>) {
        self.draws.lock().unwrap().push(Draw::Listener {
            default_result: event.default_result,
            result: *event.result,
            value: event.random.next_int(),
        });
        *event.result = SpawnCheck::Fail;
        // A mod-owned stream can outlive the event. Swapping this reference must
        // not send the next listener to that stream.
        event.random = Box::leak(Box::new(LegacyRandomSource::new(19)));
    }
}

struct NextListener(Arc<Mutex<Vec<Draw>>>);

#[mod_api::__subscriptions]
impl Hooks for NextListener {
    fn mob_spawn_placement_check(&self, _: &dyn Host, event: &mut MobSpawnPlacementCheckEvent<'_>) {
        self.0.lock().unwrap().push(Draw::Listener {
            default_result: event.default_result,
            result: *event.result,
            value: event.random.next_int(),
        });
    }
}

#[test]
#[ignore = "daily"]
fn spawners_check_registered_placements_for_types_outside_the_mob_table() {
    const GENERAL: &str = "registry_placements:probe";

    #[derive(Debug, PartialEq)]
    struct Check(SpawnReason, BlockPos, i32);
    struct RegistryPlacements(Arc<Mutex<Vec<Check>>>);

    #[mod_api::__subscriptions]
    impl Hooks for RegistryPlacements {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let checks = self.0.clone();
            let predicate: Arc<dyn SpawnPlacementPredicate> = Arc::new(
                move |_: EntityTypeKey,
                      ctx: &mut dyn SpawnPlacementContext,
                      reason: SpawnReason,
                      pos: BlockPos| {
                    checks
                        .lock()
                        .unwrap()
                        .push(Check(reason, pos, ctx.random().next_int()));
                    false
                },
            );
            r.declare_general_entity(
                "probe",
                GeneralEntityCategory::Misc,
                0.5,
                0.5,
                None,
                GeneralSavePolicy::Save,
                true,
                false,
                false,
                None,
                8,
                1,
                1,
                &[],
            )
            .unwrap();
            for (id, heightmap) in [
                ("minecraft:ender_dragon", None),
                ("minecraft:arrow", Some(Heightmap::OceanFloor)),
                (GENERAL, Some(Heightmap::OceanFloor)),
            ] {
                let kind = r.entity_type_key(id);
                r.register_spawn_placement(
                    kind,
                    Some(SpawnPlacementType::InWater),
                    heightmap,
                    predicate.clone(),
                    SpawnPlacementOperation::Replace,
                )
                .unwrap();
            }
        }
    }

    let checks = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "registry_placements",
            Box::new(RegistryPlacements(checks.clone())),
        )]
        .into(),
        || {
            with_game(|game| {
                let data = game.res.mob_data();
                assert_eq!(data.placement("minecraft:ender_dragon"), Placement::InWater);
                assert_eq!(data.placement("minecraft:arrow"), Placement::InWater);
                assert_eq!(
                    data.heightmap("minecraft:arrow"),
                    sunrise_sim::mobs::spawn_selection::Heightmap::OceanFloor
                );
                assert!(game.natural_level().mob_type("minecraft:arrow").is_none());
                game.set_difficulty_forced(2);
                game.world.game_time = 1;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 70.0, 0.5)).unwrap());
                let mut chunk = Chunk::empty(0, 0);
                chunk.lit = true;
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                // The arrow's half-width is 0.25: its box misses the adjacent stone,
                // whereas the old unit box for non-MobTypes would overlap it.
                game.world.set(3, 70, 2, stone).unwrap();
                let key = (2, 70, 2);
                let pos = BlockPos::new(2, 70, 2);
                let mut entity = Compound::default();
                entity.insert("id".into(), Tag::String("minecraft:arrow".into()));
                entity.insert(
                    "Pos".into(),
                    Tag::List(
                        [2.625_f64, 70.0, 2.5]
                            .into_iter()
                            .map(|v| Tag::Double(v.to_bits()))
                            .collect(),
                    ),
                );
                let mut ordinary = Spawner {
                    spawn_delay: 0,
                    spawn_count: 1,
                    next: Some(Next::Own(SpawnData::new(entity.clone(), None, None))),
                    ..Default::default()
                };
                game.world.random.set_seed(7);
                game.server_tick_spawner_of(key, &mut ordinary, None)
                    .unwrap();
                assert_eq!(
                    *checks.lock().unwrap(),
                    [Check(SpawnReason::Spawner, pos, -1_156_638_823)]
                );
                assert_eq!(
                    ordinary.spawn_delay, 0,
                    "the refusal stopped before creation"
                );
                assert_eq!(game.world.random.next_int(), -1_552_468_968);

                ordinary.next = Some(Next::Own(SpawnData::new(
                    entity.clone(),
                    Some(CustomSpawnRules {
                        block: (0, 15),
                        sky: (0, 15),
                    }),
                    None,
                )));
                game.server_tick_spawner_of(key, &mut ordinary, None)
                    .unwrap();
                assert_eq!(
                    checks.lock().unwrap().len(),
                    1,
                    "ordinary custom rules bypass placement"
                );
                assert!(
                    ordinary.spawn_delay > 0,
                    "creation still uses the existing path"
                );

                for id in ["minecraft:arrow", GENERAL] {
                    entity.insert("id".into(), Tag::String(id.into()));
                    let mut trial = TrialSpawner {
                        next_spawn_data: Some(SpawnData::new(
                            entity.clone(),
                            Some(CustomSpawnRules {
                                block: (15, 15),
                                sky: (0, 15),
                            }),
                            None,
                        )),
                        ..Default::default()
                    };
                    game.world.random.set_seed(7);
                    game.server_tick_trial_spawner(key, &mut trial, TrialState::Active, false)
                        .unwrap();
                    assert_eq!(
                        game.world.random.next_int(),
                        -1_552_468_968,
                        "{id} advanced the live stream"
                    );
                }
                assert_eq!(
                    *checks.lock().unwrap(),
                    [
                        Check(SpawnReason::Spawner, pos, -1_156_638_823),
                        Check(SpawnReason::TrialSpawner, pos, -1_156_638_823),
                        Check(SpawnReason::TrialSpawner, pos, -1_156_638_823),
                    ],
                    "trial spawners check placement before their custom rules"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn placement_listener_draws_after_the_registered_predicate_on_the_callers_stream() {
    let draws = Arc::new(Mutex::new(Vec::new()));
    let probe = SpawnProbe {
        predicate_result: true,
        draws: draws.clone(),
    };
    mod_platform::testing::with_mods(
        vec![
            ModEntry::new("spawn-probe", Box::new(probe)),
            ModEntry::new("next-listener", Box::new(NextListener(draws.clone()))),
        ]
        .into(),
        || {
            with_game(|game| {
                game.set_difficulty_forced(2);
                let level = game.natural_level();
                let zombie = level.mob_type("minecraft:zombie").unwrap();
                let mut random = LegacyRandom::new(7);
                assert!(!check_spawn_rules(
                    zombie.into(),
                    &level,
                    SpawnReason::Natural,
                    &mut random,
                    [0, 64, 0]
                ));
                // The first four nextInt() values of Java's Random(7), in order.
                assert_eq!(
                    *draws.lock().unwrap(),
                    [
                        Draw::Predicate(-1_156_638_823),
                        Draw::Listener {
                            default_result: true,
                            result: SpawnCheck::Default,
                            value: -1_552_468_968,
                        },
                        Draw::Listener {
                            default_result: true,
                            result: SpawnCheck::Fail,
                            value: -1_077_308_326,
                        },
                    ]
                );
                assert_eq!(
                    random.next_int(),
                    41_356_089,
                    "both listeners advanced the caller's stream despite Fail"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn patrols_and_sieges_bypass_registered_predicates_and_placement_listeners() {
    let draws = Arc::new(Mutex::new(Vec::new()));
    let probe = SpawnProbe {
        predicate_result: false,
        draws: draws.clone(),
    };
    mod_platform::testing::with_mods(
        vec![ModEntry::new("spawn-probe", Box::new(probe))].into(),
        || {
            with_game(|game| {
                game.set_difficulty_forced(2);
                game.world.day_time = 6000;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();

                // Java's Random(3) wins the 1-in-5 roll and picks (37, -42) from the origin.
                game.world.random = LevelRandom::for_level(LegacyRandom::new(3));
                for cx in 1..=2 {
                    for cz in -4..=-2 {
                        game.world
                            .chunks
                            .insert((cx, cz), Arc::new(Chunk::empty(cx, cz)));
                    }
                }
                game.world.set(37, 63, -42, stone).unwrap();
                game.tick_patrol_spawner();
                assert!(
                    game.mobs
                        .iter()
                        .any(|mob| mob.species() == Species::Pillager
                            && mob.position == Vec3::new(37.0, 64.0, -42.0)),
                    "the patrol leader passes its direct vanilla predicate"
                );

                // Java's Random(7) picks (11, 10) from (8, 8); the dark stone column is in the village.
                game.world.day_time = 18001;
                let mut chunk = Chunk::empty(0, 0);
                let section = chunk.section_index(64).unwrap();
                Arc::make_mut(&mut chunk.sections[section]).light = Light::Uniform(0);
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                game.world.set(11, 63, 10, stone).unwrap();
                let id = game
                    .spawn_mob(
                        "minecraft:villager",
                        [8.5, 64.0, 8.5],
                        false,
                        SpawnSite::Plain,
                    )
                    .unwrap();
                game.mobs
                    .iter_mut()
                    .find(|mob| mob.id == id)
                    .unwrap()
                    .extra
                    .home = Some([8, 64, 8]);
                game.world.random = LevelRandom::for_level(LegacyRandom::new(7));
                game.server.raiding.0.siege = VillageSiege {
                    tonight: true,
                    set_up: true,
                    zombies_to_spawn: 1,
                    spot: [8, 64, 8],
                    ..VillageSiege::default()
                };
                game.tick_village_siege();
                assert!(
                    game.mobs.iter().any(|mob| mob.species() == Species::Zombie
                        && mob.position == Vec3::new(11.5, 64.0, 10.5)),
                    "the siege zombie passes its direct vanilla predicate"
                );
                assert!(
                    draws.lock().unwrap().is_empty(),
                    "neither placement callbacks nor registered replacements ran"
                );
            });
        },
    );
}
