#[cfg(test)]
#[path = "transactions/player_tests.rs"]
pub(crate) mod player_tests;

#[cfg(test)]
pub(crate) mod tests {
    use crate::game::{Settings, load_resources};
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use mod_api::{
        Authority, BlockSide, BlockUpdateFlags, FLUIDS, FluidStack, ITEMS, ItemResource,
        TransactionContext, TransactionError,
    };
    use std::{sync::Arc, time::Instant};
    use sunrise_gameplay_common::game::api_values::TryIntoApi;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_hooks::ModGameExt1;
    use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::tables::WorldTablesExt;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
    use sunrise_items::items::ItemStack;
    use sunrise_level::chunk::Chunk;
    use {
        sunrise_gameplay_common::game::BlockEntity, sunrise_gameplay_common::game::Options,
        sunrise_gameplay_common::game::Progress,
    };

    #[test]
    #[ignore = "daily"]
    fn packet_refusal_rolls_back_the_child_and_reaches_the_transaction_caller() {
        use mod_api::prelude::BlockStatesHost;
        use std::sync::{atomic::AtomicUsize, mpsc};
        use sunrise_sim::lan::{PlayerSender, conn::transaction_packets::Scope};
        use {
            sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
            sunrise_gameplay::game::native_entity_refs::tests::with_game,
        };

        if run_fresh_cases(
            "cut_layout::game::transactions::tests::packet_refusal_rolls_back_the_child_and_reaches_the_transaction_caller",
            "SUNRISE_TRANSACTION_PACKET_REFUSAL_CHILD",
            &["run"],
        ) {
            return;
        }
        with_game(|game| {
            const AT: [i32; 3] = [1, 64, 1];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let before = level.block_state(game, AT);
            let stone = BlockStatesHost::block_state_id(game, "minecraft:stone").unwrap();
            let (send, receive) = mpsc::channel();
            // A real bounded LAN sender with no room left to reserve another packet.
            let sender = PlayerSender::new(send, Arc::new(AtomicUsize::new(usize::MAX)));
            let _packets = Scope::begin();
            let result = game.transaction(|host, _| {
                let child = host.transaction(|host, _| {
                    assert_eq!(
                        level.set_block(host, AT, stone, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                    assert_eq!(level.block_state(host, AT), Some(stone));
                    sender.send(vec![1]);
                    Ok::<_, ()>(())
                });
                assert_eq!(child, Err(TransactionError::CommitRefused));
                assert_eq!(level.block_state(host, AT), before);
                Ok::<_, ()>(())
            });
            assert_eq!(result, Err(TransactionError::CommitRefused));
            assert_eq!(level.block_state(game, AT), before);
            assert!(receive.try_recv().is_err());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cauldron_stat_listener_observes_committed_state_and_can_cancel() {
        use mod_api::{
            Flow, Hooks, ItemSpawnEvent, ItemTossEvent, ModEntry, Mods, PlayerStatAwardEvent,
        };
        use std::sync::Mutex;
        use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        if run_fresh_cases(
            "cut_layout::game::transactions::tests::cauldron_stat_listener_observes_committed_state_and_can_cancel",
            "SUNRISE_CAULDRON_STAT_CHILD",
            &["run"],
        ) {
            return;
        }
        struct Listener {
            seen: Arc<Mutex<Vec<(Option<mod_api::BlockState>, Option<String>)>>>,
            drops: Arc<Mutex<Vec<(&'static str, Option<mod_api::BlockState>)>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Listener {
            fn player_stat_award(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerStatAwardEvent<'_>,
            ) -> Flow {
                if event.stat.as_str() != "minecraft:fill_cauldron" {
                    return Flow::Continue;
                }
                let level = host.level("minecraft:overworld").unwrap();
                let hand = mod_api::ItemPlace::PlayerHand {
                    player: event.player,
                    hand: mod_api::client::first_person::Hand::Main,
                };
                self.seen.lock().unwrap().push((
                    level.block_state(host, [2, 64, 2]),
                    hand.with_stack(host, |s| {
                        mod_api::__private::item_key_id(s.item())
                            .unwrap()
                            .to_string()
                    }),
                ));
                Flow::Cancel
            }
            fn item_toss(&self, host: &mut dyn ServerHost, event: &mut ItemTossEvent) -> Flow {
                let potion = event.item.with_stack(host, |stack| {
                    mod_api::__private::item_key_id(stack.item()) == Some("minecraft:potion")
                });
                if potion == Some(true) {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("toss", level.block_state(host, [2, 64, 2])));
                }
                Flow::Continue
            }
            fn item_spawn(&self, host: &mut dyn ServerHost, event: &mut ItemSpawnEvent<'_>) {
                if event.item == "minecraft:potion" {
                    let level = host.level("minecraft:overworld").unwrap();
                    self.drops
                        .lock()
                        .unwrap()
                        .push(("spawn", level.block_state(host, [2, 64, 2])));
                }
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let drops = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "observer",
            Box::new(Listener {
                seen: seen.clone(),
                drops: drops.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods);
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            let full = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "3".into())].into_iter().collect(),
                })
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.as_guest(12, |game| {
                game.selected = 0;
                game.creative = false;
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                assert!(seen.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(seen.lock().unwrap().is_empty());
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                assert_eq!(
                    seen.lock().unwrap().as_slice(),
                    &[(
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )]
                );
                game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
                game.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                assert_eq!(seen.lock().unwrap().len(), 1);
                game.close_storage_transaction(true);
            })
            .unwrap();
            assert_eq!(
                *seen.lock().unwrap(),
                vec![
                    (
                        Some(empty.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    ),
                    (
                        Some(full.try_into_api().unwrap()),
                        Some("minecraft:bucket".into())
                    )
                ]
            );
            let level2 = game
                .pack
                .assets
                .registry
                .resolve(&sunrise_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".into(),
                    properties: [("level".into(), "2".into())].into_iter().collect(),
                })
                .unwrap();
            game.as_guest(12, |game| {
                let setup = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                    game.world.set(pos[0], pos[1], pos[2], full).unwrap();
                    game.inventory
                        .iter_mut()
                        .for_each(|s| *s = Some(ItemStack::new("minecraft:stone", 64)));
                    game.offhand = Some(ItemStack::new("minecraft:stone", 64));
                    game.inventory[0] = Some(ItemStack::new("minecraft:glass_bottle", 2));
                };
                setup(game);
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(full.try_into_api().unwrap())),
                        ("spawn", Some(full.try_into_api().unwrap()))
                    ]
                );
                drops.lock().unwrap().clear();
                setup(game);
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                assert!(drops.lock().unwrap().is_empty());
                game.close_storage_transaction(false);
                assert!(drops.lock().unwrap().is_empty());
                game.open_storage_transaction();
                assert_eq!(
                    game.use_on_cauldron(pos).unwrap(),
                    sunrise_sim::cauldron::Outcome::Success
                );
                game.close_storage_transaction(true);
                assert_eq!(
                    drops.lock().unwrap().as_slice(),
                    &[
                        ("toss", Some(level2.try_into_api().unwrap())),
                        ("spawn", Some(level2.try_into_api().unwrap()))
                    ]
                );
            })
            .unwrap();
            assert_eq!(
                game.server.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:fill_cauldron"),
                0
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_menu_write_outside_a_transaction_keeps_no_journal() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            game.journal_menus(player);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn menu_journals_keep_only_the_first_write_inside_a_transaction() {
        sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
            let player = game.player(12).unwrap();
            let menu = game.player_container_menu(player).unwrap();
            let stone = ItemStack::new("minecraft:stone", 4);
            *game.native_menu_storage(menu, 1).unwrap() = Some(stone.clone());
            let place = mod_api::ItemPlace::MenuSlot { menu, slot: 1 };
            let resource = ItemResource::of(&stone.try_into_api().unwrap());
            let aborted: Result<(), ()> = game.storage_transaction(|game, tx| {
                assert_eq!(place.insert(game, tx, &resource, 2), Ok(2));
                assert_eq!(place.insert(game, tx, &resource, 3), Ok(3));
                let journal = &game.transactions.last().unwrap().menus;
                assert_eq!(journal.len(), 1);
                assert_eq!(journal[&12].craft_grid[0], Some(stone.clone()));
                assert_eq!(menu.slot(game, 1).unwrap().unwrap().count(), 9);
                Err(())
            });
            assert_eq!(aborted, Err(()));
            assert!(game.transactions.is_empty());
            assert_eq!(
                menu.slot(game, 1),
                Some(Some(stone.try_into_api().unwrap()))
            );
        });
    }

    pub(crate) fn insert(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        stack: ItemStack,
        tx: &TransactionContext,
    ) -> Option<ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let inserted = handler
            .insert_at(
                host,
                tx,
                slot,
                &ItemResource::of(&stack.try_into_api().unwrap()),
                stack.count,
            )
            .unwrap();
        (inserted < stack.count).then(|| ItemStack {
            count: stack.count - inserted,
            ..stack
        })
    }

    pub(crate) fn extract(
        host: &mut dyn ServerHost,
        at: BlockSide,
        slot: usize,
        amount: u32,
        tx: &TransactionContext,
    ) -> Option<mod_api::ItemStack> {
        let handler = at
            .level
            .capability(host, &ITEMS, at.pos, at.side)
            .expect("item capability");
        let resource = handler.resource(host, slot)?;
        let taken = handler
            .extract_at(host, tx, slot, &resource, amount)
            .unwrap();
        resource.to_stack(taken)
    }

    pub(crate) fn extract_fluid(
        host: &mut dyn ServerHost,
        at: BlockSide,
        tank: usize,
        amount: u32,
        tx: &TransactionContext,
    ) -> Option<FluidStack> {
        let handler = at
            .level
            .capability(host, &FLUIDS, at.pos, at.side)
            .expect("fluid capability");
        let resource = handler.resource(host, tank)?;
        let taken = handler
            .extract_at(host, tx, tank, &resource, amount)
            .unwrap();
        (taken > 0).then(|| resource.to_stack(taken))
    }

    pub(crate) fn chest_items(game: &sunrise_gameplay::game::ServerGame<'_>) -> Vec<(usize, u32)> {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0)) else {
            panic!("chest block entity missing")
        };
        let held = items.iter().enumerate();
        held.filter_map(|(i, s)| Some((i, s.as_ref()?.count)))
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn container_limits_preserve_capacity_and_transaction_rollback() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            for (pos, id) in [
                ([0, 64, 0], "minecraft:chest"),
                ([4, 64, 0], "minecraft:jukebox"),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.level_set_block(level, pos, state, BlockUpdateFlags::NEIGHBORS)
                    .unwrap();
            }
            let mut large = ItemStack::new("minecraft:stick", 1);
            large.data_mut().set_other(
                "minecraft:max_stack_size",
                Some(sunrise_core::nbt::Tag::Int(99)),
            );
            let large = ItemResource::of(&large.try_into_api().unwrap());
            let handler = level.capability(game, &ITEMS, [0, 64, 0], None).unwrap();
            assert_eq!(handler.capacity(game, 0, Some(&large)), 99);
            assert_eq!(game.world.container_stack_limit((0, 64, 0)), None);
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let host: &mut dyn ServerHost = game;
            let handler = level
                .capability(host, &mod_api::ITEMS, [0, 64, 0], None)
                .unwrap();
            let aborted = host.transaction(|host, tx| {
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(10));
                host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 4), Ok(true));
                    Ok::<_, ()>(())
                })
                .unwrap();
                assert_eq!(handler.capacity(host, 0, Some(&stick)), 4);
                Err::<(), ()>(())
            });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!(handler.capacity(host, 0, Some(&stick)), 64);
            assert_eq!(handler.capacity(host, 0, Some(&large)), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                let _ = host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 4), Ok(true));
                    Err::<(), ()>(())
                });
                assert_eq!(handler.capacity(host, 0, Some(&stick)), 10);
                for invalid in [0, 100, u32::MAX] {
                    assert_eq!(handler.set_max_stack_size(host, tx, invalid), Ok(false));
                    assert_eq!(handler.capacity(host, 0, Some(&stick)), 10);
                }
                assert_eq!(handler.set_max_stack_size(host, tx, 64), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 20), Ok(20));
                assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                assert_eq!(handler.amount(host, 0), 20);
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 1), Ok(0));
                assert_eq!(handler.extract_at(host, tx, 0, &stick, 15), Ok(15));
                assert_eq!(handler.amount(host, 0), 5);
                assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(5));
                Err::<(), ()>(())
            })
            .unwrap_err();

            let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
                    assert_eq!(handler.set_max_stack_size(host, tx, 10), Ok(true));
                    assert_eq!(handler.insert_at(host, tx, 0, &stick, 64), Ok(10));
                    panic!("abort container writes");
                });
            }));
            assert!(panic.is_err());
            assert_eq!(handler.capacity(host, 0, Some(&stick)), 64);
            assert_eq!(handler.capacity(host, 0, Some(&large)), 99);
            assert_eq!(handler.amount(host, 0), 0);
            host.transaction(|host, tx| {
                let pearl = ItemResource::of(
                    &ItemStack::new("minecraft:ender_pearl", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert_eq!(handler.set_max_stack_size(host, tx, 32), Ok(true));
                assert_eq!(handler.insert_at(host, tx, 0, &pearl, 64), Ok(16));
                let disc = level.capability(host, &ITEMS, [4, 64, 0], None).unwrap();
                assert_eq!(disc.set_max_stack_size(host, tx, 10), Ok(true));
                let record = ItemResource::of(
                    &ItemStack::new("minecraft:music_disc_13", 1)
                        .try_into_api()
                        .unwrap(),
                );
                assert_eq!(disc.insert_at(host, tx, 0, &record, 64), Ok(1));
                Err::<(), ()>(())
            })
            .unwrap_err();
            // Player clicks and shift-clicks use the menu's separate slot rules.
            let env = game.menu_env();
            let mut menu = sunrise_sim::menu::Menu::new(sunrise_sim::menu::Kind::Chest(1));
            menu.set_container_limits(vec![Some(10); 9]);
            menu.carried = Some(ItemStack::new("minecraft:stick", 20));
            menu.click(&env, 0, 0, sunrise_sim::menu::Input::Pickup)
                .unwrap();
            assert_eq!(menu.slot(0).unwrap().count, 10);
            assert_eq!(menu.carried.as_ref().unwrap().count, 10);
            menu.carried = None;
            menu.set_slot(&env, 9, Some(ItemStack::new("minecraft:stick", 20)));
            menu.click(&env, 9, 0, sunrise_sim::menu::Input::QuickMove)
                .unwrap();
            assert_eq!(menu.slot(1).unwrap().count, 10);
            assert_eq!(menu.slot(2).unwrap().count, 10);
            let hopper = sunrise_world::chunk::BlockState {
                name: "minecraft:hopper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            let flags = BlockUpdateFlags::NEIGHBORS;
            game.level_set_block(level, [0, 64, 4], hopper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(ItemStack::new("minecraft:stick", 2));
            }
            let sticks = |count| ItemStack::new("minecraft:stick", count);
            let slots = |game: &sunrise_gameplay::game::ServerGame<'_>, pos| match game
                .world
                .block_entities
                .get(&pos)
            {
                Some(BlockEntity::Chest(items)) => {
                    items.iter().flatten().map(|s| s.count).sum::<u32>()
                }
                _ => panic!("missing chest"),
            };
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], chest, flags),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(1, 64, 4))
            {
                items.fill(Some(sticks(10)));
                items[0] = Some(sticks(9));
            }
            let target = level.capability(game, &ITEMS, [1, 64, 4], None).unwrap();
            game.storage_transaction(|host, tx| {
                assert_eq!(target.set_max_stack_size(host, tx, 10), Ok(true));
                Ok::<_, ()>(())
            })
            .unwrap();
            game.test_clear_hopper((0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            let _ = game.storage_transaction(|host, tx| {
                assert_eq!(target.set_max_stack_size(host, tx, 64), Ok(true));
                Err::<(), ()>(())
            });
            game.test_clear_hopper((0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 1);
            // Dropper uses add_item, dispenser ejects into the world instead.
            let dropper = sunrise_world::chunk::BlockState {
                name: "minecraft:dropper".into(),
                properties: [("facing".into(), "east".into())].into_iter().collect(),
            };
            let dropper = game.pack.assets.registry.resolve(&dropper).unwrap();
            game.level_set_block(level, [0, 64, 4], dropper, flags)
                .unwrap();
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            game.dispense_from(sunrise_core::position::BlockPos::new(0, 64, 4), dropper)
                .unwrap();
            assert_eq!(slots(game, (1, 64, 4)), 270);
            assert_eq!(slots(game, (0, 64, 4)), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed() {
        let Some(root) = crate::game::native_entity_refs::tests::declared_part_root(
            "cut_layout::game::transactions::tests::an_aborted_transaction_puts_back_what_it_and_its_committed_children_changed",
        ) else {
            return;
        };
        let settings = Settings::default();
        let loaded = load_resources(
            sunrise_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role =
            sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer;
        options.dedicated = Some(Vec::new());
        let mut game = sunrise_gameplay::game::ServerGame::new(
            &loaded.resources,
            loaded.pack,
            options,
            settings.sim_settings(),
            None,
            crate::game::profile_resolver,
        )
        .unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(Some(Authority::IntegratedServer));
        sunrise_gameplay::game::native_entity_refs::tests::resolve_parts(&game).unwrap();
        // Pins the opaque backend: a real game always hands one out, so a None
        // from Host::backend is a bug, never a fallback.
        assert!(mod_api::Host::backend(&game).is_some());
        assert!(ServerHost::backend_mut(&mut game).is_some());
        let level = game.level("minecraft:overworld").unwrap();
        let registry = &game.pack.assets.registry;
        let chest = registry.default_state("minecraft:chest").unwrap();
        let full = sunrise_world::chunk::BlockState {
            name: "minecraft:water_cauldron".to_owned(),
            properties: [("level".to_owned(), "3".to_owned())].into_iter().collect(),
        };
        let full = registry.resolve(&full).unwrap();
        let empty = registry.default_state("minecraft:cauldron").unwrap();
        let jukebox = registry.default_state("minecraft:jukebox").unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [0, 64, 0], chest, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [4, 64, 0], jukebox, flags),
            Ok(true)
        );
        let sticks = |count| ItemStack::new("minecraft:stick", count);
        let stick = ItemResource::of(&sticks(1).try_into_api().unwrap());
        let rod = ItemResource::of(
            &ItemStack::new("minecraft:blaze_rod", 1)
                .try_into_api()
                .unwrap(),
        );
        let on = |pos, side| BlockSide { level, pos, side };
        let host: &mut dyn ServerHost = &mut game;

        // Five in, then a committed child takes two and an aborted one adds three: the
        // outer abort puts all of it back, the cauldron's water too.
        let refused: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
            assert!(insert(host, on([0, 64, 0], None), 0, sticks(5), tx).is_none());
            let taken = host
                .transaction(|host, tx| Ok::<_, ()>(extract(host, on([0, 64, 0], None), 0, 2, tx)));
            assert_eq!(taken, Ok(Some(sticks(2).try_into_api().unwrap())));
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            let water = extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            assert_eq!(water.map(|w| w.amount), Some(1000));
            Err(())
        });
        assert_eq!(refused, Err(TransactionError::Aborted(())));
        assert_eq!(chest_items(&game), []);
        assert_eq!(game.world.block(2, 64, 0), full);
        assert!(game.transactions.is_empty());

        // The same, committed: what the children left stands, the cauldron is set for good.
        let host: &mut dyn ServerHost = &mut game;
        let done: Result<(), TransactionError<()>> = host.transaction(|host, tx| {
            insert(host, on([0, 64, 0], None), 0, sticks(5), tx);
            let _ = host.transaction(|host, tx| {
                extract(host, on([0, 64, 0], None), 0, 2, tx);
                Ok::<_, ()>(())
            });
            let _ = host.transaction(|host, tx| {
                insert(host, on([0, 64, 0], None), 1, sticks(3), tx);
                Err::<(), ()>(())
            });
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Ok(())
        });
        assert_eq!(done, Ok(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert_eq!(game.world.block(2, 64, 0), empty);

        // A transaction a mod opened through the helpers and never closed is aborted before
        // the next tick.
        game.open_storage_transaction();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            Ok::<_, ()>(insert(host, on([0, 64, 0], None), 0, sticks(1), tx))
        });
        game.abort_open_transactions();
        assert_eq!(chest_items(&game), [(0, 3)]);

        // A panic caught outside the root: what it changed is gone before the catch returns,
        // and the transaction is closed.
        let host: &mut dyn ServerHost = &mut game;
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.transaction(|host, tx| -> Result<(), ()> {
                insert(host, on([0, 64, 0], None), 0, sticks(1), tx);
                panic!("a broken handler")
            })
        }));
        assert!(caught.is_err());
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // `transaction_matches` checks the exact innermost open frame only, and passing an
        // outer/ancestor `TransactionContext` to a write inside a nested transaction is refused
        // cleanly without modifying state.
        let host: &mut dyn ServerHost = &mut game;
        assert!(host.transaction_matches(None));
        let ancestor_check: Result<(), TransactionError<()>> = host.transaction(|host, outer| {
            assert!(!host.transaction_matches(None));
            assert!(host.transaction_matches(Some(outer)));
            let inner_ok: Result<(), TransactionError<()>> = host.transaction(|host, inner| {
                assert!(!host.transaction_matches(None));
                assert!(!host.transaction_matches(Some(outer)));
                assert!(host.transaction_matches(Some(inner)));
                let handler = level
                    .capability(host, &ITEMS, [0, 64, 0], None)
                    .expect("item capability");
                assert_eq!(
                    handler.insert_at(
                        host,
                        outer,
                        0,
                        &ItemResource::of(&sticks(1).try_into_api().unwrap()),
                        1,
                    ),
                    Err(TransactionError::InvalidContext)
                );
                Ok(())
            });
            assert_eq!(inner_ok, Ok(()));
            assert!(host.transaction_matches(Some(outer)));
            Ok(())
        });
        assert_eq!(ancestor_check, Ok(()));
        assert!(game.transaction_matches(None));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // A panic inside a nested `storage_transaction` rolls back only the inner frame;
        // the outer frame stays open and keeps its own writes until it closes.
        let outer_aborted: Result<(), ()> = game.storage_transaction(|game, outer| {
            assert!(insert(game, on([0, 64, 0], None), 0, sticks(1), outer).is_none());
            assert_eq!(chest_items(game), [(0, 4)]);
            let inner_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _: Result<(), ()> = game.storage_transaction(|game, inner| {
                    assert!(insert(game, on([0, 64, 0], None), 0, sticks(2), inner).is_none());
                    assert_eq!(chest_items(game), [(0, 6)]);
                    panic!("nested storage panic");
                });
            }));
            assert!(inner_panic.is_err());
            assert!(game.transaction_matches(Some(outer)));
            assert_eq!(chest_items(game), [(0, 4)]);
            Err(())
        });
        assert_eq!(outer_aborted, Err(()));
        assert_eq!(chest_items(&game), [(0, 3)]);
        assert!(game.transactions.is_empty());

        // An attachment map a write made inside an aborted transaction is gone with it.
        let chest_holder = mod_api::Holder::BlockEntity([0, 64, 0]);
        let chest_key = sunrise_mod_runtime::attachments::AttachmentType {
            namespace: "transaction_test",
            name: "chest",
            default: || 0u32,
            codec: None,
            copy_on_death: false,
            sync_codec: None,
        };
        // So is the unsaved mark it put on a clean chunk, through a committed child too.
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        game.world.modified.clear();
        game.open_storage_transaction();
        game.open_storage_transaction();
        chest_key.set(&mut game, chest_holder, 31).unwrap();
        assert!(game.world.modified.contains(&(0, 0)));
        game.close_storage_transaction(true);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_some());
        game.close_storage_transaction(false);
        assert!(game.world.block_entities.attachments((0, 64, 0)).is_none());
        assert!(!game.world.modified.contains(&(0, 0)), "clean again");
        // A committed one keeps it.
        game.open_storage_transaction();
        chest_key.set(&mut game, chest_holder, 31).unwrap();
        game.close_storage_transaction(true);
        assert!(game.world.modified.contains(&(0, 0)));
        game.world.block_entities.set_attachments((0, 64, 0), None);

        // An aborted cauldron change leaves no write behind in the world's queues.
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        game.world.block_log = Some(Vec::new());
        game.world.path_writes = Some(Vec::new());
        let queues = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let world = &game.world;
            (
                world.block_log.as_ref().map(Vec::len),
                world.path_writes.as_ref().map(Vec::len),
                world.light_edits.len(),
                world.dirty_light.len(),
                world.watched_writes.len(),
            )
        };
        let before = queues(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            extract_fluid(host, on([2, 64, 0], None), 0, 1000, tx);
            Err::<(), ()>(())
        });
        assert_eq!(game.world.block(2, 64, 0), full);
        assert_eq!(queues(&game), before);

        // A disc swapped for another in one transaction changes the song: the old one's
        // stops and the new one's starts.
        let disc = |item: &str| ItemStack::new(item, 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let cat = disc("minecraft:music_disc_cat");
            Ok::<_, ()>(insert(host, on([4, 64, 0], None), 0, cat, tx))
        });
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get_mut(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert!(j.playing, "a disc put in through a transfer plays");
        j.ticks = 500;
        let host: &mut dyn ServerHost = &mut game;
        let swapped = host.transaction(|host, tx| {
            extract(host, on([4, 64, 0], None), 0, 1, tx);
            Ok::<_, ()>(insert(
                host,
                on([4, 64, 0], None),
                0,
                disc("minecraft:music_disc_13"),
                tx,
            ))
        });
        assert_eq!(swapped, Ok(None));
        let Some(BlockEntity::Jukebox(j)) = game.world.block_entities.get(&(4, 64, 0)) else {
            panic!("jukebox block entity missing")
        };
        assert_eq!(
            j.record().map(|r| &*r.item),
            Some("minecraft:music_disc_13")
        );
        assert!(j.playing);
        assert_eq!(j.ticks, 0, "the new disc's song starts from its beginning");

        // A player's place: one of three buckets in the hand is exchanged for a filled one; the
        // hand keeps two and the filled one goes to the inventory, and what doesn't fit would be
        // thrown. Aborted, the hand and the inventory are as they were, and nothing was thrown.
        game.player = Some(
            super::super::PlayerMotion::new(
                game.motion,
                sunrise_core::geometry::Vec3::new(0.5, 65.0, 3.5),
            )
            .unwrap(),
        );
        let bucket = |item: &str, count| ItemStack::new(item, count);
        game.inventory
            .iter_mut()
            .for_each(|slot| *slot = Some(bucket("minecraft:stone", 64)));
        game.selected = 0;
        game.inventory[0] = Some(bucket("minecraft:bucket", 3));
        game.carried = None;
        let before = (
            game.inventory.clone(),
            game.offhand.clone(),
            game.carried.clone(),
        );
        let dropped = game.dropped.len();
        let me = game.native_acting_id();
        let hand = mod_api::ItemPlace::PlayerHand {
            player: me,
            hand: mod_api::client::first_person::Hand::Main,
        };
        let water = ItemResource::of(&bucket("minecraft:water_bucket", 1).try_into_api().unwrap());
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(hand.exchange(host, tx, &water, 1), Ok(1));
            Err::<(), ()>(())
        });
        assert_eq!(
            (
                game.inventory.clone(),
                game.offhand.clone(),
                game.carried.clone()
            ),
            before
        );
        assert_eq!(
            game.dropped.len(),
            dropped,
            "a thrown bucket waits for the root commit"
        );

        // Committed, the split stands and the full inventory throws the filled bucket.
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| hand.exchange(host, tx, &water, 1));
        assert_eq!(game.inventory[0], Some(bucket("minecraft:bucket", 2)));
        assert_eq!(game.dropped.len(), dropped + 1);
        let thrown = &game.dropped[game.dropped.len() - 1].stack;
        assert_eq!(&*thrown.item, "minecraft:water_bucket");

        // A chest minecart's, a chest boat's and a chested donkey's slots: an abort puts back
        // what an insert changed; a commit keeps an exchange, whose new item goes into the next
        // free slot.
        let mut chested = sunrise_core::nbt::Compound::new();
        chested.insert("ChestedHorse".into(), sunrise_core::nbt::Tag::Byte(1));
        let vehicles = [
            (
                "minecraft:chest_minecart",
                sunrise_core::nbt::Compound::new(),
            ),
            (
                "minecraft:oak_chest_boat",
                sunrise_core::nbt::Compound::new(),
            ),
            ("minecraft:donkey", chested),
        ];
        // An aborted write leaves a chested donkey's item list as long as it was.
        let level = game.level("minecraft:overworld").unwrap();
        let id = game
            .level_summon(level, "minecraft:donkey", [8.5, 70.0, 8.5], {
                let mut chested = sunrise_core::nbt::Compound::new();
                chested.insert("ChestedHorse".into(), sunrise_core::nbt::Tag::Byte(1));
                chested
            })
            .unwrap()
            .unwrap()
            .id();
        let len = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.mobs
                .iter()
                .find(|m| m.id == id)
                .unwrap()
                .extra
                .items
                .len()
        };
        // Its list is short (none kept yet): reading it as slots grows it, and the abort puts
        // that back too.
        let mob = game.mobs.iter_mut().find(|m| m.id == id).unwrap();
        mob.extra.items.clear();
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            let place = mod_api::ItemPlace::EntitySlot {
                entity: id,
                slot: 0,
            };
            assert!(place.with_stack(host, |_| ()).is_none());
            assert_eq!(place.insert(host, tx, &stick, 1), Ok(1));
            Err::<(), ()>(())
        });
        assert_eq!(len(&game), 0);
        // Grown three transactions deep, the middle one committing: the root's abort still
        // truncates it.
        let place = mod_api::ItemPlace::EntitySlot {
            entity: id,
            slot: 0,
        };
        let host: &mut dyn ServerHost = &mut game;
        let result = host.transaction(|host, _| {
            host.transaction(|host, _| {
                host.transaction(|host, tx| {
                    assert!(place.with_stack(host, |_| ()).is_none());
                    assert_eq!(place.insert(host, tx, &stick, 1), Ok(1));
                    Ok::<_, ()>(())
                })
                .unwrap();
                Ok::<_, ()>(())
            })
            .unwrap();
            Err::<(), ()>(())
        });
        assert!(result.is_err());
        assert_eq!(len(&game), 0);

        for (kind, nbt) in vehicles {
            let id = game
                .level_summon(level, kind, [8.5, 70.0, 8.5], nbt)
                .unwrap()
                .unwrap_or_else(|| panic!("{kind} not summoned"))
                .id();
            let place = |slot| mod_api::ItemPlace::EntitySlot { entity: id, slot };
            let host: &mut dyn ServerHost = &mut game;
            let went = host.transaction(|host, tx| place(0).insert(host, tx, &stick, 5));
            assert_eq!(went, Ok(5), "{kind}");
            let _ = host.transaction(|host, tx| {
                assert_eq!(place(0).insert(host, tx, &stick, 4), Ok(4), "{kind}");
                assert_eq!(place(1).insert(host, tx, &stick, 2), Ok(2), "{kind}");
                Err::<(), ()>(())
            });
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).with_stack(host, |stack| stack.project()).flatten(),
                Some(sticks(5).try_into_api().unwrap()),
                "{kind}"
            );
            assert_eq!(place(1).with_stack(host, |_| ()), None, "{kind}");
            let _ = host.transaction(|host, tx| place(0).exchange(host, tx, &rod, 1));
            let host: &mut dyn ServerHost = &mut game;
            assert_eq!(
                place(0).with_stack(host, |stack| stack.project()).flatten(),
                Some(sticks(4).try_into_api().unwrap()),
                "{kind}"
            );
            let rod =
                place(1).with_stack(host, |s| mod_api::__private::item_key_id(s.item()).unwrap());
            assert_eq!(rod, Some("minecraft:blaze_rod"), "{kind}");
        }

        // An item entity: emptied in an aborted transaction, it's back whole; one of four
        // exchanged in a committed one is dropped beside it at the commit; emptied in a
        // committed one, it's gone.
        game.spawn_item(sticks(4), [0.5, 70.0, 6.5], [0.0; 3], 10);
        let id = game.dropped.last().unwrap().id;
        let entity = mod_api::ItemPlace::ItemEntity { id };
        let count = |game: &sunrise_gameplay::game::ServerGame<'_>| game.dropped.len();
        let items = count(&game);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| {
            assert_eq!(entity.extract(host, tx, &stick, 4), Ok(4));
            assert!(entity.with_stack(host, |_| ()).is_none());
            Err::<(), ()>(())
        });
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(
            entity.with_stack(host, |stack| stack.project()).flatten(),
            Some(sticks(4).try_into_api().unwrap())
        );
        let _ = host.transaction(|host, tx| entity.exchange(host, tx, &rod, 1));
        let host: &mut dyn ServerHost = &mut game;
        assert_eq!(
            entity.with_stack(host, |stack| stack.project()).flatten(),
            Some(sticks(3).try_into_api().unwrap())
        );
        assert_eq!(count(&game), items + 1);
        let host: &mut dyn ServerHost = &mut game;
        let _ = host.transaction(|host, tx| entity.extract(host, tx, &stick, 3));
        assert!(game.dropped.iter().all(|e| e.id != id));
        assert_eq!(count(&game), items);

        // A hopper moving into a mod's part: a truthful part (a pipe into the chest east of
        // it) takes the item; a part answering more than it was offered has its move undone,
        // so the item stays in the hopper and the chest it wrote is as it was.
        let hopper = sunrise_world::chunk::BlockState {
            name: "minecraft:hopper".to_owned(),
            properties: [("facing".to_owned(), "east".to_owned())]
                .into_iter()
                .collect(),
        };
        let hopper = game.pack.assets.registry.resolve(&hopper).unwrap();
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        assert_eq!(
            game.level_set_block(level, [0, 64, 4], hopper, flags),
            Ok(true)
        );
        assert_eq!(
            game.level_set_block(level, [2, 64, 4], chest, flags),
            Ok(true)
        );
        let slots = |game: &sunrise_gameplay::game::ServerGame<'_>, pos| match game
            .world
            .block_entities
            .get(&pos)
        {
            Some(BlockEntity::Chest(items)) => items.iter().flatten().map(|s| s.count).sum::<u32>(),
            _ => panic!("no container at {pos:?}"),
        };
        for (id, moved) in [("test:pipe", 1), ("test:greedy", 0)] {
            let state = game.pack.assets.registry.default_state(id).unwrap();
            assert_eq!(
                game.level_set_block(level, [1, 64, 4], state, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 4))
            {
                items[0] = Some(sticks(2));
            }
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(2, 64, 4))
            {
                items.fill(None);
            }
            game.test_clear_hopper((0, 64, 4));
            game.hopper_try_move_items((0, 64, 4));
            assert_eq!(slots(&game, (0, 64, 4)), 2 - moved, "{id}");
            assert_eq!(slots(&game, (2, 64, 4)), moved, "{id}");
            assert!(game.transactions.is_empty());
        }
    }
}

#[cfg(test)]
pub(crate) mod mod_capability_tests {
    use mod_api::ServerHost;
    use mod_api::{BlockSide, HostTransaction, ResourceHandler, resource_util::move_resources};
    use mod_platform::__private::part_tests::{CHEMICALS, ChemTank, Chemical};
    use std::sync::Arc;
    use sunrise_core::nbt::{Compound, Tag};
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;
    use sunrise_level::chunk::Chunk;
    use sunrise_level::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields};

    /// Two mods sharing a capability one of them defines, as Mekanism's chemicals are shared
    /// through its API: the first's tank and the second's vat each answer `CHEMICALS`, and a
    /// move between them in a committed transaction moves the chemical, kept in their saved
    /// data; in an aborted one, both are put back.
    #[test]
    #[ignore = "daily"]
    fn a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back() {
        use {
            crate::game::server_side::native_entity_refs::tests::declared_part_root,
            sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
        };
        let Some(root) = declared_part_root(
            "cut_layout::game::transactions::mod_capability_tests::a_chemical_moves_between_two_mods_parts_and_an_abort_puts_it_back",
        ) else {
            return;
        };
        with_game_at(root, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let hydrogen = Chemical("test:hydrogen".into());
            let mut nbt = Compound::new();
            nbt.insert("chemical".into(), Tag::String("test:hydrogen".into()));
            nbt.insert("amount".into(), Tag::Int(600));
            for (x, id, nbt) in [
                (0, "test:chem_tank", nbt),
                (1, "test:chem_vat", Compound::new()),
            ] {
                let state = game.pack.assets.registry.default_state(id).unwrap();
                game.world.set(x, 64, 0, state).unwrap();
                let fields = ModBlockEntityFields {
                    nbt,
                    ..Default::default()
                };
                let entity = BlockEntity::Mod(Box::new(ModBlockEntity::new(id.into(), fields)));
                game.world.block_entities.insert((x, 64, 0), entity);
            }
            let host: &mut dyn ServerHost = game;
            let tank = level
                .capability(host, &CHEMICALS, [0, 64, 0], None)
                .unwrap();
            let vat = level
                .capability(host, &CHEMICALS, [1, 64, 0], None)
                .unwrap();
            // Each answered by its own mod.
            assert_ne!(tank.owner(), vat.owner());
            assert!(![tank.owner(), vat.owner()].contains(&"the game".to_owned()));
            let amounts = |host: &mut dyn ServerHost| {
                let side = |x| BlockSide {
                    level,
                    pos: [x, 64, 0],
                    side: None,
                };
                (
                    ChemTank(side(0)).amount(host, 0),
                    ChemTank(side(1)).amount(host, 0),
                )
            };

            let moved = host.transaction(|host, tx| {
                move_resources(host, &tank, &vat, |c| *c == hydrogen, 250, Some(tx)).map_err(drop)
            });
            assert_eq!(moved, Ok(250));
            assert_eq!(amounts(host), (350, 250));
            assert_eq!(vat.resource(host, 0), Some(hydrogen.clone()));

            let aborted = host.transaction(|host, tx| {
                let moved = move_resources(host, &tank, &vat, |_| true, 100, Some(tx));
                assert_eq!(moved.ok(), Some(100));
                assert_eq!(amounts(host), (250, 350));
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(amounts(host), (350, 250));
            assert!(game.transactions.is_empty());
        });
    }

    /// A mod's attachments on an entity and on a player are kept by a transaction as a block
    /// entity's are: a commit keeps what it wrote, an abort puts each map back as it was before
    /// the transaction first reached it (none for an entity that had none), and a guest's is
    /// put back wherever their state lives at the abort, swapped in for their turn or not.
    #[test]
    #[ignore = "daily"]
    fn entity_and_player_attachments_stay_on_commit_and_go_back_on_abort() {
        use mod_api::{AttachmentDescriptor, AttachmentKey, Holder, Hooks, ModEntry, Registrar};
        use std::sync::Mutex;
        use sunrise_core::geometry::Vec3;
        use sunrise_sim::entities::ArmorStand;
        struct EnergyMod(Arc<Mutex<Option<AttachmentKey<i32>>>>);
        #[mod_api::__subscriptions]
        impl Hooks for EnergyMod {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let key = r
                    .register_attachment_type(AttachmentDescriptor::new("robot:energy", || 0i32))
                    .unwrap();
                *self.0.lock().unwrap() = Some(key);
            }
        }
        let energy_slot = Arc::new(Mutex::new(None));
        let mods: mod_api::Mods = vec![ModEntry::new(
            "robot",
            Box::new(EnergyMod(energy_slot.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let energy_key = energy_slot
                    .lock()
                    .unwrap()
                    .clone()
                    .expect("EnergyMod registered");
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.common
                    .armor_stands
                    .push(ArmorStand::new(13, Vec3::ZERO, 0.0));
                game.common
                    .armor_stands
                    .push(ArmorStand::new(14, Vec3::ZERO, 0.0));
                let guest = 0x5eed_u128;
                let config = sunrise_sim::lan::hub::HubConfig {
                    default_game_mode: 0,
                    guest_commands: false,
                    force_game_mode: false,
                    hardcore: false,
                    view_distance: 10,
                    simulation_distance: 10,
                    dimension: "minecraft:overworld".into(),
                    allow_flight: false,
                    seed: 0,
                    max_players: 20,
                    integrated: false,
                    levels: Default::default(),
                    level_seeds: Default::default(),
                };
                game.lan_host(
                    sunrise_gameplay::game::lan::LanSettings {
                        port: 0,
                        guest_commands: false,
                        force_game_mode: false,
                    },
                    sunrise_sim::lan::NetConfig {
                        announce: false,
                        ..sunrise_sim::lan::NetConfig::lan(0, "test".into(), 2)
                    },
                    config,
                )
                .unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let player_ref = game.fake_player(level, guest, "test").unwrap();
                let conn = game
                    .server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.entity_id as u32 == player_ref.entity().id())
                    .unwrap()
                    .conn;
                let (stand, fresh, player) = (
                    Holder::Entity(13),
                    Holder::Entity(14),
                    Holder::Player(guest),
                );
                let energy = |host: &mut dyn ServerHost, holder| {
                    energy_key
                        .get_existing(host, holder, |value| value.cloned())
                        .unwrap()
                };

                let host: &mut dyn ServerHost = game;
                let done = host.transaction(|host, _| {
                    energy_key.set(host, stand, 100)?;
                    energy_key.set(host, player, 40)
                });
                assert_eq!(done, Ok(()));
                assert_eq!(
                    (energy(host, stand), energy(host, player)),
                    (Some(100), Some(40))
                );

                let aborted: Result<(), _> = host.transaction(|host, _| {
                    energy_key.set(host, stand, 250)?;
                    energy_key.set(host, player, 90)?;
                    energy_key.set(host, fresh, 7)?;
                    host.transaction(|host, _| energy_key.set(host, stand, 300))
                        .unwrap();
                    assert_eq!(energy(host, stand), Some(300));
                    Err(mod_api::AttachmentError::MissingHolder)
                });
                assert!(aborted.is_err());
                assert_eq!(
                    (energy(host, stand), energy(host, player)),
                    (Some(100), Some(40))
                );
                assert_eq!(energy(host, fresh), None);

                game.storage_scope(|game, _| {
                    game.server.lan_acting_uuid = Some(guest);
                    std::mem::swap(
                        &mut game.common.player_state.attachments,
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .attachments_mut(conn)
                            .unwrap(),
                    );
                    let host: &mut dyn ServerHost = game;
                    energy_key.set(host, player, 70).unwrap();
                    std::mem::swap(
                        &mut game.common.player_state.attachments,
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .attachments_mut(conn)
                            .unwrap(),
                    );
                    game.server.lan_acting_uuid = None;
                    ((), false)
                });
                let host: &mut dyn ServerHost = game;
                assert_eq!(energy(host, player), Some(40));
                assert!(game.transactions.is_empty());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn cauldron_effects_wait_for_root_and_abort_with_inventory() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let pos = [2, 64, 2];
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let empty = game
                .pack
                .assets
                .registry
                .default_state("minecraft:cauldron")
                .unwrap();
            game.world.set(pos[0], pos[1], pos[2], empty).unwrap();
            game.player = Some(
                super::super::PlayerMotion::new(
                    game.motion,
                    sunrise_core::geometry::Vec3::new(0.5, 65.0, 3.5),
                )
                .unwrap(),
            );
            game.selected = 0;
            game.creative = false;
            game.inventory[0] = Some(sunrise_items::items::ItemStack::new(
                "minecraft:water_bucket",
                1,
            ));
            let before = game.stats.get("custom", "minecraft:fill_cauldron");
            game.start_audio_capture();
            game.open_storage_transaction();
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_sim::cauldron::Outcome::Success
            );
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.server.pending_sounds.is_empty());
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.server.pending_sounds.is_empty());
            game.open_storage_transaction();
            game.open_storage_transaction();
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_sim::cauldron::Outcome::Success
            );
            game.close_storage_transaction(false);
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:water_bucket"
            );
            game.close_storage_transaction(true);
            assert_eq!(game.stats.get("custom", "minecraft:fill_cauldron"), before);
            assert!(game.server.pending_sounds.is_empty());
            assert_eq!(
                game.use_on_cauldron(pos).unwrap(),
                sunrise_sim::cauldron::Outcome::Success
            );
            assert_ne!(game.world.block(pos[0], pos[1], pos[2]), empty);
            assert_eq!(
                &*game.inventory[0].as_ref().unwrap().item,
                "minecraft:bucket"
            );
            assert_eq!(
                game.stats.get("custom", "minecraft:fill_cauldron"),
                before + 1
            );
            assert_eq!(
                game.server
                    .pending_sounds
                    .iter()
                    .map(|s| s.event.as_str())
                    .collect::<Vec<_>>(),
                ["minecraft:item.bucket.empty"]
            );
        });
    }
}

#[path = "transactions/scratch.rs"]
pub(crate) mod scratch;

#[path = "transactions/decision.rs"]
pub(crate) mod decision;
