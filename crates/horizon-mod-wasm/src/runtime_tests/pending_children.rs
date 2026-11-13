//! Real wasm imports exercise the same arena backend as the generated native loopback.
//! These tests cover transport and trap cleanup, not Game's parent/RNG transaction policy.
use super::*;
use horizon_mod_runtime::pending_children::{Action, Decision, Error, PendingChildren};
use mod_api::{
    Identifier, PendingChildrenEdits, PendingMobSpec, UnspawnedMobError, UnspawnedMobRef,
    WorldEpoch,
};
use std::cell::RefCell;

#[path = "../../tests/support/pending_children.rs"]
mod fixture;
use fixture::Editor;

fn child() -> PendingMobSpec {
    PendingMobSpec::new(
        Identifier::parse("minecraft:cow").unwrap(),
        Identifier::parse("minecraft:overworld").unwrap(),
        [8.5, 64.0, 8.5],
        0.0,
        true,
    )
}

/// Call the generated set_yaw service from wasm, returning its actual encoded result.
/// The input is encoded by Wire; no field offsets or enum discriminants are assumed.
fn yaw_module(trap: bool) -> WasmMod {
    let id = crate::boundary::import_id("PendingChildrenEdits", "set_yaw").unwrap();
    module(&format!(
        "i32.const {id} local.get 1 local.get 2 i32.const 1024 i32.const 128 call $host {}",
        if trap {
            "drop unreachable"
        } else {
            // Pack the host's reply length with its destination pointer (1024 << 32).
            "i64.extend_i32_u i64.const 4398046511104 i64.or"
        }
    ))
}

fn yaw(
    module: &WasmMod,
    backend: &Editor<'_>,
    mob: UnspawnedMobRef,
    value: f32,
) -> Option<Result<(), UnspawnedMobError>> {
    let mut frame = Frame::default();
    frame.set_pending_children_edits(backend);
    module.call(
        CallClass::Hook,
        frame,
        0,
        None,
        |w| {
            w.put(&mob);
            w.put(&value);
        },
        |r| r.take(),
    )
}

#[test]
fn reentrancy_and_retained_wasm_tokens_cannot_retarget_children() {
    crate::test_stack::on_guest_stack(|| {
        let module = yaw_module(false);
        let arena = RefCell::new(PendingChildren::default());
        let mut retained = None;
        let mut frozen = PendingChildren::run_scoped(
            &arena,
            17,
            64,
            vec![child()],
            |action| {
                let backend = Editor {
                    arena: &arena,
                    action,
                };
                let receiver = backend.receiver();
                let a = backend.child(receiver, 0).unwrap();
                assert_eq!(yaw(&module, &backend, a, 90.0), Some(Ok(())));
                let (_, lease, slot) = a.pending_parts().unwrap();
                for forged in [
                    UnspawnedMobRef::pending(WorldEpoch::new(18), action.id(), lease, slot),
                    UnspawnedMobRef::pending(a.epoch(), action.id() + 1, lease, slot),
                    UnspawnedMobRef::pending(a.epoch(), action.id(), lease + 1, slot),
                    UnspawnedMobRef::pending(a.epoch(), action.id(), lease, slot + 1),
                ] {
                    assert_eq!(
                        yaw(&module, &backend, forged, 180.0),
                        Some(Err(UnspawnedMobError::Expired))
                    );
                }
                let inner = PendingChildren::run_scoped(
                    &arena,
                    17,
                    1,
                    vec![child()],
                    |nested| {
                        let inner = Editor {
                            arena: &arena,
                            action: nested,
                        };
                        assert_eq!(
                            yaw(&module, &inner, a, 180.0),
                            Some(Err(UnspawnedMobError::Expired))
                        );
                        Decision::Accept
                    },
                    |_| true,
                )
                .unwrap();
                drop(inner);
                assert_eq!(
                    yaw(&module, &backend, a, 180.0),
                    Some(Err(UnspawnedMobError::Expired))
                );
                let fresh = backend.child(backend.receiver(), 0).unwrap();
                assert_eq!(yaw(&module, &backend, fresh, 45.0), Some(Ok(())));
                retained = Some(fresh);
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
        let retained = retained.unwrap();
        let backend = Editor {
            arena: &arena,
            action: Action::from_id(retained.pending_parts().unwrap().0),
        };
        assert_eq!(
            yaw(&module, &backend, retained, 270.0),
            Some(Err(UnspawnedMobError::Expired))
        );
        assert!(
            frozen
                .consume_next::<()>(|child| {
                    assert_eq!(child.yaw, 45.0);
                    Ok(())
                })
                .unwrap()
        );
        assert!(
            !frozen
                .consume_next::<()>(|_| panic!("already consumed"))
                .unwrap()
        );
        assert!(
            !module.is_stopped(),
            "typed refusals do not poison the guest"
        );
    });
}

#[test]
fn trap_before_commit_withdraws_pending_ownership_after_a_real_wasm_edit() {
    crate::test_stack::on_guest_stack(|| {
        let module = yaw_module(true);
        let arena = RefCell::new(PendingChildren::default());
        let mut retained = None;
        let result = PendingChildren::run_scoped(
            &arena,
            17,
            1,
            vec![child()],
            |action| {
                let backend = Editor {
                    arena: &arena,
                    action,
                };
                let a = backend.child(backend.receiver(), 0).unwrap();
                retained = Some(a);
                assert_eq!(yaw(&module, &backend, a, 90.0), None);
                // The import did edit the real pending object before wasm trapped.
                let (action, lease, slot) = a.pending_parts().unwrap();
                let key = horizon_mod_runtime::pending_children::Borrow::from_parts(
                    a.epoch().get(),
                    action,
                    lease,
                    slot,
                );
                assert_eq!(arena.borrow().get(key).unwrap().yaw, 90.0);
                Decision::Failed
            },
            |_| panic!("failed dispatch must never reach final validation or a consumer"),
        );
        assert!(matches!(result, Err(Error::Failed)));
        assert!(module.is_stopped());
        let retained = retained.unwrap();
        let backend = Editor {
            arena: &arena,
            action: Action::from_id(retained.pending_parts().unwrap().0),
        };
        assert_eq!(
            backend.set_yaw(retained, 180.0),
            Err(UnspawnedMobError::Expired)
        );
        // Faulting one action leaves the shared arena usable by a later independent action.
        assert!(
            PendingChildren::run_scoped(
                &arena,
                17,
                1,
                vec![child()],
                |_| Decision::Accept,
                |_| true,
            )
            .is_ok()
        );
    });
}
