//! Transport integration: run the same collection through native calls and the generated
//! guest proxy/host service. Species/gameplay policy belongs to the real game adapter tests.
use super::*;
use horizon_mod_runtime::pending_children::{Action, Decision, PendingChildren};
use mod_api::{Identifier, PendingChildrenEdits, PendingMobSpec, UnspawnedMobError};
use std::cell::RefCell;
#[path = "../../../tests/support/pending_children.rs"]
mod fixture;
use fixture::Editor;

struct Transport;
impl Drop for Transport {
    fn drop(&mut self) {
        guest::set_transport(None);
    }
}

#[test]
fn pending_children_native_and_generated_wire_edit_the_same_owned_outputs() {
    for wire in [false, true] {
        let arena = RefCell::new(PendingChildren::default());
        let mut retained = None;
        let mut frozen = PendingChildren::run_scoped(
            &arena,
            17,
            64,
            Vec::new(),
            |action| {
                let backend = Editor {
                    arena: &arena,
                    action,
                };
                let children = backend.receiver();
                let mut cx = Box::new(boundary::Cx::default());
                let mut frame = boundary::Frame::default();
                frame.set_pending_children_edits(&backend);
                cx.frames.push(frame);
                let pointer = &mut *cx as *mut boundary::Cx;
                guest::set_transport(Some(Rc::new(move |id, bytes| {
                    let mut out = Writer::new();
                    // The frame and backend outlive this synchronous scope; no arena borrow is held.
                    unsafe { boundary::serve(id, pointer, bytes, &mut out) }.unwrap();
                    out.into_bytes()
                })));
                let _transport = Transport;
                let edits: &dyn PendingChildrenEdits = if wire { &guest::Guest } else { &backend };
                assert_eq!(edits.is_empty(children), Ok(true));
                for x in [1.0, 2.0, 3.0] {
                    let spec = PendingMobSpec::new(
                        Identifier::parse("minecraft:slime").unwrap(),
                        Identifier::parse("minecraft:overworld").unwrap(),
                        [x, 64.0, 8.5],
                        0.0,
                        false,
                    );
                    let owned = edits.construct(children, spec).unwrap();
                    edits.append(children, owned).unwrap();
                    assert_eq!(
                        edits.append(children, owned),
                        Err(UnspawnedMobError::Ownership)
                    );
                }
                let a = edits.child(children, 0).unwrap();
                let b = edits.child(children, 1).unwrap();
                edits.set_yaw(a, 90.0).unwrap();
                edits.set_position(a, [1.25, -63.0, 9.5]).unwrap();
                edits.set_baby(a, true).unwrap();
                edits.set_size(a, 2).unwrap();
                edits.remove(children, 1).unwrap();
                assert_eq!(edits.set_yaw(b, 180.0), Err(UnspawnedMobError::Expired));
                let d = edits
                    .construct(
                        children,
                        PendingMobSpec::new(
                            Identifier::parse("minecraft:pig").unwrap(),
                            Identifier::parse("minecraft:overworld").unwrap(),
                            [4.0, 64.0, 8.5],
                            45.0,
                            true,
                        ),
                    )
                    .unwrap();
                edits.append(children, d).unwrap();
                assert_eq!(edits.len(children), Ok(3));
                let e = edits
                    .construct(
                        children,
                        PendingMobSpec::new(
                            Identifier::parse("minecraft:cow").unwrap(),
                            Identifier::parse("minecraft:overworld").unwrap(),
                            [5.0, 64.0, 8.5],
                            270.0,
                            true,
                        ),
                    )
                    .unwrap();
                edits.replace(children, 2, e).unwrap();
                assert_eq!(edits.append(children, e), Err(UnspawnedMobError::Ownership));
                arena.borrow_mut().next_listener(action).unwrap();
                assert_eq!(edits.len(children), Err(UnspawnedMobError::Expired));
                assert_eq!(edits.set_yaw(a, 0.0), Err(UnspawnedMobError::Expired));
                let fresh = backend.receiver();
                assert_eq!(edits.len(fresh), Ok(3));
                retained = Some((fresh, edits.child(fresh, 0).unwrap()));
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
        let mut output = Vec::new();
        while frozen
            .consume_next::<()>(|m| {
                output.push((m.kind.to_string(), m.position, m.yaw, m.baby, m.size));
                Ok(())
            })
            .unwrap()
        {}
        assert_eq!(
            output,
            [
                (
                    "minecraft:slime".into(),
                    [1.25, -63.0, 9.5],
                    90.0,
                    true,
                    Some(2)
                ),
                ("minecraft:slime".into(), [3.0, 64.0, 8.5], 0.0, false, None),
                ("minecraft:cow".into(), [5.0, 64.0, 8.5], 270.0, true, None),
            ]
        );
        let (collection, mob) = retained.unwrap();
        let (_, action, _) = collection.parts();
        let backend = Editor {
            arena: &arena,
            action: Action::from_id(action),
        };
        assert_eq!(backend.len(collection), Err(UnspawnedMobError::Expired));
        assert_eq!(backend.set_yaw(mob, 0.0), Err(UnspawnedMobError::Expired));
    }
}
