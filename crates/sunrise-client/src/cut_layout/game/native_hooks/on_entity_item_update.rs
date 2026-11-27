#![cfg(test)]

#[cfg(test)]
pub(crate) mod on_entity_item_update_tests {
    use mod_api::{Flow, Hooks, ItemPart, OnEntityItemUpdateEvent, Registrar, RegistrationContext};
    use std::sync::Mutex;
    use sunrise_gameplay::game::server_side::native_hooks_server::on_entity_item_update::ItemEntityTickAccess;
    use sunrise_gameplay_common::game::native_item_entity_refs::NativeItemEntityRefsGameExt1;
    use sunrise_level::chunk::Chunk;
    use {
        crate::game::native_hooks::*,
        sunrise_gameplay_common::game::native_hooks::{Arc, ItemStack, mod_api},
    };
    use {
        sunrise_gameplay::game::native_entity_refs::tests::with_game,
        sunrise_gameplay_common::game::api_values::STACK_PROJECTIONS,
        sunrise_gameplay_common::game::api_values::test_snapshot,
    };

    /// An item part that writes the entity's pickup delay to 7 on every update and
    /// records the entities it was told about; `cancels` ends the tick.
    #[derive(Clone)]
    pub(crate) struct Update {
        pub item: &'static str,
        pub(in crate::game) cancels: bool,
        pub(in crate::game) remove: bool,
        pub(in crate::game) mixed: bool,
        pub(in crate::game) heard: Arc<Mutex<Vec<u32>>>,
    }

    #[mod_api::sunrise_part]
    impl ItemPart for Update {
        fn on_entity_item_update(&self, event: &mut OnEntityItemUpdateEvent<'_>) -> Flow {
            let handle = event
                .subject
                .with_view(|view| view.handle())
                .expect("live resident item entity");
            ORDER.with(|order| {
                order.borrow_mut().push((handle.entity().id(), "update"));
            });
            self.heard.lock().unwrap().push(handle.entity().id());
            if self.remove {
                assert!(
                    handle
                        .entity()
                        .remove(event.subject.host(), mod_api::RemovalReason::Discarded)
                );
                assert!(event.subject.with_view(|_| ()).is_none());
                return Flow::Continue;
            }
            assert_eq!(
                event.subject.with_view(|mut view| view.set_pickup_delay(7)),
                Some(true)
            );
            if self.cancels && (!self.mixed || handle.entity().id().is_multiple_of(2)) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    pub(crate) struct Mod(pub(crate) Vec<Update>);

    thread_local! { pub(in crate::game) static ORDER: std::cell::RefCell<Vec<(u32, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) }; }

    /// Each entity a tick hook was told, in order, with the hook's phase.
    pub(crate) fn told(entities: &[u32], phase: &'static str) {
        ORDER.with(|order| {
            let mut order = order.borrow_mut();
            order.extend(entities.iter().map(|&entity| (entity, phase)));
        });
    }

    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn entity_tick_pre(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> mod_api::Flow {
            told(std::slice::from_ref(&event.entity), "pre");
            mod_api::Flow::Continue
        }
        fn entity_tick_post(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
            told(std::slice::from_ref(&event.entity), "post");
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for part in &self.0 {
                let key = r.item_key(part.item);
                r.item_part(key, part.clone());
            }
        }
    }

    /// A serving level with a stone floor at y 63 and nothing dropped.
    pub(crate) fn floor(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle
            .begin_open(Some(mod_api::Authority::IntegratedServer));
        game.world = sunrise_level::world::World::new(game.res.facts().air, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.dropped.clear();
    }

    #[allow(unused_imports)]
pub(crate) use sunrise_gameplay::game::server_side::native_hooks_server::on_entity_item_update::on_entity_item_update_tests::rest;

    pub(crate) fn counts(game: &sunrise_gameplay::game::ServerGame<'_>, ids: &[u32]) -> Vec<u32> {
        game.dropped
            .iter()
            .filter(|item| ids.contains(&item.id))
            .map(|item| item.stack.count)
            .collect()
    }

    /// An item the part of its own item cancels does not age, merge or lose pickup
    /// delay for the tick, and keeps the part's write; an item whose part does not
    /// cancel ticks as vanilla does after the write; an item without a part ticks
    /// and merges as before and is never told.
    #[test]
    #[ignore = "daily"]
    fn a_canceling_part_holds_its_own_item_and_nothing_else() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::on_entity_item_update::on_entity_item_update_tests::a_canceling_part_holds_its_own_item_and_nothing_else",
            a_canceling_part_holds_its_own_item_and_nothing_else_isolated,
        );
    }

    pub(crate) fn a_canceling_part_holds_its_own_item_and_nothing_else_isolated() {
        let iron = Update {
            item: "minecraft:iron_ingot",
            cancels: true,
            remove: false,
            mixed: false,
            heard: Default::default(),
        };
        let gold = Update {
            item: "minecraft:gold_ingot",
            cancels: false,
            remove: false,
            mixed: false,
            heard: Default::default(),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "item_update",
            Box::new(Mod(vec![
                iron.clone(),
                gold.clone(),
                Update {
                    item: "minecraft:emerald",
                    cancels: false,
                    remove: true,
                    mixed: false,
                    heard: Default::default(),
                },
                Update {
                    item: "minecraft:copper_ingot",
                    cancels: true,
                    remove: false,
                    mixed: true,
                    heard: Default::default(),
                },
            ])),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                floor(game);
                // Two of each of iron and diamond side by side, a gold one apart.
                let irons = [
                    rest(game, "minecraft:iron_ingot", 2.5, 2.5),
                    rest(game, "minecraft:iron_ingot", 2.7, 2.5),
                ];
                let diamonds = [
                    rest(game, "minecraft:diamond", 10.5, 2.5),
                    rest(game, "minecraft:diamond", 10.7, 2.5),
                ];
                let gold_id = rest(game, "minecraft:gold_ingot", 6.5, 10.5);

                ORDER.with(|order| order.borrow_mut().clear());
                STACK_PROJECTIONS.set(0);
                game.tick_items();
                assert_eq!(
                    STACK_PROJECTIONS.get(),
                    0,
                    "routing does not project stacks"
                );
                for id in irons.into_iter().chain([gold_id]) {
                    assert_eq!(
                        ORDER.with(|order| order
                            .borrow()
                            .iter()
                            .filter(|(entity, _)| *entity == id)
                            .map(|(_, phase)| *phase)
                            .collect::<Vec<_>>()),
                        ["pre", "update", "post"]
                    );
                }
                for id in irons {
                    let held = game.dropped.iter().find(|item| item.id == id).unwrap();
                    // Nothing of the entity's own tick ran: not its age, nor its delay's
                    // countdown; only the part's write is there.
                    assert_eq!((held.age, held.ticks, held.pickup_delay), (0, 0, 7));
                }
                for id in diamonds {
                    let ticked = game.dropped.iter().find(|item| item.id == id).unwrap();
                    assert_eq!((ticked.age, ticked.pickup_delay), (1, 19));
                }
                let gold_item = game.dropped.iter().find(|item| item.id == gold_id).unwrap();
                // The part wrote 7, and then the item's own tick counted down.
                assert_eq!((gold_item.age, gold_item.pickup_delay), (1, 6));

                // A resting item merges within 40 ticks (`ItemEntity.tick`): the part-less
                // pair does, the pair held by its part never does.
                for _ in 0..40 {
                    game.tick_items();
                }
                assert_eq!(counts(game, &irons), [1, 1]);
                assert_eq!(counts(game, &diamonds), [2]);
                assert!(
                    game.dropped
                        .iter()
                        .filter(|item| irons.contains(&item.id))
                        .all(|item| item.age == 0)
                );

                // Told once per tick of each item of its own: 41 ticks, two iron items; 41, one gold item.
                let mut heard = iron.heard.lock().unwrap().clone();
                heard.sort_unstable();
                let mut expected: Vec<u32> = irons.iter().flat_map(|id| [*id; 41]).collect();
                expected.sort_unstable();
                assert_eq!(heard, expected);
                assert_eq!(*gold.heard.lock().unwrap(), vec![gold_id; 41]);
                let removed = rest(game, "minecraft:emerald", 12.5, 12.5);
                game.tick_items();
                assert_eq!(
                    ORDER.with(|order| order
                        .borrow()
                        .iter()
                        .filter(|(id, _)| *id == removed)
                        .map(|(_, phase)| *phase)
                        .collect::<Vec<_>>()),
                    ["pre", "update", "post"]
                );
                assert!(game.entity(removed).is_none());
                game.native_removals_flush();
                assert!(!game.dropped.iter().any(|item| item.id == removed));
                let pair = [
                    rest(game, "minecraft:copper_ingot", 8.5, 8.5),
                    rest(game, "minecraft:copper_ingot", 8.7, 8.5),
                ];
                for _ in 0..80 {
                    game.tick_items();
                }
                assert_eq!(counts(game, &pair), [2]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn dropped_subject_and_host_writes_commit_or_abort_together() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::on_entity_item_update::on_entity_item_update_tests::dropped_subject_and_host_writes_commit_or_abort_together",
            || {
                with_game(|game| {
                    floor(game);
                    let item = rest(game, "minecraft:iron_ingot", 2.5, 2.5);
                    let peer = rest(game, "minecraft:stick", 6.5, 2.5);
                    let subject = game.item_entity(game.entity(item).unwrap()).unwrap();
                    let peer = game.item_entity(game.entity(peer).unwrap()).unwrap();
                    let earlier = test_snapshot(&ItemStack::new("minecraft:iron_ingot", 2));
                    let later = test_snapshot(&ItemStack::new("minecraft:iron_ingot", 3));
                    let peer_stack = test_snapshot(&ItemStack::new("minecraft:stick", 4));
                    for commit in [false, true] {
                        let mut access = ItemEntityTickAccess {
                            game: &mut *game,
                            subject,
                            index: 0,
                        };
                        let mut scope = mod_api::ItemEntityServerScope::new(
                            &mut access,
                            "minecraft:iron_ingot",
                        );
                        let result = scope.transaction(|scope, _| {
                            assert_eq!(
                                scope.with_view(|mut view| view.set_stack(Some(earlier.clone()))),
                                Some(Ok(true))
                            );
                            scope
                                .transaction(|scope, _| {
                                    scope
                                        .with_view(|mut view| {
                                            assert_eq!(
                                                view.set_stack(Some(later.clone())),
                                                Ok(true)
                                            );
                                            assert!(view.set_pickup_delay(7));
                                            assert!(view.set_target(Some(8)));
                                            assert!(view.set_velocity([9.0; 3]));
                                            STACK_PROJECTIONS.set(0);
                                            assert_eq!(
                                                view.with_stack(|stack| stack.count()),
                                                Some(3)
                                            );
                                            assert_eq!(STACK_PROJECTIONS.get(), 0);
                                            assert_eq!(
                                                view.with_stack(|stack| stack.project()).flatten(),
                                                Some(later.clone())
                                            );
                                            assert_eq!(STACK_PROJECTIONS.get(), 1);
                                        })
                                        .unwrap();
                                    assert_eq!(
                                        peer.set_stack(scope.host(), Some(peer_stack.clone())),
                                        Ok(true)
                                    );
                                    Ok::<_, ()>(())
                                })
                                .unwrap();
                            commit.then_some(()).ok_or(())
                        });
                        assert_eq!(
                            result,
                            commit
                                .then_some(())
                                .ok_or(mod_api::TransactionError::Aborted(()))
                        );
                        let item = &game.dropped[0];
                        assert_eq!(
                            (
                                item.stack.count,
                                item.pickup_delay,
                                item.target,
                                item.velocity.components(),
                                game.dropped[1].stack.count
                            ),
                            if commit {
                                (3, 7, Some([0, 0, 0, 8]), [9.0; 3], 4)
                            } else {
                                (1, 20, None, [0.0; 3], 1)
                            }
                        );
                    }
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn dropped_scope_reborrows_its_item_after_slots_shift() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::native_hooks::on_entity_item_update::on_entity_item_update_tests::dropped_scope_reborrows_its_item_after_slots_shift",
            || {
                with_game(|game| {
                    floor(game);
                    let ahead = rest(game, "minecraft:stick", 1.5, 2.5);
                    let subject = rest(game, "minecraft:iron_ingot", 2.5, 2.5);
                    let behind = rest(game, "minecraft:gold_ingot", 3.5, 2.5);
                    let ahead = game.entity(ahead).unwrap();
                    let subject = game.item_entity(game.entity(subject).unwrap()).unwrap();
                    let mut access = ItemEntityTickAccess {
                        game,
                        subject,
                        index: 1,
                    };
                    let mut scope =
                        mod_api::ItemEntityServerScope::new(&mut access, "minecraft:iron_ingot");
                    assert_eq!(scope.with_view(|view| view.handle()), Some(subject));

                    // Sweep the preceding row discarded through the Host, shifting both live items.
                    assert!(ahead.discard(scope.host()));
                    access.game.native_removals_flush();
                    let mut scope =
                        mod_api::ItemEntityServerScope::new(&mut access, "minecraft:iron_ingot");
                    assert_eq!(
                        scope.with_view(|mut view| (view.handle(), view.set_pickup_delay(7))),
                        Some((subject, true))
                    );
                    assert!(subject.entity().discard(scope.host()));
                    assert!(scope.with_view(|_| ()).is_none());
                    assert_eq!(access.game.dropped[0].pickup_delay, 7);
                    assert_eq!(
                        (
                            access.game.dropped[1].id,
                            access.game.dropped[1].pickup_delay
                        ),
                        (behind, 20)
                    );
                })
            },
        );
    }

    /// With no part attached to any item, every item ticks and merges as it did before the hook.
    #[test]
    #[ignore = "daily"]
    fn without_parts_items_tick_and_merge() {
        with_game(|game| {
            floor(game);
            let ids = [
                rest(game, "minecraft:iron_ingot", 2.5, 2.5),
                rest(game, "minecraft:iron_ingot", 2.7, 2.5),
            ];
            STACK_PROJECTIONS.set(0);
            game.tick_items();
            assert_eq!(STACK_PROJECTIONS.get(), 0);
            assert!(
                game.dropped
                    .iter()
                    .all(|item| (item.age, item.pickup_delay) == (1, 19))
            );
            for _ in 0..40 {
                game.tick_items();
            }
            assert_eq!(counts(game, &ids), [2]);
        });
    }
}
