//! `menu_clicked` runs directly on the receiving menu before default click execution.
//! `menu_button`, `menu_quick_move` and `menu_slot_take` lend live resident storage through
//! `BlockEntityScope`. The fixtures need prepared resources.
use mod_api::{
    BlockEntityPart, Flow, Hooks, HostBlockStates, HostTransaction, ITEMS, ItemPlace, ItemResource,
    MenuButtonEvent, MenuClickedEvent, MenuQuickMoveEvent, MenuSlotTakeEvent, ModEntry, Mods,
    Registrar, ServerHost,
};
use std::{
    cell::{Cell, RefCell},
    sync::Arc,
};
use sunrise_items::items::ItemStack;
use sunrise_level::containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer};
use sunrise_protocol::packets::play::{ContainerButtonClick, ContainerClick, Serverbound};
use sunrise_sim::lan::menu::Slots;
use {
    sunrise_gameplay::game::native_guest_turn_tests::register_test_menu,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
};

#[path = "mod_menu_decisions_tests/container_layout.rs"]
mod container_layout;

#[derive(Clone, Default)]
struct Script {
    /// Put dirt in guest slot 20 through the `Host`, in the listener's own nested transaction.
    write_dirt: bool,
    panic: bool,
    /// `menu_quick_move`: take 9 of the stack into slots 1 and 2 of the block (4, then 5).
    take: bool,
    all: bool,
    /// ...and answer with a stack past the item's maximum in slot 1.
    invalid: bool,
    /// `menu_quick_move`: move 4 of the stack into the second block through its `ITEMS`
    /// capability, leave 6 in `stack`, and return unhandled unless `handle_cross` is set.
    cross: bool,
    handle_cross: bool,
    /// Explicitly notify the destination, whose `contents_changed` Part inserts into itself.
    notify_destination: bool,
    /// Replace the destination with a clone of the source after committing the `ITEMS` move.
    replace_destination: bool,
    fail_take: bool,
    /// `menu_button`: replace the open menu through the host, returning its cursor to inventory.
    replace_menu: bool,
    /// `menu_button`: propose a cursor edit before replacing the menu, to ensure stale edits lose.
    change_cursor: bool,
    /// `menu_button`: consume the player's carried stack.
    clear_cursor: bool,
    /// `menu_button` or `menu_quick_move`: issue an explicit notice whose listener panics.
    panic_in_contents_changed: bool,
    /// `menu_button`: edit subject and world storage inside `subject.transaction(...)`, then panic.
    transactional_panic: bool,
    /// `menu_quick_move`: remove the backing block at `POS` through the journaled `Host` path.
    remove_backing: bool,
    /// `menu_quick_move`: return `Flow::Cancel` after `remove_backing` (with a remainder).
    handle_replace: bool,
    /// `menu_quick_move`: remove the backing block at `POS` and open a fresh menu through the `Host`.
    remove_and_reopen: bool,
    /// Put a one-shot update in the block entity (and edit data slots for `menu_button` or slots for `menu_quick_move`).
    write_update: bool,
    clicked: Option<Flow>,
    write_other_block: bool,
    write_clicked_block: bool,
    continue_after_player_write: bool,
    replace_display: bool,
    replace_other: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static TAKE: Cell<Option<(u32, u32)>> = const { Cell::new(None) };
    /// `menu_button` calls on the depot.
    static BUTTONS: Cell<usize> = const { Cell::new(0) };
    /// `menu_button` calls that received the depot's actual backing entity.
    static BUTTON_ENTITIES: Cell<usize> = const { Cell::new(0) };
    /// Optional positions delivered to menu-button listeners.
    static BUTTON_POSITIONS: RefCell<Vec<Option<[i32; 3]>>> = const { RefCell::new(Vec::new()) };
    /// Whether a transaction was open at each dispatch: the action's savepoint.
    static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    TAKE.set(None);
    sunrise_gameplay::game::server_side::decided_action::DECIDED.set(0);
    IN_ACTION.with(|a| a.borrow_mut().clear());
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn write_dirt_in_tx(host: &mut dyn ServerHost, tx: &mod_api::TransactionContext) {
    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 20,
    };
    assert!(!host.transaction_matches(None), "inside the action");
    assert_eq!(place.insert(host, tx, &ItemResource::of(&dirt), 2), Ok(2));
}

struct Part;
#[mod_api::sunrise_part]
impl mod_api::BlockPart for Part {
    fn contents_changed(&self, _: &mut dyn ServerHost, _: &mut mod_api::ContentsChangedEvent) {
        if SCRIPT.with(|s| s.borrow().panic_in_contents_changed) {
            panic!("the nested contents_changed listener failed");
        }
    }
}
#[mod_api::sunrise_part]
impl BlockEntityPart for Part {
    fn menu_quick_move(&self, event: &mut MenuQuickMoveEvent<'_>) -> Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| {
            a.borrow_mut()
                .push(!event.subject.host().transaction_matches(None))
        });
        let s = SCRIPT.with(|s| s.borrow().clone());
        let stone = |count| {
            mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                count,
            )
            .unwrap()
        };
        if s.write_dirt {
            let _: Result<(), mod_api::TransactionError<()>> =
                event.subject.transaction(|scope, tx| {
                    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
                    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
                    let place = ItemPlace::PlayerSlot {
                        player: 12,
                        slot: 20,
                    };
                    assert_eq!(
                        place.insert(scope.host(), tx, &ItemResource::of(&dirt), 2),
                        Ok(2)
                    );
                    Ok(())
                });
        }
        if s.invalid {
            assert_eq!(
                event
                    .subject
                    .with_view(|mut entity| entity.set_item_slot(2, Some(stone(4)))),
                Some(Ok(true)),
                "the rejected move must have a real destination write to roll back"
            );
            let err = event
                .subject
                .with_view(|mut entity| entity.set_item_slot(1, Some(stone(65))))
                .expect("live resident block entity")
                .expect_err("oversized stack must be refused at the boundary");
            assert!(matches!(
                err,
                mod_api::BlockEntityItemError::StackRejected(_)
            ));
            *event.stack = Some(stone(65));
            return Flow::Cancel;
        }
        if s.transactional_panic {
            *event.stack = Some(stone(1));
            let _: Result<(), mod_api::TransactionError<()>> =
                event.subject.transaction(|scope, tx| {
                    scope
                        .with_view(|mut entity| {
                            assert_eq!(entity.set_item_slot(1, Some(stone(4))), Ok(true));
                            assert_eq!(entity.set_item_slot(2, Some(stone(5))), Ok(true));
                            if s.write_update {
                                entity.set_client_update_fields(mod_api::Compound::from([(
                                    "panic_fixture".into(),
                                    mod_api::Tag::Int(1),
                                )]));
                            }
                        })
                        .expect("live resident block entity");
                    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
                    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
                    let place = ItemPlace::PlayerSlot {
                        player: 12,
                        slot: 20,
                    };
                    assert_eq!(
                        place.insert(scope.host(), tx, &ItemResource::of(&dirt), 2),
                        Ok(2)
                    );
                    panic!("a listener failed");
                });
        }
        if s.notify_destination {
            let host = event.subject.host();
            let level = host.level("minecraft:overworld").unwrap();
            let destination = host.block_entity(level, [2, 64, 1]).unwrap();
            assert!(destination.set_changed(host).unwrap());
        }
        if s.take {
            event
                .subject
                .with_view(|mut entity| {
                    assert_eq!(entity.set_item_slot(1, Some(stone(4))), Ok(true));
                    assert_eq!(
                        entity.set_item_slot(2, Some(stone(if s.all { 6 } else { 5 }))),
                        Ok(true)
                    );
                    if s.write_update {
                        entity.set_client_update_fields(mod_api::Compound::from([(
                            "panic_fixture".into(),
                            mod_api::Tag::Int(1),
                        )]));
                    }
                })
                .expect("live resident block entity");
            if s.all {
                *event.stack = None;
            } else {
                event.stack.as_mut().unwrap().set_count(1).unwrap();
            }
        }
        if s.cross {
            let _: Result<(), mod_api::TransactionError<()>> =
                event.subject.transaction(|scope, tx| {
                    let host = scope.host();
                    let level = host.level("minecraft:overworld").expect("the level");
                    let items = level
                        .capability(host, &ITEMS, [2, 64, 1], None)
                        .expect("the second block's items");
                    assert_eq!(
                        items.insert_at(host, tx, 0, &ItemResource::of(&stone(1)), 4),
                        Ok(4)
                    );
                    Ok(())
                });
            event.stack.as_mut().unwrap().set_count(6).unwrap();
        }
        if s.replace_destination {
            let host = event.subject.host();
            assert!(host.commands().run_command("setblock 2 64 1 minecraft:air"));
            assert!(
                host.commands()
                    .run_command("clone 1 64 1 1 64 1 2 64 1 replace force")
            );
        }
        if s.remove_backing || s.remove_and_reopen {
            let host = event.subject.host();
            let level = host.level("minecraft:overworld").unwrap();
            let air = mod_api::BlockState::of(
                &host.block_states(),
                mod_api::__private::block_key("minecraft:air").unwrap(),
                &[],
            )
            .unwrap();
            assert_eq!(
                level.set_block(
                    host,
                    [POS.0, POS.1, POS.2],
                    air,
                    mod_api::BlockUpdateFlags::KNOWN_SHAPE,
                ),
                Ok(true)
            );
            if s.remove_and_reopen {
                assert_eq!(
                    host.player(12).unwrap().open_chest_menu(
                        host,
                        1,
                        mod_api::Text::literal("reopened by menu_quick_move"),
                        Vec::new(),
                        mod_api::MenuReplace::InPlace,
                    ),
                    Err(mod_api::OpenMenuError::ClickInProgress),
                    "a tentative quick-move transaction refuses speculative screen replacement"
                );
            }
            assert!(event.subject.with_view(|_| ()).is_none());
            if s.handle_replace || s.remove_and_reopen {
                *event.stack = Some(stone(1));
                return Flow::Cancel;
            }
            return Flow::Continue;
        }
        if s.panic_in_contents_changed {
            let entity = event.subject.with_view(|view| view.handle()).unwrap();
            assert!(entity.set_changed(event.subject.host()).unwrap());
        }
        if s.panic {
            panic!("a listener failed");
        }
        if s.take || s.handle_cross {
            return Flow::Cancel;
        }
        Flow::Continue
    }
}
/// `menu_slot_take` for the live depot's menu type. A part of its own, so the depot's other menu
/// hooks (`menu_clicked`, `menu_button`) stay off the quick-move menu.
struct Taker;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Taker {
    fn menu_slot_take(&self, event: &mut MenuSlotTakeEvent<'_>) {
        if !SCRIPT.with(|s| {
            let s = s.borrow();
            s.handle_cross || s.remove_backing
        }) {
            return;
        }
        TAKE.set(Some((
            event.taken.count(),
            event.left.map_or(0, |left| left.count()),
        )));
        if SCRIPT.with(|s| s.borrow().fail_take) {
            let _: Result<(), mod_api::TransactionError<()>> =
                event.subject.transaction(|scope, tx| {
                    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
                    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
                    let place = ItemPlace::PlayerSlot {
                        player: 12,
                        slot: 20,
                    };
                    assert_eq!(
                        place.insert(scope.host(), tx, &ItemResource::of(&dirt), 2),
                        Ok(2)
                    );
                    panic!("the take listener failed after the cross-block move");
                });
        }
    }
}
#[mod_api::sunrise_part]
impl mod_api::MenuPart for Part {
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        let s = SCRIPT.with(|s| s.borrow().clone());
        let Some(flow) = s.clicked else {
            return Flow::Continue;
        };
        assert_eq!(event.menu, DEPOT_MENU);
        let player = host.player(event.uuid).unwrap();
        let menu = player.container_menu(host).unwrap();
        assert!(matches!(
            menu.slot_backing(host, 0),
            Some(mod_api::MenuSlotBacking::BlockEntity { .. })
        ));
        assert!(host.transaction_matches(None));
        if s.write_other_block || s.write_clicked_block {
            let level = host.level("minecraft:overworld").unwrap();
            let (pos, id) = if s.write_clicked_block {
                ([1, 64, 1], "minecraft:stone")
            } else {
                ([2, 64, 1], "minecraft:dirt")
            };
            let stack =
                mod_api::ItemStack::new(mod_api::__private::item_key(id).unwrap(), 1).unwrap();
            let items = level.capability(host, &ITEMS, pos, None).unwrap();
            host.transaction(|host, tx| {
                assert_eq!(
                    items.insert_at(host, tx, 0, &ItemResource::of(&stack), 2),
                    Ok(2)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        flow
    }

    fn menu_button_valid(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::MenuButtonValidEvent<'_>,
    ) -> Flow {
        if matches!(event.id, 3 | 4) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn menu_button(&self, event: &mut MenuButtonEvent<'_>) {
        BUTTONS.set(BUTTONS.get() + 1);
        if event.subject.with_view(|_| ()).is_some() {
            BUTTON_ENTITIES.set(BUTTON_ENTITIES.get() + 1);
        }
        BUTTON_POSITIONS.with(|positions| positions.borrow_mut().push(event.context.pos));
        let script = SCRIPT.with(|script| script.borrow().clone());
        if script.clear_cursor {
            *event.carried = None;
        }
        if script.change_cursor {
            *event.carried = Some(
                mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:gold_ingot").unwrap(),
                    1,
                )
                .unwrap(),
            );
        }
        if script.transactional_panic {
            let _: Result<(), mod_api::TransactionError<()>> =
                event.subject.transaction(|scope, tx| {
                    scope
                        .with_view(|mut entity| {
                            assert!(entity.set_data_slots(&[77]));
                            entity.set_client_update_fields(mod_api::Compound::from([(
                                "panic_fixture".into(),
                                mod_api::Tag::Int(2),
                            )]));
                        })
                        .expect("live resident block entity");
                    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
                    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
                    let place = ItemPlace::PlayerSlot {
                        player: 12,
                        slot: 20,
                    };
                    assert_eq!(
                        place.insert(scope.host(), tx, &ItemResource::of(&dirt), 2),
                        Ok(2)
                    );
                    panic!("menu button panicked inside subject.transaction");
                });
        }
        if script.write_update {
            event
                .subject
                .with_view(|mut entity| {
                    assert!(entity.set_data_slots(&[42]));
                    entity.set_client_update_fields(mod_api::Compound::from([(
                        "panic_fixture".into(),
                        mod_api::Tag::Int(2),
                    )]));
                })
                .expect("live resident block entity");
        }
        if script.panic_in_contents_changed {
            let entity = event.subject.with_view(|view| view.handle()).unwrap();
            assert!(entity.set_changed(event.subject.host()).unwrap());
        }
        if script.panic {
            panic!("menu button failed after editing the entity and cursor");
        }
        if script.replace_menu {
            let host = event.subject.host();
            host.player(event.context.player)
                .expect("the menu button's player is online")
                .open_chest_menu(
                    host,
                    1,
                    mod_api::Text::literal("opened by menu_button"),
                    Vec::new(),
                    mod_api::MenuReplace::InPlace,
                )
                .unwrap();
        }
    }
}

struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("depot").blocks(&[]).build();
        r.block_entity_part(&kind, Part).unwrap();
        r.menu_part(DEPOT_MENU, Part);
        r.menu_part(STATUS_MENU, Part);
        r.menu_part(QUICK_MOVE_MENU, Taker);
    }
}

struct ClickMod;
#[mod_api::__subscriptions]
impl Hooks for ClickMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part("minecraft:generic_9x1", ClickPart);
    }
}

struct ClickPart;
#[mod_api::sunrise_part]
impl mod_api::MenuPart for ClickPart {
    fn menu_clicked(
        &self,
        host: &mut dyn ServerHost,
        event: &mut MenuClickedEvent<'_>,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        IN_ACTION.with(|a| a.borrow_mut().push(!host.transaction_matches(None)));
        assert_eq!(event.menu, "minecraft:generic_9x1");
        assert_eq!(event.uuid, 12);
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.replace_other {
            host.player(13)
                .unwrap()
                .open_chest_menu(
                    host,
                    1,
                    mod_api::Text::literal("Other player's display"),
                    vec![],
                    mod_api::MenuReplace::InPlace,
                )
                .unwrap();
        }
        if s.write_dirt && s.panic {
            let _ = host.transaction(|host, tx| -> Result<(), ()> {
                write_dirt_in_tx(host, tx);
                panic!("a listener failed");
            });
        } else if s.write_dirt {
            host.transaction(|host, tx| {
                write_dirt_in_tx(host, tx);
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        if s.replace_display {
            let item =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:dirt").unwrap(), 1)
                    .unwrap();
            host.player(event.uuid)
                .unwrap()
                .container_menu(host)
                .unwrap()
                .set_items(host, vec![Some(item)])
                .unwrap();
        }
        if s.panic {
            panic!("a listener failed");
        }
        if s.replace_display || (s.write_dirt && !s.continue_after_player_write) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn with_mod(test: impl FnOnce()) {
    struct Presenter;
    #[mod_api::__subscriptions]
    impl Hooks for Presenter {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.register_menu_screen(
                DEPOT_MENU,
                std::sync::Arc::new(
                    |_: &dyn mod_api::ClientHost, _: mod_api::MenuScreenOpen<'_>| None,
                ),
            )
            .unwrap();
        }
    }
    let mods: Mods = vec![
        ModEntry::new("movemod", Box::new(Mod)),
        ModEntry::new("presenter", Box::new(Presenter)),
    ]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_api::__private::install_content_owners(
            [],
            [],
            [],
            [(DEPOT_MENU, "movemod"), (STATUS_MENU, "movemod")],
        );
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        test();
    });
}

const HEALTHY_MENU: &str = "healthy_menu:depot_menu";

fn with_publication_mods(test: impl FnOnce()) {
    struct HealthyMod;
    #[mod_api::__subscriptions]
    impl Hooks for HealthyMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type("depot").blocks(&[]).build();
            r.block_entity_part(&kind, Part).unwrap();
            r.menu_part(HEALTHY_MENU, Part);
            let cobble = r.block_key("minecraft:cobblestone");
            r.block_part(cobble, Part);
        }
    }
    let mods: Mods = vec![
        ModEntry::new("movemod", Box::new(Mod)),
        ModEntry::new("healthy_menu", Box::new(HealthyMod)),
    ]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        test();
    });
}

/// How many decisions were entered since the last `script`.
fn decided() -> usize {
    sunrise_gameplay::game::server_side::decided_action::DECIDED.get()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &sunrise_gameplay::game::ServerGame<'_>, slot: usize) -> u32 {
    count(&game.server.lan.as_ref().unwrap().hub.guests[0].inventory[slot])
}

#[test]
#[ignore = "daily"]
fn a_click_on_a_mod_menu_tells_the_mods_inside_an_action_and_a_failure_undoes_it() {
    struct HealthyMod;
    #[mod_api::__subscriptions]
    impl Hooks for HealthyMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("minecraft:generic_9x1", HealthyPart);
        }
    }
    struct HealthyPart;
    #[mod_api::sunrise_part]
    impl mod_api::MenuPart for HealthyPart {
        fn menu_clicked(&self, _: &mut dyn ServerHost, _: &mut MenuClickedEvent<'_>) -> Flow {
            HEALTHY_DISPATCHES.set(HEALTHY_DISPATCHES.get() + 1);
            Flow::Continue
        }
    }
    thread_local! {
        static HEALTHY_DISPATCHES: Cell<usize> = const { Cell::new(0) };
    }

    let mods: Mods = vec![
        ModEntry::new("clickmod", Box::new(ClickMod)),
        ModEntry::new("healthymod", Box::new(HealthyMod)),
    ]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            let window = open_native_menu(game);
            let click = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.lan_guest_packet(12, container_click(window, 3, 0));
                game.run_native_menu_events();
            };
            HEALTHY_DISPATCHES.set(0);
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(HEALTHY_DISPATCHES.get(), 0, "Cancel stops later parts");
            assert_eq!(decided(), 0, "no synthetic decided-action wrapper");
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [false]);
            assert_eq!(inventory(game, 20), 2);
            assert!(game.transactions.is_empty());

            // Continue after a player inventory write keeps the write and continues to later parts.
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[20] = None;
            });
            HEALTHY_DISPATCHES.set(0);
            // Row 90: heard inside a transaction that then aborts: the write is undone.
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            game.storage_scope(|game, _| {
                click(game);
                assert_eq!(inventory(game, 20), 2);
                ((), false)
            });
            assert_eq!(inventory(game, 20), 0);
            script(Script {
                write_dirt: true,
                continue_after_player_write: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(HEALTHY_DISPATCHES.get(), 1);
            assert_eq!(
                inventory(game, 20),
                2,
                "Continue keeps prior callback writes"
            );
            assert!(game.transactions.is_empty());

            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory[20] = None;
            });
            script(Script {
                write_dirt: true,
                ..Script::default()
            });
            game.storage_scope(|game, _| {
                click(game);
                assert_eq!(DISPATCHES.get(), 1);
                assert_eq!(inventory(game, 20), 2);
                ((), false)
            });
            assert_eq!(inventory(game, 20), 0);

            HEALTHY_DISPATCHES.set(0);
            script(Script {
                write_dirt: true,
                panic: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(
                HEALTHY_DISPATCHES.get(),
                1,
                "sibling mod still runs after a panicking part"
            );
            assert_eq!(
                inventory(game, 20),
                0,
                "the panicking callback's open transaction rolled back"
            );
            assert!(game.transactions.is_empty());
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "clickmod",
                    hook: "menu_clicked",
                }],
            );

            HEALTHY_DISPATCHES.set(0);
            script(Script::default());
            click(game);
            assert_eq!(
                DISPATCHES.get(),
                0,
                "poisoned mod is skipped for the session"
            );
            assert_eq!(HEALTHY_DISPATCHES.get(), 1);
            assert!(mod_api::__private::take_hook_panics().is_empty());
        });
    });
}

#[test]
#[ignore = "daily"]
fn without_a_listener_a_click_on_a_mod_menu_is_heard_plainly() {
    // Row 92: no mod at all: the plain call, no transaction and no journaled write.
    with_guests(|game| {
        let window = open_native_menu(game);
        script(Script::default());
        let writes = game.transactions.writes();
        game.lan_guest_packet(12, container_click(window, 3, 0));
        game.run_native_menu_events();
        assert_eq!(DISPATCHES.get(), 0);
        // The ordinary path: the decision is never entered.
        assert_eq!(decided(), 0);
        assert_eq!(game.transactions.writes(), writes);
        assert!(game.transactions.is_empty());
    });
}

fn container_click(window: i32, slot: i16, input: i32) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        container: window,
        state: 0,
        slot,
        button: 0,
        input,
        changed: Vec::new(),
        carried: None,
    })
}

/// A menu a mod opened (`Slots::Native`) for guest 12.
fn open_native_menu(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    use sunrise_sim::lan::menu::Menu;
    let menu = Menu {
        window: 5,
        kind: 2,
        pos: None,
        slots: Slots::Native,
        size: 9,
        state: 0,
        sent: Vec::new(),
        drag: None,
        sent_data: Vec::new(),
        reported_unsent_data_len: None,
        blockless_data: Vec::new(),

        mod_type: None,
        registry_id: "minecraft:generic_9x1".into(),
        craft: None,
        links: Vec::new(),
        asks_still_valid: false,
        opening_item: None,
        opener_block: None,
        removed: false,
        slot_changed_seen: None,
    };
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update(0, |guest| guest.menu = Some(menu));
    5
}

const POS: (i32, i32, i32) = (1, 64, 1);
const QUICK_MOVE_MENU: &str = "movemod:quick_move";

fn with_depot_guests(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    with_guests(|game| {
        // The game fixture stops before content loading freezes the block-entity type registry.
        // Resolve this fixture's declarations in its scoped Parts table, not the engine global.
        let types = mod_api::__private::block_entity_types();
        if !types.is_empty() {
            mod_api::__private::resolve_block_entity_types(
                types.iter().map(|(_, kind)| kind.id.as_str()),
                |id| types.iter().any(|(_, kind)| kind.id == id).then_some(()),
            )
            .unwrap();
        }
        test(game);
    });
}

fn with_live_depot_guests(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    sunrise_level::containers::with_mod_menu_types(
        [(
            QUICK_MOVE_MENU.into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        )],
        || {
            with_depot_guests(|game| {
                register_test_menu(game, QUICK_MOVE_MENU);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.brand = "sunrise".into();
                        guest.sender.menu_extra_rev =
                            sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                    });
                test(game);
            });
        },
    );
}

/// A resident mod container (a block entity of the mod's registered type) with stone 10 in slot 0.
fn place_depot(game: &mut sunrise_gameplay::game::ServerGame<'_>, menu: Option<&str>) {
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
    );
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(POS.0, POS.1, POS.2, state).unwrap();
    game.world.block_entities.set_mod_containers(
        [(
            state.0,
            Arc::new(ModContainer {
                slots: 9,
                inputs: vec![0],
                menu: menu.map(str::to_owned),
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
    let mut fields = ModBlockEntityFields {
        items: vec![None; 9],
        ..Default::default()
    };
    fields.items[0] = Some(ItemStack::new("minecraft:stone", 10));
    game.world.block_entities.insert(
        POS,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "movemod:depot".into(),
            fields,
        ))),
    );
}

/// A mod container opening a stock 9-slot layout.
fn open_depot(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    place_depot(game, None);
    let host = game.server.lan.as_mut().unwrap();
    host.hub.open_container(
        12,
        "minecraft:generic_9x1",
        sunrise_core::nbt::Tag::String("test".into()),
        POS,
        Slots::Block(POS),
        vec![None; 9],
        None,
    );
    host.hub.guests[0].menu.as_ref().unwrap().window
}

/// A block-backed mod layout opened through the ordinary server path.
fn open_live_depot(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> i32 {
    place_depot(game, Some(QUICK_MOVE_MENU));
    assert_eq!(
        game.open_block_menu(12, POS),
        sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
    );
    let menu = game.server.lan.as_ref().unwrap().hub.guests[0]
        .menu
        .as_ref()
        .unwrap();
    assert_eq!(menu.mod_type.unwrap().id, QUICK_MOVE_MENU);
    menu.window
}

#[test]
#[ignore = "daily"]
fn a_replaced_block_entity_makes_an_admitted_quick_move_stale() {
    use sunrise_gameplay::game::server_side::native_quick_move::Admitted;
    with_guests(|game| {
        let window = open_depot(game);
        let admitted = Admitted::of(game, 12, POS, 9, window).expect("the menu is open");
        assert!(admitted.holds(game));
        // The same values written again keep the block entity's identity: still fresh.
        let same = game.world.block_entities.get(&POS).unwrap().clone();
        game.world.block_entities.insert(POS, same.clone());
        assert!(admitted.holds(game));
        // Another instance with equal values, which a listener's command could have placed: stale.
        game.world.block_entities.replace(POS, same);
        assert!(!admitted.holds(game));
        // A fresh admission on the replacement holds, and a closed window does not.
        let admitted = Admitted::of(game, 12, POS, 9, window).unwrap();
        assert!(admitted.holds(game));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.menu = None);
        assert!(!admitted.holds(game));
    });
}

const SECOND: (i32, i32, i32) = (2, 64, 1);

/// A second mod container with the same state/layout as the depot, empty, that a listener
/// can move items into through its `ITEMS` capability.
fn second_block(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(SECOND.0, SECOND.1, SECOND.2, state).unwrap();
    game.world.block_entities.insert(
        SECOND,
        BlockEntity::Mod(Box::new(ModBlockEntity::new(
            "movemod:depot".into(),
            ModBlockEntityFields {
                items: vec![None; 9],
                ..Default::default()
            },
        ))),
    );
}

/// Every stone in the depot, the second block and guest 12's inventory: where a move left them.
fn stone_everywhere(game: &sunrise_gameplay::game::ServerGame<'_>) -> (u32, u32, u32) {
    let held = |items: &[Option<ItemStack>]| {
        items
            .iter()
            .flatten()
            .filter(|s| &*s.item == "minecraft:stone")
            .map(|s| s.count)
            .sum::<u32>()
    };
    let block = |pos| {
        let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&pos) else {
            panic!("the block entity is gone");
        };
        held(&m.items)
    };
    (
        block(POS),
        block(SECOND),
        held(&game.server.lan.as_ref().unwrap().hub.guests[0].inventory),
    )
}

/// The block's slots 0, 1 and 2, and guest 12's slot 20.
fn depot(game: &sunrise_gameplay::game::ServerGame<'_>) -> (u32, u32, u32, u32) {
    let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&POS) else {
        panic!("the block entity is gone");
    };
    let slot = |k: usize| count(m.items.get(k).unwrap_or(&None));
    (slot(0), slot(1), slot(2), inventory(game, 20))
}

#[test]
#[ignore = "daily"]
fn a_mod_quick_move_is_one_action_and_a_refusal_leaves_no_prefix() {
    with_mod(|| {
        with_depot_guests(|game| {
            open_depot(game);
            let click = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                let window = game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .unwrap()
                    .window;
                game.lan_guest_packet(12, container_click(window, 0, 1));
            };
            // The mod takes 9 of the 10 into two live slots and skips plain routing.
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(depot(game), (1, 4, 5, 0));
            assert!(game.transactions.is_empty());
            // A complete move empties the source through the optional API remainder.
            open_depot(game);
            script(Script {
                take: true,
                all: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (0, 4, 6, 0));
            // Writing guest inventory alongside the live quick move is allowed (no Epoch::Player
            // staleness guard).
            open_depot(game);
            script(Script {
                take: true,
                write_dirt: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (1, 4, 5, 2));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.inventory[20] = None);
            // The oversized remainder refuses the whole move, including the valid write of
            // four stone to a second slot before the invalid output was proposed.
            open_depot(game);
            script(Script {
                take: true,
                invalid: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(depot(game), (10, 0, 0, 0));
            // Applied inside an enclosing transaction that then aborts: live edits roll back.
            open_depot(game);
            script(Script {
                take: true,
                ..Script::default()
            });
            game.storage_scope(|game, _| {
                click(game);
                assert_eq!(depot(game), (1, 4, 5, 0));
                ((), false)
            });
            assert_eq!(depot(game), (10, 0, 0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_panicking_live_quick_move_rolls_back_and_disables_only_its_callback() {
    // The log sink is process-global.
    if super::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::client_side::mod_menu_decisions_tests::a_panicking_live_quick_move_rolls_back_and_disables_only_its_callback",
        "SUNRISE_TEST_LIVE_QUICK_MOVE_PANIC_CHILD",
        &["panic"],
    ) {
        return;
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct ErrorLog;
    static ERRORS: AtomicUsize = AtomicUsize::new(0);
    impl log::Log for ErrorLog {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() == log::Level::Error
        }

        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata())
                && record
                    .args()
                    .to_string()
                    .contains("mod movemod panicked in hook menu_quick_move")
            {
                ERRORS.fetch_add(1, Ordering::Relaxed);
            }
        }

        fn flush(&self) {}
    }
    log::set_logger(&ErrorLog).unwrap();
    log::set_max_level(log::LevelFilter::Error);
    with_publication_mods(|| {
        with_live_depot_guests(|game| {
            let window = open_live_depot(game);
            let click = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                game.lan_guest_packet(12, container_click(window, 0, 1));
            };
            // An explicit transaction spans the subject and guest inventory, then panics.
            script(Script {
                transactional_panic: true,
                write_update: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (10, 0, 0, 0));
            assert_eq!(inventory(game, 8), 0);
            assert!(game.transactions.is_empty());
            assert_eq!(ERRORS.load(Ordering::Relaxed), 1);

            // The same callback stays disabled; normal routing now moves the original stack.
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (0, 0, 0, 0));
            assert_eq!(inventory(game, 8), 10);
            assert_eq!(ERRORS.load(Ordering::Relaxed), 1);

            let healthy = ModBlockEntity::new(
                "healthy_menu:depot".into(),
                ModBlockEntityFields {
                    items: std::iter::once(Some(ItemStack::new("minecraft:stone", 10)))
                        .chain(std::iter::repeat_n(None, 8))
                        .collect(),
                    ..Default::default()
                },
            );
            game.world
                .block_entities
                .replace(POS, BlockEntity::Mod(Box::new(healthy)));
            script(Script {
                take: true,
                ..Script::default()
            });
            click(game);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(depot(game), (1, 4, 5, 0));

            // The normal toast collector names the failing mod and hook once.
            crate::game::server_side::native_entity_refs::tests::with_client_game(|client| {
                client.add_hook_panic_toasts();
                assert_eq!(client.client.toast_manager.queued.len(), 1);
                let Some(sunrise_client_gameplay::game::toast_manager::Toast::Diagnostic {
                    toast,
                    ..
                }) = client.client.toast_manager.queued.front()
                else {
                    panic!("expected one shared hook error toast");
                };
                assert_eq!(
                    toast.message.as_ref().unwrap().component(),
                    serde_json::json!({
                        "translate": "sunrise.mods.hook_panic.description",
                        "with": [{"text": "movemod"}, {"text": "menu_quick_move"}],
                    })
                );
            });
        });
    });
}

fn mod_entity_at<'a>(
    game: &'a sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
) -> &'a ModBlockEntity {
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos) else {
        panic!("live resident block entity missing at {pos:?}");
    };
    entity.as_ref()
}

#[test]
#[ignore = "daily"]
fn menu_button_validates_writes_and_contains_panics_on_live_resident_block_entity() {
    let menus = [
        (
            DEPOT_MENU.into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        ),
        (
            HEALTHY_MENU.into(),
            "healthy_menu".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        ),
    ];
    sunrise_level::containers::with_mod_menu_types(menus, || {
        with_publication_mods(|| {
            with_guests(|game| {
                open_depot(game);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let cobble = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:cobblestone")
                    .unwrap();
                game.world.set(POS.0, POS.1, POS.2, stone).unwrap();
                game.world
                    .set(SECOND.0, SECOND.1, SECOND.2, cobble)
                    .unwrap();
                game.world.block_entities.set_mod_containers(
                    [
                        (
                            stone.0,
                            Arc::new(ModContainer {
                                slots: 9,
                                menu: Some(DEPOT_MENU.into()),
                                ..Default::default()
                            }),
                        ),
                        (
                            cobble.0,
                            Arc::new(ModContainer {
                                slots: 9,
                                menu: Some(HEALTHY_MENU.into()),
                                ..Default::default()
                            }),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                );
                game.world.block_entities.insert(
                    POS,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "movemod:depot".into(),
                        ModBlockEntityFields {
                            items: vec![None; 9],
                            data: vec![7],
                            ..Default::default()
                        },
                    ))),
                );
                game.world.block_entities.insert(
                    SECOND,
                    BlockEntity::Mod(Box::new(ModBlockEntity::new(
                        "healthy_menu:depot".into(),
                        ModBlockEntityFields {
                            items: vec![None; 9],
                            data: vec![11],
                            ..Default::default()
                        },
                    ))),
                );
                let context_at = |pos: (i32, i32, i32)| mod_api::item::menu::MenuButtonContext {
                    player: 12,
                    player_entity_id: 1,
                    pos: Some([pos.0, pos.1, pos.2]),
                    id: 3,
                };
                let mut carried = Some(ItemStack::new("minecraft:stick", 3));
                BUTTONS.set(0);
                BUTTON_ENTITIES.set(0);

                // A panic inside `event.subject.transaction(...)` rolls back subject + world edits
                // together, restores `carried`, and records the hook panic.
                script(Script {
                    change_cursor: true,
                    transactional_panic: true,
                    ..Script::default()
                });
                game.mod_menu_button(DEPOT_MENU, context_at(POS), None, Some(POS), &mut carried);
                assert_eq!(BUTTONS.get(), 1);
                assert_eq!(BUTTON_ENTITIES.get(), 1);
                assert_eq!(mod_entity_at(game, POS).data, [7]);
                assert!(!mod_entity_at(game, POS).has_client_update_fields());
                assert_eq!(inventory(game, 20), 0);
                assert!(game.transactions.is_empty());
                assert_eq!(carried, Some(ItemStack::new("minecraft:stick", 3)));
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "movemod",
                        hook: "menu_button",
                    }]
                );

                // The failed owner is disabled for subsequent button presses. A healthy button
                // callback's cursor edit survives a failure in the separate contents_changed hook.
                script(Script {
                    change_cursor: true,
                    write_update: true,
                    panic_in_contents_changed: true,
                    ..Script::default()
                });
                game.mod_menu_button(DEPOT_MENU, context_at(POS), None, Some(POS), &mut carried);
                assert_eq!(BUTTONS.get(), 1);
                assert_eq!(mod_entity_at(game, POS).data, [7]);
                game.mod_menu_button(
                    HEALTHY_MENU,
                    context_at(SECOND),
                    None,
                    Some(SECOND),
                    &mut carried,
                );
                // This menu dispatches only to its owner. The nested panic is from the
                // separate contents_changed callback; it is not a second menu-button call.
                assert_eq!(BUTTONS.get(), 2);
                assert_eq!(BUTTON_ENTITIES.get(), 2);
                assert_eq!(mod_entity_at(game, SECOND).data, [42]);
                assert!(mod_entity_at(game, SECOND).has_client_update_fields());
                assert_eq!(carried, Some(ItemStack::new("minecraft:gold_ingot", 1)));
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "healthy_menu",
                        hook: "contents_changed",
                    }]
                );
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn menu_quick_move_validates_writes_and_contains_panics_on_live_resident_block_entity() {
    with_publication_mods(|| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(SECOND.0, SECOND.1, SECOND.2, stone).unwrap();
            let mut healthy_fields = ModBlockEntityFields {
                items: vec![None; 9],
                ..Default::default()
            };
            healthy_fields.items[0] = Some(ItemStack::new("minecraft:stone", 10));
            game.world.block_entities.insert(
                SECOND,
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "healthy_menu:depot".into(),
                    healthy_fields,
                ))),
            );

            // Invalid boundary write and oversized remainder leave the live resident entity untouched.
            script(Script {
                invalid: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(
                mod_entity_at(game, POS).items[0],
                Some(ItemStack::new("minecraft:stone", 10))
            );
            assert!(
                mod_entity_at(game, POS).items[1..3]
                    .iter()
                    .all(Option::is_none)
            );
            assert!(!mod_entity_at(game, POS).has_client_update_fields());

            // A panic inside `subject.transaction(...)` rolls back subject + world edits together,
            // discards the remainder/handled outputs, and disables `"movemod"` for `menu_quick_move`.
            script(Script {
                write_update: true,
                transactional_panic: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(
                mod_entity_at(game, POS).items[0],
                Some(ItemStack::new("minecraft:stone", 10))
            );
            assert!(
                mod_entity_at(game, POS).items[1..3]
                    .iter()
                    .all(Option::is_none)
            );
            assert!(!mod_entity_at(game, POS).has_client_update_fields());
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "movemod",
                    hook: "menu_quick_move",
                }]
            );

            // Subsequent shift-clicks on `"movemod:depot"` skip the disabled listener and fall
            // through to vanilla routing.
            let edits = Script {
                take: true,
                write_update: true,
                ..Script::default()
            };
            script(edits.clone());
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert_eq!(DISPATCHES.get(), 0);
            assert_eq!(mod_entity_at(game, POS).items[0], None);
            assert!(
                mod_entity_at(game, POS).items[1..3]
                    .iter()
                    .all(Option::is_none)
            );

            // A healthy owner's block entity still runs `menu_quick_move` in place.
            let host = game.server.lan.as_mut().unwrap();
            host.hub.open_container(
                12,
                "minecraft:generic_9x1",
                sunrise_core::nbt::Tag::String("healthy".into()),
                SECOND,
                Slots::Block(SECOND),
                vec![None; 9],
                None,
            );
            let healthy_window = host.hub.guests[0].menu.as_ref().unwrap().window;
            script(edits);
            game.lan_guest_packet(12, container_click(healthy_window, 0, 1));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(
                mod_entity_at(game, SECOND).items[0],
                Some(ItemStack::new("minecraft:stone", 1))
            );
            assert_eq!(
                mod_entity_at(game, SECOND).items[1],
                Some(ItemStack::new("minecraft:stone", 4))
            );
            assert_eq!(
                mod_entity_at(game, SECOND).items[2],
                Some(ItemStack::new("minecraft:stone", 5))
            );
            assert!(mod_entity_at(game, SECOND).has_client_update_fields());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_stale_quick_move_restores_the_original_source_and_menu_without_fallback() {
    with_publication_mods(|| {
        with_live_depot_guests(|game| {
            game.build_registries();
            let _serving = sunrise_gameplay::game::server_side::integrated::tests::install_serving(
                game, false,
            );
            for (handled, reopen) in [(true, false), (false, false), (true, true)] {
                let window = open_live_depot(game);
                let holder = game.world.block_entities.identity(POS).unwrap();
                let generation = game.server.lan.as_ref().unwrap().hub.guests[0].menu_generation;
                // The source is removed using the journal, with and without a new menu.
                // Neither a proposed remainder, a take callback nor ordinary routing can
                // settle this stale move. The inventory and original window roll back too.
                script(Script {
                    write_dirt: true,
                    remove_backing: true,
                    handle_replace: handled,
                    remove_and_reopen: reopen,
                    ..Script::default()
                });
                game.lan_guest_packet(12, container_click(window, 0, 1));
                assert_eq!(DISPATCHES.get(), 1);
                assert_eq!(TAKE.get(), None);
                assert_eq!(game.world.block_entities.identity(POS), Some(holder));
                assert_eq!(depot(game), (10, 0, 0, 0));
                let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                assert!(guest.inventory.iter().all(Option::is_none));
                assert_eq!(guest.menu_generation, generation);
                assert_eq!(
                    guest.menu.as_ref().map(|m| (m.window, m.slots)),
                    Some((window, Slots::Block(POS)))
                );
                assert!(game.transactions.is_empty());
            }
        });
    });
}

#[derive(Debug, PartialEq, Eq)]
struct QuickMoveNotice {
    at: [i32; 3],
    source_slots: [u32; 3],
}

#[derive(Clone, Default)]
struct QuickMoveObserver(Arc<std::sync::Mutex<Vec<QuickMoveNotice>>>);

#[mod_api::sunrise_part]
impl mod_api::BlockPart for QuickMoveObserver {
    fn contents_changed(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ContentsChangedEvent,
    ) {
        if SCRIPT.with(|s| s.borrow().panic_in_contents_changed) {
            panic!("the nested contents_changed listener failed");
        }
        let items = event
            .level
            .capability(host, &ITEMS, [POS.0, POS.1, POS.2], None)
            .unwrap();
        let at = [event.pos.x, event.pos.y, event.pos.z];
        self.0.lock().unwrap().push(QuickMoveNotice {
            at,
            source_slots: [0, 1, 2].map(|slot| items.amount(host, slot)),
        });
        if at == [2, 64, 1] && SCRIPT.with(|s| s.borrow().notify_destination) {
            use mod_api::HostTransaction;

            let items = event.level.capability(host, &ITEMS, at, None).unwrap();
            let stone = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                1,
            )
            .unwrap();
            assert_eq!(
                host.transaction(|host, tx| {
                    items.insert_at(host, tx, 0, &ItemResource::of(&stone), 2)
                }),
                Ok(2)
            );
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for QuickMoveObserver {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let kind = r.block_entity_type("depot").build();
        r.block_entity_part(&kind, Part).unwrap();
        let stone = r.block_key("minecraft:stone");
        r.block_part(stone, self.clone());
    }
}

fn with_quick_move_observer(test: impl FnOnce(&QuickMoveObserver)) {
    let observer = QuickMoveObserver::default();
    let mods: Mods = vec![ModEntry::new("movemod", Box::new(observer.clone()))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        test(&observer);
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_quick_move_notifies_contents_after_settling_the_source() {
    with_quick_move_observer(|observer| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            observer.0.lock().unwrap().clear();
            script(Script {
                take: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_eq!(
                observer.0.lock().unwrap().as_slice(),
                [QuickMoveNotice {
                    at: [1, 64, 1],
                    source_slots: [1, 4, 5],
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_items_quick_move_notifies_the_destination_after_settling_the_source() {
    with_quick_move_observer(|observer| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            second_block(game);
            observer.0.lock().unwrap().clear();
            script(Script {
                cross: true,
                handle_cross: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_eq!(stone_everywhere(game), (6, 4, 0));
            assert_eq!(
                observer.0.lock().unwrap().as_slice(),
                [
                    QuickMoveNotice {
                        at: [2, 64, 1],
                        source_slots: [6, 0, 0]
                    },
                    QuickMoveNotice {
                        at: [1, 64, 1],
                        source_slots: [6, 0, 0]
                    },
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_explicit_quick_move_destination_notice_does_not_reenter_after_a_self_write() {
    with_quick_move_observer(|observer| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            second_block(game);
            script(Script {
                take: true,
                notify_destination: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_eq!(stone_everywhere(game), (10, 2, 0));
            assert_eq!(
                observer.0.lock().unwrap().as_slice(),
                [
                    QuickMoveNotice {
                        at: [2, 64, 1],
                        source_slots: [10, 0, 0],
                    },
                    QuickMoveNotice {
                        at: [1, 64, 1],
                        source_slots: [1, 4, 5],
                    },
                ]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_quick_move_does_not_notify_a_replaced_items_destination_or_its_watchers() {
    with_quick_move_observer(|observer| {
        with_depot_guests(|game| {
            game.build_registries();
            let window = open_depot(game);
            second_block(game);
            let before = game.world.block_entities.identity(SECOND).unwrap();
            let level = game.native_level_here().unwrap();
            game.block_entity(level, [1, 64, 1])
                .unwrap()
                .watch_contents(game, mod_api::BlockPos::new(2, 64, 1), true)
                .unwrap();
            script(Script {
                cross: true,
                handle_cross: true,
                replace_destination: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_ne!(game.world.block_entities.identity(SECOND).unwrap(), before);
            assert_eq!(stone_everywhere(game), (6, 10, 0));
            assert_eq!(
                observer.0.lock().unwrap().as_slice(),
                [QuickMoveNotice {
                    at: [1, 64, 1],
                    source_slots: [6, 0, 0],
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_contents_panic_does_not_reject_an_unanswered_quick_move() {
    with_quick_move_observer(|_| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            second_block(game);
            script(Script {
                panic_in_contents_changed: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_eq!(
                DISPATCHES.get(),
                1,
                "the notice must originate in the listener"
            );
            assert_eq!(stone_everywhere(game), (0, 0, 10));
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "movemod",
                    hook: "contents_changed",
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_later_part_can_handle_the_same_quick_move_after_a_panic() {
    struct FirstPart;
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for FirstPart {}
    #[mod_api::sunrise_part]
    impl BlockEntityPart for FirstPart {
        fn menu_quick_move(&self, event: &mut MenuQuickMoveEvent<'_>) -> Flow {
            let stack = event.stack.as_ref().unwrap().clone();
            assert_eq!(
                event
                    .subject
                    .with_view(|mut entity| entity.set_item_slot(3, Some(stack))),
                Some(Ok(true))
            );
            *event.stack = None;
            panic!("the first quick-move listener failed");
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for FirstPart {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type("depot").build();
            r.block_entity_part(&kind, FirstPart).unwrap();
        }
    }
    struct LaterMod;
    #[mod_api::__subscriptions]
    impl Hooks for LaterMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type_key("movemod:depot");
            r.block_entity_part(&kind, Part).unwrap();
        }
    }

    let mods: Mods = vec![
        ModEntry::new("movemod", Box::new(FirstPart)),
        ModEntry::new("later_move", Box::new(LaterMod)),
    ]
    .into();
    let mut dependencies = mod_api::__private::EffectiveDependencies::default();
    dependencies.insert("later_move", "movemod", false);
    mod_platform::__private::with_test_dependencies(dependencies, || {
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            with_depot_guests(|game| {
                let window = open_depot(game);
                script(Script {
                    take: true,
                    ..Script::default()
                });
                game.lan_guest_packet(12, container_click(window, 0, 1));

                assert_eq!(depot(game), (1, 4, 5, 0));
                assert_eq!(mod_entity_at(game, POS).items[3], None);
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "movemod",
                        hook: "menu_quick_move",
                    }]
                );
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_quick_move_take_panic_after_live_slot_write_still_publishes_contents_changed() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TakePanicPart(Arc<AtomicUsize>);
    #[mod_api::sunrise_part]
    impl mod_api::BlockPart for TakePanicPart {
        fn contents_changed(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::ContentsChangedEvent,
        ) {
            if event.pos == mod_api::BlockPos::new(POS.0, POS.1, POS.2) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    #[mod_api::sunrise_part]
    impl BlockEntityPart for TakePanicPart {
        fn menu_quick_move(&self, event: &mut MenuQuickMoveEvent<'_>) -> Flow {
            event.stack.as_mut().unwrap().set_count(6).unwrap();
            Flow::Cancel
        }
    }
    #[mod_api::sunrise_part]
    impl mod_api::MenuPart for TakePanicPart {
        fn menu_slot_take(&self, event: &mut MenuSlotTakeEvent<'_>) {
            assert_eq!(event.taken.count(), 4);
            let dirt =
                mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:dirt").unwrap(), 3)
                    .unwrap();
            event
                .subject
                .with_view(|mut entity| {
                    assert_eq!(entity.set_item_slot(4, Some(dirt)), Ok(true));
                })
                .expect("live resident block entity");
            panic!("take listener panicked after writing a resident slot");
        }
    }
    struct TakePanicMod(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for TakePanicMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let kind = r.block_entity_type("depot").blocks(&[]).build();
            r.block_entity_part(&kind, TakePanicPart(self.0.clone()))
                .unwrap();
            r.menu_part(QUICK_MOVE_MENU, TakePanicPart(self.0.clone()));
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, TakePanicPart(self.0.clone()));
        }
    }

    let notices = Arc::new(AtomicUsize::new(0));
    let mods: Mods = vec![ModEntry::new(
        "movemod",
        Box::new(TakePanicMod(notices.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_live_depot_guests(|game| {
            let window = open_live_depot(game);
            notices.store(0, Ordering::Relaxed);
            game.lan_guest_packet(12, container_click(window, 0, 1));

            assert_eq!(
                mod_entity_at(game, POS).items[0],
                Some(ItemStack::new("minecraft:stone", 6)),
                "earlier quick-move remainder survives"
            );
            assert_eq!(
                mod_entity_at(game, POS).items[4],
                Some(ItemStack::new("minecraft:dirt", 3)),
                "live slot edit outside an explicit transaction survives the take panic"
            );
            assert_eq!(
                notices.load(Ordering::Relaxed),
                2,
                "both the remainder settlement and the surviving take-listener write publish contents_changed"
            );
            assert!(game.transactions.is_empty());
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "movemod",
                    hook: "menu_slot_take",
                }]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_mod_quick_move_no_mod_took_is_left_to_the_games_routing() {
    let run = || {
        let mut result = None;
        with_depot_guests(|game| {
            let window = open_depot(game);
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert!(game.transactions.is_empty());
            result = Some(depot(game));
        });
        result.unwrap()
    };
    // With no part for the hook, the plain click: no dispatch, and the result is the
    // game's own routing.
    let plain = run();
    assert_eq!(DISPATCHES.get(), 0);
    assert_eq!(decided(), 0);
    // A listener that only looks is asked once and leaves the click to the same routing.
    with_mod(|| {
        assert_eq!(run(), plain);
        assert_eq!(DISPATCHES.get(), 1);
        assert_eq!(decided(), 0);
        assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
    });
}

#[test]
#[ignore = "daily"]
fn an_untaken_quick_move_discards_its_transfer_and_routes_once() {
    with_mod(|| {
        with_depot_guests(|game| {
            let window = open_depot(game);
            second_block(game);
            // The listener moves 4 into the second block and leaves 6 in the proposed source,
            // but returns unhandled. Roll its transfer back before normal routing moves the 10.
            script(Script {
                cross: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 1));
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(stone_everywhere(game), (0, 0, 10));
            assert!(game.transactions.is_empty());

            // Aborting an enclosing transaction also undoes that click's normal routing.
            let window = open_depot(game);
            second_block(game);
            game.storage_scope(|game, _| {
                game.lan_guest_packet(12, container_click(window, 0, 1));
                assert_eq!(stone_everywhere(game), (0, 0, 20));
                ((), false)
            });
            assert_eq!(stone_everywhere(game), (10, 0, 10));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_handled_cross_block_move_fires_slot_take_and_contains_take_panics() {
    with_mod(|| {
        with_live_depot_guests(|game| {
            for fail_take in [false, true] {
                let window = open_live_depot(game);
                second_block(game);
                script(Script {
                    cross: true,
                    handle_cross: true,
                    fail_take,
                    ..Script::default()
                });
                game.lan_guest_packet(12, container_click(window, 0, 1));
                assert_eq!(DISPATCHES.get(), 1);
                assert_eq!(TAKE.get(), Some((4, 6)));
                // The cross-block move committed before `menu_slot_take`; when `menu_slot_take`
                // panics inside its own `subject.transaction(...)`, its transactional write is
                // rolled back and the panic is contained without undoing the earlier move.
                assert_eq!(stone_everywhere(game), (6, 4, 0));
                assert_eq!(inventory(game, 20), 0);
                assert!(game.transactions.is_empty());
                if fail_take {
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        [mod_api::__private::HookPanicReport {
                            mod_id: "movemod",
                            hook: "menu_slot_take",
                        }]
                    );
                }
            }
        })
    });
}

/// A content mod's menu type, the one the depot's container opens (`depot_container`).
const DEPOT_MENU: &str = "movemod:depot_menu";
const STATUS_MENU: &str = "movemod:status_menu";

/// Makes the depot at `POS` a mod container whose menu is `DEPOT_MENU`, so `mod_menu_at(POS)`
/// names it.
fn depot_container(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(POS.0, POS.1, POS.2, state).unwrap();
    game.world.block_entities.set_mod_containers(
        [(
            state.0,
            Arc::new(ModContainer {
                slots: 9,
                inputs: vec![0],
                menu: Some(DEPOT_MENU.into()),
                ..Default::default()
            }),
        )]
        .into_iter()
        .collect(),
    );
}

#[test]
#[ignore = "daily"]
fn a_quick_move_cannot_reach_a_player_slot_omitted_from_the_menu() {
    sunrise_level::containers::with_mod_menu_types(
        [(
            "minecraft:generic_9x1".into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        )],
        || {
            with_mod(|| {
                with_depot_guests(|game| {
                    let window = open_depot(game);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| {
                            guest.inventory[9] = Some(ItemStack::new("minecraft:stone", 10));
                        });
                    script(Script {
                        take: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 9, 1));
                    assert_eq!(DISPATCHES.get(), 0);
                    assert_eq!(inventory(game, 9), 10);
                })
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_code_opened_menus_button_reaches_a_blocks_menu_button_only_for_its_menu_type() {
    let menus = [
        (
            DEPOT_MENU.into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        ),
        (
            STATUS_MENU.into(),
            "movemod".into(),
            0,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            false,
            None,
        ),
    ];
    sunrise_level::containers::with_mod_menu_types(menus, || {
        with_mod(|| {
            with_guests(|game| {
                let button = |game: &mut sunrise_gameplay::game::ServerGame<'_>, window, id| {
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerButtonClick(ContainerButtonClick {
                            container: window,
                            button: id,
                        }),
                    );
                };
                BUTTONS.set(0);
                BUTTON_ENTITIES.set(0);
                BUTTON_POSITIONS.with(|positions| positions.borrow_mut().clear());
                let window = open_depot(game);
                depot_container(game);
                // The prepared protocol registry has no fixture mod IDs. Use the generic
                // window's protocol slot but give the server menu its actual mod owner.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        let menu = guest.menu.as_mut().unwrap();
                        menu.registry_id = DEPOT_MENU.into();
                        menu.mod_type = sunrise_level::containers::mod_menu_type(DEPOT_MENU);
                    });
                button(game, window, 99);
                assert_eq!(BUTTONS.get(), 0, "a menu rejects unsupported button IDs");
                button(game, window + 1, 3);
                assert_eq!(BUTTONS.get(), 0, "a stale window has no button owner");
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    1,
                    "the block's own menu reaches its menu_button"
                );
                assert_eq!(BUTTON_ENTITIES.get(), 1, "the block menu lends its entity");
                BUTTON_POSITIONS.with(|positions| {
                    assert_eq!(
                        positions.borrow().as_slice(),
                        &[Some([POS.0, POS.1, POS.2])]
                    );
                });
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                    });
                SCRIPT.with(|script| {
                    *script.borrow_mut() = Script {
                        clear_cursor: true,
                        ..Script::default()
                    }
                });
                button(game, window, 4);
                SCRIPT.with(|script| *script.borrow_mut() = Script::default());
                let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                assert_eq!(
                    guest.carried, None,
                    "a button can consume the carried stack"
                );
                assert!(
                    !guest
                        .inventory
                        .iter()
                        .flatten()
                        .any(|stack| { &*stack.item == "minecraft:diamond" && stack.count == 2 })
                );
                assert_eq!(BUTTONS.get(), 2);
                assert_eq!(BUTTON_ENTITIES.get(), 2);
                // A slotless, code-opened menu routes by its live menu owner. A position hint
                // does not lend an entity; only Slots::Block does.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.brand = "sunrise".into();
                        guest.sender.menu_extra_rev =
                            sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                    });
                let player = game.player(12).unwrap();
                let key = register_test_menu(game, STATUS_MENU);
                let menu = player
                    .open_mod_menu(game, {
                        let mut request =
                            mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Status"));
                        request.pos = Some([POS.0, POS.1, POS.2]);
                        request.replace = mod_api::MenuReplace::InPlace;
                        request
                    })
                    .unwrap()
                    .unwrap();
                let window = menu.container_id(game).unwrap();
                assert_eq!(menu.slot_count(game), Some(0));
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    3,
                    "the menu type owns its button even without block backing"
                );
                assert_eq!(
                    BUTTON_ENTITIES.get(),
                    2,
                    "a code-opened menu's position alone does not lend the block entity"
                );
                BUTTON_POSITIONS.with(|positions| {
                    assert_eq!(
                        positions.borrow().as_slice(),
                        &[
                            Some([POS.0, POS.1, POS.2]),
                            Some([POS.0, POS.1, POS.2]),
                            None,
                        ],
                        "a blockless menu supplies no position even if it has a screen hint"
                    );
                });
                // Another menu type at that position (another mod's menu at a machine): not the
                // block's.
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.menu.as_mut().unwrap().registry_id = "test:native".into();
                    });
                button(game, window, 3);
                assert_eq!(
                    BUTTONS.get(),
                    3,
                    "another menu type reaches no block's menu_button"
                );
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.menu.as_mut().unwrap().registry_id = STATUS_MENU.into();
                        guest.carried = Some(ItemStack::new("minecraft:diamond", 2));
                    });
                SCRIPT.with(|script| {
                    *script.borrow_mut() = Script {
                        replace_menu: true,
                        change_cursor: true,
                        ..Script::default()
                    }
                });
                button(game, window, 4);
                SCRIPT.with(|script| *script.borrow_mut() = Script::default());
                assert_eq!(BUTTONS.get(), 4);
                assert_eq!(BUTTON_ENTITIES.get(), 2);
                let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                assert_eq!(
                    guest.carried, None,
                    "a cursor edit from the old menu cannot overwrite the replacement"
                );
                assert!(
                    guest
                        .inventory
                        .iter()
                        .flatten()
                        .any(|stack| { &*stack.item == "minecraft:diamond" && stack.count == 2 }),
                    "the returned cursor stack stays in the player's inventory"
                );
                assert!(
                    !guest.inventory.iter().flatten().any(|stack| {
                        &*stack.item == "minecraft:gold_ingot" && stack.count == 1
                    }),
                    "the stale menu's explicit cursor edit is not applied"
                );
            });
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_block_menu_click_commits_only_the_replacement_or_a_fresh_default() {
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound};
    use sunrise_sim::lan::{Out, PlayerSender, menu::input::SWAP};

    sunrise_level::containers::with_mod_menu_types(
        [(
            DEPOT_MENU.into(),
            "movemod".into(),
            9,
            0,
            Vec::new(),
            Vec::new(),
            None,
            None,
            true,
            None,
        )],
        || {
            with_mod(|| {
                with_depot_guests(|game| {
                    register_test_menu(game, DEPOT_MENU);
                    place_depot(game, Some(DEPOT_MENU));
                    second_block(game);
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.brand = "sunrise".into();
                        g.sender.menu_extra_rev = sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                    });
                    assert_eq!(
                        game.open_block_menu(12, POS),
                        sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                    );
                    let menu = game.server.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .unwrap();
                    assert_eq!(menu.mod_type.unwrap().id, DEPOT_MENU);
                    let window = menu.window;
                    let content = |game: &sunrise_gameplay::game::ServerGame<'_>,
                                   pos,
                                   slot: usize| {
                        let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get(&pos)
                        else {
                            panic!("missing {pos:?}")
                        };
                        count(&entity.items[slot])
                    };

                    script(Script {
                        clicked: Some(Flow::Cancel),
                        write_other_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(decided(), 0);
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(content(game, SECOND, 0), 2);
                    assert!(
                        game.server.lan.as_ref().unwrap().hub.guests[0]
                            .carried
                            .is_none()
                    );

                    // A visible player inventory still does not include the offhand.
                    let (tx, rx) = std::sync::mpsc::channel();
                    let offhand = Some(ItemStack::new("minecraft:iron_ingot", 1));
                    let hub = &mut game.server.lan.as_mut().unwrap().hub;
                    hub.guests.update(0, |g| {
                        let revision = g.sender.menu_extra_rev;
                        g.sender = PlayerSender::new(tx, Arc::default());
                        g.sender.menu_extra_rev = revision;
                        g.offhand = offhand.clone();
                    });
                    let menu = hub.guests[0].menu.as_ref().unwrap();
                    assert_eq!(menu.player_slots(), 36);
                    let state = menu.state;
                    let expected = hub.stack(&offhand);
                    assert!(expected.is_some());
                    script(Script {
                        clicked: Some(Flow::Cancel),
                        ..Script::default()
                    });
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state,
                            slot: 0,
                            button: 40,
                            input: SWAP,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(decided(), 0);
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(
                        game.server.lan.as_ref().unwrap().hub.guests[0].offhand,
                        offhand
                    );
                    let corrections: Vec<_> = rx
                        .try_iter()
                        .flat_map(|out| match out {
                            Out::Packet(body) => vec![body],
                            Out::Packets(bodies) => bodies,
                            Out::Disconnect(_) => Vec::new(),
                        })
                        .filter_map(|body| {
                            let mut reader = Reader::new(&body);
                            let id = reader.varint().unwrap();
                            match Clientbound::decode(id, reader.rest()).unwrap() {
                                Clientbound::SetPlayerInventory(p) if p.slot == 40 => Some(p.item),
                                _ => None,
                            }
                        })
                        .collect();
                    assert_eq!(corrections, [expected]);

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_clicked_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(decided(), 0);
                    assert_eq!(content(game, POS, 0), 0);
                    assert_eq!(
                        count(&game.server.lan.as_ref().unwrap().hub.guests[0].carried),
                        12,
                        "default click ran on the live menu after callback writes"
                    );

                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.carried = None;
                    });
                    let Some(BlockEntity::Mod(depot)) = game.world.block_entities.get_mut(&POS)
                    else {
                        panic!("missing depot at {POS:?}");
                    };
                    depot.items[0] = Some(ItemStack::new("minecraft:stone", 10));

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_other_block: true,
                        take: true,
                        invalid: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 1));
                    assert_eq!(decided(), 0);
                    assert_eq!(DISPATCHES.get(), 1, "the quick-move listener was reached");
                    IN_ACTION.with(|seen| {
                        assert_eq!(
                            *seen.borrow(),
                            [true],
                            "the quick move ran inside its own storage transaction"
                        );
                    });
                    assert_eq!(content(game, POS, 0), 10);
                    assert_eq!(
                        content(game, SECOND, 0),
                        4,
                        "a refused default keeps prior unrelated menu_clicked writes"
                    );

                    script(Script {
                        clicked: Some(Flow::Continue),
                        write_other_block: true,
                        ..Script::default()
                    });
                    game.lan_guest_packet(12, container_click(window, 0, 0));
                    assert_eq!(content(game, POS, 0), 0);
                    assert_eq!(content(game, SECOND, 0), 6);
                    assert_eq!(
                        count(&game.server.lan.as_ref().unwrap().hub.guests[0].carried),
                        10
                    );
                })
            })
        },
    );
}

#[test]
#[ignore = "daily"]
fn a_listener_for_another_menu_type_does_not_admit_a_click_decision() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_native_menu(game);
            script(Script::default());
            game.lan_guest_packet(12, container_click(window, 3, 0));
            assert_eq!(decided(), 0);
        })
    });
}

fn chest(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    dimension: mod_api::DimensionKey,
) -> mod_api::BlockEntityRef {
    use sunrise_level::chunk::Chunk;
    use sunrise_world::chunk::BlockState;

    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let state = game
        .pack
        .assets
        .registry
        .resolve(&BlockState {
            name: "minecraft:chest".into(),
            properties: [
                ("facing".into(), "north".into()),
                ("type".into(), "single".into()),
                ("waterlogged".into(), "false".into()),
            ]
            .into_iter()
            .collect(),
        })
        .unwrap();
    let level = mod_api::LevelRef::new(game.native_lifecycle.epoch().unwrap(), dimension);
    let pos = [POS.0, POS.1, POS.2];
    assert_eq!(
        game.level_set_block(level, pos, state, mod_api::BlockUpdateFlags::NEIGHBORS),
        Ok(true)
    );
    game.block_entity(level, pos).unwrap()
}

#[test]
#[ignore = "daily"]
fn block_and_ender_menu_providers_replace_the_live_window_only_on_commit() {
    with_guests(|game| {
        let chest = chest(game, mod_api::DimensionKey::OVERWORLD);
        let player = game.player(12).unwrap();
        let active = |game: &sunrise_gameplay::game::ServerGame<'_>| {
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(12)
                .unwrap()
                .menu
                .as_ref()
                .map(|menu| menu.slots)
        };
        for commit in [false, true] {
            let result = game.transaction(|host, _| {
                assert_eq!(player.open_menu(host, chest), Ok(true));
                assert_eq!(
                    player.container_menu(host).unwrap().container_id(host),
                    Some(0)
                );
                if commit { Ok(()) } else { Err(()) }
            });
            assert_eq!(result.is_ok(), commit);
            assert_eq!(active(game), commit.then_some(Slots::Block(POS)));
            assert_eq!(
                game.server.lid_openers.get(&POS).copied(),
                commit.then_some(1)
            );
        }
        let window = player
            .container_menu(game)
            .unwrap()
            .container_id(game)
            .unwrap();
        for commit in [false, true] {
            let result = game.transaction(|host, _| {
                assert_eq!(
                    player.open_ender_chest(host, mod_api::Text::literal("Ender chest")),
                    Ok(true)
                );
                assert_eq!(
                    player.container_menu(host).unwrap().container_id(host),
                    Some(window)
                );
                if commit { Ok(()) } else { Err(()) }
            });
            assert_eq!(result.is_ok(), commit);
            assert_eq!(
                active(game),
                Some(if commit {
                    Slots::Ender
                } else {
                    Slots::Block(POS)
                })
            );
            assert_eq!(
                game.server.lid_openers.get(&POS).copied(),
                (!commit).then_some(1)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_click_replaces_another_players_container_in_its_own_level() {
    use sunrise_gameplay_common::game::Dim;

    let mods: Mods = vec![ModEntry::new("clickmod", Box::new(ClickMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(13, |g| {
                    g.dimension = Dim::Nether.id().to_owned();
                })
                .unwrap();
            let window = open_native_menu(game);
            chest(game, mod_api::DimensionKey::OVERWORLD);
            // An opener at the same coordinates in the caller's level must not be closed.
            assert!(game.container_openers_changed(POS, true));
            game.enter_level(Dim::Nether).unwrap();
            let other_chest = chest(game, mod_api::DimensionKey::NETHER);
            let other = game.player(13).unwrap();
            assert_eq!(other.open_menu(game, other_chest), Ok(true));
            assert_eq!(game.server.lid_openers.get(&POS), Some(&1));
            game.enter_level(Dim::Overworld).unwrap();

            script(Script {
                replace_other: true,
                ..Script::default()
            });
            game.lan_guest_packet(12, container_click(window, 0, 0));
            assert_eq!(game.dim, Dim::Overworld);
            assert_eq!(game.server.lid_openers.get(&POS), Some(&1));
            let hub = &game.server.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guest(12).unwrap().menu.as_ref().unwrap().window, window);
            assert_eq!(
                hub.guest(13).unwrap().menu.as_ref().unwrap().slots,
                Slots::Native
            );
            game.enter_level(Dim::Nether).unwrap();
            assert!(!game.server.lid_openers.contains_key(&POS));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_display_replacement_is_journaled_along_with_the_click() {
    let mods: Mods = vec![ModEntry::new("clickmod", Box::new(ClickMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_guests(|game| {
            let window = game
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .open_native_menu(
                    12,
                    1,
                    sunrise_core::nbt::Tag::String("display".into()),
                    vec![None; 9],
                )
                .unwrap();
            script(Script {
                replace_display: true,
                ..Script::default()
            });
            game.storage_scope(|game, _| {
                game.lan_guest_packet(12, container_click(window, 0, 0));
                assert_eq!(
                    count(&game.server.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                    1
                );
                ((), false)
            });
            assert_eq!(
                count(&game.server.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                0
            );
            game.lan_guest_packet(12, container_click(window, 0, 0));
            assert_eq!(
                count(&game.server.lan.as_ref().unwrap().hub.guests[0].native_menu[0]),
                1
            );
        });
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
