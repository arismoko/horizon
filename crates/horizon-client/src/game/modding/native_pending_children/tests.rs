use super::*;
use crate::game::{native_entity_refs::tests::with_game, native_unspawned_mob::Edits};
use horizon_mod_runtime::pending_children::Decision;
use mod_api::UnspawnedMobEdits;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn initial(game: &Game<'_>, id: u32) -> PendingMob {
    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
    PendingMob {
        mob: Mob::new(id, kind, Vec3::new(id as f64, 64.0, 8.5), 0.0),
        uuid: Some([1, 2, 3, id as i32]),
        attachments: Attachments::default(),
        dimension: game.dim,
    }
}

fn editor(
    game: &Game<'_>,
    arena: &Rc<RefCell<PendingChildren<PendingMob>>>,
    action: Action,
    breeding: bool,
) -> Children {
    assert!(Rc::ptr_eq(arena, &game.lent_mobs.borrow().pending));
    Children::new(game, action, breeding)
}

#[derive(Debug)]
struct NeverClone(Arc<AtomicUsize>);
impl Drop for NeverClone {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl horizon_client::mobs::goals::Goal for NeverClone {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(
        &mut self,
        _: &mut Mob,
        _: &mut horizon_client::mobs::goals::Cx<'_, '_, '_>,
    ) -> bool {
        false
    }
    fn box_clone(&self) -> Box<dyn horizon_client::mobs::goals::Goal> {
        panic!("pending ownership must never clone opaque goals")
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn edit_remove_append_uses_the_actual_mob_and_expires_metadata_borrows() {
    with_game(|game| {
        let arena = game.lent_mobs.borrow().pending.clone();
        let before = (
            game.mobs.len(),
            game.persist.uuids.len(),
            game.entity_attachments.len(),
        );
        let drops = Arc::new(AtomicUsize::new(0));
        let mut a = initial(game, 71);
        a.mob.ai.goals.add(99, NeverClone(drops.clone()));
        let mut retained = None;
        let mut frozen = PendingChildren::run_scoped(
            &arena,
            game.lend_epoch().get(),
            64,
            vec![a, initial(game, 72), initial(game, 73)],
            |action| {
                let edits = editor(game, &arena, action, false);
                let children = edits.receiver().unwrap();
                let a = edits.child(children, 0).unwrap();
                let b = edits.child(children, 1).unwrap();
                assert_eq!(
                    game.unspawned_mob_kind(a).unwrap().to_string(),
                    "minecraft:cow"
                );
                edits.set_yaw(a, 90.0).unwrap();
                edits.set_position(a, [1.25, 65.0, -2.5]).unwrap();
                Edits { game }.add_tag(a, "test:kept").unwrap();
                edits.remove(children, 1).unwrap();
                assert_eq!(game.unspawned_mob_kind(b), Err(UnspawnedMobError::Expired));
                assert_eq!(
                    game.unspawned_mob_position(b),
                    Err(UnspawnedMobError::Expired)
                );
                let d = edits
                    .construct(
                        children,
                        PendingMobSpec::new(
                            mod_api::Identifier::parse("minecraft:pig").unwrap(),
                            mod_api::Identifier::parse(game.dim.id()).unwrap(),
                            [4.5, 64.0, 8.5],
                            45.0,
                            true,
                        ),
                    )
                    .unwrap();
                edits.append(children, d).unwrap();
                assert_eq!(edits.append(children, d), Err(UnspawnedMobError::Ownership));
                // A nested dispatch expires even kind/position borrows of the outer action.
                let mut inner = PendingChildren::run_scoped(
                    &arena,
                    game.lend_epoch().get(),
                    1,
                    vec![initial(game, 80)],
                    |_| {
                        assert_eq!(game.unspawned_mob_kind(a), Err(UnspawnedMobError::Expired));
                        assert_eq!(
                            game.unspawned_mob_position(a),
                            Err(UnspawnedMobError::Expired)
                        );
                        Decision::Accept
                    },
                    |_| true,
                )
                .unwrap();
                assert!(inner.consume_next::<()>(|_| Ok(())).unwrap());
                assert_eq!(edits.len(children), Err(UnspawnedMobError::Expired));
                let fresh = edits.receiver().unwrap();
                let a = edits.child(fresh, 0).unwrap();
                assert_eq!(game.unspawned_mob_position(a), Ok([1.25, 65.0, -2.5]));
                assert_eq!(game.unspawned_mob_tags(a).unwrap(), ["test:kept"]);
                retained = Some(a);
                assert!(
                    game.lent_mobs.borrow().is_empty(),
                    "no second lend or stand-in Mob"
                );
                assert_eq!(
                    (
                        game.mobs.len(),
                        game.persist.uuids.len(),
                        game.entity_attachments.len()
                    ),
                    before
                );
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
        let mut output = Vec::new();
        while frozen
            .consume_next::<()>(|child| {
                if child.mob.id == 71 {
                    assert!(
                        child
                            .mob
                            .ai
                            .goals
                            .entries()
                            .any(|(_, name, _)| name == "NeverClone")
                    );
                    assert_eq!(child.uuid, Some([1, 2, 3, 71]));
                    assert!(!child.attachments.has_index_entries());
                }
                output.push((child.mob.id, child.mob.kind.id.clone(), child.mob.yaw));
                Ok(())
            })
            .unwrap()
        {}
        assert_eq!(
            output,
            [
                (71, "minecraft:cow".into(), 90.0),
                (73, "minecraft:cow".into(), 0.0),
                (0, "minecraft:pig".into(), 45.0)
            ]
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            game.unspawned_mob_kind(retained.unwrap()),
            Err(UnspawnedMobError::Expired)
        );
        assert_eq!(
            game.unspawned_mob_position(retained.unwrap()),
            Err(UnspawnedMobError::Expired)
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn invalid_input_and_second_baby_refuse_before_changing_the_collection() {
    with_game(|game| {
        let arena = game.lent_mobs.borrow().pending.clone();
        let mut frozen = PendingChildren::run_scoped(
            &arena,
            game.lend_epoch().get(),
            1,
            vec![initial(game, 71)],
            |action| {
                let edits = editor(game, &arena, action, true);
                let children = edits.receiver().unwrap();
                let a = edits.child(children, 0).unwrap();
                let spec = PendingMobSpec::new(
                    mod_api::Identifier::parse("minecraft:pig").unwrap(),
                    mod_api::Identifier::parse(game.dim.id()).unwrap(),
                    [8.5, 64.0, 8.5],
                    0.0,
                    true,
                );
                for invalid in [
                    {
                        let mut s = spec.clone();
                        s.position[0] = f64::NAN;
                        s
                    },
                    {
                        let mut s = spec.clone();
                        s.yaw = f32::NAN;
                        s
                    },
                    {
                        let mut s = spec.clone();
                        s.kind = mod_api::Identifier::parse("test:unknown").unwrap();
                        s
                    },
                    {
                        let mut s = spec.clone();
                        s.dimension = mod_api::Identifier::parse("test:other").unwrap();
                        s
                    },
                    {
                        let mut s = spec.clone();
                        s.baby = false;
                        s
                    },
                    {
                        let mut s = spec.clone();
                        s.size = Some(2);
                        s
                    },
                ] {
                    assert_eq!(
                        edits.construct(children, invalid),
                        Err(UnspawnedMobError::Invalid)
                    );
                }
                assert_eq!(
                    edits.set_position(a, [f64::NAN, 0.0, 0.0]),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(
                    edits.set_yaw(a, f32::INFINITY),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(edits.set_baby(a, false), Err(UnspawnedMobError::Invalid));
                assert_eq!(edits.set_size(a, 2), Err(UnspawnedMobError::Invalid));
                let replacement = edits.construct(children, spec).unwrap();
                assert_eq!(
                    edits.append(children, replacement),
                    Err(UnspawnedMobError::Capacity)
                );
                assert_eq!(edits.len(children), Ok(1));
                assert_eq!(game.unspawned_mob_position(a), Ok([71.0, 64.0, 8.5]));
                // A rejected append leaves ownership available for a replacement.
                edits.replace(children, 0, replacement).unwrap();
                assert_eq!(game.unspawned_mob_kind(a), Err(UnspawnedMobError::Expired));
                assert_eq!(
                    edits.replace(children, 0, replacement),
                    Err(UnspawnedMobError::Ownership)
                );
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
        assert!(
            frozen
                .consume_next::<()>(|child| {
                    assert_eq!(child.mob.kind.id, "minecraft:pig");
                    assert!(child.mob.is_baby());
                    assert_eq!(child.mob.extra.age, -24000);
                    Ok(())
                })
                .unwrap()
        );
        assert!(
            !frozen
                .consume_next::<()>(|_| panic!("one baby only"))
                .unwrap()
        );
    });
}

/// The collection editor carries no Game borrow: dispatch can lend the mutable server Host
/// and the editor together. Its read-only resource snapshot is still valid afterward.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn collection_editor_can_share_a_callback_with_mutable_server_host() {
    fn callback(
        host: &mut dyn mod_api::ServerHost,
        edits: &dyn PendingChildrenEdits,
        children: PendingChildrenRef,
    ) {
        // The game's Host resolves the same arena object as its independent editor.
        let a = edits.child(children, 0).unwrap();
        assert_eq!(a.kind(host).unwrap().to_string(), "minecraft:cow");
        edits.set_yaw(a, 90.0).unwrap();
        edits.set_position(a, [9.25, 65.0, 4.5]).unwrap();
        assert_eq!(a.position(host), Ok([9.25, 65.0, 4.5]));
    }
    with_game(|game| {
        let arena = game.lent_mobs.borrow().pending.clone();
        let mut frozen = PendingChildren::run_scoped(
            &arena,
            game.lend_epoch().get(),
            64,
            vec![initial(game, 71)],
            |action| {
                let edits = Children::new(game, action, false);
                callback(game, &edits, edits.receiver().unwrap());
                assert!(
                    edits.valid_child(
                        arena
                            .borrow()
                            .get(arena.borrow().borrow(action, 0).unwrap())
                            .unwrap()
                    )
                );
                Decision::Accept
            },
            |_| true,
        )
        .unwrap();
        assert!(
            frozen
                .consume_next::<()>(|child| {
                    assert_eq!(child.mob.yaw, 90.0);
                    assert_eq!(child.mob.position, Vec3::new(9.25, 65.0, 4.5));
                    Ok(())
                })
                .unwrap()
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn pending_reentrancy_expires_standalone_kind_and_position_metadata() {
    with_game(|game| {
        let arena = game.lent_mobs.borrow().pending.clone();
        game.lend_unspawned("minecraft:cow", [8.5, 64.0, 8.5], None, |outer, _| {
            assert!(game.unspawned_mob_kind(outer).is_ok());
            let mut frozen = PendingChildren::run_scoped(
                &arena,
                game.lend_epoch().get(),
                64,
                vec![initial(game, 71)],
                |action| {
                    let edits = Children::new(game, action, false);
                    let children = edits.receiver().unwrap();
                    assert_eq!(edits.len(children), Ok(1));
                    assert_eq!(
                        game.unspawned_mob_kind(outer),
                        Err(UnspawnedMobError::Expired)
                    );
                    assert_eq!(
                        game.unspawned_mob_position(outer),
                        Err(UnspawnedMobError::Expired)
                    );
                    Decision::Accept
                },
                |_| true,
            )
            .unwrap();
            assert!(frozen.consume_next::<()>(|_| Ok(())).unwrap());
            assert_eq!(
                game.unspawned_mob_kind(outer),
                Err(UnspawnedMobError::Expired)
            );
            assert_eq!(
                game.unspawned_mob_position(outer),
                Err(UnspawnedMobError::Expired)
            );
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn final_invalid_child_suppresses_the_entire_frozen_output() {
    with_game(|game| {
        let arena = game.lent_mobs.borrow().pending.clone();
        let resident = game.mobs.len();
        // Model invalidity discovered after an accepted operation. No child may enter an
        // insertion consumer when a final object violates the action policy.
        for corrupt in 0..4 {
            let validator = RefCell::new(None::<Children>);
            let mut a = initial(game, 71);
            finalize::set_baby(&mut a.mob, true);
            let result = PendingChildren::run_scoped(
                &arena,
                game.lend_epoch().get(),
                1,
                vec![a],
                |action| {
                    let edits = Children::new(game, action, true);
                    let key = arena.borrow().borrow(action, 0).unwrap();
                    assert!(edits.valid_child(arena.borrow().get(key).unwrap()));
                    let mut borrowed = arena.borrow_mut();
                    let child = borrowed.get_mut(key).unwrap();
                    match corrupt {
                        0 => child.mob.position.x = f64::NAN,
                        1 => child.mob.head_yaw = f32::INFINITY,
                        2 => child.mob.extra.age = 0,
                        3 => child.dimension = super::super::Dim::Nether,
                        _ => unreachable!(),
                    }
                    drop(borrowed);
                    *validator.borrow_mut() = Some(edits);
                    Decision::Accept
                },
                |child| validator.borrow().as_ref().unwrap().valid_child(child),
            );
            assert!(matches!(result, Err(Error::Invalid)));
            assert_eq!(game.mobs.len(), resident);
        }
    });
}
