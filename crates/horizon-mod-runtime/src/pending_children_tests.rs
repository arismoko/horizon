use super::*;
use std::{cell::Cell, rc::Rc};

// Deliberately not Clone: ownership transfers must preserve opaque state.
#[derive(Debug, PartialEq)]
struct Child {
    name: char,
    yaw: f32,
}

fn child(name: char) -> Child {
    Child { name, yaw: 0.0 }
}

fn collect(mut frozen: FrozenChildren<Child>) -> Vec<Child> {
    let mut out = Vec::new();
    while frozen
        .consume_next::<()>(|object| {
            out.push(object);
            Ok(())
        })
        .unwrap()
    {}
    assert_eq!(frozen.completed(), out.len());
    out
}

#[test]
fn edit_remove_append() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            7,
            64,
            vec![child('A'), child('B'), child('C')],
            |arena, action| {
                let a = arena.borrow(action, 0).unwrap();
                let b = arena.borrow(action, 1).unwrap();
                arena.get_mut(a).unwrap().yaw = 90.0;
                arena.remove(action, 1).unwrap();
                assert_eq!(arena.get(b), Err(Error::Expired));
                let d = arena.construct(action, child('D')).unwrap();
                arena.append(action, d).unwrap();
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(
        collect(frozen),
        vec![
            Child {
                name: 'A',
                yaw: 90.0
            },
            child('C'),
            child('D')
        ]
    );
}

#[test]
fn replace_and_ownership_error() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            7,
            64,
            vec![child('A'), child('B'), child('C')],
            |arena, action| {
                let c = arena.borrow(action, 2).unwrap();
                let e = arena.construct(action, child('E')).unwrap();
                arena.replace(action, 2, e).unwrap();
                assert_eq!(arena.get(c), Err(Error::Expired));
                assert_eq!(arena.append(action, e), Err(Error::Ownership));
                assert_eq!(arena.len(action), Ok(3));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(collect(frozen), vec![child('A'), child('B'), child('E')]);
}

#[test]
fn reentrancy_and_retained_tokens() {
    let mut arena = PendingChildren::default();
    let mut retained = None;
    let mut owner = None;
    let frozen = arena
        .run(
            7,
            64,
            vec![child('A')],
            |arena, outer| {
                let old = arena.borrow(outer, 0).unwrap();
                let detached = arena.construct(outer, child('D')).unwrap();
                let nested = arena
                    .run(
                        7,
                        1,
                        vec![child('B')],
                        |arena, inner| {
                            assert_eq!(arena.get(old), Err(Error::Expired));
                            assert_eq!(arena.append(inner, detached), Err(Error::Expired));
                            assert_eq!(arena.remove(outer, 0), Err(Error::Expired));
                            Decision::Accept
                        },
                        |_| true,
                    )
                    .unwrap();
                assert_eq!(collect(nested), vec![child('B')]);
                assert_eq!(arena.get(old), Err(Error::Expired));
                assert_eq!(arena.append(outer, detached), Err(Error::Expired));
                let fresh = arena.borrow(outer, 0).unwrap();
                arena.get_mut(fresh).unwrap().yaw = 90.0;
                retained = Some(fresh);
                owner = Some(arena.construct(outer, child('E')).unwrap());
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(arena.get(retained.unwrap()), Err(Error::Expired));
    arena
        .run(
            8,
            1,
            vec![child('F')],
            |arena, next| {
                assert_eq!(arena.get(retained.unwrap()), Err(Error::Expired));
                assert_eq!(arena.append(next, owner.unwrap()), Err(Error::Expired));
                Decision::Cancel
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(
        collect(frozen),
        vec![Child {
            name: 'A',
            yaw: 90.0
        }]
    );
}

#[test]
fn listeners_share_edits_but_not_leases() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                let first = arena.borrow(action, 0).unwrap();
                arena.get_mut(first).unwrap().yaw = 90.0;
                let detached = arena.construct(action, child('D')).unwrap();
                arena.next_listener(action).unwrap();
                assert_eq!(arena.get(first), Err(Error::Expired));
                assert_eq!(arena.borrow_owned(detached), Err(Error::Expired));
                let second = arena.borrow(action, 0).unwrap();
                assert_eq!(arena.get(second).unwrap().yaw, 90.0);
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(collect(frozen)[0].yaw, 90.0);
}

#[test]
fn forged_world_action_lease_and_slot_refuse_before_mutation() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            7,
            1,
            vec![child('A')],
            |arena, action| {
                let key = arena.borrow(action, 0).unwrap();
                let invalid = [
                    Borrow { world: 8, ..key },
                    Borrow {
                        action: Action(0),
                        ..key
                    },
                    Borrow { lease: 0, ..key },
                    Borrow {
                        slot: usize::MAX,
                        ..key
                    },
                ];
                for key in invalid {
                    assert_eq!(arena.get_mut(key), Err(Error::Expired));
                    assert_eq!(arena.append(action, Owned(key)), Err(Error::Expired));
                }
                assert_eq!(arena.get(key), Ok(&child('A')));
                assert_eq!(arena.len(action), Ok(1));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(collect(frozen), vec![child('A')]);
}

#[test]
fn oversized_initial_list_refuses_without_entering_dispatch() {
    let mut arena = PendingChildren::default();
    for (capacity, count) in [(0, 0), (65, 0), (64, 65), (1, 2)] {
        assert!(matches!(
            arena.run(
                1,
                capacity,
                vec![0; count],
                |_, _| { panic!("invalid initial list reached listener") },
                |_| true
            ),
            Err(Error::Capacity)
        ));
        assert!(arena.frames.is_empty());
    }
}

#[test]
fn invalid_input_capacity_and_duplicate_token() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            64,
            (0..64).collect(),
            |arena, action| {
                let extra = arena.construct(action, 64).unwrap();
                assert_eq!(arena.append(action, extra), Err(Error::Capacity));
                assert_eq!(arena.len(action), Ok(64));
                assert_eq!(arena.get(arena.borrow(action, 63).unwrap()), Ok(&63));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(
        frozen.objects.into_iter().collect::<Vec<_>>(),
        (0..64).collect::<Vec<_>>()
    );
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                let second = arena.construct(action, child('B')).unwrap();
                assert_eq!(arena.append(action, second), Err(Error::Capacity));
                // Failed append has not consumed ownership. Replacement may still consume it.
                assert_eq!(arena.replace(action, 2, second), Err(Error::Invalid));
                arena.replace(action, 0, second).unwrap();
                assert_eq!(arena.replace(action, 0, second), Err(Error::Ownership));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(collect(frozen), vec![child('B')]);
}

#[test]
fn cancel_is_monotonic_and_removing_all_children_suppresses_output() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                arena.cancel(action).unwrap();
                arena.next_listener(action).unwrap();
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert!(collect(frozen).is_empty());
    let frozen = arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                arena.remove(action, 0).unwrap();
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert!(collect(frozen).is_empty());
}

#[test]
fn stale_and_final_validation_refuse_before_any_consumer() {
    let mut arena = PendingChildren::default();
    let mut calls = 0;
    assert!(matches!(
        arena.run(
            1,
            1,
            vec![child('A')],
            |_, _| {
                calls += 1;
                Decision::Stale
            },
            |_| true
        ),
        Err(Error::Stale)
    ));
    assert_eq!(calls, 1, "a stale action must never be replayed");
    assert!(matches!(
        arena.run(
            1,
            64,
            vec![child('A'), child('B')],
            |arena, action| {
                let b = arena.borrow(action, 1).unwrap();
                arena.get_mut(b).unwrap().yaw = f32::NAN;
                Decision::Accept
            },
            |object| object.yaw.is_finite()
        ),
        Err(Error::Invalid)
    ));
    assert!(arena.frames.is_empty());
}

#[test]
fn lifetime_slot_budget_does_not_reuse_removed_slots() {
    let mut arena = PendingChildren::default();
    arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                let old = arena.borrow(action, 0).unwrap();
                arena.remove(action, 0).unwrap();
                for _ in 1..MAX_SLOTS {
                    let owned = arena.construct(action, child('B')).unwrap();
                    arena.append(action, owned).unwrap();
                    arena.remove(action, 0).unwrap();
                }
                assert_eq!(arena.construct(action, child('C')), Err(Error::Budget));
                assert_eq!(arena.get(old), Err(Error::Expired));
                assert_eq!(arena.len(action), Ok(0));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
}

#[test]
fn nesting_budget_refuses_without_disturbing_outer_actions() {
    fn nest(arena: &mut PendingChildren<Child>, depth: usize) {
        let result = arena.run(
            1,
            1,
            vec![child('A')],
            |arena, action| {
                nest(arena, depth + 1);
                assert_eq!(arena.len(action), Ok(1));
                Decision::Accept
            },
            |_| true,
        );
        if depth == MAX_DEPTH {
            assert!(matches!(result, Err(Error::Budget)));
        } else {
            assert_eq!(collect(result.unwrap()), vec![child('A')]);
        }
    }
    nest(&mut PendingChildren::default(), 0);
}

struct DropCount(Rc<Cell<usize>>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn panic_before_freeze_drops_pending_objects_and_expires_tokens() {
    let dropped = Rc::new(Cell::new(0));
    let mut arena = PendingChildren::default();
    let mut retained = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = arena.run(
            1,
            1,
            vec![DropCount(dropped.clone())],
            |arena, action| {
                retained = Some(arena.borrow(action, 0).unwrap());
                arena.construct(action, DropCount(dropped.clone())).unwrap();
                panic!("listener");
            },
            |_| true,
        );
    }));
    assert!(result.is_err());
    assert_eq!(dropped.get(), 2);
    assert!(matches!(arena.get(retained.unwrap()), Err(Error::Expired)));
    assert!(arena.frames.is_empty());
}

#[test]
fn validation_panic_also_closes_the_action() {
    let dropped = Rc::new(Cell::new(0));
    let mut arena = PendingChildren::default();
    let mut retained = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = arena.run(
            1,
            1,
            vec![DropCount(dropped.clone())],
            |arena, action| {
                retained = Some(arena.borrow(action, 0).unwrap());
                Decision::Accept
            },
            |_| panic!("validator"),
        );
    }));
    assert!(result.is_err());
    assert_eq!(dropped.get(), 1);
    assert!(matches!(arena.get(retained.unwrap()), Err(Error::Expired)));
    assert!(arena.frames.is_empty());
}

#[test]
fn removal_replacement_and_unattached_cleanup_drop_each_owner_once() {
    let dropped = Rc::new(Cell::new(0));
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            64,
            vec![DropCount(dropped.clone()), DropCount(dropped.clone())],
            |arena, action| {
                arena.remove(action, 0).unwrap();
                assert_eq!(dropped.get(), 1);
                let replacement = arena.construct(action, DropCount(dropped.clone())).unwrap();
                arena.replace(action, 0, replacement).unwrap();
                assert_eq!(dropped.get(), 2);
                arena.construct(action, DropCount(dropped.clone())).unwrap();
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(
        dropped.get(),
        3,
        "unattached object dies when action closes"
    );
    drop(frozen);
    assert_eq!(
        dropped.get(),
        4,
        "unconsumed output remains owned, then drops once"
    );
}

#[test]
fn nested_panic_closes_only_nested_frame() {
    let mut arena = PendingChildren::default();
    let frozen = arena
        .run(
            1,
            1,
            vec![child('A')],
            |arena, outer| {
                let old = arena.borrow(outer, 0).unwrap();
                let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = arena.run(1, 1, vec![child('B')], |_, _| panic!("nested"), |_| true);
                }));
                assert!(failure.is_err());
                assert_eq!(arena.get(old), Err(Error::Expired));
                assert_eq!(arena.get(arena.borrow(outer, 0).unwrap()), Ok(&child('A')));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
    assert_eq!(collect(frozen), vec![child('A')]);
}

#[test]
fn consumer_failure_never_replays_completed_or_entered_children() {
    let mut arena = PendingChildren::default();
    let mut frozen = arena
        .run(
            1,
            64,
            vec![child('A'), child('B'), child('C')],
            |_, _| Decision::Accept,
            |_| true,
        )
        .unwrap();
    let mut entered = Vec::new();
    frozen
        .consume_next::<()>(|object| {
            entered.push(object.name);
            Ok(())
        })
        .unwrap();
    assert_eq!(
        frozen.consume_next(|object| {
            entered.push(object.name);
            Err("insertion")
        }),
        Err(ConsumeError::Consumer("insertion"))
    );
    assert_eq!(
        frozen.consume_next::<()>(|object| {
            entered.push(object.name);
            Ok(())
        }),
        Err(ConsumeError::RecoveryRequired)
    );
    assert_eq!(entered, ['A', 'B']);
    assert_eq!(frozen.completed(), 1);
    assert!(frozen.recovery_required());
}

#[test]
fn consumer_panic_never_authorizes_replay() {
    let mut arena = PendingChildren::default();
    let mut frozen = arena
        .run(1, 1, vec![child('A')], |_, _| Decision::Accept, |_| true)
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = frozen.consume_next::<()>(|_| panic!("entered insertion"));
        }))
        .is_err()
    );
    assert_eq!(frozen.completed(), 0);
    assert!(frozen.recovery_required());
    assert_eq!(
        frozen.consume_next::<()>(|_| Ok(())),
        Err(ConsumeError::RecoveryRequired)
    );
}

#[test]
fn handled_join_veto_consumes_one_child_and_continues_in_order() {
    let mut arena = PendingChildren::default();
    let mut frozen = arena
        .run(
            1,
            64,
            vec![child('A'), child('B'), child('C')],
            |_, _| Decision::Accept,
            |_| true,
        )
        .unwrap();
    let mut joined = Vec::new();
    while frozen
        .consume_next::<()>(|object| {
            if object.name != 'B' {
                joined.push(object.name);
            }
            Ok(())
        })
        .unwrap()
    {}
    assert_eq!(joined, ['A', 'C']);
    assert_eq!(frozen.completed(), 3);
}
