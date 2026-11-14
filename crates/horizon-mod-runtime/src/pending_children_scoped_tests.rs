use super::*;
use std::cell::Cell;
use std::rc::Rc;

// Represents opaque mod goal state; ownership must work without Clone.
struct Child(char, Rc<Cell<usize>>);
impl Drop for Child {
    fn drop(&mut self) {
        self.1.set(self.1.get() + 1);
    }
}

fn names(mut frozen: FrozenChildren<Child>) -> Vec<char> {
    let mut result = Vec::new();
    while frozen
        .consume_next::<()>(|child| {
            result.push(child.0);
            Ok(())
        })
        .unwrap()
    {}
    result
}

#[test]
fn shared_host_edits_and_nested_dispatch_use_one_arena() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let mut retained = None;
    let frozen = PendingChildren::run_scoped(
        &arena,
        7,
        64,
        vec![Child('A', drops.clone()), Child('B', drops.clone())],
        |outer| {
            let old = arena.borrow().borrow(outer, 0).unwrap();
            arena.borrow_mut().get_mut(old).unwrap().0 = 'C';
            let detached = arena
                .borrow_mut()
                .construct(outer, Child('D', drops.clone()))
                .unwrap();
            let nested = PendingChildren::run_scoped(
                &arena,
                7,
                1,
                vec![Child('E', drops.clone())],
                |inner| {
                    assert!(matches!(arena.borrow().get(old), Err(Error::Expired)));
                    assert_eq!(
                        arena.borrow_mut().append(inner, detached),
                        Err(Error::Expired)
                    );
                    Decision::Accept
                },
                |_| true,
            )
            .unwrap();
            assert_eq!(names(nested), ['E']);
            assert_eq!(
                drops.get(),
                2,
                "detached outer child and consumed inner child dropped"
            );
            assert!(matches!(arena.borrow().get(old), Err(Error::Expired)));
            let fresh = arena.borrow().borrow(outer, 0).unwrap();
            assert_eq!(arena.borrow().get(fresh).unwrap().0, 'C');
            retained = Some(fresh);
            Decision::Accept
        },
        |_| true,
    )
    .unwrap();
    assert!(matches!(
        arena.borrow().get(retained.unwrap()),
        Err(Error::Expired)
    ));
    assert_eq!(names(frozen), ['C', 'B']);
    assert_eq!(drops.get(), 4);
    assert!(arena.borrow().frames.is_empty());
}

#[test]
fn shared_native_panic_discards_children_and_preserves_completed_output() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let unrelated = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('U', drops.clone())],
        |_| Decision::Accept,
        |_| true,
    )
    .unwrap();
    let mut retained = None;
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = PendingChildren::run_scoped(
            &arena,
            7,
            1,
            vec![Child('A', drops.clone())],
            |action| {
                retained = Some(arena.borrow().borrow(action, 0).unwrap());
                arena
                    .borrow_mut()
                    .construct(action, Child('B', drops.clone()))
                    .unwrap();
                panic!("native listener");
            },
            |_| true,
        );
    }));
    assert!(failed.is_err());
    assert_eq!(drops.get(), 2);
    assert!(matches!(
        arena.borrow().get(retained.unwrap()),
        Err(Error::Expired)
    ));
    assert!(arena.borrow().frames.is_empty());
    assert_eq!(names(unrelated), ['U']);
    assert_eq!(drops.get(), 3);
}

#[test]
fn shared_trap_verdict_discards_children_without_replaying_dispatch() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let mut calls = 0;
    let result = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('A', drops.clone())],
        |_| {
            calls += 1;
            Decision::Failed
        },
        |_| panic!("failed action must not validate"),
    );
    assert!(matches!(result, Err(Error::Failed)));
    assert_eq!(calls, 1);
    assert_eq!(drops.get(), 1);
    assert!(arena.borrow().frames.is_empty());
}

#[test]
fn shared_validation_panic_withdraws_exactly_once() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = PendingChildren::run_scoped(
            &arena,
            7,
            1,
            vec![Child('A', drops.clone())],
            |_| Decision::Accept,
            |_| panic!("final invariant validation"),
        );
    }));
    assert!(failure.is_err());
    assert_eq!(drops.get(), 1);
    assert!(arena.borrow().frames.is_empty());
    let next = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('B', drops.clone())],
        |_| Decision::Accept,
        |_| true,
    )
    .unwrap();
    assert_eq!(names(next), ['B']);
    assert_eq!(drops.get(), 2);
}

#[test]
fn shared_nested_panic_allows_outer_editor_reacquisition() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let frozen = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('A', drops.clone())],
        |outer| {
            let old = arena.borrow().borrow(outer, 0).unwrap();
            let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = PendingChildren::run_scoped(
                    &arena,
                    7,
                    1,
                    vec![Child('B', drops.clone())],
                    |_| panic!("nested listener"),
                    |_| true,
                );
            }));
            assert!(failure.is_err());
            assert_eq!(drops.get(), 1);
            assert!(matches!(arena.borrow().get(old), Err(Error::Expired)));
            let fresh = arena.borrow().borrow(outer, 0).unwrap();
            arena.borrow_mut().get_mut(fresh).unwrap().0 = 'C';
            Decision::Accept
        },
        |_| true,
    )
    .unwrap();
    assert_eq!(names(frozen), ['C']);
    assert_eq!(drops.get(), 2);
}

#[test]
fn shared_cancel_and_stale_verdicts_never_validate_or_consume() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let frozen = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('A', drops.clone())],
        |action| {
            arena.borrow_mut().cancel(action).unwrap();
            arena.borrow_mut().next_listener(action).unwrap();
            Decision::Accept
        },
        |_| panic!("cancelled action must not validate"),
    )
    .unwrap();
    assert!(names(frozen).is_empty());
    let stale = PendingChildren::run_scoped(
        &arena,
        7,
        1,
        vec![Child('B', drops.clone())],
        |_| Decision::Stale,
        |_| panic!("stale action must not validate"),
    );
    assert!(matches!(stale, Err(Error::Stale)));
    assert_eq!(drops.get(), 2);
    assert!(arena.borrow().frames.is_empty());
}

#[test]
fn a_moved_listener_count_ends_the_previous_lease_and_an_unmoved_one_keeps_it() {
    let arena = RefCell::new(PendingChildren::default());
    let drops = Rc::new(Cell::new(0));
    let frozen = PendingChildren::run_scoped(
        &arena,
        7,
        64,
        vec![Child('A', drops.clone())],
        |action| {
            arena.borrow_mut().sync_listener(10).unwrap();
            // The first listener's lease and borrow.
            let (_, first) = arena.borrow().lease(action).unwrap();
            let kept = arena.borrow().borrow(action, 0).unwrap();
            let token = arena
                .borrow_mut()
                .construct(action, Child('T', drops.clone()))
                .unwrap();
            let token = arena.borrow().borrow_owned(token).unwrap();
            // The same count: nothing ends.
            arena.borrow_mut().sync_listener(10).unwrap();
            assert!(arena.borrow().get(kept).is_ok());
            // A listener entered: its predecessor's lease, borrows and detached children end.
            arena.borrow_mut().sync_listener(11).unwrap();
            let (_, second) = arena.borrow().lease(action).unwrap();
            assert_ne!(first, second);
            assert_eq!(arena.borrow().get(kept).err(), Some(Error::Expired));
            assert_eq!(arena.borrow().get(token).err(), Some(Error::Expired));
            // The attached child survives into the new lease.
            assert!(arena.borrow().borrow(action, 0).is_ok());
            Decision::Accept
        },
        |_| true,
    )
    .unwrap();
    assert_eq!(names(frozen), ['A']);
    assert_eq!(
        drops.get(),
        2,
        "the detached token, then the consumed child"
    );
}
