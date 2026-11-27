#![cfg(test)]
#[cfg(test)]
pub(crate) mod sweep_tests {
    use sunrise_gameplay::game::native_hooks::entity_inside::sweep_tests::{
        item_at_blocks, sweep_game_player_live,
    };
    use sunrise_gameplay_common::game::entity_common::EntityCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
    use sunrise_gameplay_common::game::native_hooks::BLOCK_CONTACT_HOOKS;
    use sunrise_gameplay_common::game::native_hooks::entity_inside::sweep_tests::{
        InsidePart, arrow_shot, install_inside_mod,
    };

    fn fresh(name: &str) -> bool {
        let module = module_path!().split_once("::").unwrap().1;
        sunrise_gameplay_common::game::test_fixtures::run_fresh_cases(
            &format!("{module}::{name}"),
            "SUNRISE_INSIDE_SWEEP_CHILD",
            &["sweep"],
        )
    }

    #[test]
    #[ignore = "daily"]
    fn item_tick_sweeps_once_and_keeps_the_single_hit() {
        if fresh("item_tick_sweeps_once_and_keeps_the_single_hit") {
            return;
        }
        use sunrise_core::geometry::Vec3;
        let part = InsidePart {
            hurt_after: Some((0, 1.0)),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-item-tick", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let mut item = single_stone_item(game);
                let grass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:short_grass")
                    .unwrap();
                game.world.set(1, 100, 0, grass).unwrap();
                item.position = Vec3::new(1.5, 100.25, 0.5);
                let outcome = sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.tick_item_with_block_contacts(
                            &mut item,
                            None,
                            &[],
                            false,
                            None,
                            &mut |_| {},
                        )
                    },
                );
                assert_eq!(outcome, (true, false));
                assert_eq!(*part.contacts.lock().unwrap(), [[1, 100, 0]]);
                assert_eq!(item.health, 4);
                assert_eq!(
                    item.velocity.x.to_bits(),
                    (0.5 * f64::from(0.98_f32)).to_bits()
                );
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn item_tick_places_vanilla_fire_inside_the_ignition_slot() {
        if fresh("item_tick_places_vanilla_fire_inside_the_ignition_slot") {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;

        #[derive(Clone, Default)]
        struct FireProbe(Arc<Mutex<Vec<i32>>>);
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for FireProbe {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let seen = self.0.clone();
                event.run_before(mod_api::InsideEffect::FireIgnite, move |view| {
                    seen.lock().unwrap().push(view.fire_ticks());
                });
                let seen = self.0.clone();
                event.run_after(mod_api::InsideEffect::FireIgnite, move |view| {
                    seen.lock().unwrap().push(view.fire_ticks());
                });
            }
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for FireProbe {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let fire = r.block_key("minecraft:fire");
                r.block_part(fire, self.clone());
            }
        }
        let part = FireProbe::default();
        let entry = || mod_api::ModEntry::new("inside-item-fire", Box::new(part.clone()));
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        mod_platform::testing::with_mods(vec![entry()].into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let mut item = single_stone_item(game);
                let fire = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:fire")
                    .unwrap();
                game.world.set(1, 100, 0, fire).unwrap();
                item.position = Vec3::new(1.5, 100.25, 0.5);
                let outcome = sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.tick_item_with_block_contacts(
                            &mut item,
                            None,
                            &[],
                            false,
                            None,
                            &mut |_| {},
                        )
                    },
                );
                assert_eq!(outcome, (true, false));
                assert_eq!(*part.0.lock().unwrap(), [0, 160]);
                assert_eq!((item.fire_ticks, item.health), (160, 4));
            });
        });
    }

    /// A killing write stops the item's flush: the walk still reaches both blocks,
    /// the writes after the destruction never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_items_flush() {
        if fresh("a_killing_write_stops_the_items_flush") {
            return;
        }
        let part = InsidePart {
            hurt_after: Some((0, 0.5)),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-item-kill", &part);
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let mut item = item_at_blocks(game);
                use sunrise_items::items::ItemStack;
                let diamonds = ItemStack::new("minecraft:diamond", 3);
                item.stack = ItemStack::new("minecraft:shulker_box", 1);
                item.stack.data_mut().container = vec![(0, diamonds.clone())];
                item.health = 1;
                let before = item.velocity.x;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut item, None);
                    },
                );
                assert_eq!(part.contacts.lock().unwrap().len(), 2);
                assert_eq!(item.health, 0, "subtract 0.5 before truncating health");
                assert_eq!(game.dropped.len(), 1);
                assert_eq!(game.dropped[0].stack, diamonds);
                assert_eq!(game.dropped[0].position, item.position);
                assert_eq!(
                    item.velocity.x.to_bits(),
                    before.to_bits(),
                    "the writes after the destruction never apply"
                );

                // Run the real batch on a merge tick. The victim touches a
                // non-solid block, its neighbour is close enough to merge
                // but doesn't touch that block.
                let grass = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:short_grass")
                    .unwrap();
                game.world.set(1, 104, 0, grass).unwrap();
                game.server.tickets.0.update(None);
                game.server.ticked_entities_through = u32::MAX;
                for (health, count) in [(5, 3), (1, 2)] {
                    part.contacts.lock().unwrap().clear();
                    game.dropped.clear();
                    for (x, count, ticks, health) in [(1.25, 1, 39, health), (0.75, 2, 0, 5)] {
                        game.spawn_item(
                            ItemStack::new("minecraft:stone", count),
                            [x, 104.5, 0.5],
                            [0.0; 3],
                            20,
                        );
                        let item = game.dropped.last_mut().unwrap();
                        item.no_gravity = true;
                        item.health = health;
                        item.ticks = ticks;
                    }
                    let neighbour = game.dropped[1].id;
                    sunrise_sim::block_contact::in_level(
                        sunrise_sim::parallel::TickThread::mint(),
                        Some("minecraft:overworld"),
                        || game.tick_items(),
                    );
                    assert_eq!(*part.contacts.lock().unwrap(), [[1, 104, 0]]);
                    assert_eq!(game.dropped.len(), 1);
                    assert_eq!(game.dropped[0].id, neighbour);
                    assert_eq!(game.dropped[0].stack.count, count);
                }
            });
        });
    }

    /// Set up a single stone block at `(1, 100, 0)` and return an item sweeping strictly
    /// inside that cell.
    fn single_stone_item(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
    ) -> sunrise_sim::entities::ItemEntity {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        game.world.set(1, 100, 0, stone).unwrap();
        let mut item = sunrise_sim::entities::ItemEntity::new(
            7,
            sunrise_items::items::ItemStack::new("minecraft:stone", 1),
            sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
            sunrise_core::geometry::Vec3::ZERO,
            0.0,
        );
        item.previous = sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5);
        item
    }

    /// Callback level writes apply directly against the live level even after the touched
    /// block's state has changed; an invalid `BlockState` returns `false` every time without
    /// latching or rolling back sibling writes, and `destroy_block(..., true)` drops resources.
    #[test]
    #[ignore = "daily"]
    fn callback_level_writes_apply_after_block_state_changes_and_do_not_rollback_siblings() {
        if fresh(
            "callback_level_writes_apply_after_block_state_changes_and_do_not_rollback_siblings",
        ) {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;

        #[derive(Clone, Copy)]
        struct WriteStates {
            dirt: mod_api::BlockState,
            cobble: mod_api::BlockState,
            planks: mod_api::BlockState,
            invalid: mod_api::BlockState,
        }
        #[derive(Clone, Default)]
        struct WriteResults {
            dirt_set: Option<bool>,
            read_after_dirt: Option<Option<mod_api::BlockState>>,
            invalid_1: Option<bool>,
            read_after_invalid_1: Option<Option<mod_api::BlockState>>,
            cobble_set: Option<bool>,
            neighbor_destroyed: Option<bool>,
            invalid_2: Option<bool>,
            planks_set: Option<bool>,
        }
        #[derive(Clone, Default)]
        struct LevelWritePart {
            states: Arc<Mutex<Option<WriteStates>>>,
            results: Arc<Mutex<WriteResults>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for LevelWritePart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let WriteStates {
                    dirt,
                    cobble,
                    planks,
                    invalid,
                } = self.states.lock().unwrap().unwrap();
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let neighbor = [event.pos.x + 1, event.pos.y, event.pos.z];
                let second = [event.pos.x + 2, event.pos.y, event.pos.z];

                let before_results = self.results.clone();
                event.run_before(mod_api::InsideEffect::Freeze, move |view| {
                    let ok = view.set_block(pos, dirt);
                    let seen = view.block_state(pos);
                    let mut out = before_results.lock().unwrap();
                    out.dirt_set = Some(ok);
                    out.read_after_dirt = Some(seen);
                });

                let after_results = self.results.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    let invalid_1 = view.set_block(pos, invalid);
                    let after_invalid_1 = view.block_state(pos);
                    let cobble_set = view.set_block(pos, cobble);
                    let neighbor_destroyed = view.destroy_block(neighbor, true);
                    let invalid_2 = view.set_block(second, invalid);
                    let planks_set = view.set_block(second, planks);
                    let mut out = after_results.lock().unwrap();
                    out.invalid_1 = Some(invalid_1);
                    out.read_after_invalid_1 = Some(after_invalid_1);
                    out.cobble_set = Some(cobble_set);
                    out.neighbor_destroyed = Some(neighbor_destroyed);
                    out.invalid_2 = Some(invalid_2);
                    out.planks_set = Some(planks_set);
                });
            }
        }
        struct LevelWriteMod(LevelWritePart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for LevelWriteMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }
        }

        let part = LevelWritePart::default();
        let entry =
            || mod_api::ModEntry::new("inside-level-writes", Box::new(LevelWriteMod(part.clone())));
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        let mods: mod_api::Mods = vec![entry()].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let (dirt, cobblestone, oak_planks, air) = (
                    state("minecraft:dirt"),
                    state("minecraft:cobblestone"),
                    state("minecraft:oak_planks"),
                    state("minecraft:air"),
                );
                let mut item = single_stone_item(game);
                game.world.set(2, 100, 0, dirt).unwrap();

                let dirt_api = dirt.try_into_api().unwrap();
                let cobble_api = cobblestone.try_into_api().unwrap();
                let planks_api = oak_planks.try_into_api().unwrap();
                let invalid_api =
                    mod_api::__private::block_state_from_ordinal(dirt_api.block(), u16::MAX);
                *part.states.lock().unwrap() = Some(WriteStates {
                    dirt: dirt_api,
                    cobble: cobble_api,
                    planks: planks_api,
                    invalid: invalid_api,
                });

                let drops_before = game.dropped.len();
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut item, None);
                    },
                );

                let out = part.results.lock().unwrap().clone();
                assert_eq!(out.dirt_set, Some(true));
                assert_eq!(out.read_after_dirt, Some(Some(dirt_api)));
                assert_eq!(out.invalid_1, Some(false));
                assert_eq!(
                    out.read_after_invalid_1,
                    Some(Some(dirt_api)),
                    "invalid set_block must not roll back prior dirt write"
                );
                assert_eq!(
                    out.cobble_set,
                    Some(true),
                    "set_block succeeds even after touched block changed from stone to dirt"
                );
                assert_eq!(out.neighbor_destroyed, Some(true));
                assert_eq!(
                    out.invalid_2,
                    Some(false),
                    "second invalid set_block still returns false without latching"
                );
                assert_eq!(
                    out.planks_set,
                    Some(true),
                    "valid write right after second refusal still succeeds"
                );
                assert_eq!(game.world.block(1, 100, 0), cobblestone);
                assert_eq!(game.world.block(2, 100, 0), air);
                assert_eq!(game.world.block(3, 100, 0), oak_planks);
                assert_eq!(
                    game.dropped.len(),
                    drops_before + 1,
                    "destroy_block with drop = true drops neighbor resources"
                );
                let dropped = &game.dropped[drops_before];
                assert_eq!(dropped.stack.item.as_ref(), "minecraft:dirt");
                assert_eq!(dropped.position.x.floor() as i32, 2);
            });
        });
    }

    /// The item tick lifts its batch; a source in that batch is still live, but a
    /// retired handle refuses just that call without rolling back the other writes.
    #[test]
    #[ignore = "daily"]
    fn callback_hurt_resolves_a_lifted_sibling_but_not_a_retired_handle() {
        if fresh("callback_hurt_resolves_a_lifted_sibling_but_not_a_retired_handle") {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_items::items::ItemStack;

        #[derive(Clone, Default)]
        struct SourcePart {
            dirt: Arc<Mutex<Option<mod_api::BlockState>>>,
            sources: Arc<Mutex<Option<[mod_api::EntityRef; 3]>>>,
            outcomes: Arc<Mutex<Option<[bool; 3]>>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for SourcePart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let dirt = self.dirt.lock().unwrap().unwrap();
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let [direct, causing, stale] = self.sources.lock().unwrap().unwrap();
                let outcomes = self.outcomes.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    let retired_hit =
                        view.hurt_with_source("minecraft:generic", 2.0, Some(stale), None);
                    let live_hit = view.hurt_with_source(
                        "minecraft:generic",
                        1.0,
                        Some(direct),
                        Some(causing),
                    );
                    let block_ok = view.set_block(pos, dirt);
                    *outcomes.lock().unwrap() = Some([retired_hit, live_hit, block_ok]);
                });
            }
        }
        type Hits = Arc<Mutex<Vec<(i32, i32, Option<[f64; 3]>)>>>;
        struct SourceMod(SourcePart, Hits);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for SourceMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let grass = r.block_key("minecraft:short_grass");
                r.block_part(grass, self.0.clone());
            }

            fn entity_invulnerability_check(
                &self,
                _: mod_api::LevelSide<'_>,
                event: &mut mod_api::EntityInvulnerabilityCheckEvent<'_>,
            ) {
                if event.kind == "minecraft:item" && event.source.damage_type == "minecraft:generic"
                {
                    self.1.lock().unwrap().push((
                        event.source.direct,
                        event.source.causing,
                        event.source.position,
                    ));
                }
            }
        }

        let part = SourcePart::default();
        let hits = Hits::default();
        let entry = || {
            mod_api::ModEntry::new(
                "inside-batch-source",
                Box::new(SourceMod(part.clone(), hits.clone())),
            )
        };
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        mod_platform::testing::with_mods(vec![entry()].into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let _ = single_stone_item(game); // the contact test chunk
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let grass = state("minecraft:short_grass");
                let dirt = state("minecraft:dirt");
                game.world.set(1, 100, 0, grass).unwrap();
                *part.dirt.lock().unwrap() = Some(dirt.try_into_api().unwrap());
                for x in [1.5, 5.5, 8.5, 11.5] {
                    game.spawn_item(
                        ItemStack::new("minecraft:stone", 1),
                        [x, 100.25, 0.5],
                        [0.0; 3],
                        20,
                    );
                    game.dropped.last_mut().unwrap().no_gravity = true;
                }
                let ids = std::array::from_fn::<_, 3, _>(|i| game.dropped[i + 1].id);
                let [direct, causing, stale] = ids.map(|id| game.entity(id).unwrap());
                *part.sources.lock().unwrap() = Some([direct, causing, stale]);
                game.dropped.remove(3);
                game.server.tickets.0.update(None);
                game.server.ticked_entities_through = u32::MAX;

                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.tick_items(),
                );

                assert_eq!(*part.outcomes.lock().unwrap(), Some([false, true, true]));
                assert_eq!(
                    *hits.lock().unwrap(),
                    [(ids[0] as i32, ids[1] as i32, Some([5.5, 100.25, 0.5]))]
                );
                assert_eq!(game.dropped[0].health, 4);
                assert_eq!(game.world.block(1, 100, 0), dirt);
            });
        });
    }

    /// A panicking `run_after` callback disables only its owning mod for `entity_inside`,
    /// preserves its completed live write, leaves other mods running on the first and
    /// subsequent sweeps, and queues an error toast naming the panicking mod.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn panicking_mod_callback_disables_only_that_mod_and_queues_error_toast() {
        if fresh("panicking_mod_callback_disables_only_that_mod_and_queues_error_toast") {
            return;
        }
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };

        #[derive(Clone, Default)]
        struct PanickingPart {
            part_calls: Arc<AtomicUsize>,
            callback_runs: Arc<AtomicUsize>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for PanickingPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                self.part_calls.fetch_add(1, Ordering::SeqCst);
                let runs = self.callback_runs.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    view.push([0.5, 0.0, 0.0]);
                    panic!("mod_panics run_after boom");
                });
            }
        }
        struct PanickingMod(PanickingPart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PanickingMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }
        }

        #[derive(Clone, Default)]
        struct HealthyPart {
            part_calls: Arc<AtomicUsize>,
            callback_runs: Arc<AtomicUsize>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for HealthyPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                self.part_calls.fetch_add(1, Ordering::SeqCst);
                let runs = self.callback_runs.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    runs.fetch_add(1, Ordering::SeqCst);
                    view.push([0.25, 0.0, 0.0]);
                });
            }
        }
        struct HealthyMod(HealthyPart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for HealthyMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }
        }

        let panics_part = PanickingPart::default();
        let healthy_part = HealthyPart::default();
        let make_entries = || {
            vec![
                mod_api::ModEntry::new("mod_panics", Box::new(PanickingMod(panics_part.clone()))),
                mod_api::ModEntry::new("mod_healthy", Box::new(HealthyMod(healthy_part.clone()))),
            ]
        };
        sunrise_gameplay_common::game::api_values::install_parts(&make_entries()).unwrap();
        let mods: mod_api::Mods = make_entries().into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let mut item1 = single_stone_item(game);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut item1, None);
                    },
                );
                assert_eq!(panics_part.part_calls.load(Ordering::SeqCst), 1);
                assert_eq!(panics_part.callback_runs.load(Ordering::SeqCst), 1);
                assert_eq!(healthy_part.part_calls.load(Ordering::SeqCst), 1);
                assert_eq!(healthy_part.callback_runs.load(Ordering::SeqCst), 1);
                assert_eq!(item1.velocity.x, 0.75, "both live pushes applied");

                let mut item2 = single_stone_item(game);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || {
                        game.sweep_item_inside(&mut item2, None);
                    },
                );
                assert_eq!(
                    panics_part.part_calls.load(Ordering::SeqCst),
                    1,
                    "mod_panics must be skipped on the second sweep"
                );
                assert_eq!(panics_part.callback_runs.load(Ordering::SeqCst), 1);
                assert_eq!(
                    healthy_part.part_calls.load(Ordering::SeqCst),
                    2,
                    "mod_healthy still runs on the second sweep"
                );
                assert_eq!(healthy_part.callback_runs.load(Ordering::SeqCst), 2);
                assert_eq!(item2.velocity.x, 0.25, "only the healthy mod pushed");
            });

            // The client toast pass drains the shared queue even when F1 hides the HUD.
            crate::game::server_side::native_entity_refs::tests::with_client_game(|client| {
                let mut batch = sunrise_client::hud::HudBatch::scratch([640, 480]);
                client.client.chat_ui.player_hides_gui = true;
                client.draw_every_toast(&mut batch);
                assert!(mod_api::__private::take_hook_panics().is_empty());
                let [
                    sunrise_client_gameplay::game::toast_manager::VisibleToast {
                        toast:
                            sunrise_client_gameplay::game::toast_manager::Toast::Diagnostic {
                                toast,
                                ..
                            },
                        ..
                    },
                ] = client.client.toast_manager.visible.as_slice()
                else {
                    panic!("expected exactly one shared error toast");
                };
                assert_eq!(
                    toast.message.as_ref().unwrap().component(),
                    serde_json::json!({
                        "translate": "sunrise.mods.hook_panic.description",
                        "with": [{"text": "mod_panics"}, {"text": "entity_inside"}],
                    })
                );
            });
        });
    }

    /// Live player flush supports `set_block`/`destroy_block` with accurate `bool` returns
    /// (same-state `false`, already-air `false`), live `block_state`, Peaceful `add_effect`,
    /// Creative `set_remaining_fire_ticks` clamping, and fire-immune `is_on_fire` predicate.
    #[test]
    #[ignore = "daily"]
    fn player_live_inside_flush_level_writes_peaceful_effect_and_fire_semantics() {
        if fresh("player_live_inside_flush_level_writes_peaceful_effect_and_fire_semantics") {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;
        use sunrise_sim::lan::guest::GuestMove;

        #[derive(Clone, Default)]
        struct LivePlayerPart {
            dirt: Arc<Mutex<Option<mod_api::BlockState>>>,
            speed: Arc<Mutex<Option<mod_api::EffectKey>>>,
            results: Arc<Mutex<Option<(bool, bool, bool, bool, bool, bool, bool, i32)>>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for LivePlayerPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let dirt = self.dirt.lock().unwrap().unwrap();
                let speed = self.speed.lock().unwrap().unwrap();
                let pos = [event.pos.x, event.pos.y, event.pos.z];
                let results = self.results.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    let set_changed = view.set_block(pos, dirt);
                    let set_same = view.set_block(pos, dirt);
                    let saw_dirt = view.block_state(pos) == Some(dirt);
                    let destroy_first = view.destroy_block(pos, false);
                    let destroy_air = view.destroy_block(pos, false);
                    let expired_admitted = view.add_effect(speed, -2, 0);
                    let effect_ok = view.add_effect(speed, 60, 0) && expired_admitted;
                    view.set_remaining_fire_ticks(100);
                    let on_fire = view.is_on_fire();
                    let ticks = view.fire_ticks();
                    *results.lock().unwrap() = Some((
                        set_changed,
                        set_same,
                        saw_dirt,
                        destroy_first,
                        destroy_air,
                        effect_ok,
                        on_fire,
                        ticks,
                    ));
                });
            }
        }
        struct LivePlayerMod(LivePlayerPart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for LivePlayerMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                *self.0.speed.lock().unwrap() = Some(r.effect_key("minecraft:speed"));
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }
        }

        let part = LivePlayerPart::default();
        let entry =
            || mod_api::ModEntry::new("inside-live-player", Box::new(LivePlayerMod(part.clone())));
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        let mods: mod_api::Mods = vec![entry()].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.ui.difficulty = 0; // Peaceful
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Peaceful;
                open_to_lan(game);
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let dirt = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:dirt")
                    .unwrap();
                *part.dirt.lock().unwrap() = Some(dirt.try_into_api().unwrap());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );
                game.world.set(1, 100, 0, stone).unwrap();
                let mut player =
                    sunrise_sim::motion::PlayerMotion::new(game.motion, Vec3::new(1.5, 100.0, 0.5))
                        .unwrap();
                player.mod_fire_immune = true;
                let mut guest = guest(1);
                guest.loaded = true;
                guest.position = [1.5, 100.0, 0.5];
                guest.game_mode = 1;
                guest.abilities.invulnerable = true;
                guest.motion = Some(Box::new(player));
                guest.moves.push(GuestMove {
                    position: guest.position,
                    delta: [1.0, 0.0, 0.0],
                    on_ground: false,
                    sneaking: false,
                });
                game.server.lan.as_mut().unwrap().hub.guests.push(guest);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.guest_server_tick(1, true),
                );

                assert_eq!(
                    *part.results.lock().unwrap(),
                    Some((true, false, true, true, false, true, false, 1)),
                    "set/destroy returns, live read, Peaceful speed effect, fire-immune is_on_fire, and creative fire clamp (1)"
                );
                let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                assert_eq!(guest.effects.get("speed").unwrap().duration, 59);
            });
        });
    }

    /// The real NoAI tick lends both the mob and the level: destroying a waterlogged
    /// block exposes its retained fluid, and a hit can name the swept mob as its direct source.
    #[test]
    #[ignore = "daily"]
    fn mob_inside_tick_keeps_water_and_resolves_live_sources() {
        if fresh("mob_inside_tick_keeps_water_and_resolves_live_sources") {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_core::geometry::Vec3;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::{mobs::Mob, ticket_levels::Sources};

        #[derive(Clone)]
        struct Contact {
            source: Arc<Mutex<Option<(mod_api::EntityRef, mod_api::EntityRef)>>>,
            water: Arc<Mutex<Option<mod_api::BlockState>>>,
            result: Arc<Mutex<Option<[bool; 4]>>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for Contact {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let (subject, causing) = self.source.lock().unwrap().unwrap();
                if event.entity as u32 != subject.id() {
                    return;
                }
                let water = self.water.lock().unwrap().unwrap();
                let result = Arc::clone(&self.result);
                event.run_before(mod_api::InsideEffect::Freeze, move |view| {
                    let at = [2, 100, 0];
                    let destroyed = view.destroy_block(at, false);
                    let retained = view.block_state(at) == Some(water);
                    let unchanged = !view.set_block(at, water);
                    let hurt = view.hurt_with_source(
                        "minecraft:explosion",
                        2.0,
                        Some(subject),
                        Some(causing),
                    );
                    *result.lock().unwrap() = Some([destroyed, retained, unchanged, hurt]);
                });
            }
        }
        type SourcesSeen = Arc<Mutex<Vec<(i32, i32, String)>>>;
        struct Probe(Contact, SourcesSeen);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Probe {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }

            fn living_incoming_damage(
                &self,
                _: &mut dyn mod_api::HitHost,
                event: &mut mod_api::LivingIncomingDamageEvent<'_>,
            ) -> mod_api::Flow {
                self.1.lock().unwrap().push((
                    event.source.direct,
                    event.source.causing,
                    event.source.damage_type.to_owned(),
                ));
                mod_api::Flow::Continue
            }
        }

        let probe = Contact {
            source: Arc::default(),
            water: Arc::default(),
            result: Arc::default(),
        };
        let seen: SourcesSeen = Arc::default();
        let entry = || {
            mod_api::ModEntry::new(
                "inside-live-mob",
                Box::new(Probe(probe.clone(), Arc::clone(&seen))),
            )
        };
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        mod_platform::testing::with_mods(vec![entry()].into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.server.tickets.0.update(Some(Sources {
                    players: vec![(0, 0)],
                    forced: Vec::new(),
                    portal: Vec::new(),
                    spawn: Vec::new(),
                    dragon: None,
                    viewed: Vec::new(),
                    view_distance: 2,
                    simulation_distance: 2,
                }));
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                let chest = registry
                    .resolve(&sunrise_world::chunk::BlockState {
                        name: "minecraft:chest".into(),
                        properties: [
                            ("facing".into(), "north".into()),
                            ("type".into(), "single".into()),
                            ("waterlogged".into(), "true".into()),
                        ]
                        .into(),
                    })
                    .unwrap();
                let water = registry.default_state("minecraft:water").unwrap();
                game.world.set(1, 100, 0, stone).unwrap();
                game.world.set(2, 100, 0, chest).unwrap();
                *probe.water.lock().unwrap() = Some(water.try_into_api().unwrap());
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for (id, x) in [(200, 1.5), (201, 6.5)] {
                    let mut mob = Mob::new(id, pig.clone(), Vec3::new(x, 100.0, 0.5), 0.0);
                    mob.no_ai = true;
                    mob.no_gravity = true;
                    game.mobs.push(mob);
                }
                *probe.source.lock().unwrap() =
                    Some((game.entity(200).unwrap(), game.entity(201).unwrap()));
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.tick_creatures().unwrap(),
                );
                assert_eq!(*probe.result.lock().unwrap(), Some([true; 4]));
                assert_eq!(game.world.block(2, 100, 0), water);
                assert!(seen.lock().unwrap().iter().any(|(direct, causing, ty)| {
                    (*direct, *causing, ty.as_str()) == (200, 201, "minecraft:explosion")
                }));
            });
        });
    }

    /// A deferred hurt on the real mob tick must finish its Enderman teleport
    /// before the next callback reads the live entity.
    #[test]
    #[ignore = "daily"]
    fn mob_inside_hurt_teleports_before_the_next_callback() {
        if fresh("mob_inside_hurt_teleports_before_the_next_callback") {
            return;
        }
        use std::sync::{Arc, Mutex};
        use sunrise_core::{geometry::Vec3, random::LegacyRandom};
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::{mobs::Mob, ticket_levels::Sources};

        type TeleportObservation = Arc<Mutex<Option<(bool, [f64; 3])>>>;
        #[derive(Clone, Default)]
        struct TeleportPart(TeleportObservation);
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for TeleportPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let seen = self.0.clone();
                event.run_before(mod_api::InsideEffect::Freeze, move |view| {
                    let hurt = view.hurt("minecraft:generic", 2.0);
                    *seen.lock().unwrap() = Some((hurt, [0.0; 3]));
                });
                let seen = self.0.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    seen.lock().unwrap().as_mut().unwrap().1 = view.position();
                });
            }
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for TeleportPart {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let grass = r.block_key("minecraft:short_grass");
                r.block_part(grass, self.clone());
            }
        }
        let part = TeleportPart::default();
        let entry = || mod_api::ModEntry::new("inside-ender-hurt", Box::new(part.clone()));
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        mod_platform::testing::with_mods(vec![entry()].into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.health_rules.difficulty = sunrise_world::food::Difficulty::Normal;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.server.tickets.0.update(Some(Sources {
                    players: vec![(0, 0)],
                    forced: Vec::new(),
                    portal: Vec::new(),
                    spawn: Vec::new(),
                    dragon: None,
                    viewed: Vec::new(),
                    view_distance: 2,
                    simulation_distance: 2,
                }));
                let state = |name| game.pack.assets.registry.default_state(name).unwrap();
                let stone = state("minecraft:stone");
                let grass = state("minecraft:short_grass");
                for x in 0..16 {
                    for z in 0..16 {
                        game.world.set(x, 99, z, stone).unwrap();
                    }
                }
                game.world.set(8, 100, 8, grass).unwrap();
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:enderman")
                    .unwrap()
                    .clone();
                let mut mob = Mob::new(200, kind, Vec3::new(8.5, 100.0, 8.5), 0.0);
                mob.no_ai = true;
                mob.no_gravity = true;
                mob.persistent = true;
                mob.random = LegacyRandom::new(118);
                game.mobs.push(mob);
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.tick_creatures().unwrap(),
                );
                // Independent Java Random trace from seed 118: Mob.baseTick's ambient
                // roll, the two hurt pitch draws, then Enderman.hurtServer's roll
                // (2) and its x/y/z draws. The flat floor stops the downward search at y=100.
                let expected = [
                    0x4001_b983_f817_1d50,
                    0x4059_0000_0000_0000,
                    0x4027_5895_7e8d_db18,
                ];
                let (hurt, at_hurt_return) = part.0.lock().unwrap().unwrap();
                assert!(hurt);
                assert_eq!(at_hurt_return.map(f64::to_bits), expected);
                let mob = game.mobs.iter().find(|m| m.id == 200).unwrap();
                assert_eq!(mob.position.components().map(f64::to_bits), expected);
                assert_eq!(mob.random.state(), 0xf46e_d8cd_4912);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn burning_mob_sweeping_into_dry_powder_snow_melts_it_and_draws_the_hiss() {
        if fresh("burning_mob_sweeping_into_dry_powder_snow_melts_it_and_draws_the_hiss") {
            return;
        }
        use std::sync::Arc;
        use sunrise_core::{geometry::Vec3, random::LegacyRandom};
        use sunrise_level::chunk::Chunk;
        use sunrise_sim::{
            mobs::{Mob, goals::GoalSelector},
            ticket_levels::Sources,
        };

        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.server.tickets.0.update(Some(Sources {
                players: vec![(0, 0)],
                forced: Vec::new(),
                portal: Vec::new(),
                spawn: Vec::new(),
                dragon: None,
                viewed: Vec::new(),
                view_distance: 2,
                simulation_distance: 2,
            }));
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let snow = state("minecraft:powder_snow");
            let air = state("minecraft:air");
            game.world.set(8, 100, 8, snow).unwrap();
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut mob = Mob::new(200, kind, Vec3::new(7.5, 100.25, 8.5), 0.0);
            mob.ai.goals = GoalSelector::default();
            mob.ai.targets = GoalSelector::default();
            mob.no_gravity = true;
            mob.persistent = true;
            mob.velocity = Vec3::new(0.25, 0.0, 0.0);
            mob.fire_ticks = 21;
            mob.random = LegacyRandom::new(1);
            game.mobs.push(mob);
            sunrise_sim::block_contact::in_level(
                sunrise_sim::parallel::TickThread::mint(),
                Some("minecraft:overworld"),
                || game.tick_creatures().unwrap(),
            );
            let mob = game.mobs.iter().find(|m| m.id == 200).unwrap();
            assert_eq!(game.world.block(8, 100, 8), air);
            assert_eq!((mob.fire_ticks, mob.ticks_frozen, mob.health), (0, 1, 10.0));
            // Java Random(seed=1) after the ambient nextInt(1000) and the two
            // extinguish nextFloat() calls. No periodic fire damage at 21 ticks.
            assert_eq!(mob.random.state(), 0x68fb_0e6f_f399);
        });
    }

    /// Item float-health subtraction before truncation, bundle drop on fatal hurt, and
    /// projectile `mark_hurt` + `is_on_fire`.
    #[test]
    #[ignore = "daily"]
    fn item_float_hurt_bundle_spill_and_projectile_mark_hurt() {
        if fresh("item_float_hurt_bundle_spill_and_projectile_mark_hurt") {
            return;
        }
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct HurtEntityPart {
            fatal_item: Arc<Mutex<bool>>,
            proj_outcomes: Arc<Mutex<Option<(bool, bool)>>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for HurtEntityPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let fatal = *self.fatal_item.lock().unwrap();
                let proj_outcomes = self.proj_outcomes.clone();
                event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                    if view.kind() == "minecraft:item" {
                        if fatal {
                            view.hurt("minecraft:in_fire", 10.0);
                        } else {
                            view.hurt("minecraft:in_fire", 0.4);
                            view.hurt("minecraft:in_fire", 0.6);
                        }
                    } else {
                        let on_fire = view.is_on_fire();
                        let hurt = view.hurt("minecraft:generic", 1.0);
                        *proj_outcomes.lock().unwrap() = Some((on_fire, hurt));
                    }
                });
            }
        }
        struct HurtEntityMod(HurtEntityPart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for HurtEntityMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let stone = r.block_key("minecraft:stone");
                r.block_part(stone, self.0.clone());
            }
        }

        let part = HurtEntityPart::default();
        let entry =
            || mod_api::ModEntry::new("inside-item-proj", Box::new(HurtEntityMod(part.clone())));
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        let mods: mod_api::Mods = vec![entry()].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);

                // Fractional damage: 5.0 - 0.4 = 4.6 -> 4, then 4.0 - 0.6 = 3.4 -> 3.
                let mut item = single_stone_item(game);
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.sweep_item_inside(&mut item, None),
                );
                assert_eq!(item.health, 3, "float health subtraction before truncation");

                // Fatal damage on a bundle spawns its contained items.
                *part.fatal_item.lock().unwrap() = true;
                let mut bundle = sunrise_items::items::ItemStack::new("minecraft:bundle", 1);
                bundle
                    .data_mut()
                    .bundle_contents
                    .push(sunrise_items::items::ItemStack::new("minecraft:stick", 2));
                let mut bundle_item = sunrise_sim::entities::ItemEntity::new(
                    8,
                    bundle,
                    sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                    sunrise_core::geometry::Vec3::ZERO,
                    0.0,
                );
                bundle_item.previous = sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5);
                let dropped_before = game.dropped.len();
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.sweep_item_inside(&mut bundle_item, None),
                );
                assert!(bundle_item.health <= 0);
                assert!(
                    game.dropped.len() > dropped_before
                        && game
                            .dropped
                            .iter()
                            .any(|d| d.stack.item.as_ref() == "minecraft:stick"
                                && d.stack.count == 2),
                    "fatal item hurt spills bundle contents"
                );

                // Projectile `hurt` sets `sync_velocity` (`markHurt`) and `is_on_fire` matches `shot.on_fire()`.
                let mut shot = arrow_shot(9, (1.2, 100.0, 0.5), (1.5, 100.0, 0.5));
                shot.fire_ticks = 40;
                sunrise_sim::block_contact::in_level(
                    sunrise_sim::parallel::TickThread::mint(),
                    Some("minecraft:overworld"),
                    || game.sweep_projectile_inside(&mut shot, None),
                );
                assert_eq!(*part.proj_outcomes.lock().unwrap(), Some((true, false)));
                assert!(
                    shot.sync_velocity,
                    "projectile hurt sets sync_velocity (markHurt)"
                );
            });
        });
    }

    /// Vanilla powder-snow melt and fire ignition/hurt queue into `InsideEffects` so mod
    /// callbacks order against them (`run_after(Freeze)` before `PowderSnowMelt` before
    /// `Extinguish`; `run_after(FireIgnite)` sees post-ignition `fire_ticks`).
    #[test]
    #[ignore = "daily"]
    fn player_powder_snow_and_fire_builtin_ordering() {
        if fresh("player_powder_snow_and_fire_builtin_ordering") {
            return;
        }
        use std::sync::{Arc, Mutex};

        #[derive(Clone, Default)]
        struct BuiltinOrderPart {
            snow_key: Arc<Mutex<Option<mod_api::BlockKey>>>,
            fire_key: Arc<Mutex<Option<mod_api::BlockKey>>>,
            kill_in_snow: Arc<Mutex<bool>>,
            fire_ticks_after_ignite: Arc<Mutex<Option<i32>>>,
        }
        #[mod_api::sunrise_part]
        impl mod_api::BlockPart for BuiltinOrderPart {
            fn entity_inside(
                &self,
                _: &mut dyn mod_api::LevelRandomHost,
                event: &mut mod_api::EntityInsideEvent<'_>,
            ) {
                let kill = *self.kill_in_snow.lock().unwrap();
                let saw_fire = self.fire_ticks_after_ignite.clone();
                if Some(event.state.block()) == *self.snow_key.lock().unwrap() {
                    event.run_after(mod_api::InsideEffect::Freeze, move |view| {
                        view.set_remaining_fire_ticks(40);
                        if kill {
                            view.hurt("minecraft:generic", 1000.0);
                        }
                    });
                } else if Some(event.state.block()) == *self.fire_key.lock().unwrap() {
                    event.run_after(mod_api::InsideEffect::FireIgnite, move |view| {
                        *saw_fire.lock().unwrap() = Some(view.fire_ticks());
                    });
                }
            }
        }
        struct BuiltinOrderMod(BuiltinOrderPart);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BuiltinOrderMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let snow = r.block_key("minecraft:powder_snow");
                let fire = r.block_key("minecraft:fire");
                *self.0.snow_key.lock().unwrap() = Some(snow);
                *self.0.fire_key.lock().unwrap() = Some(fire);
                r.block_part(snow, self.0.clone());
                r.block_part(fire, self.0.clone());
            }
        }

        let part = BuiltinOrderPart::default();
        let entry = || {
            mod_api::ModEntry::new(
                "inside-builtin-order",
                Box::new(BuiltinOrderMod(part.clone())),
            )
        };
        sunrise_gameplay_common::game::api_values::install_parts(&[entry()]).unwrap();
        let mods: mod_api::Mods = vec![entry()].into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                let _ = sunrise_sim::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let snow = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:powder_snow")
                    .unwrap();
                let fire = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:fire")
                    .unwrap();
                let air = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
                );

                // 1) Igniting in `run_before(Extinguish)` melts powder snow before Extinguish runs.
                game.world.set(1, 100, 0, snow).unwrap();
                let mut player = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                )
                .unwrap();
                player
                    .history
                    .add(sunrise_world::entity_move::MovementRecord {
                        from: sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                        to: sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                        original: None,
                    });
                game.player = Some(player);
                sweep_game_player_live(
                    game,
                    sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                    false,
                );
                assert_eq!(
                    game.world.block(1, 100, 0),
                    air,
                    "igniting before Extinguish triggers vanilla powder-snow melt"
                );
                assert!(
                    !game.player.as_ref().unwrap().is_on_fire(),
                    "Extinguish then puts out the player"
                );

                // 2) Killing in `run_before(Extinguish)` stops the flush before powder-snow melt.
                *part.kill_in_snow.lock().unwrap() = true;
                game.world.set(1, 100, 0, snow).unwrap();
                let mut player = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                )
                .unwrap();
                player
                    .history
                    .add(sunrise_world::entity_move::MovementRecord {
                        from: sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                        to: sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                        original: None,
                    });
                game.player = Some(player);
                sweep_game_player_live(
                    game,
                    sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                    false,
                );
                assert_eq!(
                    game.world.block(1, 100, 0),
                    snow,
                    "death before PowderSnowMelt stops the flush before melting the snow"
                );

                // 3) Standing in fire: `run_after(FireIgnite)` sees post-ignition fire_ticks.
                game.world.set(1, 100, 0, fire).unwrap();
                let mut player = sunrise_sim::motion::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                )
                .unwrap();
                player.fire_ticks = 0;
                player
                    .history
                    .add(sunrise_world::entity_move::MovementRecord {
                        from: sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                        to: sunrise_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                        original: None,
                    });
                game.player = Some(player);
                sweep_game_player_live(
                    game,
                    sunrise_core::geometry::Vec3::new(1.2, 100.0, 0.5),
                    false,
                );
                assert!(
                    part.fire_ticks_after_ignite
                        .lock()
                        .unwrap()
                        .is_some_and(|t| t > 0),
                    "run_after(FireIgnite) observes post-ignition fire_ticks"
                );
            });
        });
    }
}
