#![cfg(test)]
use sunrise_core::nbt::Tag;
#[cfg(test)]
pub(crate) mod tests {

    use super::*;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;

    macro_rules! pending {
        ($list:expr) => {{
            let mut owner = $list.pop().unwrap();
            owner.id = 0;
            $list.push(owner);
        }};
    }

    use crate::game::native_entity_refs::tests::collect_entities_in;

    use mod_api::ServerHost;

    #[test]
    #[ignore = "daily"]
    fn saved_display_and_utility_entities_join_once_and_are_queryable() {
        use mod_api::{EntityJoinLevelEvent, Flow, Hooks};
        use std::sync::{Arc, Mutex};
        let joins = Arc::new(Mutex::new(Vec::new()));
        struct Observer(Arc<Mutex<Vec<(String, bool)>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                let entity = host.entity(event.subject.id()).expect("joining handle");
                assert_eq!(
                    entity.kind(host).and_then(|kind| kind.id(host)).as_deref(),
                    Some(event.kind)
                );
                assert_eq!(entity.position(host), Some(event.pos));
                let level = host.level(event.dimension).unwrap();
                if event.kind != "minecraft:eye_of_ender" {
                    assert!(
                        collect_entities_in(host, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0])
                            .unwrap()
                            .contains(&entity)
                    );
                }
                self.0
                    .lock()
                    .unwrap()
                    .push((event.kind.to_owned(), event.loaded_from_disk));
                Flow::Continue
            }
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "saved-display-joins",
            Box::new(Observer(joins.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                game.native_joins_skip();
                let kinds = [
                    "minecraft:block_display",
                    "minecraft:item_display",
                    "minecraft:text_display",
                    "minecraft:marker",
                    "minecraft:interaction",
                ];
                for kind in kinds {
                    game.summon_with_nbt(kind, [5.0, 64.0, 5.0], "{}").unwrap();
                }
                game.native_joins_flush(false, 0);
                assert_eq!(
                    *joins.lock().unwrap(),
                    kinds.map(|kind| (kind.to_owned(), false))
                );
                let level = game.level("minecraft:overworld").unwrap();
                let handles =
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]).unwrap();
                assert_eq!(handles.len(), 5);
                let mut chunks =
                    game.entity_chunks(&std::collections::HashSet::from([(0, 0)]), true);
                assert_eq!(chunks.len(), 1);
                let (_, saved) = chunks.pop().unwrap();
                assert_eq!(saved.len(), 5);
                game.native_leaves_flush();
                assert!(
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0])
                        .unwrap()
                        .is_empty()
                );
                game.world.queue_entity_nbt((0, 0), saved);
                game.spawn_loaded_entities();
                game.native_joins_flush(true, 0);
                game.native_leaves_flush();
                assert_eq!(
                    *joins.lock().unwrap(),
                    kinds
                        .into_iter()
                        .map(|k| (k.to_owned(), false))
                        .chain(kinds.into_iter().map(|k| (k.to_owned(), true)))
                        .collect::<Vec<_>>()
                );
                let reloaded =
                    collect_entities_in(game, level, [4.0, 63.0, 4.0], [6.0, 66.0, 6.0]).unwrap();
                assert_eq!(reloaded.len(), 5);
                assert!(handles.iter().all(|e| e.kind(game).is_none()));
                assert!(
                    reloaded
                        .iter()
                        .all(|e| e.position(game) == Some([5.0, 64.0, 5.0]))
                );
                assert_eq!(
                    game.persist.live.len(),
                    5,
                    "resident utilities must not spuriously leave"
                );
                // The join moment covers the fallback store, not a fixed list of kinds.
                game.summon_with_nbt("minecraft:eye_of_ender", [5.0, 64.0, 5.0], "{}")
                    .unwrap();
                game.native_joins_flush(false, 0);
                assert_eq!(joins.lock().unwrap().len(), 11);
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&("minecraft:eye_of_ender".to_owned(), false))
                );
                // A quarantined missing-mod record shares storage, not level membership.
                game.server.next_entity += 1;
                let mut record = Tag::compound([(
                    "Pos",
                    Tag::List(
                        [5.0_f64, 64.0, 5.0]
                            .map(|n| Tag::Double(n.to_bits()))
                            .to_vec(),
                    ),
                )]);
                let opaque = game
                    .other_from_nbt(&mut record, game.server.next_entity, "missing:object")
                    .unwrap();
                game.server.others.0.push(opaque);
                game.native_joins_flush(true, 0);
                assert_eq!(
                    joins.lock().unwrap().len(),
                    11,
                    "opaque saved data must not join"
                );
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn pending_ids_keep_counter_order_and_assign_zero_only_once() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for (kind, nbt) in [
                ("minecraft:tnt", "{fuse:80s}"),
                (
                    "minecraft:falling_block",
                    "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
                ),
                ("minecraft:arrow", "{}"),
                ("minecraft:trident", "{}"),
                ("minecraft:firework_rocket", "{LifeTime:20}"),
            ] {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
            }
            pending!(game.tnts);
            pending!(game.falling);
            pending!(game.creatures.projectiles);
            pending!(game.tridents);
            pending!(game.rockets);
            let base = game.server.next_entity;
            let joined = game.persist.joined_seq;
            let announced = game.persist.live.clone();
            game.assign_pending_join_ids();
            let ids = [
                game.tnts[0].id,
                game.falling[0].id,
                game.creatures.projectiles[0].id,
                game.tridents[0].id,
                game.rockets[0].id,
            ];
            assert_eq!(ids, [base + 1, base + 2, base + 3, base + 4, base + 5]);
            let mut identities =
                crate::game::server_side::entity_server::taken_pass_tests::SlotIdentities::default(
                );
            identities.observe(
                &mut game.common.falling,
                game.common.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            game.assign_pending_join_ids();
            identities.observe(
                &mut game.common.falling,
                game.common.native_lifecycle.epoch().unwrap(),
                |f| f.id,
            );
            assert_eq!(game.server.next_entity, base + 5);
            assert_eq!(game.persist.joined_seq, joined);
            assert_eq!(game.persist.live, announced);
            assert_eq!(
                [
                    game.tnts[0].id,
                    game.falling[0].id,
                    game.creatures.projectiles[0].id,
                    game.tridents[0].id,
                    game.rockets[0].id
                ],
                ids
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn native_roots_keep_the_loaded_id_and_do_not_allocate_twice() {
        use sunrise_level::chunk::Chunk;
        for (kind, nbt) in [
            ("minecraft:tnt", "{fuse:80s}"),
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:firework_rocket", "{LifeTime:20}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
                game.summon_with_nbt(kind, [2.0, 70.0, 1.0], nbt).unwrap();
                match kind {
                    "minecraft:tnt" => pending!(game.tnts),
                    "minecraft:falling_block" => pending!(game.falling),
                    _ => pending!(game.rockets),
                }
                let base = game.server.next_entity;
                let (Tag::Compound(tag), _) = sunrise_core::nbt::parse_snbt(nbt).unwrap() else {
                    unreachable!()
                };
                let level = game.level(game.dim.id()).unwrap();
                let entity = game
                    .level_summon(level, kind, [1.0, 70.0, 1.0], tag)
                    .unwrap()
                    .unwrap();
                let id = entity.id();
                assert_eq!(id, base + 1);
                assert_eq!(game.server.next_entity, base + 1);
                assert_eq!(game.entity(id), Some(entity));
                let waiting = match kind {
                    "minecraft:tnt" => game.tnts[0].id,
                    "minecraft:falling_block" => game.falling[0].id,
                    _ => game.rockets[0].id,
                };
                assert_eq!(waiting, 0);
                if kind == "minecraft:falling_block" {
                    let (Tag::Compound(unknown_state), _) = sunrise_core::nbt::parse_snbt(
                        "{BlockState:{Name:\"test:missing\"},Time:1}",
                    )
                    .unwrap() else {
                        unreachable!()
                    };
                    let fallback = game
                        .level_summon(level, kind, [1.0, 70.0, 1.0], unknown_state)
                        .unwrap()
                        .unwrap()
                        .id();
                    assert_eq!(fallback, base + 2);
                    assert_eq!(game.server.next_entity, base + 2);
                    assert_eq!(game.falling.len(), 3);
                    assert_eq!(game.falling[0].id, 0);
                    assert_eq!(game.falling[1].id, id);
                    assert_eq!(game.falling[2].id, fallback);
                    assert_eq!(
                        game.falling[2].state,
                        game.pack
                            .assets
                            .registry
                            .default_state("minecraft:sand")
                            .unwrap()
                    );
                }
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn portal_ids_follow_only_the_existing_moving_entity_filters() {
        super::super::native_entity_refs::tests::with_game(|game| {
            for _ in 0..3 {
                game.summon_with_nbt("minecraft:trident", [1.0, 70.0, 1.0], "{}")
                    .unwrap();
                game.summon_with_nbt(
                    "minecraft:firework_rocket",
                    [1.0, 70.0, 1.0],
                    "{LifeTime:20}",
                )
                .unwrap();
            }
            for t in &mut game.tridents {
                t.id = 0;
                t.in_ground = true;
                t.previous = t.position;
            }
            game.tridents[0].in_ground = false;
            game.tridents[2].previous.x -= 1.0;
            for r in &mut game.rockets {
                r.id = 0;
                r.attached = false;
                r.attached_to = None;
            }
            game.rockets[1].attached_to = Some(123);
            game.rockets[2].attached = true;
            let base = game.server.next_entity;
            game.find_ridden_vehicle_portals(&[999]);
            assert_eq!(game.server.next_entity, base);
            game.find_entity_portals();
            assert_eq!(
                game.tridents.iter().map(|t| t.id).collect::<Vec<_>>(),
                [base + 1, 0, base + 2]
            );
            assert_eq!(
                game.rockets.iter().map(|r| r.id).collect::<Vec<_>>(),
                [base + 3, 0, 0]
            );
            game.find_entity_portals();
            assert_eq!(game.server.next_entity, base + 3);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn pending_assignment_refuses_a_bound_zero_owner_before_consuming_an_id() {
        use std::sync::{Arc, Mutex};
        use sunrise_mod_runtime::attachments::{
            AttachmentType, Attachments, Holder, index::AttachmentIndex,
        };
        static VALUE: AttachmentType<i32> = AttachmentType {
            namespace: "test",
            name: "pending_id",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        for (kind, nbt) in [
            (
                "minecraft:falling_block",
                "{BlockState:{Name:\"minecraft:sand\"},Time:1}",
            ),
            ("minecraft:arrow", "{}"),
        ] {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.summon_with_nbt(kind, [1.0, 70.0, 1.0], nbt).unwrap();
                let mut values = Attachments::default();
                VALUE.set(&mut values, Holder::Entity(0), 7).unwrap();
                let index = Arc::new(Mutex::new(AttachmentIndex::default()));
                let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 0);
                macro_rules! bind_zero {
                    ($list:expr) => {{
                        let mut owner = $list.pop().unwrap();
                        owner.id = 0;
                        owner.attachment_lifetime.bind(&mut values, &index, holder);
                        $list.push(owner);
                    }};
                }
                if kind == "minecraft:arrow" {
                    bind_zero!(game.creatures.projectiles);
                } else {
                    bind_zero!(game.falling);
                }
                let before = game.server.next_entity;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.assign_pending_join_ids()
                }));
                assert!(result.is_err());
                assert_eq!(game.server.next_entity, before);
                assert!(game.falling.iter().all(|owner| owner.id == 0));
                assert!(game.creatures.projectiles.iter().all(|owner| owner.id == 0));
            });
        }
    }
    #[test]
    #[ignore = "daily"]
    fn joins_follow_insertion_order_and_canceled_entities_never_leave() {
        use mod_api::{self, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow, Hooks};
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::mobs::Mob;

        type Joins = Arc<Mutex<Vec<(u32, String, bool)>>>;
        struct Observer {
            joins: Joins,
            leaves: Arc<Mutex<Vec<u32>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Observer {
            fn entity_join_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                self.joins.lock().unwrap().push((
                    event.subject.id(),
                    event.kind.into(),
                    event.loaded_from_disk,
                ));
                if event.kind == "minecraft:sheep" {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
            fn entity_leave_level(
                &self,
                _: &mut dyn ServerHost,
                event: &mut EntityLeaveLevelEvent<'_>,
            ) {
                self.leaves.lock().unwrap().push(event.entity);
            }
        }
        let joins = Joins::default();
        let leaves = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "join-order-observer",
            Box::new(Observer {
                joins: joins.clone(),
                leaves: leaves.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_entity_refs::tests::with_game(|game| {
                game.native_joins_skip();
                let base = game.server.next_entity;
                let cow = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let sheep = game.res.mob_types().get("minecraft:sheep").unwrap().clone();
                let child = Mob::new(base + 1, cow.clone(), Vec3::ZERO, 0.0);
                game.server.next_entity += 2;
                game.mobs.push(Mob::new(base + 2, cow, Vec3::ZERO, 0.0));
                game.native_joins_flush(false, 0);
                game.mobs.push(child);
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(
                    *joins.lock().unwrap(),
                    vec![
                        (base + 2, "minecraft:cow".into(), false),
                        (base + 1, "minecraft:cow".into(), false)
                    ]
                );
                game.server.next_entity += 1;
                game.mobs.push(Mob::new(base + 3, sheep, Vec3::ZERO, 0.0));
                assert!(!game.native_joins_flush(true, base + 3));
                assert!(!game.mobs.iter().any(|mob| mob.id == base + 3));
                game.native_entity_left(base + 3);
                assert!(leaves.lock().unwrap().is_empty());
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 3, "minecraft:sheep".into(), true))
                );

                game.crystals
                    .push(sunrise_gameplay_common::game::dragon_common::Crystal::new(
                        base + 4,
                        Vec3::ZERO,
                        true,
                        0,
                    ));
                game.cushions.push(sunrise_sim::entities::Cushion {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 5,
                    position: Vec3::ZERO,
                    pos: [0; 3],
                    yaw: 0.0,
                    color: 0,
                    check_interval: 0,
                    custom_name: None,
                });
                game.paintings.push(sunrise_sim::entities::Painting {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 6,
                    pos: [0; 3],
                    facing: 0,
                    variant: "minecraft:kebab".into(),
                    asset: None,
                    width: 1,
                    height: 1,
                    check_interval: 0,
                    ticks: 0,
                });
                game.item_frames.push(sunrise_sim::entities::ItemFrame {
                    attachment_lifetime: Default::default(),
                    join: Default::default(),
                    id: base + 7,
                    pos: [0; 3],
                    facing: 0,
                    glow: false,
                    item: None,
                    rotation: 0,
                    drop_chance: 1.0,
                    fixed: false,
                    invisible: false,
                    custom_name: None,
                    random: sunrise_core::random::LegacyRandom::new(0),
                    check_interval: 0,
                    ticks: 0,
                });
                let mut dragon =
                    sunrise_gameplay_common::game::dragon_common::Dragon::new(Vec3::ZERO, 0.0, 0);
                dragon.id = base + 8;
                dragon.join = sunrise_sim::join_seq::JoinSeq::next();
                game.dragon.replace(dragon);
                game.server.next_entity = base + 8;
                game.native_joins_flush(true, 0);
                game.native_joins_flush(true, 0);
                let seen = joins.lock().unwrap();
                assert_eq!(
                    &seen[3..],
                    &[
                        (base + 4, "minecraft:end_crystal".into(), true),
                        (base + 5, "minecraft:cushion".into(), true),
                        (base + 6, "minecraft:painting".into(), true),
                        (base + 7, "minecraft:item_frame".into(), true),
                        (base + 8, "minecraft:ender_dragon".into(), true)
                    ]
                );
                drop(seen);
                game.tnts
                    .push(sunrise_gameplay_common::game::tnt_common::PrimedTnt {
                        size_answer: None,
                        attachment_lifetime: Default::default(),
                        rotation: [0.0, 0.0],
                        join: Default::default(),
                        id: 0,
                        position: Vec3::ZERO,
                        previous: Vec3::ZERO,
                        velocity: Vec3::ZERO,
                        fuse: 80,
                        ticks: 0,
                        ground: Default::default(),
                        in_water: false,
                        stuck: Vec3::ZERO,
                        fall_distance: 0.0,
                        owner: None,
                        guest: None,
                        no_gravity: false,
                        power: 4.0,
                        used_portal: false,
                        block_state: None,
                    });
                game.native_joins_flush(false, 0);
                game.native_joins_flush(false, 0);
                assert_eq!(game.tnts.last().unwrap().id, base + 9);
                assert_eq!(
                    joins.lock().unwrap().last(),
                    Some(&(base + 9, "minecraft:tnt".into(), false))
                );
                assert_eq!(joins.lock().unwrap().len(), 9);
            });
        });
    }
}
