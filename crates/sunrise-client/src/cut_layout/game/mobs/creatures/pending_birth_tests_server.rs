use crate::game::creatures::pending_birth_tests::AT;
use crate::game::creatures::pending_birth_tests::IGNORE;
pub(crate) use crate::game::creatures::pending_birth_tests::SEED;
pub(crate) use crate::game::creatures::pending_birth_tests::YAWS;
use crate::game::creatures::pending_birth_tests::animals_bred;
pub(crate) use crate::game::creatures::pending_birth_tests::find;
use crate::game::creatures::pending_birth_tests::spawned;
use crate::game::creatures::pending_birth_tests::spec;
use crate::game::creatures::pending_birth_tests::with_baby;
use crate::game::creatures::pending_birth_tests::{NEXT_INT, SPLIT_DISPLACEMENT};
use crate::game::creatures::pending_birth_tests::{
    assert_vanilla_children, breed_pair, fixture_resident_column,
};
use crate::game::creatures::pending_birth_tests::{with_split, with_splits};
use mod_api::Flow;
use std::sync::{Arc, Mutex};
use sunrise_core::geometry::Vec3;
use sunrise_core::random::RandomSource;
use sunrise_gameplay::game::native_entity_refs::tests::with_game;

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn baby_slot_replacement_does_not_publish_on_engine_unwind() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use sunrise_gameplay::game::server_side::pending_birth_fault::{self as escaping_fault, At};

    let replaced = Arc::new(AtomicBool::new(false));
    let observed = replaced.clone();
    with_baby(
        move |host, event| {
            let pig = event
                .child
                .construct(
                    host,
                    spec(
                        "minecraft:pig",
                        [0.5, 64.0, 0.0],
                        0.0,
                        None,
                        event.dimension,
                    ),
                )
                .unwrap();
            let original = event.child.replace(pig).unwrap().unwrap();
            assert_eq!(original.kind().unwrap().to_string(), "minecraft:cow");
            replaced.store(true, Ordering::Relaxed);
            Flow::Continue
        },
        |game| {
            let (uuids, attachments) = (game.persist.uuids.len(), game.entity_attachments.len());
            let parent = game.server.next_entity + 1;
            escaping_fault::expect(At::BabyDispatched, parent, || {
                breed_pair(game, "minecraft:cow");
            });
            assert!(observed.load(Ordering::Relaxed));
            assert_eq!(game.persist.uuids.len(), uuids, "the baby's UUID is gone");
            assert_eq!(game.entity_attachments.len(), attachments);
            assert_eq!(game.mobs.len(), 2, "only the parents joined");
        },
    );
}

#[test]
#[ignore = "daily"]
fn panicking_spawn_listeners_lose_cancellation_but_keep_direct_child_edits() {
    with_split(
        |_, event| {
            event.children.get_mut(0).unwrap().set_yaw(1.0).unwrap();
            drop(event.children.remove(1).unwrap());
            event.cancelled = true;
            panic!("split listener failed");
        },
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            game.split_cube_children(&mut dying);
            assert_eq!(
                game.common.mobs.len(),
                2,
                "the cancellation was not applied"
            );
            assert_eq!(game.common.mobs[0].yaw, 1.0);
            assert_eq!(game.common.mobs[1].yaw, YAWS[2]);
        },
    );
    with_baby(
        |host, event| {
            let pig = event
                .child
                .construct(
                    host,
                    spec(
                        "minecraft:pig",
                        [0.5, 64.0, 0.0],
                        0.0,
                        None,
                        event.dimension,
                    ),
                )
                .unwrap();
            drop(event.child.replace(pig).unwrap());
            event.cancelled = true;
            panic!("breeding listener failed");
        },
        |game| {
            let (_, _, child) = breed_pair(game, "minecraft:cow");
            let child = find(game, child.expect("the cancellation was not applied"));
            assert_eq!(child.kind.id, "minecraft:pig");
            assert!(child.is_baby());
        },
    );
}

/// What a listener's host write does to a parent, stood in for by a closure the production
/// dispatch runs where the listeners run (the host methods that move a mob are not reachable
/// from a hook's `ServerHost` in the fixture world).
#[test]
#[ignore = "daily"]
fn breeding_spawns_after_parent_host_edit_and_cancel_resets_both_parents() {
    with_baby(
        |host, event| {
            let parent = host.entity(event.parent).expect("parent handle");
            assert!(parent.teleport_to(host, [2.0, 64.0, 0.0], None));
            let mut nbt = mod_api::Compound::new();
            nbt.insert("InLove".into(), mod_api::Tag::Int(300));
            assert!(parent.merge_nbt(host, nbt));
            Flow::Continue
        },
        |game| {
            fixture_resident_column(&mut game.common.world, (0, 0));
            let (parent, partner, child) = breed_pair(game, "minecraft:cow");
            let baby = find(game, child.expect("parent host edit must not abort birth"));
            assert_eq!(baby.kind.id, "minecraft:cow");
            assert_eq!(baby.extra.age, -24000);
            assert_eq!(find(game, parent).position.x, 2.0);
            // Post-event snap tracks the live parent after listener teleport and zeroes yaw/pitch.
            assert_eq!(baby.position, Vec3::new(2.0, 64.0, 0.0));
            assert_eq!(baby.yaw, 0.0);
            assert_eq!(baby.head_pitch, 0.0);
            for id in [parent, partner] {
                let mob = find(game, id);
                assert_eq!(mob.extra.age, 6000);
                assert_eq!(mob.extra.in_love, 0);
            }
        },
    );
    with_baby(
        |host, event| {
            for id in [event.parent, event.partner] {
                let entity = host.entity(id).expect("parent handle");
                let mut nbt = mod_api::Compound::new();
                nbt.insert("Age".into(), mod_api::Tag::Int(0));
                nbt.insert("InLove".into(), mod_api::Tag::Int(300));
                assert!(entity.merge_nbt(host, nbt));
            }
            Flow::Cancel
        },
        |game| {
            fixture_resident_column(&mut game.common.world, (0, 0));
            let (parent, partner, child) = breed_pair(game, "minecraft:cow");
            assert_eq!(child, None);
            assert!(
                game.common.mobs.iter().all(|m| m.extra.age >= 0),
                "no baby joined"
            );
            for id in [parent, partner] {
                let mob = find(game, id);
                assert_eq!(mob.extra.age, 6000);
                assert_eq!(mob.extra.in_love, 0);
            }
        },
    );
}

pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::breed_pair_credited;

#[test]
#[ignore = "daily"]
fn cancelled_split_spawns_nothing_but_keeps_the_construction_draws() {
    with_split(
        |_, _| Flow::Cancel,
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let next_id = game.server.next_entity;
            game.split_cube_children(&mut dying);
            assert!(
                game.common.mobs.is_empty(),
                "a cancelled split spawns no child"
            );
            assert_eq!(
                game.server.next_entity,
                next_id + 3,
                "all proposals were constructed"
            );
            // The count and the three yaw draws were spent before the callback.
            assert_eq!(dying.random.next_int(), NEXT_INT);
        },
    );
}

#[test]
#[ignore = "daily"]
fn credit_goes_to_the_player_only_when_the_baby_is_accepted() {
    with_baby(
        |_, _| Flow::Continue,
        |game| {
            let (parent, partner, child) = breed_pair_credited(game, "minecraft:cow", true);
            assert!(child.is_some());
            assert_eq!(animals_bred(game), 1, "an accepted baby is credited once");
            assert!(!game.server.love_cause.contains_key(&parent));
            assert!(!game.server.love_cause.contains_key(&partner));
        },
    );
    with_baby(
        |_, _| Flow::Cancel,
        |game| {
            let (parent, partner, child) = breed_pair_credited(game, "minecraft:cow", true);
            assert_eq!(child, None);
            assert_eq!(animals_bred(game), 0, "a vetoed baby earns no statistic");
            assert!(
                !game.server.love_cause.contains_key(&parent),
                "the credit is spent"
            );
            assert!(!game.server.love_cause.contains_key(&partner));
        },
    );
}

pub(crate) use sunrise_gameplay::game::server_side::creatures_server::pending_birth_matrix_tests::cube;

#[test]
#[ignore = "daily"]
fn edit_remove_push_commits_a_c_d_once() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!(
            "{}::edit_remove_push_commits_a_c_d_once",
            module_path!().split_once("::").unwrap().1
        ),
        "SUNRISE_SPLIT_EDIT_COORDINATES",
        &["coordinates"],
    ) {
        return;
    }
    let second_saw = Arc::new(Mutex::new(None));
    let record = second_saw.clone();
    with_splits(
        vec![
            Box::new(move |host, event| {
                event.children.get_mut(0).unwrap().set_yaw(90.0).unwrap();
                let removed = event.children.remove(1).unwrap();
                assert_eq!(removed.kind().unwrap().to_string(), "minecraft:slime");
                drop(removed);
                let d = event
                    .children
                    .construct(
                        host,
                        spec(
                            "minecraft:slime",
                            [3.0, 70.0, 4.0],
                            12.0,
                            Some(2),
                            event.dimension,
                        ),
                    )
                    .unwrap();
                event.children.push(d).unwrap();
                // A detached child dropped without attaching never joins.
                let _dropped = event
                    .children
                    .construct(
                        host,
                        spec(
                            "minecraft:slime",
                            [9.0, 70.0, 9.0],
                            0.0,
                            Some(2),
                            event.dimension,
                        ),
                    )
                    .unwrap();
                assert_eq!(event.children.len(), 3);
                Flow::Continue
            }),
            Box::new(move |_, event| {
                *record.lock().unwrap() = Some((
                    event.children.len(),
                    event.children.get(2).unwrap().position(),
                ));
                Flow::Continue
            }),
        ],
        |game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            game.split_cube_children(&mut dying);
            let kids = spawned(game);
            // [A, C, D]: A with yaw 90, C the vanilla third child, D pushed.
            assert_eq!(kids.len(), 3);
            assert_eq!(kids[0].yaw, 90.0);
            assert_eq!(kids[0].id, first);
            assert_eq!(
                kids[1].position.x.to_bits(),
                (AT[0] - SPLIT_DISPLACEMENT).to_bits()
            );
            assert_eq!(kids[1].position.y.to_bits(), (AT[1] + 0.5).to_bits());
            assert_eq!(
                kids[1].position.z.to_bits(),
                (AT[2] + SPLIT_DISPLACEMENT).to_bits()
            );
            assert_eq!(kids[1].yaw, YAWS[2]);
            assert_eq!(kids[1].id, first + 2);
            assert_eq!(kids[2].position.x, 3.0);
            assert_eq!(kids[2].position.y, 70.0);
            assert_eq!(kids[2].yaw, 12.0);
            assert_eq!(kids[2].id, first + 3);
            assert_eq!(kids[2].extra.size, 2);
        },
    );
    assert_eq!(
        *second_saw.lock().unwrap(),
        Some((3, Ok([3.0, 70.0, 4.0]))),
        "later listeners see earlier listeners' edits"
    );
}

#[test]
#[ignore = "daily"]
fn no_listener_split_is_exactly_vanilla() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!(
            "{}::no_listener_split_is_exactly_vanilla",
            module_path!().split_once("::").unwrap().1
        ),
        "SUNRISE_SPLIT_NO_LISTENER_COORDINATES",
        &["coordinates"],
    ) {
        return;
    }
    let mods: mod_api::Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let mut dying = cube(game, "minecraft:slime", 4, SEED);
            let first = game.server.next_entity + 1;
            game.split_cube_children(&mut dying);
            assert_vanilla_children(&spawned(game), "minecraft:slime", first);
            assert_eq!(dying.random.next_int(), NEXT_INT);
        });
    });
}

#[test]
#[ignore = "daily"]
fn size_four_slime_and_magma_split_three_children_and_draw_the_vanilla_yaws() {
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        &format!(
            "{}::size_four_slime_and_magma_split_three_children_and_draw_the_vanilla_yaws",
            module_path!().split_once("::").unwrap().1
        ),
        "SUNRISE_SPLIT_COORDINATES",
        &["coordinates"],
    ) {
        return;
    }
    let _ = IGNORE;
    for kind in ["minecraft:slime", "minecraft:magma_cube"] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        with_split(
            move |_, event| {
                let len = event.children.len();
                record.lock().unwrap().push((
                    event.dimension.to_owned(),
                    event.kind.to_owned(),
                    event.size,
                    event.child_size,
                    len,
                ));
                Flow::Continue
            },
            |game| {
                let mut dying = cube(game, kind, 4, SEED);
                let first = game.server.next_entity + 1;
                game.split_cube_children(&mut dying);
                assert_vanilla_children(&spawned(game), kind, first);
                assert_eq!(dying.random.next_int(), NEXT_INT);
            },
        );
        assert_eq!(
            *seen.lock().unwrap(),
            [("minecraft:overworld".to_owned(), kind.to_owned(), 4, 2, 3)]
        );
    }
}
