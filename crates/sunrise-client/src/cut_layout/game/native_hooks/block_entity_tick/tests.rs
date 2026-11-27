use mod_api::BlockEntityRef;
use mod_api::BlockEntityScope;
use mod_api::ServerHost;
use mod_api::{
    BlockEdit, BlockEntityLoadAdditionalEvent, BlockEntityPart, BlockEntitySaveAdditionalEvent,
    BlockEntityTickEvent, BlockPart, BlockRun, BlockUpdateFlags, Direction, EntityJoinLevelEvent,
    Flow, Hooks, HostTransaction, ItemResource, ItemSpawnEvent, LevelHost, ModEntry,
    NeighborNotifyEvent, Registrar, STORAGE,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use sunrise_core::geometry::Vec3;
use sunrise_core::nbt::Compound;
use sunrise_core::nbt::Tag;
use sunrise_gameplay::game::server_side::native_hooks_server::block_entity_tick::{
    Loan, ResidentBlockEntityAccess,
};
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_gameplay_common::game::TryIntoApi;
use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_level::block_entities::Pos;
use sunrise_level::containers::BlockEntity;
use sunrise_level::{
    chunk::Chunk,
    containers::{ModBlockEntity, ModBlockEntityFields, ModContainer},
};
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::registry::BlockStateId;
use {
    sunrise_gameplay::game::native_entity_refs::tests::bind_local_player,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::native_entity_refs::tests::with_game_at,
};

pub(crate) const POS: Pos = (1, 64, 1);
pub(crate) const CHUNK: (i32, i32) = (0, 0);
pub(crate) const AT: [i32; 3] = [POS.0, POS.1, POS.2];
pub(crate) const KIND: &str = "view_tick:subscribed";
pub(crate) const UNSUBSCRIBED: &str = "view_tick:unsubscribed";
pub(crate) const MACHINE: &str = "view_tick:machine";
pub(crate) const CONTAINER_MACHINE: &str = "view_tick:container_machine";
pub(crate) const UNSUBSCRIBED_MACHINE: &str = "view_tick:unsubscribed_machine";

pub(crate) fn row(kind: &str, value: i32) -> BlockEntity {
    BlockEntity::Mod(Box::new(ModBlockEntity::new(
        kind.into(),
        ModBlockEntityFields {
            data: vec![value],
            ..Default::default()
        },
    )))
}

pub(crate) fn place_subject(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    kind: &str,
) -> BlockEntityRef {
    game.world
        .chunks
        .insert(CHUNK, Arc::new(Chunk::empty(0, 0)));
    let block_id = if kind == UNSUBSCRIBED {
        UNSUBSCRIBED_MACHINE
    } else {
        MACHINE
    };
    let block = game.pack.assets.registry.default_state(block_id).unwrap();
    game.world.set(POS.0, POS.1, POS.2, block).unwrap();
    game.world.block_entities.insert(POS, row(kind, 23));
    game.world.modified.clear();
    let level = game.level(game.dim.id()).unwrap();
    game.block_entity(level, [POS.0, POS.1, POS.2]).unwrap()
}

pub(crate) fn access<'a, 'g>(
    game: &'a mut sunrise_gameplay::game::ServerGame<'g>,
    subject: BlockEntityRef,
) -> ResidentBlockEntityAccess<'a, 'g> {
    ResidentBlockEntityAccess::new(game, Some(subject), Loan::Tick(None))
}

pub(crate) fn enable_block_ticks(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    game.player = Some(
        sunrise_sim::motion::PlayerMotion::new(
            game.motion,
            sunrise_core::geometry::Vec3::new(8.5, 65.0, 8.5),
        )
        .unwrap(),
    );
    game.update_chunk_tickets();
}

#[test]
#[ignore = "daily"]
fn retaking_the_view_checks_the_instance_and_state() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::retaking_the_view_checks_the_instance_and_state";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_RETAKE", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        // Enter through the real dispatched Part, then change the block through its Host.
        game.options.dedicated = Some(Vec::new());
        place_subject(game, KIND);
        enable_block_ticks(game);
        *effects.retake.lock().unwrap() = Retake::Requested;
        game.tick_block_entities_with(&mut ()).unwrap();
        assert_eq!(
            *effects.retake.lock().unwrap(),
            Retake::Observed {
                before: 23,
                after: None,
            }
        );

        // Raw writes can also leave the row present on an incompatible block, and
        // replacing it at the same position must not resurrect the original loan.
        let subject = place_subject(game, KIND);
        let unlit = game.world.block(POS.0, POS.1, POS.2);
        let lit = sunrise_gameplay_common::game::native_hooks::with_property(
            game.pack.assets.registry,
            unlit,
            "lit",
            "true",
        )
        .unwrap();
        assert_ne!(lit, unlit);
        let unlit_api = unlit.try_into_api().unwrap();
        let lit_api = lit.try_into_api().unwrap();
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        let mut access = access(game, subject);
        let mut scope = BlockEntityScope::new(&mut access);
        assert_eq!(
            scope.with_view(|view| (view.data_slots()[0], view.state())),
            Some((23, unlit_api))
        );
        let host = scope.host();
        let level = subject.level(host).unwrap();
        let pos = subject.position(host).unwrap();
        access.game.world.set(pos[0], pos[1], pos[2], lit).unwrap();
        assert_eq!(
            BlockEntityScope::new(&mut access)
                .with_view(|view| (view.data_slots()[0], view.state())),
            Some((23, lit_api))
        );
        access
            .game
            .world
            .set(pos[0], pos[1], pos[2], glass)
            .unwrap();
        assert_eq!(access.game.block_entity(level, pos), Some(subject));
        assert_eq!(BlockEntityScope::new(&mut access).with_view(|_| ()), None);
        access.game.world.set(pos[0], pos[1], pos[2], lit).unwrap();
        assert_eq!(
            BlockEntityScope::new(&mut access).with_view(|view| view.state()),
            Some(lit_api)
        );
        access.game.world.block_entities.remove(&POS);
        assert_eq!(BlockEntityScope::new(&mut access).with_view(|_| ()), None);
        access.game.world.block_entities.insert(POS, row(KIND, 99));
        assert_eq!(BlockEntityScope::new(&mut access).with_view(|_| ()), None);
    });
}

#[test]
#[ignore = "daily"]
fn read_only_and_noop_loans_do_not_journal_or_dirty() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::read_only_and_noop_loans_do_not_journal_or_dirty";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_NOOP", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let result: Result<(), ()> = game.storage_transaction(|game, _| {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert_eq!(view.data_slots(), [23]);
                    assert!(!view.set_data_slot(0, 23));
                    assert!(!view.set_data_slot(1, 24));
                    assert!(!view.set_data_slots(&[23]));
                })
                .unwrap();
            assert_eq!(game.journal_kept(), (0, 0));
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert!(game.world.modified.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_scope_transaction_aborts_subject_and_dropped_item_edits_together() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::a_scope_transaction_aborts_subject_and_dropped_item_edits_together";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ABORT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        game.spawn_item(
            sunrise_items::items::ItemStack::new("minecraft:stone", 5),
            [1.0, 64.0, 1.0],
            [0.0; 3],
            0,
        );
        let drop_id = game.dropped[0].id;
        let item = game.item_entity(game.entity(drop_id).unwrap()).unwrap();
        let count = item.with_stack(game, |stack| stack.count()).unwrap();
        game.world.modified.clear();
        let mut access = access(game, subject);
        let mut scope = BlockEntityScope::new(&mut access);
        let result: Result<(), mod_api::TransactionError<()>> = scope.transaction(|scope, _| {
            scope
                .with_view(|mut view| {
                    assert!(view.set_data_slot(0, 24));
                    assert!(view.set_nbt("recipe".into(), Some(Tag::Int(6))));
                })
                .ok_or(())?;
            let host = scope.host();
            let rest = item
                .with_stack(host, |stack| stack.project())
                .flatten()
                .unwrap()
                .shrink(1);
            assert!(item.set_stack(host, rest).unwrap());
            assert_eq!(
                item.with_stack(host, |stack| stack.count()),
                Some(count - 1)
            );
            scope
                .with_view(|view| assert_eq!(view.data_slots(), [24]))
                .ok_or(())?;
            Err(())
        });
        assert_eq!(result, Err(mod_api::TransactionError::Aborted(())));
        scope
            .with_view(|view| {
                assert_eq!(view.data_slots(), [23]);
                assert!(view.nbt().is_empty());
            })
            .unwrap();
        assert_eq!(
            item.with_stack(scope.host(), |stack| stack.count()),
            Some(count)
        );
    });
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) enum Retake {
    #[default]
    Off,
    Requested,
    Observed {
        before: i32,
        after: Option<i32>,
    },
}

#[test]
#[ignore = "daily"]
fn aborted_block_entity_edits_preserve_a_queued_client_update() {
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender};
    use {
        sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
        sunrise_gameplay::game::native_guest_turn_tests::with_guests_at,
    };

    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::aborted_block_entity_edits_preserve_a_queued_client_update";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_SYNC", &["run"]) {
        return;
    }

    const MACHINE: &str = "view_tick_update:machine";
    struct Machine;
    #[mod_api::__subscriptions]
    impl Hooks for Machine {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.block_entity_type("machine").blocks(&[MACHINE]).build();
        }

        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new(MACHINE, "minecraft:stone").with_item(false))
                .unwrap();
        }
    }

    let mods: mod_api::Mods = vec![ModEntry::new("view_tick_update", Box::new(Machine))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let root =
            sunrise_gameplay_common::game::mods::content_root(&fixture_root(), &Default::default())
                .unwrap();
        with_guests_at(root.prepared().to_owned(), |game| {
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            let kind = hub
                .registry_id("minecraft:block_entity_type", MACHINE)
                .unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            hub.update_guest(12, |guest| {
                guest.sender = PlayerSender::new(tx, Arc::default());
                guest.sent_chunks.insert(CHUNK);
            })
            .unwrap();
            let received_tags = || {
                rx.try_iter()
                    .filter_map(|out| {
                        let Out::Packet(body) = out else {
                            return None;
                        };
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        let Clientbound::BlockEntityData(packet) =
                            Clientbound::decode(id, reader.rest()).unwrap()
                        else {
                            return None;
                        };
                        assert_eq!(packet.pos, AT);
                        assert_eq!(packet.kind, kind);
                        Some(packet.tag)
                    })
                    .collect::<Vec<_>>()
            };

            let registry = game.pack.assets.registry;
            let machine = registry.default_state(MACHINE).unwrap();
            let replacement = registry
                .default_state("minecraft:glass")
                .unwrap()
                .try_into_api()
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            for replace in [false, true] {
                game.world
                    .chunks
                    .insert(CHUNK, Arc::new(Chunk::empty(0, 0)));
                game.world.set(POS.0, POS.1, POS.2, machine).unwrap();
                game.world.block_entities.insert(POS, row(MACHINE, 23));
                // Discard setup changes; the first broadcast must use the sync queued below.
                game.world.block_log.as_mut().unwrap().clear();
                let subject = game.block_entity(level, AT).unwrap();
                let waiting = Compound::from([("effect".into(), Tag::Int(7))]);
                let mut tick = access(game, subject);
                let mut scope = BlockEntityScope::new(&mut tick);
                scope
                    .with_view(|mut view| view.set_client_update_fields(waiting.clone()))
                    .unwrap();
                assert_eq!(subject.sync(scope.host()), Ok(()));

                let result: Result<(), mod_api::TransactionError<()>> =
                    scope.transaction(|scope, _| {
                        if replace {
                            // The first write captures the row through the world, not a ticker loan.
                            assert_eq!(
                                level.set_block(
                                    scope.host(),
                                    AT,
                                    replacement,
                                    BlockUpdateFlags::KNOWN_SHAPE
                                ),
                                Ok(true)
                            );
                            assert_eq!(scope.with_view(|_| ()), None);
                        } else {
                            scope
                                .with_view(|mut view| {
                                    view.set_client_update_fields(Compound::from([(
                                        "effect".into(),
                                        Tag::Int(9),
                                    )]));
                                })
                                .unwrap();
                        }
                        Err(())
                    });
                assert_eq!(result, Err(mod_api::TransactionError::Aborted(())));

                let (common, server) =
                    sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .broadcast_changes(&mut common.world);
                assert_eq!(
                    received_tags(),
                    [Tag::Compound(waiting)],
                    "replace={replace}"
                );

                // A later sync of the restored entity must not replay either batch.
                assert_eq!(subject.sync(game), Ok(()));
                let (common, server) =
                    sunrise_gameplay_common::game::SideGame::common_and_state_mut(game);
                server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .broadcast_changes(&mut common.world);
                assert!(received_tags().is_empty(), "replace={replace}");
            }
        });
    });
}

pub(crate) fn check_edit_then_replace(commit: bool, observer: Option<ModEntry>) {
    with_ticker_mods(observer, |game, _| {
        let subject = place_subject(game, KIND);
        let original = game.world.block(POS.0, POS.1, POS.2);
        let replacement = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        let replacement_state = replacement.try_into_api().unwrap();
        game.world.block_entities.set_mod_containers(
            [(
                original.0,
                Arc::new(ModContainer {
                    slots: 2,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        let taken = ItemStack::new("minecraft:diamond", 1);
        let left = ItemStack::new("minecraft:iron_ingot", 1);
        let BlockEntity::Mod(row) = game.world.block_entities.get_mut(&POS).unwrap() else {
            panic!("expected a mod container");
        };
        row.items = vec![Some(taken.clone()), Some(left.clone())];
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        bind_local_player(game, 20, 123);
        let player = game.player(123).unwrap();
        game.selected = 0;
        assert!(game.inventory[0].is_none());

        let mut access = access(game, subject);
        let mut scope = BlockEntityScope::new(&mut access);
        let result = scope.transaction(|scope, _| {
            let (level, pos, stack) = scope
                .with_view(|mut view| {
                    let stack = view.item_slot(0).unwrap().to_owned().unwrap();
                    assert_eq!(view.set_item_slot(0, None), Ok(true));
                    (view.level(), view.position(), stack)
                })
                .unwrap();
            let host = scope.host();
            player.set_inventory_slot(host, 0, Some(stack)).unwrap();
            // The child keeps the row it removed; the parent has the earlier snapshot.
            host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(
                        host,
                        pos,
                        replacement_state,
                        BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::KNOWN_SHAPE
                    ),
                    Ok(true)
                );
                Ok::<(), ()>(())
            })
            .unwrap();
            assert_eq!(scope.with_view(|_| ()), None);
            if commit { Ok(()) } else { Err(()) }
        });
        assert_eq!(
            result,
            commit
                .then_some(())
                .ok_or(mod_api::TransactionError::Aborted(()))
        );
        assert_eq!(
            scope.with_view(|view| view.handle()),
            (!commit).then_some(subject)
        );
        game.flush_spill();

        if commit {
            assert_eq!(game.world.block(POS.0, POS.1, POS.2), replacement);
            assert!(game.world.block_entities.get(&POS).is_none());
            assert_eq!(game.inventory[0], Some(taken));
            let [drop] = game.dropped.as_ref() else {
                panic!("expected exactly the item left in the replaced container");
            };
            assert_eq!(drop.stack, left);
        } else {
            assert_eq!(game.world.block(POS.0, POS.1, POS.2), original);
            assert_eq!(
                game.world.block_entities.get(&POS).unwrap().slots(),
                [Some(taken), Some(left)]
            );
            assert!(game.inventory[0].is_none());
            assert!(game.dropped.is_empty());
        }
    });
}

#[test]
#[ignore = "daily"]
fn edit_then_replace_commit_spills_only_the_remaining_contents() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::edit_then_replace_commit_spills_only_the_remaining_contents";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_REMOVAL", &["run"]) {
        return;
    }
    #[derive(Debug, PartialEq)]
    enum Seen {
        Join { row_present: bool },
        Spawn,
        Neighbor { row_present: bool },
    }
    struct Listener(Arc<Mutex<Vec<Seen>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item" {
                let level = host.level("minecraft:overworld").unwrap();
                self.0.lock().unwrap().push(Seen::Join {
                    row_present: host.block_entity(level, AT).is_some(),
                });
            }
            Flow::Continue
        }
        fn item_spawn(&self, _: &mut dyn ServerHost, _: &mut ItemSpawnEvent<'_>) {
            self.0.lock().unwrap().push(Seen::Spawn);
        }
        fn neighbor_notify(&self, host: &dyn LevelHost, e: &mut NeighborNotifyEvent) -> Flow {
            if e.pos == mod_api::BlockPos::new(POS.0, POS.1, POS.2) {
                self.0.lock().unwrap().push(Seen::Neighbor {
                    row_present: host.block_entity(AT).is_some(),
                });
            }
            Flow::Continue
        }
    }
    let observed = Arc::new(Mutex::new(Vec::new()));
    let observer = ModEntry::new("removal_order", Box::new(Listener(observed.clone())));
    check_edit_then_replace(true, Some(observer));
    assert_eq!(
        *observed.lock().unwrap(),
        [
            Seen::Join { row_present: true },
            Seen::Spawn,
            Seen::Neighbor { row_present: false },
        ]
    );
}

#[test]
#[ignore = "daily"]
fn edit_then_replace_abort_restores_the_original_contents() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::edit_then_replace_abort_restores_the_original_contents";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_REMOVAL_ABORT", &["run"]) {
        return;
    }
    check_edit_then_replace(false, None);
}

#[test]
#[ignore = "daily"]
fn reentrant_removal_does_not_spill_the_same_saved_pot_twice() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::reentrant_removal_does_not_spill_the_same_saved_pot_twice";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_REENTRANT_REMOVAL", &["run"]) {
        return;
    }

    struct Listener(std::sync::atomic::AtomicBool);
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item" && !self.0.swap(true, Ordering::Relaxed) {
                let level = host.level("minecraft:overworld").unwrap();
                // Re-place a pot and remove it while the first pot is spilling its contents.
                let pot = level.block_state(host, [2, 64, 1]).unwrap();
                assert_eq!(
                    level.set_block(host, AT, pot, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                assert!(level.remove_block(host, AT, false));
            }
            Flow::Continue
        }
    }

    let mods: mod_api::Mods = vec![ModEntry::new(
        "removal_reentry",
        Box::new(Listener(false.into())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            use sunrise_core::nbt::Tag;

            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert(CHUNK, Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let pot = registry.default_state("minecraft:decorated_pot").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(POS.0, POS.1, POS.2, pot).unwrap();
            game.world.set(2, 64, 1, pot).unwrap();
            let stock = ItemStack::new("minecraft:diamond", 64);
            let item = sunrise_sim::save::item_to_nbt(&stock, game.res.items());
            sunrise_gameplay_common::game::spill_common::put_saved_entity(
                &mut game.world,
                AT,
                Tag::compound([
                    ("id", Tag::String("minecraft:decorated_pot".into())),
                    ("x", Tag::Int(POS.0)),
                    ("y", Tag::Int(POS.1)),
                    ("z", Tag::Int(POS.2)),
                    ("item", item),
                ]),
            );
            let level = game.level("minecraft:overworld").unwrap();
            game.storage_scope(|game, _| {
                assert_eq!(
                    game.level_set_block(level, AT, stone, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                ((), true)
            });
            game.flush_spill();

            assert_eq!(game.world.block(POS.0, POS.1, POS.2), game.world.air);
            assert!(crate::game::spill_common::saved_entity(&game.world, AT).is_none());
            assert!(game.dropped.len() > 1);
            assert!(
                game.dropped
                    .iter()
                    .all(|drop| drop.stack.item == stock.item)
            );
            let spilled: u32 = game.dropped.iter().map(|drop| drop.stack.count).sum();
            assert_eq!(spilled, 64);
        });
    });
}

#[test]
#[ignore = "daily"]
fn spill_join_keeps_the_replacement_furnace_and_its_contents() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::spill_join_keeps_the_replacement_furnace_and_its_contents";
    const CASE: &str = "SUNRISE_RESIDENT_VIEW_REPLACEMENT_FURNACE";
    if run_fresh_cases(NAME, CASE, &["stone", "furnace"]) {
        return;
    }
    let outer_requested_furnace = std::env::var(CASE).unwrap() == "furnace";

    const LABEL: &str = "Restocked furnace";
    struct Listener {
        replacement: Arc<Mutex<Option<BlockEntityRef>>>,
        lock: Compound,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_join_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.kind == "minecraft:item" {
                let level = host.level("minecraft:overworld").unwrap();
                let furnace = host.block_state_id("minecraft:furnace").unwrap();
                if level.block_state(host, AT) == Some(furnace) {
                    // Leave and then restore the state the outer write is setting.
                    let stone = host.block_state_id("minecraft:stone").unwrap();
                    assert_eq!(
                        level.set_block(host, AT, stone, BlockUpdateFlags::NONE),
                        Ok(true)
                    );
                }
                assert_eq!(
                    level.set_block(host, AT, furnace, BlockUpdateFlags::NONE),
                    Ok(true)
                );
                let replacement = host.block_entity(level, AT).unwrap();
                let storage = level
                    .capability(host, &STORAGE, AT, Some(Direction::Up))
                    .unwrap();
                let stock = ItemStack::new("minecraft:raw_iron", 1)
                    .try_into_api()
                    .unwrap();
                host.transaction(|host, tx| {
                    assert_eq!(
                        storage.insert(host, tx, &ItemResource::of(&stock), 7),
                        Ok(7)
                    );
                    assert_eq!(replacement.set_name(host, Some(LABEL)), Ok(true));
                    assert_eq!(
                        replacement.set_lock(host, Some(self.lock.clone())),
                        Ok(true)
                    );
                    Ok::<_, ()>(())
                })
                .unwrap();
                *self.replacement.lock().unwrap() = Some(replacement);
            }
            Flow::Continue
        }
    }

    let replacement = Arc::new(Mutex::new(None));
    let lock = Compound::from([("items".into(), Tag::String("minecraft:stick".into()))]);
    let mods = vec![ModEntry::new(
        "removal_replacement",
        Box::new(Listener {
            replacement: replacement.clone(),
            lock: lock.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert(CHUNK, Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let furnace = registry.default_state("minecraft:furnace").unwrap();
            game.world.set(POS.0, POS.1, POS.2, chest).unwrap();
            let spilled = ItemStack::new("minecraft:diamond", 1);
            let mut row = BlockEntity::for_block("minecraft:chest").unwrap();
            row.slots_mut()[0] = Some(spilled.clone());
            game.world.block_entities.insert(POS, row);
            let level = game.level("minecraft:overworld").unwrap();
            let original = game.block_entity(level, AT).unwrap();
            let chest_lock =
                Compound::from([("items".into(), Tag::String("minecraft:apple".into()))]);
            assert_eq!(original.set_lock(game, Some(chest_lock)), Ok(true));

            // With stone in place during the spill, the listener installs the furnace
            // over the still-resident chest row and its saved tag.
            let requested = if outer_requested_furnace {
                furnace
            } else {
                registry.default_state("minecraft:stone").unwrap()
            };
            assert_eq!(
                game.level_set_block(level, AT, requested, BlockUpdateFlags::NONE),
                Ok(outer_requested_furnace)
            );

            let replacement = replacement
                .lock()
                .unwrap()
                .expect("listener installed a furnace");
            assert_ne!(replacement, original);
            assert_eq!(game.block_entity(level, AT), Some(replacement));
            assert_eq!(game.world.block(POS.0, POS.1, POS.2), furnace);
            let row = game.world.block_entities.get(&POS).unwrap();
            assert!(matches!(row, BlockEntity::Furnace(_)));
            assert_eq!(
                row.slots(),
                [Some(ItemStack::new("minecraft:raw_iron", 7)), None, None]
            );
            assert_eq!(replacement.name(game).as_deref(), Some(LABEL));
            assert_eq!(replacement.lock(game), Some(&lock));
            let [spilled_drop] = game.dropped.as_ref() else {
                panic!("expected exactly one drop, from the original chest");
            };
            assert_eq!(spilled_drop.stack, spilled);

            let scratch = tempfile::tempdir().unwrap();
            let save_root = scratch.path().to_owned();
            let shape = game.world.shape;
            let save = sunrise_sim::save::SaveDir::new(
                save_root.clone(),
                shape,
                Some(game.res.shared_items()),
            );
            let chunks = game.with_entities([game.world.chunks.get(&CHUNK).unwrap()]);
            save.save(&chunks, registry, game.res.biomes()).unwrap();
            drop(save);

            let mut loaded = sunrise_sim::save::SaveDir::new(save_root, shape, None)
                .load(CHUNK.0, CHUNK.1, registry, game.res.biomes())
                .unwrap();
            game.world.install_chunk_block_entities(&mut loaded);
            game.world.chunks.insert(CHUNK, Arc::new(loaded));
            let reloaded = game.block_entity(level, AT).unwrap();
            assert_eq!(reloaded.lock(game), Some(&lock));
        });
    });
}

#[test]
#[ignore = "daily"]
fn strict_bulk_replacement_aborts_the_block_handle_and_inventory_together() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::strict_bulk_replacement_aborts_the_block_handle_and_inventory_together";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_STRICT_ABORT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        game.options.dedicated = Some(Vec::new());
        let subject = place_subject(game, KIND);
        let original = game.world.block(POS.0, POS.1, POS.2);
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        game.world.block_entities.set_mod_containers(
            [(
                original.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        let stack = ItemStack::new("minecraft:diamond", 7);
        let BlockEntity::Mod(row) = game.world.block_entities.get_mut(&POS).unwrap() else {
            panic!("expected a mod container");
        };
        row.items = vec![Some(stack.clone())];
        let edit = BlockEdit {
            palette: vec![chest.try_into_api().unwrap()],
            filters: Vec::new(),
            runs: vec![BlockRun {
                x: POS.0,
                y: POS.1,
                z: POS.2,
                height: 1,
                surface: None,
                block: 0,
                only: None,
            }],
        };
        game.world.block_log = Some(Vec::new());
        let mut access = access(game, subject);
        let mut scope = BlockEntityScope::new(&mut access);
        let result: Result<(), mod_api::TransactionError<()>> = scope.transaction(|tick, _| {
            tick.with_view(|mut view| assert_eq!(view.set_item_slot(0, None), Ok(true)))
                .unwrap();
            let host = tick.host();
            let level = host.level("minecraft:overworld").unwrap();
            assert_eq!(level.edit_blocks(host, &edit), Ok(1));
            assert_ne!(host.block_entity(level, AT), Some(subject));
            assert!(host.block_entity(level, AT).is_some());
            Err(())
        });
        assert_eq!(result, Err(mod_api::TransactionError::Aborted(())));
        let level = game.level("minecraft:overworld").unwrap();
        assert_eq!(game.world.block(POS.0, POS.1, POS.2), original);
        assert_eq!(game.block_entity(level, AT), Some(subject));
        assert_eq!(
            game.world.block_entities.get(&POS).unwrap().slots(),
            [Some(stack)]
        );
        assert!(game.dropped.is_empty());
        assert!(game.world.bulk_log.is_empty());
        assert!(game.world.modified.is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn strict_bulk_commit_publishes_whole_chunks_without_drops() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::strict_bulk_commit_publishes_whole_chunks_without_drops";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_STRICT_COMMIT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        game.options.dedicated = Some(Vec::new());
        let subject = place_subject(game, KIND);
        let chest = game
            .pack
            .assets
            .registry
            .default_state("minecraft:chest")
            .unwrap();
        let edit = BlockEdit {
            palette: vec![chest.try_into_api().unwrap()],
            filters: Vec::new(),
            runs: vec![BlockRun {
                x: POS.0,
                y: POS.1,
                z: POS.2,
                height: 1,
                surface: None,
                block: 0,
                only: None,
            }],
        };
        game.world.block_log = Some(Vec::new());
        let level = game.level("minecraft:overworld").unwrap();
        game.storage_scope(|game, _| {
            assert_eq!(level.edit_blocks(game, &edit), Ok(1));
            assert!(game.world.bulk_log.is_empty());
            ((), true)
        });
        assert_eq!(game.world.block(POS.0, POS.1, POS.2), chest);
        let resident = game.block_entity(level, AT).unwrap();
        assert_ne!(resident, subject);
        assert!(matches!(
            game.world.block_entities.get(&POS),
            Some(BlockEntity::Chest(_))
        ));
        assert!(game.dropped.is_empty());
        assert!(game.world.bulk_log.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_replacement_between_strict_edits_uses_its_own_resident_row() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::ordinary_replacement_between_strict_edits_uses_its_own_resident_row";
    const CASE: &str = "SUNRISE_RESIDENT_STRICT_ORDER";
    if run_fresh_cases(NAME, CASE, &["ordinary_first", "strict_first"]) {
        return;
    }
    let strict_first = std::env::var(CASE).as_deref() == Ok("strict_first");
    with_ticker(|game, _| {
        game.options.dedicated = Some(Vec::new());
        let subject = place_subject(game, KIND);
        let original = game.world.block(POS.0, POS.1, POS.2);
        let registry = game.pack.assets.registry;
        let chest = registry.default_state("minecraft:chest").unwrap();
        let glass = registry.default_state("minecraft:glass").unwrap();
        let level = game.level("minecraft:overworld").unwrap();
        let edit = BlockEdit {
            palette: vec![chest.try_into_api().unwrap()],
            filters: Vec::new(),
            runs: vec![BlockRun {
                x: POS.0,
                y: POS.1,
                z: POS.2,
                height: 1,
                surface: None,
                block: 0,
                only: None,
            }],
        };
        game.world.block_entities.set_mod_containers(
            [(
                original.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        let original_stack = ItemStack::new("minecraft:diamond", 1);
        let chest_stack = ItemStack::new("minecraft:iron_ingot", 1);
        let BlockEntity::Mod(row) = game.world.block_entities.get_mut(&POS).unwrap() else {
            panic!("expected mod container");
        };
        row.items = vec![Some(original_stack.clone())];
        game.world.block_log = Some(Vec::new());

        let last_chest = game.storage_scope(|game, _| {
            if strict_first {
                assert_eq!(level.edit_blocks(game, &edit), Ok(1));
                game.world.block_entities.get_mut(&POS).unwrap().slots_mut()[0] =
                    Some(chest_stack.clone());
            }
            assert_eq!(
                game.level_set_block(level, AT, glass, BlockUpdateFlags::KNOWN_SHAPE),
                Ok(true)
            );
            assert_eq!(level.edit_blocks(game, &edit), Ok(1));
            let last_chest = game.block_entity(level, AT).unwrap();
            assert_ne!(last_chest, subject);
            (last_chest, true)
        });

        assert_eq!(game.world.block(POS.0, POS.1, POS.2), chest);
        assert_eq!(game.block_entity(level, AT), Some(last_chest));
        assert!(
            game.world
                .block_entities
                .get(&POS)
                .unwrap()
                .slots()
                .iter()
                .all(Option::is_none)
        );
        assert!(game.world.bulk_log.contains(&CHUNK));
        let [drop] = game.dropped.as_ref() else {
            panic!("one ordinary spill, none from strict")
        };
        assert_eq!(
            drop.stack,
            if strict_first {
                chest_stack
            } else {
                original_stack
            }
        );
    });
}

#[derive(Clone, Default)]
pub(crate) struct Effects {
    pub(crate) notices: Arc<AtomicU32>,
    pub(crate) ticks: Arc<AtomicU32>,
    pub(crate) loaded_items: Arc<Mutex<Option<Tag>>>,
    pub(crate) saved_items: Arc<Mutex<Option<Tag>>>,
    pub(crate) retake: Arc<Mutex<Retake>>,
}
#[mod_api::sunrise_part]
impl BlockEntityPart for Effects {
    fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        if *self.retake.lock().unwrap() == Retake::Requested {
            let (handle, before) = event
                .subject
                .with_view(|view| (view.handle(), view.data_slots()[0]))
                .unwrap();
            let host = event.subject.host();
            let level = handle.level(host).unwrap();
            let pos = handle.position(host).unwrap();
            let glass = host.block_state_id("minecraft:glass").unwrap();
            assert_eq!(
                level.set_block(host, pos, glass, mod_api::BlockUpdateFlags::NONE),
                Ok(true)
            );
            let after = event.subject.with_view(|view| view.data_slots()[0]);
            *self.retake.lock().unwrap() = Retake::Observed { before, after };
        } else {
            let _ = event.subject.with_view(|mut view| view.set_data_slot(0, 1));
        }
    }

    fn block_entity_load_additional(&self, event: &mut BlockEntityLoadAdditionalEvent<'_>) {
        *self.loaded_items.lock().unwrap() = event.tag.get(&"Items".into()).cloned();
    }

    fn block_entity_save_additional(
        &self,
        _: &dyn LevelHost,
        event: &mut BlockEntitySaveAdditionalEvent<'_>,
    ) {
        *self.saved_items.lock().unwrap() = event.tag.get(&"Items".into()).cloned();
    }
}
#[mod_api::sunrise_part]
impl BlockPart for Effects {
    fn contents_changed(&self, _: &mut dyn ServerHost, event: &mut mod_api::ContentsChangedEvent) {
        if event.pos == mod_api::BlockPos::new(POS.0, POS.1, POS.2) {
            self.notices.fetch_add(1, Ordering::Relaxed);
        }
    }
}
#[mod_api::__subscriptions]
impl Hooks for Effects {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r
            .block_entity_type("subscribed")
            .blocks(&[MACHINE, CONTAINER_MACHINE])
            .build();
        r.block_entity_part(&kind, self.clone()).unwrap();
        r.block_entity_type("unsubscribed")
            .blocks(&[UNSUBSCRIBED_MACHINE])
            .build();
        let container_machine = r.block_key(CONTAINER_MACHINE);
        r.block_container(
            &container_machine,
            mod_api::ModContainer {
                slots: 1,
                ..Default::default()
            },
        );
    }

    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new(MACHINE, "minecraft:furnace").with_item(false))
            .unwrap();
        event
            .register(mod_api::Block::new(CONTAINER_MACHINE, "minecraft:furnace").with_item(false))
            .unwrap();
        event
            .register(mod_api::Block::new(UNSUBSCRIBED_MACHINE, "minecraft:stone").with_item(false))
            .unwrap();
    }
}

pub(crate) fn with_ticker(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Effects),
) {
    with_ticker_mods(None, test);
}

pub(crate) fn with_ticker_mods(
    observer: Option<ModEntry>,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, &Effects),
) {
    let effects = Effects::default();
    let mut entries = vec![ModEntry::new("view_tick", Box::new(effects.clone()))];
    entries.extend(observer);
    let mods: mod_api::Mods = entries.into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        let base = sunrise_gameplay::game::native_entity_refs::tests::fixture_root();
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();
        with_game_at(root.prepared().to_owned(), |game| test(game, &effects));
    });
}

#[test]
#[ignore = "daily"]
fn an_invalid_resident_block_entity_is_not_dispatched() {
    use {
        sunrise_gameplay::game::block_entity_ticks::BlockEntityTick,
        sunrise_gameplay::game::block_entity_ticks::BlockEntityTickSlot,
        sunrise_gameplay_common::game::published_loops::LoopFlow,
    };

    struct Invalidate {
        glass: BlockStateId,
        visited: bool,
    }
    impl BlockEntityTickSlot for Invalidate {
        fn before(&mut self, tick: &mut BlockEntityTick<'_, '_>) -> LoopFlow {
            assert_eq!(tick.pos, POS);
            self.visited = true;
            // This before-slot's raw write keeps the row and its ticker live.
            tick.game
                .world
                .set(POS.0, POS.1, POS.2, self.glass)
                .unwrap();
            LoopFlow::Continue
        }
    }

    #[derive(Default)]
    struct ObserveAfter {
        visited: Vec<Pos>,
    }
    impl BlockEntityTickSlot for ObserveAfter {
        fn after(&mut self, tick: &mut BlockEntityTick<'_, '_>) {
            self.visited.push(tick.pos);
        }
    }

    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::an_invalid_resident_block_entity_is_not_dispatched";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_INVALID_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, KIND);
        let valid = game.world.block(POS.0, POS.1, POS.2);
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        enable_block_ticks(game);
        let mut slots = (
            Invalidate {
                glass,
                visited: false,
            },
            ObserveAfter::default(),
        );
        game.tick_block_entities_with(&mut slots).unwrap();
        assert!(slots.0.visited);
        assert!(slots.1.visited.is_empty());
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 0);
        assert_eq!(subject.data_slots(game).unwrap(), [23]);

        game.world.set(POS.0, POS.1, POS.2, valid).unwrap();
        game.tick_block_entities_with(&mut slots.1).unwrap();
        assert_eq!(slots.1.visited, [POS]);
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 1);
        assert_eq!(subject.data_slots(game).unwrap(), [1]);
    });
}

#[test]
#[ignore = "daily"]
fn invalid_resident_ticker_warns_once_until_a_valid_tick() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::invalid_resident_ticker_warns_once_until_a_valid_tick";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_TICK_WARNING", &["run"]) {
        return;
    }

    struct Capture;
    static WARNINGS: AtomicU32 = AtomicU32::new(0);
    impl log::Log for Capture {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() == log::Level::Warn
                && metadata
                    .target()
                    .ends_with("::native_hooks::block_entity_tick")
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                WARNINGS.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn flush(&self) {}
    }
    log::set_logger(&Capture).unwrap();
    log::set_max_level(log::LevelFilter::Warn);

    with_ticker(|game, effects| {
        place_subject(game, KIND);
        enable_block_ticks(game);
        let valid = game.world.block(POS.0, POS.1, POS.2);
        let glass = game
            .pack
            .assets
            .registry
            .default_state("minecraft:glass")
            .unwrap();
        // Bind the valid ticker before leaving an incompatible block under it.
        game.tick_block_entities_with(&mut ()).unwrap();
        for (state, warnings, ticks) in [
            (glass, 1, 1),
            (glass, 1, 1),
            (valid, 1, 2),
            (glass, 2, 2),
            (glass, 2, 2),
        ] {
            game.world.set(POS.0, POS.1, POS.2, state).unwrap();
            game.tick_block_entities_with(&mut ()).unwrap();
            assert_eq!(WARNINGS.load(Ordering::Relaxed), warnings);
            assert_eq!(effects.ticks.load(Ordering::Relaxed), ticks);
        }
    });
}

#[test]
#[ignore = "daily"]
fn ticker_and_handle_setters_require_explicit_set_changed() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::ticker_and_handle_setters_require_explicit_set_changed";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let notices = &effects.notices;
        let subject = place_subject(game, KIND);
        let machine_state = game.world.block(POS.0, POS.1, POS.2);
        game.world.block_entities.set_mod_containers(
            [(
                machine_state.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        game.world.block_log = Some(Vec::new());
        game.tick_mod_block_entity(POS);
        assert_eq!(subject.data_slots(game).unwrap(), [1]);
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        let stack = sunrise_items::items::ItemStack::new("minecraft:stone", 2)
            .try_into_api()
            .unwrap();
        {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_data_slots(&[10, 20]));
                    assert!(view.set_data_slot(1, 21));
                    assert!(view.set_nbt("recipe".into(), Some(Tag::Int(7))));
                    assert_eq!(view.set_item_slot(0, Some(stack)), Ok(true));
                })
                .unwrap();
        }
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        assert!(subject.set_data_slot(game, 0, 2).unwrap());
        assert_eq!(subject.set_data_slots(game, vec![3, 4]), Ok(true));
        assert_eq!(subject.data_slots(game).unwrap(), [3, 4]);
        assert!(!game.world.modified.contains(&CHUNK));
        assert!(game.world.block_log.as_ref().unwrap().is_empty());
        assert_eq!(notices.load(Ordering::Relaxed), 0);

        subject.set_changed(game).unwrap();
        assert!(game.world.modified.contains(&CHUNK));
        assert_eq!(
            notices.load(Ordering::Relaxed),
            1,
            "explicit notice is wired"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_registered_type_without_a_tick_part_is_not_woken_or_changed() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::a_registered_type_without_a_tick_part_is_not_woken_or_changed";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_NO_TICK", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, UNSUBSCRIBED);
        game.world.block_entities.sleep(POS);
        game.tick_mod_block_entity(POS);
        assert_eq!(subject.data_slots(game).unwrap(), [23]);
        assert!(game.world.block_entities.is_asleep(&POS));
        assert!(game.world.modified.is_empty());
        assert_eq!(effects.notices.load(Ordering::Relaxed), 0);
        assert_eq!(effects.ticks.load(Ordering::Relaxed), 0);
    });
}

#[test]
#[ignore = "daily"]
fn chunk_save_reloads_reordered_slotless_items() {
    use sunrise_sim::save;

    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::chunk_save_reloads_reordered_slotless_items";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ITEMS_ORDER", &["run"]) {
        return;
    }
    with_ticker(|game, effects| {
        let subject = place_subject(game, KIND);
        let item = |slot| {
            Tag::compound([
                ("Slot", Tag::Byte(slot)),
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
            ])
        };
        let initial = Tag::List(vec![item(1), item(0)]);
        let reordered = Tag::List(vec![item(0), item(1)]);
        let write_items = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           subject: BlockEntityRef,
                           items: Tag| {
            let mut access = access(game, subject);
            let mut scope = BlockEntityScope::new(&mut access);
            assert_eq!(
                scope.with_view(|mut view| view.set_nbt("Items".into(), Some(items))),
                Some(true)
            );
            subject.set_changed(scope.host()).unwrap();
        };
        let save_reload = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            let snapshots = game.with_entities([game.world.chunks.get(&CHUNK).unwrap()]);
            let registry = game.pack.assets.registry;
            let biomes = game.res.biomes();
            let encoded = save::encode(&snapshots[0], registry, biomes, game.res.items());
            save::decode(&encoded, registry, biomes).unwrap()
        };

        write_items(game, subject, initial.clone());
        let mut loaded = save_reload(game);
        assert_eq!(effects.saved_items.lock().unwrap().as_ref(), Some(&initial));
        let BlockEntity::Mod(entity) = &loaded.block_entities[0].1 else {
            panic!("expected slotless mod block entity");
        };
        assert!(entity.items.is_empty());
        assert_eq!(entity.nbt.get(&"Items".into()), Some(&initial));
        game.world.install_chunk_block_entities(&mut loaded);
        game.world.chunks.insert(CHUNK, Arc::new(loaded));
        game.world.modified.clear();
        let pos = [POS.0, POS.1, POS.2];
        let subject = game
            .block_entity(game.level(game.dim.id()).unwrap(), pos)
            .unwrap();
        game.fire_pending_block_entity_loads();
        assert_eq!(
            effects.loaded_items.lock().unwrap().as_ref(),
            Some(&initial)
        );
        assert_eq!(
            subject.data(game).unwrap().get(&"Items".into()),
            Some(&initial)
        );
        // Capture a save straight after loading as well: the hook must not sort the rows.
        *effects.saved_items.lock().unwrap() = None;
        let _ = save_reload(game);
        assert_eq!(effects.saved_items.lock().unwrap().as_ref(), Some(&initial));
        write_items(game, subject, reordered.clone());

        let reloaded = save_reload(game);
        let BlockEntity::Mod(entity) = &reloaded.block_entities[0].1 else {
            panic!("expected slotless mod block entity");
        };
        assert!(entity.items.is_empty());
        assert_eq!(entity.nbt.get(&"Items".into()), Some(&reordered));
    });
}

#[test]
#[ignore = "daily"]
fn set_nbt_items_follows_declared_slot_count_and_clears_decoded_slots() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::set_nbt_items_follows_declared_slot_count_and_clears_decoded_slots";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_VIEW_ITEMS_NBT", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let machine_state = game.world.block(POS.0, POS.1, POS.2);

        // When the block declares item slots (slot_count > 0), "Items" is engine-owned.
        game.world.block_entities.set_mod_containers(
            [(
                machine_state.0,
                Arc::new(ModContainer {
                    slots: 1,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(!view.set_nbt("Items".into(), Some(Tag::Int(1))));
                    assert!(view.nbt().get(&"Items".into()).is_none());
                })
                .unwrap();
        }

        // Without declared item slots (slot_count == 0), "Items" is ordinary NBT.
        // Seed disk-decoded `entity.items` to verify equality no-op and stale-slot clearing.
        game.world
            .block_entities
            .set_mod_containers(Default::default());
        let decoded = sunrise_items::items::ItemStack::new("minecraft:stone", 3);
        if let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&POS) {
            entity.items = vec![Some(decoded)];
        } else {
            panic!("mod entity missing");
        }
        let expected_decoded = Tag::List(vec![Tag::compound([
            ("Slot", Tag::Byte(0)),
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(3)),
        ])]);

        // Writing the equal encoded value is a no-op and does not journal.
        let result: Result<(), ()> = game.storage_transaction(|game, _| {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(!view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                })
                .unwrap();
            assert_eq!(game.journal_kept(), (0, 0));
            Ok(())
        });
        assert_eq!(result, Ok(()));

        // Mutating "Items" clears stale decoded `entity.items` and stores the new NBT.
        let replacement = Tag::List(vec![Tag::compound([(
            "custom",
            Tag::String("payload".into()),
        )])]);
        {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), Some(replacement.clone())));
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&replacement));
                    assert!(!view.set_nbt("Items".into(), Some(replacement.clone())));
                })
                .unwrap();
        }
        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&POS) else {
            panic!("mod entity missing");
        };
        assert!(
            entity.items.is_empty(),
            "stale decoded item slots must be cleared when slotless Items NBT changes"
        );
        let saved = sunrise_sim::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved.get("Items"), Some(&replacement));

        // Clearing "Items" removes it from both NBT and encoded output.
        {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), None));
                    assert!(view.nbt().get(&"Items".into()).is_none());
                    assert!(!view.set_nbt("Items".into(), None));
                })
                .unwrap();
        }
        let saved = sunrise_sim::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved.get("Items"), None);

        // Save/reload round-trip: canonical item-shaped "Items" stays verbatim in `nbt`
        // for slotless holders and is readable via `with_view`, while container-backed
        // holders still decode canonical entries into `entity.items`.
        {
            let mut access = access(game, subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert!(view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&expected_decoded));
                })
                .unwrap();
        }
        let saved_canonical = sunrise_sim::save::block_entity_encode::encode(
            POS,
            game.world.block_entities.get(&POS).unwrap(),
            MACHINE,
            game.res.items(),
        );
        assert_eq!(saved_canonical.get("Items"), Some(&expected_decoded));

        let (reloaded_pos, reloaded) =
            sunrise_sim::save::decode_block_entity_for_block(&saved_canonical, MACHINE)
                .expect("slotless entity reloads");
        assert_eq!(reloaded_pos, POS);
        let BlockEntity::Mod(reloaded_mod) = &reloaded else {
            panic!("expected mod block entity");
        };
        assert!(
            reloaded_mod.items.is_empty(),
            "slotless load must keep canonical Items in nbt, not entity.items"
        );
        assert_eq!(
            reloaded_mod.nbt.get(&"Items".into()),
            Some(&expected_decoded)
        );
        game.world.block_entities.insert(POS, reloaded);
        let level = game.level(game.dim.id()).unwrap();
        let reloaded_subject = game.block_entity(level, [POS.0, POS.1, POS.2]).unwrap();
        {
            let mut access = access(game, reloaded_subject);
            BlockEntityScope::new(&mut access)
                .with_view(|mut view| {
                    assert_eq!(view.nbt().get(&"Items".into()), Some(&expected_decoded));
                    assert!(!view.set_nbt("Items".into(), Some(expected_decoded.clone())));
                })
                .unwrap();
        }

        let (_, container_reloaded) =
            sunrise_sim::save::decode_block_entity_for_block(&saved_canonical, CONTAINER_MACHINE)
                .expect("container-backed entity reloads");
        let BlockEntity::Mod(container_mod) = &container_reloaded else {
            panic!("expected mod block entity");
        };
        assert_eq!(
            container_mod.items,
            vec![Some(sunrise_items::items::ItemStack::new(
                "minecraft:stone",
                3
            ))]
        );
        assert!(container_mod.nbt.get(&"Items".into()).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn blockless_scope_returns_none_for_view_and_keeps_host_and_transaction_working() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::blockless_scope_returns_none_for_view_and_keeps_host_and_transaction_working";
    if run_fresh_cases(NAME, "SUNRISE_BLOCKLESS_SCOPE", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let mut access = ResidentBlockEntityAccess::blockless(game);
        let mut scope = BlockEntityScope::new(&mut access);
        assert_eq!(scope.with_view(|_| 1u32), None);
        let aborted: Result<(), mod_api::TransactionError<()>> = scope.transaction(|scope, _| {
            assert_eq!(scope.with_view(|_| 1u32), None);
            assert!(subject.set_data_slot(scope.host(), 0, 50).unwrap());
            Err(())
        });
        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
        assert_eq!(subject.data_slots(scope.host()).unwrap(), [23]);
        let committed: Result<(), mod_api::TransactionError<()>> = scope.transaction(|scope, _| {
            assert!(subject.set_data_slot(scope.host(), 0, 55).unwrap());
            Ok(())
        });
        assert_eq!(committed, Ok(()));
        assert_eq!(subject.data_slots(scope.host()).unwrap(), [55]);
        assert!(!access.items_changed());
    });
}

#[test]
#[ignore = "daily"]
fn items_changed_tracks_committed_slot_writes_and_ignores_aborted_nested_transactions() {
    const NAME: &str = "cut_layout::game::server_side::native_hooks_server::block_entity_tick::tick_tests::items_changed_tracks_committed_slot_writes_and_ignores_aborted_nested_transactions";
    if run_fresh_cases(NAME, "SUNRISE_RESIDENT_ITEMS_CHANGED", &["run"]) {
        return;
    }
    with_ticker(|game, _| {
        let subject = place_subject(game, KIND);
        let (handle, state) = game.resident_block_entity_target(POS).unwrap();
        assert_eq!(handle, subject);
        game.world.block_entities.set_mod_containers(
            [(
                state.0,
                Arc::new(sunrise_level::containers::ModContainer {
                    slots: 2,
                    ..Default::default()
                }),
            )]
            .into_iter()
            .collect(),
        );
        let stone = sunrise_items::items::ItemStack::new("minecraft:stone", 1)
            .try_into_api()
            .unwrap();
        let mut access = ResidentBlockEntityAccess::for_handle(game, handle, state);
        {
            let mut scope = BlockEntityScope::new(&mut access);
            assert_eq!(
                scope
                    .with_view(|mut view| view.set_item_slot(0, None))
                    .unwrap(),
                Ok(false)
            );
        }
        assert!(!access.items_changed());

        {
            let mut scope = BlockEntityScope::new(&mut access);
            let aborted: Result<(), mod_api::TransactionError<()>> =
                scope.transaction(|scope, _| {
                    assert_eq!(
                        scope
                            .with_view(|mut view| view.set_item_slot(0, Some(stone.clone())))
                            .unwrap(),
                        Ok(true)
                    );
                    Err(())
                });
            assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
        }
        assert!(!access.items_changed());

        {
            let mut scope = BlockEntityScope::new(&mut access);
            let committed: Result<(), mod_api::TransactionError<()>> =
                scope.transaction(|scope, _| {
                    assert_eq!(
                        scope
                            .with_view(|mut view| view.set_item_slot(0, Some(stone.clone())))
                            .unwrap(),
                        Ok(true)
                    );
                    Ok(())
                });
            assert_eq!(committed, Ok(()));
            assert_eq!(
                scope
                    .with_view(|view| view.item_slot(0).map(|s| s.count()))
                    .unwrap(),
                Some(1)
            );
        }
        assert!(access.items_changed());
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_item_entity_refs::NativeItemEntityRefsGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
