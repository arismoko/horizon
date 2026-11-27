//! Real-route fixtures for the pending-children hooks: `mob_split` through the slime/magma/sulfur
//! split and `baby_entity_spawn` through breeding. The expected values are
//! written out: the seeds' draws come from Java's `java.util.Random` (`new Random(2)` gives
//! `nextInt(3) = 1`, so three children; then three `nextFloat() * 360.0F` yaws;
//! see `YAWS`), and the offsets are vanilla's `setUpSplitCube` arithmetic spelled out in the test.
//!
//! They need prepared resources (`SUNRISE_TEST_RESOURCES`) and run with a native test mod
//! installed for the test only.

use mod_api::{
    BabyEntitySpawnEvent, Flow, Hooks, MobSplitEvent, PendingMobSpec, ServerHost, UnspawnedMobError,
};
use std::sync::{Arc, Mutex};
use sunrise_gameplay_common::game::{commands_common::Target, creatures::Mob};
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
};

pub(crate) const IGNORE: &str = "needs prepared resources in SUNRISE_TEST_RESOURCES";

/// `new Random(2)`: `2 + nextInt(3)` is 3, then `nextFloat() * 360.0F` three times, then the
/// next `nextInt()`. The same three draws with no listener, a cancel or a listener.
pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::SEED;
pub(crate) const YAWS: [f32; 3] = [105.61554, 324.52115, 1.4961791];
pub(crate) const SPLIT_DISPLACEMENT: f64 = 0.52_f32 as f64;
pub(crate) const NEXT_INT: i32 = 2_133_836_778;

pub(crate) type SplitFn =
    Box<dyn Fn(&mut dyn ServerHost, &mut MobSplitEvent<'_>) -> Flow + Send + Sync>;
pub(crate) struct Split(pub(crate) SplitFn);
#[mod_api::__subscriptions]
impl Hooks for Split {
    fn mob_split(&self, host: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
        (self.0)(host, event)
    }
}

pub(crate) type BabyFn =
    Box<dyn Fn(&mut dyn ServerHost, &mut BabyEntitySpawnEvent<'_>) -> Flow + Send + Sync>;
pub(crate) struct Baby(pub(crate) BabyFn);
#[mod_api::__subscriptions]
impl Hooks for Baby {
    fn baby_entity_spawn(
        &self,
        host: &mut dyn ServerHost,
        event: &mut BabyEntitySpawnEvent<'_>,
    ) -> Flow {
        (self.0)(host, event)
    }
}

pub(crate) fn with_split(
    listener: impl Fn(&mut dyn ServerHost, &mut MobSplitEvent<'_>) -> Flow + Send + Sync + 'static,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "split",
        Box::new(Split(Box::new(listener))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(run));
}

pub(crate) fn with_baby(
    listener: impl Fn(&mut dyn ServerHost, &mut BabyEntitySpawnEvent<'_>) -> Flow
    + Send
    + Sync
    + 'static,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "baby",
        Box::new(Baby(Box::new(listener))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(run));
}

pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::AT;

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::pending_birth_tests::cube;

/// The mobs the split added, in order (the fixture world starts with none).
pub(crate) fn spawned(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<Mob> {
    game.mobs.to_vec()
}

pub(crate) fn assert_vanilla_children(children: &[Mob], kind: &str, first_id: u32) {
    assert_eq!(children.len(), 3);
    // AbstractCubeMob.remove and setUpSplitCube widen the float offsets only
    // when adding them to the double-valued parent position.
    let expected = [
        [
            AT[0] - SPLIT_DISPLACEMENT,
            AT[1] + 0.5,
            AT[2] - SPLIT_DISPLACEMENT,
        ],
        [
            AT[0] + SPLIT_DISPLACEMENT,
            AT[1] + 0.5,
            AT[2] - SPLIT_DISPLACEMENT,
        ],
        [
            AT[0] - SPLIT_DISPLACEMENT,
            AT[1] + 0.5,
            AT[2] + SPLIT_DISPLACEMENT,
        ],
    ];
    for (i, child) in children.iter().enumerate() {
        assert_eq!(child.kind.id, kind);
        assert_eq!(child.id, first_id + i as u32, "ids are issued in order");
        assert_eq!(child.extra.size, 2);
        assert_eq!(
            [child.position.x, child.position.y, child.position.z].map(f64::to_bits),
            expected[i].map(f64::to_bits)
        );
        assert_eq!(child.yaw, YAWS[i]);
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn sulfur_cube_splits_two_babies_unless_primed_and_size_one_asks_nobody() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let push_when_empty = Arc::new(Mutex::new(false));
    let record = seen.clone();
    let do_push = push_when_empty.clone();
    with_split(
        move |host, event| {
            let empty = event.children.is_empty();
            record.lock().unwrap().push((
                event.kind.to_owned(),
                event.size,
                event.child_size,
                event.children.len(),
            ));
            if empty && *do_push.lock().unwrap() {
                let extra = event
                    .children
                    .construct(
                        host,
                        spec(
                            "minecraft:slime",
                            [10.0, 64.5, 20.0],
                            15.0,
                            Some(2),
                            event.dimension,
                        ),
                    )
                    .unwrap();
                event.children.push(extra).unwrap();
            }
            Flow::Continue
        },
        |game| {
            // Unprimed, size 4: two babies of size 2.
            let dying = cube(game, "minecraft:sulfur_cube", 4, 0);
            game.split_dead_cube(dying);
            let kids = spawned(game);
            assert_eq!(kids.len(), 2);
            assert!(
                kids.iter().all(|k| k.is_baby()
                    && k.extra.size == 2
                    && k.kind.id == "minecraft:sulfur_cube")
            );
            // Primed, size 4: NeoForge's AbstractCubeMob.remove still posts MobSplitEvent with
            // an empty list. Leaving it empty spawns nothing.
            game.mobs.clear();
            let mut dying = cube(game, "minecraft:sulfur_cube", 4, 0);
            dying.extra.sulfur_fuse = sunrise_sim::mobs::sulfur_cube::Fuse::set(40);
            assert!(sunrise_sim::mobs::sulfur_cube::primed(&dying));
            game.split_dead_cube(dying);
            assert!(game.mobs.is_empty());
            // Populating the initially empty list on a primed sulfur cube spawns the added child.
            *push_when_empty.lock().unwrap() = true;
            let mut dying = cube(game, "minecraft:sulfur_cube", 4, 0);
            dying.extra.sulfur_fuse = sunrise_client::mobs::sulfur_cube::Fuse::set(40);
            assert!(sunrise_client::mobs::sulfur_cube::primed(&dying));
            game.split_dead_cube(dying);
            assert_eq!(game.mobs.len(), 1);
            assert_eq!(game.mobs[0].kind.id, "minecraft:slime");
            assert_eq!(game.mobs[0].extra.size, 2);
            // A size-1 parent never splits and never asks listeners.
            game.mobs.clear();
            let dying = cube(game, "minecraft:slime", 1, 0);
            game.split_dead_cube(dying);
            assert!(game.mobs.is_empty());
        },
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [
            ("minecraft:sulfur_cube".to_owned(), 4, 2, 2),
            ("minecraft:sulfur_cube".to_owned(), 4, 2, 0),
            ("minecraft:sulfur_cube".to_owned(), 4, 2, 0),
        ]
    );
}

pub(crate) fn spec(
    kind: &str,
    at: [f64; 3],
    yaw: f32,
    size: Option<u32>,
    dimension: &str,
) -> PendingMobSpec {
    let mut spec = PendingMobSpec::new(
        mod_api::Identifier::parse(kind).unwrap(),
        mod_api::Identifier::parse(dimension).unwrap(),
        at,
        yaw,
        false,
    );
    spec.size = size;
    spec
}

#[test]
#[ignore = "daily"]
fn replace_c_with_e_commits_a_b_e() {
    let replaced_kind = Arc::new(Mutex::new(None));
    let record = replaced_kind.clone();
    with_split(
        move |host, event| {
            let e = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:magma_cube",
                        [8.0, 65.0, 9.0],
                        33.0,
                        Some(2),
                        event.dimension,
                    ),
                )
                .unwrap();
            let prev = event.children.replace(2, e).unwrap();
            *record.lock().unwrap() = Some(prev.kind().unwrap().to_string());
            drop(prev);
            Flow::Continue
        },
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            game.split_cube_children(&mut dying);
            let kids = spawned(game);
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].kind.id, "minecraft:slime");
            assert_eq!(kids[1].kind.id, "minecraft:slime");
            assert_eq!(kids[2].kind.id, "minecraft:magma_cube");
            assert_eq!(kids[2].position.x, 8.0);
            assert_eq!(kids[2].yaw, 33.0);
        },
    );
    assert_eq!(
        replaced_kind.lock().unwrap().as_deref(),
        Some("minecraft:slime"),
        "replace returns the previous child"
    );
}

#[test]
#[ignore = "daily"]
fn invalid_input_fails_before_mutation_and_the_valid_list_stands() {
    let errors = Arc::new(Mutex::new(Vec::new()));
    let record = errors.clone();
    with_split(
        move |host, event| {
            let dim = event.dimension;
            let mut out = vec![
                event
                    .children
                    .construct(
                        host,
                        spec("minecraft:slime", [f64::NAN, 64.0, 0.0], 0.0, Some(2), dim),
                    )
                    .map(|_| ()),
                event
                    .children
                    .construct(
                        host,
                        spec("minecraft:not_a_mob", [0.0, 64.0, 0.0], 0.0, None, dim),
                    )
                    .map(|_| ()),
                event
                    .children
                    .construct(
                        host,
                        spec(
                            "minecraft:slime",
                            [0.0, 64.0, 0.0],
                            0.0,
                            Some(2),
                            "minecraft:the_end",
                        ),
                    )
                    .map(|_| ()),
            ];
            // Out-of-bounds insert and replace refuse without mutating the list.
            let oob_insert = event
                .children
                .construct(
                    host,
                    spec("minecraft:slime", [0.0, 64.0, 0.0], 0.0, Some(2), dim),
                )
                .unwrap();
            out.push(event.children.insert(99, oob_insert));
            let oob_replace = event
                .children
                .construct(
                    host,
                    spec("minecraft:slime", [0.0, 64.0, 0.0], 0.0, Some(2), dim),
                )
                .unwrap();
            out.push(event.children.replace(99, oob_replace).map(|_| ()));
            // Unbounded list grows past the former 64-child cap (Audit [69]).
            while event.children.len() < 70 {
                let more = event
                    .children
                    .construct(
                        host,
                        spec("minecraft:slime", [0.0, 64.0, 0.0], 0.0, Some(2), dim),
                    )
                    .unwrap();
                event.children.push(more).unwrap();
            }
            out.push(
                event
                    .children
                    .get_mut(0)
                    .unwrap()
                    .set_position([f64::NAN, 0.0, 0.0]),
            );
            *record.lock().unwrap() = out;
            Flow::Continue
        },
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            game.split_cube_children(&mut dying);
            assert_eq!(
                game.common.mobs.len(),
                70,
                "unbounded list grows past 64 and stands intact after refused operations"
            );
            assert!(game.common.mobs[0].position.x.is_finite());
        },
    );
    assert_eq!(
        *errors.lock().unwrap(),
        [
            Err(UnspawnedMobError::Invalid),
            Err(UnspawnedMobError::Invalid),
            Err(UnspawnedMobError::Invalid),
            Err(UnspawnedMobError::Invalid),
            Err(UnspawnedMobError::Invalid),
            Err(UnspawnedMobError::Invalid),
        ]
    );
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn split_edits_do_not_publish_on_engine_unwind() {
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::server_side::pending_birth_fault::{self as escaping_fault, At};

    if run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_tests::split_edits_do_not_publish_on_engine_unwind",
        "SUNRISE_SPLIT_DISPATCH_UNWIND_CASE",
        &["replacement"],
    ) {
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    let edits = Arc::new(AtomicUsize::new(0));
    let recorded = edits.clone();
    with_split(
        move |host, event| {
            event.children.get_mut(0).unwrap().set_yaw(1.0).unwrap();
            let replacement = event
                .children
                .construct(
                    host,
                    spec(
                        "minecraft:magma_cube",
                        [1.0, 64.0, 1.0],
                        0.0,
                        Some(2),
                        event.dimension,
                    ),
                )
                .unwrap();
            drop(event.children.replace(1, replacement).unwrap());
            recorded.fetch_add(1, Ordering::Relaxed);
            Flow::Continue
        },
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let next_id = game.server.next_entity;
            escaping_fault::expect(At::SplitDispatched, dying.id, || {
                game.split_cube_children(&mut dying);
            });
            assert_eq!(edits.load(Ordering::Relaxed), 1);
            assert!(game.mobs.is_empty(), "no child joined");
            assert_eq!(
                game.server.next_entity,
                next_id + 4,
                "all proposals and the replacement were constructed"
            );
            for id in next_id + 1..=next_id + 4 {
                assert!(!game.persist.uuids.contains_key(&id));
                assert!(!game.entity_attachments.contains_key(&id));
                assert!(
                    !game
                        .entity_initialization
                        .initialized
                        .contains(&Holder::Entity(id))
                );
            }
            // Pre-callback effects stay: the construction draws were spent.
            assert_eq!(dying.random.next_int(), NEXT_INT);
            // Subsequent splits work cleanly from a fresh stack frame.
            let mut again = cube(game, "minecraft:slime", 4, SEED);
            game.split_cube_children(&mut again);
            assert_eq!(game.mobs.len(), 3);
            assert_eq!(game.mobs[0].yaw, 1.0);
            assert_eq!(game.mobs[1].kind.id, "minecraft:magma_cube");
        },
    );
}

// Breeding.

/// Two cows bred directly, as `breeding_announces_the_orb_before_the_lower_id_child` does: the
/// parents' ages are 6000 as the goal leaves them, the partner is in love.
pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::breed_pair;

#[allow(unused_imports)]
pub(crate) use crate::game::server_side::creatures_server::pending_birth_tests::breed_pair_credited;

pub(crate) fn find(game: &sunrise_gameplay::game::ServerGame<'_>, id: u32) -> Mob {
    game.mobs.iter().find(|m| m.id == id).unwrap().clone()
}

#[test]
#[ignore = "daily"]
fn breeding_replaces_the_cow_baby_with_a_pig_baby() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    with_baby(
        move |host, event| {
            record.lock().unwrap().push((
                event.dimension.to_owned(),
                event.child.is_present(),
                event.child.get().unwrap().kind().unwrap().to_string(),
            ));
            let mut pig = event
                .child
                .construct(
                    host,
                    PendingMobSpec::new(
                        mod_api::Identifier::parse("minecraft:pig").unwrap(),
                        mod_api::Identifier::parse(event.dimension).unwrap(),
                        [0.5, 64.0, 0.0],
                        0.0,
                        false,
                    ),
                )
                .unwrap();
            pig.set_yaw(15.0).unwrap();
            pig.add_tag("replaced_pig").unwrap();
            let replaced = event
                .child
                .replace(pig)
                .unwrap()
                .expect("the cow baby was in the slot");
            assert_eq!(replaced.kind().unwrap().to_string(), "minecraft:cow");
            drop(replaced);
            // A detached child dropped without being placed in the slot never joins.
            let _detached = event
                .child
                .construct(
                    host,
                    PendingMobSpec::new(
                        mod_api::Identifier::parse("minecraft:pig").unwrap(),
                        mod_api::Identifier::parse(event.dimension).unwrap(),
                        [0.5, 64.0, 0.0],
                        0.0,
                        true,
                    ),
                )
                .unwrap();
            let mut pig = event.child.get_mut().unwrap();
            pig.set_baby(true).unwrap();
            pig.set_baby(false).unwrap();
            Flow::Continue
        },
        |game| {
            let (parent, partner, child) = breed_pair(game, "minecraft:cow");
            let live_parent = find(game, parent);
            let child = find(game, child.expect("a baby joined"));
            assert_eq!(child.kind.id, "minecraft:pig");
            assert_eq!(
                child.extra.age, -24000,
                "the adult replacement joins as a baby"
            );
            // Post-event snap places the selected child at the parent's position with yaw/pitch 0.
            assert_eq!(child.position, live_parent.position);
            assert_eq!(child.yaw, 0.0);
            assert_eq!(child.head_pitch, 0.0);
            assert!(
                game.tags_of(Target::Mob(child.id))
                    .iter()
                    .any(|t| t == "replaced_pig")
            );
            assert_eq!(
                game.common
                    .mobs
                    .iter()
                    .filter(|m| m.kind.id == "minecraft:cow")
                    .count(),
                2,
                "only the parents are cows: no cow baby"
            );
            assert_eq!(
                game.common
                    .mobs
                    .iter()
                    .filter(|m| m.kind.id == "minecraft:pig")
                    .count(),
                1,
                "only the attached pig joined, not the dropped detached pig"
            );
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 6000);
                assert_eq!(p.extra.in_love, 0);
            }
        },
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [(
            "minecraft:overworld".to_owned(),
            true,
            "minecraft:cow".to_owned()
        )]
    );
}

#[test]
#[ignore = "daily"]
fn breeding_veto_leaves_no_baby_orb_or_statistic_and_the_parents_resting() {
    with_baby(
        |_, _| Flow::Cancel,
        |game| {
            let before = game.common.mobs.len();
            let (parent, partner, child) = breed_pair(game, "minecraft:cow");
            assert_eq!(child, None);
            assert_eq!(
                game.common.mobs.len(),
                before + 2,
                "only the two parents exist"
            );
            assert!(game.common.mobs.iter().all(|m| m.extra.age >= 0), "no baby");
            assert!(
                !game
                    .mobs
                    .iter()
                    .any(|m| m.kind.id == "minecraft:experience_orb"),
                "no experience"
            );
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 6000);
                assert_eq!(p.extra.in_love, 0);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn clearing_the_baby_without_cancel_leaves_parents_and_credit_untouched() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use sunrise_sim::mobs::goals::Breed;

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    with_baby(
        move |host, event| {
            seen.fetch_add(1, Ordering::Relaxed);
            for id in [event.parent, event.partner] {
                let nbt = host.entity(id).unwrap().nbt(host).unwrap();
                assert_eq!(nbt.get(&"Age".into()), Some(&mod_api::Tag::Int(0)));
                // BreedGoal.tick runs before Animal.aiStep decrements the love clock.
                assert_eq!(nbt.get(&"InLove".into()), Some(&mod_api::Tag::Int(600)));
            }
            assert!(event.child.take().is_some());
            Flow::Continue
        },
        |game| {
            game.common
                .world
                .chunks
                .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
            assert!(game.entity_ticking_area()(1, 0));
            assert!(game.entity_tick_listed_area()(1, 0));
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            let parent = game.server.next_entity + 1;
            game.server.next_entity += 2;
            for (id, x) in [(parent, 0.5), (parent + 1, 1.5)] {
                let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
                mob.persistent = true;
                mob.no_gravity = true;
                mob.ai.goals.clear();
                mob.ai.targets.clear();
                if id == parent {
                    mob.ai.goals.add(2, Breed::new(1.0));
                }
                game.common.mobs.push(mob);
            }
            game.native_joins_skip();
            game.server.love_cause.insert(parent, None);
            for _ in 0..100 {
                for (i, mob) in game.common.mobs.iter_mut().enumerate() {
                    mob.position = Vec3::new(0.5 + i as f64, 80.0, 0.5);
                    mob.velocity = Vec3::ZERO;
                    mob.extra.in_love = 600;
                }
                game.tick_creature_entities().unwrap();
                if calls.load(Ordering::Relaxed) != 0 {
                    break;
                }
            }
            assert_eq!(
                calls.load(Ordering::Relaxed),
                1,
                "the actual goal must breed"
            );
            assert_eq!(animals_bred(game), 0);
            assert!(game.server.love_cause.contains_key(&parent));
            assert_eq!(game.common.mobs.len(), 2, "only the parents remain");
            for mob in &game.common.mobs {
                assert_eq!(mob.extra.age, 0);
                assert_eq!(mob.extra.in_love, 599);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn egg_layers_never_ask_baby_entity_spawn() {
    let calls = Arc::new(Mutex::new(0u32));
    let count = calls.clone();
    with_baby(
        move |_, _| {
            *count.lock().unwrap() += 1;
            Flow::Continue
        },
        |game| {
            for kind in ["minecraft:turtle", "minecraft:frog", "minecraft:sniffer"] {
                game.mobs.clear();
                let _ = breed_pair(game, kind);
            }
            // A live baby, by contrast, is asked once.
            game.mobs.clear();
            let _ = breed_pair(game, "minecraft:cow");
        },
    );
    assert_eq!(*calls.lock().unwrap(), 1, "only the cow's baby was asked");
}

#[test]
#[ignore = "daily"]
fn breeding_without_a_listener_is_vanilla() {
    let mods: mod_api::Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let (parent, partner, child) = breed_pair(game, "minecraft:cow");
            let baby = find(game, child.expect("a baby joined"));
            assert_eq!(baby.kind.id, "minecraft:cow");
            assert_eq!(baby.extra.age, -24000);
            for id in [parent, partner] {
                assert_eq!(find(game, id).extra.age, 6000);
            }
        });
    });
}

#[test]
#[ignore = "daily"]
fn turtle_hatching_spawns_its_turtle_without_asking_baby_entity_spawn() {
    let calls = Arc::new(Mutex::new(0u32));
    let count = calls.clone();
    with_baby(
        move |_, _| {
            *count.lock().unwrap() += 1;
            Flow::Continue
        },
        |game| {
            let egg = game
                .pack
                .assets
                .registry
                .default_state("minecraft:turtle_egg")
                .unwrap();
            game.mobs.clear();
            game.hatch_turtles([5, 64, 5], egg);
            // One egg: one baby turtle, its home the egg, and no birth hook.
            assert_eq!(game.mobs.len(), 1);
            assert_eq!(game.mobs[0].kind.id, "minecraft:turtle");
            assert_eq!(game.mobs[0].extra.age, -24000);
            assert_eq!(game.mobs[0].extra.home, Some([5, 64, 5]));
        },
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

pub(crate) fn with_splits(
    listeners: Vec<SplitFn>,
    run: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
) {
    let entries: Vec<_> = listeners
        .into_iter()
        .enumerate()
        .map(|(i, listener)| {
            mod_api::ModEntry::new(
                Box::leak(format!("split{i}").into_boxed_str()),
                Box::new(Split(listener)),
            )
        })
        .collect();
    mod_platform::testing::with_mods(entries.into(), || with_game(run));
}

pub(crate) fn animals_bred(game: &sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    game.stats.get("custom", "minecraft:animals_bred")
}

#[test]
#[ignore = "daily"]
fn a_waiting_baby_publishes_its_uuid_only_when_it_joins() {
    with_baby(
        |_, _| Flow::Continue,
        |game| {
            let (parent, _, child) = breed_pair(game, "minecraft:cow");
            let baby = child.unwrap();
            assert_eq!(baby, parent + 2);
            assert!(
                game.common.persist.uuids.contains_key(&baby),
                "published at join"
            );
        },
    );
    with_baby(
        |_, event| {
            event.child.clear();
            assert!(event.child.is_empty());
            Flow::Continue
        },
        |game| {
            let (parent, _, child) = breed_pair(game, "minecraft:cow");
            assert_eq!(child, None);
            assert!(
                !game.common.persist.uuids.contains_key(&(parent + 2)),
                "a removed baby never published one"
            );
        },
    );
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn a_breeding_constructor_engine_fault_retires_only_the_unpublished_child() {
    use sunrise_gameplay::game::server_side::pending_birth_fault::{self as escaping_fault, At};

    const CASE: &str = "SUNRISE_BREEDING_ADMISSION_FAULT";
    if run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_tests::a_breeding_constructor_engine_fault_retires_only_the_unpublished_child",
        CASE,
        &["constructor", "orb"],
    ) {
        return;
    }
    let during_orb = std::env::var(CASE).unwrap() == "orb";

    use std::sync::atomic::{AtomicBool, Ordering};

    struct Constructor {
        baby_hook_ran: Arc<AtomicBool>,
        orb_joined: Arc<AtomicBool>,
        marker: Arc<Mutex<Option<AttachmentKey<i32>>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Constructor {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let key = r
                .register_attachment_type(AttachmentDescriptor::new(
                    "pending_birth_constructor:marker",
                    || 0i32,
                ))
                .unwrap();
            *self.marker.lock().unwrap() = Some(key);
        }

        fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
            if event.kind == "minecraft:cow"
                && let HostSide::Server(host) = host
            {
                let marker = self.marker.lock().unwrap().clone().unwrap();
                marker.set(host, event.holder, 7).unwrap();
                marker.set(host, Holder::Level, 41).unwrap();
            }
        }

        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:experience_orb" {
                self.orb_joined.store(true, Ordering::Relaxed);
            }
            Flow::Continue
        }

        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            _: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            self.baby_hook_ran.store(true, Ordering::Relaxed);
            Flow::Continue
        }
    }
    let baby_hook_ran = Arc::new(AtomicBool::new(false));
    let orb_joined = Arc::new(AtomicBool::new(false));
    let marker = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "pending_birth_constructor",
        Box::new(Constructor {
            baby_hook_ran: baby_hook_ran.clone(),
            orb_joined: orb_joined.clone(),
            marker: marker.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            let marker = marker.lock().unwrap().clone().unwrap();
            let proposed_id = game.server.next_entity + 3;
            let (checkpoint, fault_id) = if during_orb {
                (At::Admission, proposed_id + 1)
            } else {
                (At::Constructed, proposed_id)
            };
            escaping_fault::expect(checkpoint, fault_id, || {
                breed_pair(game, "minecraft:cow");
            });
            assert_eq!(baby_hook_ran.load(Ordering::Relaxed), during_orb);
            assert_eq!(orb_joined.load(Ordering::Relaxed), during_orb);
            assert!(game.orbs.is_empty());
            assert_eq!(game.mobs.len(), 2);
            assert!(!game.persist.uuids.contains_key(&proposed_id));
            assert!(!game.entity_attachments.contains_key(&proposed_id));
            assert!(
                !game
                    .entity_initialization
                    .initialized
                    .contains(&Holder::Entity(proposed_id))
            );
            assert!(game.entity_initialization.active.is_none());
            assert_eq!(marker.get(game, Holder::Level, |&v| v), Ok(41));
        });
    });
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn mob_split_constructs_all_children_before_any_join_and_cleans_up_on_unwind() {
    use sunrise_gameplay::game::server_side::pending_birth_fault::{self as escaping_fault, At};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        RemoveSecond,
        Cancel,
        Keep,
    }
    struct OrderMod {
        trace: Arc<Mutex<Vec<(&'static str, u32)>>>,
        mode: Arc<Mutex<Mode>>,
        marker: Arc<Mutex<Option<AttachmentKey<i32>>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for OrderMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let key = r
                .register_attachment_type(AttachmentDescriptor::new(
                    "pending_split_order:marker",
                    || 0i32,
                ))
                .unwrap();
            *self.marker.lock().unwrap() = Some(key);
        }
        fn mob_split(&self, _: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
            let mode = *self.mode.lock().unwrap();
            let mut trace = self.trace.lock().unwrap();
            assert_eq!(trace.len(), 3, "all proposals constructed before the event");
            assert!(trace.iter().all(|(step, _)| *step == "construct"));
            match mode {
                Mode::Cancel => {
                    trace.push(("cancel", event.parent));
                    Flow::Cancel
                }
                mode => {
                    trace.push(("split", event.parent));
                    if mode == Mode::RemoveSecond {
                        drop(event.children.remove(1).unwrap());
                    }
                    Flow::Continue
                }
            }
        }
        fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
            if let Holder::Entity(id) = event.holder {
                self.trace.lock().unwrap().push(("construct", id));
                if let HostSide::Server(host) = host {
                    let marker = self.marker.lock().unwrap().clone().unwrap();
                    marker.set(host, event.holder, 7).unwrap();
                    marker.set(host, Holder::Level, 23).unwrap();
                }
            }
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            self.trace
                .lock()
                .unwrap()
                .push(("join", event.subject.id()));
            Flow::Continue
        }
    }

    let trace = Arc::new(Mutex::new(Vec::new()));
    let mode = Arc::new(Mutex::new(Mode::RemoveSecond));
    let marker = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "pending_split_order",
        Box::new(OrderMod {
            trace: trace.clone(),
            mode: mode.clone(),
            marker: marker.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            let marker = marker.lock().unwrap().clone().unwrap();
            // Even the removed proposal constructs before the split event. Only the first
            // and last proposal join, keeping their existing constructor attachment.
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            game.split_cube_children(&mut dying);
            assert_eq!(game.mobs.len(), 2);
            assert_eq!(
                *trace.lock().unwrap(),
                vec![
                    ("construct", first),
                    ("construct", first + 1),
                    ("construct", first + 2),
                    ("split", dying.id),
                    ("join", first),
                    ("join", first + 2),
                ]
            );
            assert_eq!(marker.get(game, Holder::Entity(first), |&v| v), Ok(7));
            assert!(!game.entity_attachments.contains_key(&(first + 1)));

            game.mobs.clear();
            trace.lock().unwrap().clear();
            *mode.lock().unwrap() = Mode::Cancel;
            marker.set(game, Holder::Level, 0).unwrap();
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            game.split_cube_children(&mut dying);
            assert!(game.mobs.is_empty());
            assert_eq!(
                *trace.lock().unwrap(),
                vec![
                    ("construct", first),
                    ("construct", first + 1),
                    ("construct", first + 2),
                    ("cancel", dying.id),
                ]
            );
            for id in first..=first + 2 {
                assert!(!game.entity_attachments.contains_key(&id));
            }
            assert_eq!(marker.get(game, Holder::Level, |&v| v), Ok(23));

            // An engine fault after the second constructor leaves neither proposal's state.
            trace.lock().unwrap().clear();
            *mode.lock().unwrap() = Mode::Keep;
            marker.set(game, Holder::Level, 0).unwrap();
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            escaping_fault::expect(At::Constructed, first + 1, || {
                game.split_cube_children(&mut dying);
            });
            assert_eq!(
                *trace.lock().unwrap(),
                [("construct", first), ("construct", first + 1)]
            );
            assert_eq!(marker.get(game, Holder::Level, |&v| v), Ok(23));
            assert!(game.mobs.is_empty());
            assert!(game.entity_initialization.active.is_none());
            for id in [first, first + 1] {
                assert!(!game.persist.uuids.contains_key(&id));
                assert!(!game.entity_attachments.contains_key(&id));
                assert!(
                    !game
                        .entity_initialization
                        .initialized
                        .contains(&Holder::Entity(id))
                );
            }

            // An engine fault during admission retires the failing child and the untouched tail.
            trace.lock().unwrap().clear();
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            escaping_fault::expect(At::Admission, first + 1, || {
                game.split_cube_children(&mut dying);
            });
            assert_eq!(
                *trace.lock().unwrap(),
                [
                    ("construct", first),
                    ("construct", first + 1),
                    ("construct", first + 2),
                    ("split", dying.id),
                    ("join", first),
                    ("join", first + 1),
                ]
            );
            game.native_joins_flush(false, 0);
            assert!(game.entity_initialization.active.is_none());
            assert!(game.mobs.iter().any(|m| m.id == first));
            assert_eq!(marker.get(game, Holder::Entity(first), |&v| v), Ok(7));
            for id in [first + 1, first + 2] {
                assert!(!game.mobs.iter().any(|m| m.id == id));
                assert!(!game.entity_attachments.contains_key(&id));
                assert!(!game.persist.uuids.contains_key(&id));
                assert!(
                    !game
                        .entity_initialization
                        .initialized
                        .contains(&Holder::Entity(id))
                );
            }
        });
    });
}

use sunrise_gameplay_common::game::native_hooks::ModGameExt1 as _;

#[derive(Clone, Debug)]
struct ReadyBreed(sunrise_sim::mobs::goals::Breed);
impl Goal for ReadyBreed {
    fn flags(&self) -> u8 {
        self.0.flags()
    }
    fn can_use(&mut self, mob: &mut Mob, cx: &mut Cx) -> bool {
        self.0.can_use(mob, cx)
    }
    fn tick(&mut self, mob: &mut Mob, cx: &mut Cx) {
        for _ in 0..sunrise_sim::mobs::goals::reduced(60) {
            self.0.tick(mob, cx);
        }
    }
    fn box_clone(&self) -> Box<dyn Goal> {
        Box::new(self.clone())
    }
}

fn add_ready_pair(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> u32 {
    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
    let parent = game.server.next_entity + 1;
    game.server.next_entity += 2;
    for (id, x) in [(parent, 0.5), (parent + 1, 1.5)] {
        let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 80.0, 0.5), 0.0);
        mob.persistent = true;
        mob.no_gravity = true;
        mob.random = LegacyRandom::new(42);
        mob.extra.in_love = 600;
        mob.velocity.x = 0.125;
        mob.ai.goals.clear();
        mob.ai.targets.clear();
        if id == parent {
            mob.ai
                .goals
                .add(2, ReadyBreed(sunrise_sim::mobs::goals::Breed::new(1.0)));
        }
        game.common.mobs.push(mob);
    }
    game.common
        .world
        .chunks
        .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
    game.native_joins_skip();
    parent
}

use sunrise_core::{geometry::Vec3, random::LegacyRandom};
use sunrise_sim::mobs::goals::{Cx, Goal};

#[test]
#[ignore = "daily"]
fn a_noop_birth_listener_leaves_goal_position_age_and_random_unchanged() {
    let tick = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        let parent = add_ready_pair(game);
        game.tick_creature_entities().unwrap();
        let m = find(game, parent);
        let baby = game.common.mobs.iter().find(|m| m.extra.age < 0).unwrap();
        assert_eq!(baby.position, Vec3::new(0.5, 80.0, 0.5));
        assert_eq!(m.extra.age, 5999);
        m.random.state()
    };
    let mut ordinary = None;
    mod_platform::testing::with_mods(Vec::new().into(), || {
        with_game(|game| ordinary = Some(tick(game)))
    });
    with_baby(
        |host, event| {
            let nbt = host.entity(event.parent).unwrap().nbt(host).unwrap();
            assert_eq!(nbt.get(&"InLove".into()), Some(&mod_api::Tag::Int(600)));
            Flow::Continue
        },
        |game| assert_eq!(Some(tick(game)), ordinary),
    );
}

#[test]
#[ignore = "daily"]
fn breeding_dispatches_when_proposed_child_is_none_and_accepts_replacement() {
    let breed_sterile = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
        let mut sterile = (**game.res.mob_types().get("minecraft:cow").unwrap()).clone();
        sterile.id = "test:sterile_cow".into();
        let kind = Arc::new(sterile);
        let base = game.server.next_entity;
        game.server.next_entity += 2;
        game.common.mobs.push(Mob::new(
            base + 1,
            kind.clone(),
            Vec3::new(3.0, 64.0, 4.0),
            0.0,
        ));
        game.common
            .mobs
            .push(Mob::new(base + 2, kind, Vec3::new(4.0, 64.0, 4.0), 0.0));
        game.native_joins_skip();
        for id in [base + 1, base + 2] {
            let m = game.common.mobs.iter_mut().find(|m| m.id == id).unwrap();
            m.random = LegacyRandom::new(0);
            m.extra.age = 0;
            m.extra.in_love = 600;
        }
        let child = game.breed(
            base + 1,
            base + 2,
            None,
            None,
            None,
            sunrise_sim::mobs::BreedExperience::Deferred,
            true,
        );
        (base + 1, base + 2, child)
    };
    // Populating an initially empty slot spawns the replacement and resets both parents.
    with_baby(
        |host, event| {
            assert!(event.child.is_empty());
            let mut pig = event
                .child
                .construct(
                    host,
                    PendingMobSpec::new(
                        mod_api::Identifier::parse("minecraft:pig").unwrap(),
                        mod_api::Identifier::parse(event.dimension).unwrap(),
                        [9.0, 70.0, 9.0],
                        45.0,
                        true,
                    ),
                )
                .unwrap();
            pig.add_tag("from_empty_slot").unwrap();
            assert!(event.child.replace(pig).unwrap().is_none());
            Flow::Continue
        },
        |game| {
            let (parent, partner, child) = breed_sterile(game);
            let baby = find(game, child.expect("populated empty slot should spawn"));
            assert_eq!(baby.kind.id, "minecraft:pig");
            assert_eq!(baby.position, Vec3::new(3.0, 64.0, 4.0));
            assert_eq!(baby.yaw, 0.0);
            assert!(
                game.tags_of(Target::Mob(baby.id))
                    .iter()
                    .any(|t| t == "from_empty_slot")
            );
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 6000);
                assert_eq!(p.extra.in_love, 0);
            }
        },
    );
    // Leaving an initially empty slot empty without cancelling skips finalization and cooldown.
    with_baby(
        |_, event| {
            assert!(event.child.is_empty());
            Flow::Continue
        },
        |game| {
            let (parent, partner, child) = breed_sterile(game);
            assert_eq!(child, None);
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 0);
                assert_eq!(p.extra.in_love, 600);
            }
        },
    );
    // Cancelling an initially empty slot resets both parents.
    with_baby(
        |_, event| {
            assert!(event.child.is_empty());
            Flow::Cancel
        },
        |game| {
            let (parent, partner, child) = breed_sterile(game);
            assert_eq!(child, None);
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 6000);
                assert_eq!(p.extra.in_love, 0);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn fox_breeding_rejects_non_fox_and_trusts_breeders_on_replaced_fox() {
    with_baby(
        |host, event| {
            assert_eq!(
                event
                    .child
                    .construct(
                        host,
                        PendingMobSpec::new(
                            mod_api::Identifier::parse("minecraft:cow").unwrap(),
                            mod_api::Identifier::parse(event.dimension).unwrap(),
                            [0.0, 64.0, 0.0],
                            0.0,
                            true,
                        )
                    )
                    .err(),
                Some(UnspawnedMobError::Invalid)
            );
            let mut kit = event
                .child
                .construct(
                    host,
                    PendingMobSpec::new(
                        mod_api::Identifier::parse("minecraft:fox").unwrap(),
                        mod_api::Identifier::parse(event.dimension).unwrap(),
                        [5.0, 70.0, 5.0],
                        42.0,
                        false,
                    ),
                )
                .unwrap();
            kit.add_tag("custom_kit").unwrap();
            let prev = event.child.replace(kit).unwrap();
            assert_eq!(prev.unwrap().kind().unwrap().to_string(), "minecraft:fox");
            Flow::Continue
        },
        |game| {
            game.common.player =
                Some(sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
            use sunrise_sim::advancements::{Advancement, Criterion, Rewards, Tree};
            let mut tree = Tree::default();
            tree.list.push(Advancement {
                id: "test:fox_born".into(),
                parent: None,
                display: None,
                criteria: vec![Criterion {
                    name: "pre_snap".into(),
                    trigger: "minecraft:bred_animals".into(),
                    player: None,
                    conditions: serde_json::json!({
                        "child": {
                            "minecraft:flags": {"is_baby": false},
                            "minecraft:location": {"position": {"x": 5.0, "y": 70.0, "z": 5.0}}
                        }
                    })
                    .into(),
                }],
                requirements: vec![vec!["pre_snap".into()]],
                rewards: Rewards::default(),
            });
            tree.by_id.insert("test:fox_born".into(), 0);
            tree.by_trigger
                .insert("minecraft:bred_animals".into(), vec![(0, 0)]);
            game.common.adv.tree = Arc::new(tree);
            game.common.adv.loaded = true;
            let feeder = sunrise_sim::lan::playerdata::uuid_ints(game.native_acting_id());
            let (parent, partner, child) = breed_pair_credited(game, "minecraft:fox", true);
            assert!(
                game.common.adv.progress.has("test:fox_born", "pre_snap"),
                "the advancement must see the adult replacement at its pre-snap position"
            );
            let live_parent = find(game, parent);
            let kit = find(game, child.expect("replaced fox kit should join"));
            assert_eq!(kit.kind.id, "minecraft:fox");
            assert!(kit.extra.age < 0);
            assert!(
                game.tags_of(Target::Mob(kit.id))
                    .iter()
                    .any(|t| t == "custom_kit")
            );
            assert!(sunrise_sim::mobs::fox::trusts(&kit, feeder));
            assert_eq!(kit.position, live_parent.position);
            assert_eq!(kit.yaw, 0.0);
            assert_eq!(kit.head_pitch, 0.0);
            for id in [parent, partner] {
                let p = find(game, id);
                assert_eq!(p.extra.age, 6000);
                assert_eq!(p.extra.in_love, 0);
            }
        },
    );
}

#[test]
#[ignore = "daily"]
fn baby_spawn_resolves_the_partners_feeder_when_the_first_left() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_tests::baby_spawn_resolves_the_partners_feeder_when_the_first_left",
        "SUNRISE_BABY_FEEDER_CASE",
        &["stale"],
    ) {
        return;
    }
    let heard = Arc::new(Mutex::new(Vec::new()));
    let record = heard.clone();
    let mods = vec![mod_api::ModEntry::new(
        "baby_feeder",
        Box::new(Baby(Box::new(move |_, event| {
            record
                .lock()
                .unwrap()
                .push((event.player, event.player_entity_id));
            Flow::Cancel
        }))),
    )];
    mod_platform::testing::with_mods(mods.into(), || {
        sunrise_gameplay::game::server_side::native_guest_turn_tests::with_guests(|game| {
            let parent = add_ready_pair(game);
            game.server.love_cause.insert(parent, Some(999)); // no longer connected
            game.server.love_cause.insert(parent + 1, Some(12));
            let _ = game.breed(
                parent,
                parent + 1,
                None,
                None,
                None,
                sunrise_sim::mobs::BreedExperience::Drawn(None),
                true,
            );
        });
    });
    assert_eq!(*heard.lock().unwrap(), [(Some(12), Some(12))]);
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn a_later_birth_unwind_does_not_lose_or_replay_a_cube_removal() {
    use sunrise_gameplay::game::server_side::pending_birth_fault::{self as escaping_fault, At};
    use sunrise_sim::mobs::Species;

    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mods: mod_api::Mods = vec![
        mod_api::ModEntry::new(
            "split",
            Box::new(Split(Box::new(move |host, event| {
                assert!(
                    host.entity(event.parent).is_some(),
                    "the dying parent still belongs to the level"
                );
                seen.fetch_add(1, Ordering::Relaxed);
                Flow::Continue
            }))),
        ),
        mod_api::ModEntry::new("baby", Box::new(Baby(Box::new(|_, _| Flow::Continue)))),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let cube_id = dying.id;
            dying.position = Vec3::new(10.0, 80.0, 10.0);
            dying.health = 0.0;
            dying.death_time = 19;
            dying.persistent = true;
            dying
                .extra
                .effects
                .push(sunrise_sim::mobs::StoredEffect::new(
                    "minecraft:oozing",
                    200,
                    0,
                ));
            game.mobs.push(dying);
            let parent = add_ready_pair(game);
            escaping_fault::expect(At::BabyDispatched, parent, || {
                game.tick_creature_entities().unwrap();
            });
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert!(!game.mobs.iter().any(|m| m.id == cube_id));
            let slime_counts = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let slimes = game.mobs.iter().filter(|m| m.species() == Species::Slime);
                let (mut splits, mut oozing) = (0, 0);
                for slime in slimes {
                    if slime.position.x == 10.0 && slime.position.z == 10.0 {
                        oozing += 1;
                    } else {
                        splits += 1;
                    }
                }
                (splits, oozing)
            };
            assert_eq!(slime_counts(game), (3, 2));
            for mob in &mut game.common.mobs {
                if mob.id == parent || mob.id == parent + 1 {
                    mob.no_ai = true;
                }
            }
            game.tick_creature_entities().unwrap();
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            assert_eq!(
                game.mobs
                    .iter()
                    .filter(|m| m.species() == Species::Slime)
                    .count(),
                5,
                "neither the split nor Oozing runs again"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn split_listener_can_remove_the_live_parents_oozing() {
    if run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_tests::split_listener_can_remove_the_live_parents_oozing",
        "SUNRISE_SPLIT_OOZING_CASE",
        &["remove"],
    ) {
        return;
    }
    use std::sync::atomic::{AtomicBool, Ordering};
    let removed = Arc::new(AtomicBool::new(false));
    let observed = removed.clone();
    with_split(
        move |host, event| {
            let parent = host.living(host.entity(event.parent).unwrap()).unwrap();
            let effect = host.effect_key("minecraft:oozing").unwrap();
            observed.store(parent.remove_effect(host, effect), Ordering::Relaxed);
            Flow::Continue
        },
        |game| {
            fixture_resident_column(&mut game.common.world, (0, 1));
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            dying.health = 0.0;
            dying
                .extra
                .effects
                .push(sunrise_sim::mobs::StoredEffect::new(
                    "minecraft:oozing",
                    200,
                    0,
                ));
            let first = game.server.next_entity + 1;
            game.mobs.push(dying);
            game.native_joins_skip();
            let join = game.mobs[0].join;
            game.complete_mob_removal_for_test(join, true);
            assert!(removed.load(Ordering::Relaxed));
            assert_vanilla_children(&spawned(game), "minecraft:slime", first);
        },
    );
}

#[test]
#[ignore = "daily"]
fn detached_constructed_baby_preserves_attachments_and_reattaches_across_worlds_without_collision()
{
    thread_local! {
        static STASH: std::cell::RefCell<Option<UnspawnedMob>> =
            const { std::cell::RefCell::new(None) };
    }
    struct CrossWorldMod {
        construct_calls: Arc<Mutex<Vec<(Holder, String)>>>,
        attach_on_split: Arc<Mutex<bool>>,
        marker: Arc<Mutex<Option<AttachmentKey<i32>>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for CrossWorldMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let key = r
                .register_attachment_type(AttachmentDescriptor::new(
                    "pending_birth_cross_world:marker",
                    || 0i32,
                ))
                .unwrap();
            *self.marker.lock().unwrap() = Some(key);
        }
        fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
            self.construct_calls
                .lock()
                .unwrap()
                .push((event.holder, event.kind.to_owned()));
            let marker = self.marker.lock().unwrap().clone().unwrap();
            match host {
                HostSide::Server(host) => {
                    marker.set(host, event.holder, 41).unwrap();
                }
                HostSide::Client(host) => {
                    marker.set(host, event.holder, 41).unwrap();
                }
            }
        }
        fn baby_entity_spawn(
            &self,
            _: &mut dyn ServerHost,
            event: &mut BabyEntitySpawnEvent<'_>,
        ) -> Flow {
            let taken = event.child.take();
            STASH.with(|slot| *slot.borrow_mut() = taken);
            Flow::Continue
        }
        fn mob_split(&self, _: &mut dyn ServerHost, event: &mut MobSplitEvent<'_>) -> Flow {
            if *self.attach_on_split.lock().unwrap() {
                event.children.clear();
                let stashed = STASH.with(|slot| slot.borrow_mut().take()).unwrap();
                event.children.push(stashed).unwrap();
            }
            Flow::Continue
        }
    }

    STASH.with(|slot| *slot.borrow_mut() = None);
    let construct_calls = Arc::new(Mutex::new(Vec::new()));
    let attach_on_split = Arc::new(Mutex::new(false));
    let marker = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "pending_birth_cross_world",
        Box::new(CrossWorldMod {
            construct_calls: construct_calls.clone(),
            attach_on_split: attach_on_split.clone(),
            marker: marker.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        // World 1: construct a breeding cow baby (running entity_constructing -> marker=41),
        // then take it out of the slot without spawning it.
        let old_reserved_id = {
            let mut reserved = 0;
            with_game(|game| {
                let (parent, _, child) = breed_pair(game, "minecraft:cow");
                assert_eq!(child, None);
                reserved = parent + 2;
                assert!(
                    !game.common.persist.uuids.contains_key(&reserved),
                    "detached baby must not leak UUID in World 1"
                );
                assert!(
                    !game.common.entity_attachments.contains_key(&reserved),
                    "detached baby must not leak attachments in World 1"
                );
                assert!(
                    !game
                        .entity_initialization
                        .initialized
                        .contains(&Holder::Entity(reserved)),
                    "detached baby must not leak initialization marker in World 1"
                );
                assert!(game.common.entity_initialization.active.is_none());
            });
            reserved
        };
        assert_eq!(
            construct_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, k)| k == "minecraft:cow")
                .count(),
            1,
            "cow baby constructed once in World 1"
        );

        // World 2: place a resident sheep at the exact same entity ID as World 1's old
        // reserved ID, then reattach the stashed cow baby via mob_split.
        *attach_on_split.lock().unwrap() = true;
        with_game(|game| {
            let sheep_kind = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
            game.server.next_entity = old_reserved_id;
            game.common.mobs.push(Mob::new(
                old_reserved_id,
                sheep_kind,
                Vec3::new(1.0, 64.0, 1.0),
                0.0,
            ));
            game.common
                .persist
                .uuids
                .insert(old_reserved_id, [9, 9, 9, 9]);
            game.native_joins_skip();

            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            game.split_cube_children(&mut dying);

            // Resident sheep at old_reserved_id is completely untouched.
            let sheep = find(game, old_reserved_id);
            assert_eq!(sheep.kind.id, "minecraft:sheep");
            assert_eq!(
                game.common.persist.uuids.get(&old_reserved_id),
                Some(&[9, 9, 9, 9])
            );

            // Reattached cow received a fresh ID, kept its constructor attachment (41),
            // and was NOT double-constructed.
            let cow = game
                .mobs
                .iter()
                .find(|m| m.kind.id == "minecraft:cow")
                .expect("reattached cow joined World 2")
                .clone();
            assert_ne!(cow.id, old_reserved_id);
            assert!(game.common.persist.uuids.contains_key(&cow.id));
            assert_eq!(
                marker
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap()
                    .get(game, Holder::Entity(cow.id), |&v| v),
                Ok(41)
            );
        });
        assert_eq!(
            construct_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(_, k)| k == "minecraft:cow")
                .count(),
            1,
            "reattached cow must not run entity_constructing a second time"
        );
    });
    STASH.with(|slot| *slot.borrow_mut() = None);
}

#[test]
#[ignore = "daily"]
fn split_and_detached_construction_precede_admission() {
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;

    const CASE: &str = "SUNRISE_SPLIT_CONSTRUCTOR_CASE";
    if run_fresh_cases(
        "cut_layout::game::creatures::pending_birth_tests::split_and_detached_construction_precede_admission",
        CASE,
        &["unsubscribed", "detached"],
    ) {
        return;
    }

    struct Trace {
        calls: Arc<Mutex<Vec<(&'static str, u32)>>>,
        marker: Arc<Mutex<Option<AttachmentKey<i32>>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Trace {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            *self.marker.lock().unwrap() = Some(
                r.register_attachment_type(AttachmentDescriptor::new(
                    "split_construct_trace:marker",
                    || 0i32,
                ))
                .unwrap(),
            );
        }
        fn entity_constructing(&self, host: HostSide<'_>, event: &mut EntityConstructingEvent) {
            if let (HostSide::Server(host), Holder::Entity(id)) = (host, event.holder) {
                self.calls.lock().unwrap().push(("construct", id));
                let marker = self.marker.lock().unwrap().clone().unwrap();
                marker.set(host, event.holder, 41).unwrap();
            }
        }
        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            self.calls
                .lock()
                .unwrap()
                .push(("join", event.subject.id()));
            Flow::Continue
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    let marker = Arc::new(Mutex::new(None));
    let detached = std::env::var(CASE).unwrap() == "detached";
    let mut entries = vec![mod_api::ModEntry::new(
        "split_construct_trace",
        Box::new(Trace {
            calls: calls.clone(),
            marker: marker.clone(),
        }),
    )];
    if detached {
        entries.push(mod_api::ModEntry::new(
            "split_construct_replace",
            Box::new(Split(Box::new(|host, event| {
                event.children.clear();
                let proposed = || {
                    spec(
                        "minecraft:pig",
                        [0.5, 64.0, 0.5],
                        0.0,
                        None,
                        event.dimension,
                    )
                };
                drop(event.children.construct(host, proposed()).unwrap());
                let kept = event.children.construct(host, proposed()).unwrap();
                event.children.push(kept).unwrap();
                Flow::Continue
            }))),
        ));
    }
    let mods: mod_api::Mods = entries.into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            calls.lock().unwrap().clear();
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            game.split_cube_children(&mut dying);
            let mut expected = vec![
                ("construct", first),
                ("construct", first + 1),
                ("construct", first + 2),
            ];
            if detached {
                let (discarded, kept) = (first + 3, first + 4);
                expected.extend([
                    ("construct", discarded),
                    ("construct", kept),
                    ("join", kept),
                ]);
                assert_eq!(game.common.mobs.len(), 1);
                assert_eq!(game.common.mobs[0].kind.id, "minecraft:pig");
                assert_eq!(
                    marker.lock().unwrap().clone().unwrap().get(
                        game,
                        Holder::Entity(kept),
                        |&value| value
                    ),
                    Ok(41)
                );
                assert!(!game.common.persist.uuids.contains_key(&discarded));
                assert!(!game.common.entity_attachments.contains_key(&discarded));
                assert!(
                    !game
                        .entity_initialization
                        .initialized
                        .contains(&Holder::Entity(discarded))
                );
            } else {
                expected.extend([("join", first), ("join", first + 1), ("join", first + 2)]);
                assert_eq!(game.common.mobs.len(), 3);
            }
            assert_eq!(*calls.lock().unwrap(), expected);
        });
    });
}

use mod_api::{
    AttachmentDescriptor, AttachmentKey, EntityConstructingEvent, EntityJoinLevelEvent, Holder,
    HostSide, UnspawnedMob,
};

pub(crate) fn fixture_resident_column(world: &mut sunrise_level::world::World, key: (i32, i32)) {
    if !world.chunks.contains_key(&key) {
        let chunk = sunrise_level::chunk::Chunk::with_layout(key.0, key.1, world.layout());
        world.chunks.insert(key, std::sync::Arc::new(chunk));
    }
}
