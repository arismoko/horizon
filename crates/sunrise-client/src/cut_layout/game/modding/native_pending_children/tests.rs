use mod_api::{
    BabyEntitySpawnChild, Identifier, MobSplitChildren, PendingMobSpec, UnspawnedMob,
    UnspawnedMobError,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_gameplay::game::{
    ServerGame,
    native_entity_refs::tests::with_game,
    server_side::native_pending_children::{
        BabyChildSlot, ChildConstraint, PendingMob, SplitChildList,
    },
};
use sunrise_mod_runtime::attachments::Attachments;
use sunrise_sim::mobs::{Mob, finalize};

fn initial(game: &ServerGame<'_>, id: u32, breeding: bool) -> PendingMob {
    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
    let mut mob = Mob::new(id, kind, Vec3::new(id as f64, 64.0, 8.5), 0.0);
    if breeding {
        finalize::set_baby(&mut mob, true);
    }
    PendingMob {
        mob,
        uuid: Some([1, 2, 3, id as i32]),
        attachments: Attachments::default(),
        initialized: false,
        origin_reservation: None,
        dimension: game.dim,
        resources: game.res.clone(),
        attributes: game.motion.attributes.clone(),
    }
}

fn cow_spec(dim: &str, at: [f64; 3], yaw: f32, baby: bool) -> PendingMobSpec {
    PendingMobSpec::new(
        Identifier::parse("minecraft:cow").unwrap(),
        Identifier::parse(dim).unwrap(),
        at,
        yaw,
        baby,
    )
}

fn slime_spec(dim: &str, at: [f64; 3], yaw: f32, size: Option<u32>) -> PendingMobSpec {
    let mut spec = PendingMobSpec::new(
        Identifier::parse("minecraft:slime").unwrap(),
        Identifier::parse(dim).unwrap(),
        at,
        yaw,
        false,
    );
    spec.size = size;
    spec
}

#[derive(Debug)]
struct NeverClone(Arc<AtomicUsize>);
impl Drop for NeverClone {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl sunrise_sim::mobs::goals::Goal for NeverClone {
    fn flags(&self) -> u8 {
        0
    }
    fn can_use(&mut self, _: &mut Mob, _: &mut sunrise_sim::mobs::goals::Cx<'_, '_, '_>) -> bool {
        false
    }
    fn box_clone(&self) -> Box<dyn sunrise_sim::mobs::goals::Goal> {
        panic!("pending ownership must never clone opaque goals")
    }
}

#[test]
#[ignore = "daily"]
fn direct_edits_and_raii_drop_preserve_actual_mob_without_publishing() {
    with_game(|game| {
        let dim = game.dim.id();
        let before = (
            game.mobs.len(),
            game.common.persist.uuids.len(),
            game.entity_attachments.len(),
        );
        let kept_drops = Arc::new(AtomicUsize::new(0));
        let removed_drops = Arc::new(AtomicUsize::new(0));
        let mut a = initial(game, 71, false);
        a.mob.ai.goals.add(99, NeverClone(kept_drops.clone()));
        let mut b = initial(game, 72, false);
        b.mob.ai.goals.add(99, NeverClone(removed_drops.clone()));
        let mut pending = vec![a, b, initial(game, 73, false)];
        {
            let mut list_backing = SplitChildList {
                action: 0,
                children: &mut pending,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
            };
            let mut list = MobSplitChildren::new(&mut list_backing);
            assert_eq!(list.len(), 3);
            assert!(!list.is_empty());
            assert_eq!(
                list.get(0).unwrap().kind().unwrap().to_string(),
                "minecraft:cow"
            );
            {
                let mut child = list.get_mut(0).unwrap();
                child.set_yaw(90.0).unwrap();
                child.set_position([1.25, 65.0, -2.5]).unwrap();
                child.set_baby(true).unwrap();
                assert_eq!(child.add_tag("test:kept"), Ok(true));
                assert_eq!(child.position(), Ok([1.25, 65.0, -2.5]));
                assert_eq!(child.tags().unwrap(), ["test:kept"]);
            }
            let removed = list.remove(1).expect("child B exists");
            assert_eq!(removed.kind().unwrap().to_string(), "minecraft:cow");
            assert_eq!(removed_drops.load(Ordering::SeqCst), 0);
            drop(removed);
            assert_eq!(
                removed_drops.load(Ordering::SeqCst),
                1,
                "dropping a removed UnspawnedMob runs its RAII Drop immediately"
            );
            let mut d = list
                .construct(game, slime_spec(dim, [4.5, 64.0, 8.5], 45.0, Some(2)))
                .unwrap();
            d.set_size(4).unwrap();
            assert_eq!(d.add_tag("test:detached_edit"), Ok(true));
            assert_eq!(d.tags().unwrap(), ["test:detached_edit"]);
            list.push(d).unwrap();
            // A detached mob constructed and dropped without attaching leaves no world state.
            let _dropped = list
                .construct(game, cow_spec(dim, [2.0, 64.0, 2.0], 0.0, true))
                .unwrap();
            assert_eq!(
                (
                    game.mobs.len(),
                    game.common.persist.uuids.len(),
                    game.entity_attachments.len()
                ),
                before
            );
        }
        assert_eq!(pending.len(), 3);
        assert_eq!(pending[0].mob.id, 71);
        assert_eq!(pending[0].mob.yaw, 90.0);
        assert_eq!(pending[0].mob.position, Vec3::new(1.25, 65.0, -2.5));
        assert!(pending[0].mob.is_baby());
        assert_eq!(pending[0].mob.extra.spawn_tags, ["test:kept"]);
        assert!(
            pending[0]
                .mob
                .ai
                .goals
                .entries()
                .any(|(_, name, _)| name == "NeverClone")
        );
        assert_eq!(pending[1].mob.id, 73);
        assert_eq!(pending[2].mob.kind.id, "minecraft:slime");
        assert_eq!(pending[2].mob.extra.size, 4);
        assert_eq!(pending[2].mob.extra.spawn_tags, ["test:detached_edit"]);
        assert_eq!(kept_drops.load(Ordering::SeqCst), 0);
        drop(pending);
        assert_eq!(kept_drops.load(Ordering::SeqCst), 1);
    });
}

#[test]
#[ignore = "daily"]
fn split_child_list_grows_unbounded_past_sixty_four_and_supports_list_ops() {
    with_game(|game| {
        let dim = game.dim.id();
        let mut pending = vec![initial(game, 71, false)];
        let mut list_backing = SplitChildList {
            action: 0,
            children: &mut pending,
            resources: game.res.clone(),
            attributes: game.motion.attributes.clone(),
            dimension: game.dim,
        };
        let mut list = MobSplitChildren::new(&mut list_backing);
        // Push past the former 64-child cap (Audit [69]): 80 children succeed.
        while list.len() < 80 {
            let i = list.len();
            let child = list
                .construct(
                    game,
                    slime_spec(dim, [i as f64, 64.0, 0.0], i as f32, Some(2)),
                )
                .unwrap();
            list.push(child).unwrap();
        }
        assert_eq!(list.len(), 80);
        let front = list
            .construct(game, cow_spec(dim, [0.5, 65.0, 0.5], 15.0, false))
            .unwrap();
        list.insert(0, front).unwrap();
        assert_eq!(list.len(), 81);
        assert_eq!(list.get(0).unwrap().position(), Ok([0.5, 65.0, 0.5]));
        let replacement = list
            .construct(game, slime_spec(dim, [9.5, 66.0, 9.5], 25.0, Some(1)))
            .unwrap();
        let prev = list.replace(1, replacement).unwrap();
        assert_eq!(prev.kind().unwrap().to_string(), "minecraft:cow");
        assert_eq!(list.get(1).unwrap().position(), Ok([9.5, 66.0, 9.5]));
        assert!(list.get(81).is_none());
        assert!(list.get_mut(81).is_none());
        assert!(list.remove(81).is_none());
        list.clear();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    });
}

#[test]
#[ignore = "daily"]
fn invalid_spec_and_edits_refuse_without_mutating_slot_or_list() {
    with_game(|game| {
        let dim = game.dim.id();
        // Breeding slot atomic refusal.
        let mut slot = Some(initial(game, 71, true));
        {
            let mut slot_backing = BabyChildSlot {
                action: 0,
                slot: &mut slot,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
                constraint: ChildConstraint::BreedingAgeable,
            };
            let mut baby = BabyEntitySpawnChild::new(&mut slot_backing);
            let valid = cow_spec(dim, [8.5, 64.0, 8.5], 0.0, true);
            for invalid in [
                {
                    let mut s = valid.clone();
                    s.position[0] = f64::NAN;
                    s
                },
                {
                    let mut s = valid.clone();
                    s.yaw = f32::NAN;
                    s
                },
                {
                    let mut s = valid.clone();
                    s.kind = Identifier::parse("test:unknown").unwrap();
                    s
                },
                {
                    let mut s = valid.clone();
                    s.dimension = Identifier::parse("test:other").unwrap();
                    s
                },
                {
                    let mut s = valid.clone();
                    s.size = Some(2);
                    s
                },
                // NeoForge BabyEntitySpawnEvent.setChild takes AgeableMob, not baby monsters.
                PendingMobSpec::new(
                    Identifier::parse("minecraft:zombie").unwrap(),
                    Identifier::parse(dim).unwrap(),
                    [8.5, 64.0, 8.5],
                    0.0,
                    true,
                ),
                PendingMobSpec::new(
                    Identifier::parse("minecraft:piglin").unwrap(),
                    Identifier::parse(dim).unwrap(),
                    [8.5, 64.0, 8.5],
                    0.0,
                    true,
                ),
            ] {
                assert_eq!(
                    baby.construct(game, invalid).err(),
                    Some(UnspawnedMobError::Invalid)
                );
            }
            // A baby zombie constructed via a split list is also rejected by a breeding slot.
            let mut split_tmp = Vec::new();
            let baby_zombie = {
                let mut list_backing = SplitChildList {
                    action: 0,
                    children: &mut split_tmp,
                    resources: game.res.clone(),
                    attributes: game.motion.attributes.clone(),
                    dimension: game.dim,
                };
                MobSplitChildren::new(&mut list_backing)
                    .construct(
                        game,
                        PendingMobSpec::new(
                            Identifier::parse("minecraft:zombie").unwrap(),
                            Identifier::parse(dim).unwrap(),
                            [8.5, 64.0, 8.5],
                            0.0,
                            true,
                        ),
                    )
                    .unwrap()
            };
            assert_eq!(
                baby.replace(baby_zombie).err(),
                Some(UnspawnedMobError::Invalid)
            );
            {
                let mut child = baby.get_mut().unwrap();
                assert_eq!(
                    child.set_position([f64::NAN, 0.0, 0.0]),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(
                    child.set_yaw(f32::INFINITY),
                    Err(UnspawnedMobError::Invalid)
                );
                assert_eq!(child.set_size(2), Err(UnspawnedMobError::Invalid));
            }
            assert!(baby.is_present());
            assert_eq!(baby.get().unwrap().position(), Ok([71.0, 64.0, 8.5]));
        }

        // Fox breeding slot requires a fox offspring (Fox.FoxBreedGoal.breed casts to Fox).
        let mut fox_slot = None;
        {
            let mut slot_backing = BabyChildSlot {
                action: 0,
                slot: &mut fox_slot,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
                constraint: ChildConstraint::BreedingFox,
            };
            let mut fox_baby = BabyEntitySpawnChild::new(&mut slot_backing);
            assert_eq!(
                fox_baby
                    .construct(game, cow_spec(dim, [8.5, 64.0, 8.5], 0.0, true))
                    .err(),
                Some(UnspawnedMobError::Invalid)
            );
            let cow_baby = UnspawnedMob::new(Box::new(initial(game, 75, true)));
            assert_eq!(
                fox_baby.replace(cow_baby).err(),
                Some(UnspawnedMobError::Invalid)
            );
            let kit = fox_baby
                .construct(
                    game,
                    PendingMobSpec::new(
                        Identifier::parse("minecraft:fox").unwrap(),
                        Identifier::parse(dim).unwrap(),
                        [8.5, 64.0, 8.5],
                        0.0,
                        true,
                    ),
                )
                .unwrap();
            assert!(fox_baby.replace(kit).unwrap().is_none());
            assert_eq!(
                fox_baby.get().unwrap().kind().unwrap().to_string(),
                "minecraft:fox"
            );
        }

        // Split list atomic refusal on out-of-bounds insert/replace and invalid edits.
        let mut pending = vec![initial(game, 72, false)];
        {
            let mut list_backing = SplitChildList {
                action: 0,
                children: &mut pending,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
            };
            let mut list = MobSplitChildren::new(&mut list_backing);
            assert_eq!(
                list.construct(game, slime_spec(dim, [0.0, 64.0, 0.0], 0.0, None))
                    .err(),
                Some(UnspawnedMobError::Invalid)
            );
            let extra = list
                .construct(game, slime_spec(dim, [1.0, 64.0, 1.0], 0.0, Some(2)))
                .unwrap();
            assert_eq!(list.insert(2, extra), Err(UnspawnedMobError::Invalid));
            let extra2 = list
                .construct(game, slime_spec(dim, [1.0, 64.0, 1.0], 0.0, Some(2)))
                .unwrap();
            assert_eq!(
                list.replace(1, extra2).err(),
                Some(UnspawnedMobError::Invalid)
            );
            assert_eq!(
                list.get_mut(0).unwrap().set_position([f64::NAN, 0.0, 0.0]),
                Err(UnspawnedMobError::Invalid)
            );
            assert_eq!(list.len(), 1);
            assert_eq!(list.get(0).unwrap().position(), Ok([72.0, 64.0, 8.5]));
        }
    });
}

/// Stack-local split and breeding editors carry no `Game` borrow, so dispatch can lend
/// `&mut dyn ServerHost` alongside them.
#[test]
#[ignore = "daily"]
fn scoped_editors_coexist_with_mutable_server_host() {
    fn callback(
        host: &mut dyn mod_api::ServerHost,
        mut split: MobSplitChildren<'_>,
        mut baby: BabyEntitySpawnChild<'_>,
    ) {
        assert!(host.entity(0).is_none());
        let mut child = split.get_mut(0).unwrap();
        assert_eq!(child.kind().unwrap().to_string(), "minecraft:cow");
        child.set_yaw(90.0).unwrap();
        child.set_position([9.25, 65.0, 4.5]).unwrap();
        assert_eq!(child.position(), Ok([9.25, 65.0, 4.5]));
        let mut calf = baby.get_mut().unwrap();
        calf.set_yaw(45.0).unwrap();
        assert_eq!(calf.kind().unwrap().to_string(), "minecraft:cow");
    }
    with_game(|game| {
        let mut pending = vec![initial(game, 71, false)];
        let mut slot = Some(initial(game, 72, true));
        {
            let mut split_backing = SplitChildList {
                action: 0,
                children: &mut pending,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
            };
            let mut baby_backing = BabyChildSlot {
                action: 0,
                slot: &mut slot,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
                constraint: ChildConstraint::BreedingAgeable,
            };
            callback(
                game,
                MobSplitChildren::new(&mut split_backing),
                BabyEntitySpawnChild::new(&mut baby_backing),
            );
        }
        assert_eq!(pending[0].mob.yaw, 90.0);
        assert_eq!(pending[0].mob.position, Vec3::new(9.25, 65.0, 4.5));
        assert_eq!(slot.unwrap().mob.yaw, 45.0);
    });
}

#[test]
#[ignore = "daily"]
fn nested_pending_editors_use_independent_stack_frames() {
    with_game(|game| {
        let mut outer_children = vec![initial(game, 71, false)];
        {
            let mut outer_backing = SplitChildList {
                action: 0,
                children: &mut outer_children,
                resources: game.res.clone(),
                attributes: game.motion.attributes.clone(),
                dimension: game.dim,
            };
            let mut outer = MobSplitChildren::new(&mut outer_backing);
            outer.get_mut(0).unwrap().set_yaw(30.0).unwrap();

            // A nested dispatch creates its own stack-local slot without touching outer state.
            let mut inner_slot = Some(initial(game, 80, true));
            {
                let mut inner_backing = BabyChildSlot {
                    action: 0,
                    slot: &mut inner_slot,
                    resources: game.res.clone(),
                    attributes: game.motion.attributes.clone(),
                    dimension: game.dim,
                    constraint: ChildConstraint::BreedingAgeable,
                };
                let mut inner = BabyEntitySpawnChild::new(&mut inner_backing);
                inner.get_mut().unwrap().set_yaw(120.0).unwrap();
                assert_eq!(inner.get().unwrap().position(), Ok([80.0, 64.0, 8.5]));
            }
            assert_eq!(inner_slot.unwrap().mob.yaw, 120.0);

            // Outer frame remains intact and usable afterward.
            assert_eq!(outer.len(), 1);
            assert_eq!(outer.get(0).unwrap().position(), Ok([71.0, 64.0, 8.5]));
            outer
                .get_mut(0)
                .unwrap()
                .set_position([5.0, 65.0, 5.0])
                .unwrap();
        }
        assert_eq!(outer_children[0].mob.yaw, 30.0);
        assert_eq!(outer_children[0].mob.position, Vec3::new(5.0, 65.0, 5.0));
    });
}
