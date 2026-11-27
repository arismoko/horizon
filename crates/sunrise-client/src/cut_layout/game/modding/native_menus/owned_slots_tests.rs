use mod_api::{
    Flow, Hand, Hooks, HostTransaction, ItemPlace, ItemResource, MenuClickedEvent, MenuContainer,
    MenuItemsRefusal, MenuKey, MenuPart, MenuRef, MenuRemovedEvent, MenuReplace,
    MenuSlotBackRefusal, MenuSlotBacking, MenuSlotChangedEvent, MenuSlotPlace, MenuSlotWrite,
    MenuStillValidEvent, ModEntry, Mods, PlayerContainerCloseEvent, PlayerRef, Registrar,
    ServerHost,
};
use std::sync::mpsc;
use sunrise_gameplay_common::game::GameGameExt1;
use sunrise_items::items::ItemStack;
use sunrise_protocol::packets::play::{
    Clientbound as C, ContainerClick, ContainerClose, Serverbound as S, SetCursorItem,
};
use sunrise_sim::lan::Out;
use {
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay_common::game::api_values,
    sunrise_gameplay_common::game::api_values::TryIntoApi,
};
use {
    sunrise_gameplay::game::server_side::native_menus::tests::mod_menu_guest,
    sunrise_gameplay_common::game::server_side::native_menus::tests::MOD_MENU,
    sunrise_gameplay_common::game::server_side::native_menus::tests::sent,
};

#[derive(Clone, Copy)]
enum ClickAction {
    Close { commit: Option<bool> },
    Replace { mod_menu: MenuKey },
    InvalidateOpeningItem,
    PanicAfterCommit,
}

thread_local! {
    static SCRIPTED_CLICK: std::cell::Cell<Option<Flow>> = const { std::cell::Cell::new(None) };
    static CLICK_ACTION: std::cell::Cell<Option<ClickAction>> = const { std::cell::Cell::new(None) };
    static CLOSE_NOTICES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LOCK_FIRST_SLOT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static REOPEN_ON_REMOVED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static RECORD_CLOSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CLOSE_TRACE: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(Vec::new()) };
    static LAST_CLOSE_DIMENSION: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    static LAST_REMOVED: std::cell::RefCell<Option<(Option<PlayerRef>, u128, i32, Option<Hand>, Vec<Option<mod_api::ItemStack>>)>> = const { std::cell::RefCell::new(None) };
    static LAST_CONTAINER_CLOSE: std::cell::RefCell<Option<(PlayerRef, String, Option<MenuRef>, Option<MenuRef>, Option<Option<mod_api::ItemStack>>)>> = const { std::cell::RefCell::new(None) };
}

struct Part {
    return_contents: bool,
}
#[mod_api::sunrise_part]
impl MenuPart for Part {
    fn menu_still_valid(&self, _: &dyn mod_api::LoopContext, event: &mut MenuStillValidEvent<'_>) {
        if let Some(item) = event.opening_item {
            assert_eq!(item.hand, Hand::Off);
            event.valid = item.stack_matches;
        }
    }
    fn menu_slot_rules(&self, _: &dyn mod_api::Host, event: &mut mod_api::MenuSlotRulesEvent<'_>) {
        if LOCK_FIRST_SLOT.get() && event.slot == 0 {
            assert_eq!(event.context.player, Some(123));
            assert!(
                event
                    .context
                    .slot(mod_api::MenuContainer::Main, 0)
                    .is_some()
            );
            event.may_pickup = false;
            event.may_place = false;
        }
    }

    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        if let Some(action) = CLICK_ACTION.get() {
            let player = host.player(event.uuid).unwrap();
            let stack = |id| mod_api::ItemStack::of(mod_api::__private::item_key(id).unwrap());
            match action {
                ClickAction::Close { commit } => {
                    let menu = player.container_menu(host).unwrap();
                    let close = |host: &mut dyn ServerHost| assert!(menu.close_container(host));
                    if let Some(commit) = commit {
                        let result = host.transaction(|host, tx| {
                            let slot = ItemPlace::PlayerSlot {
                                player: event.uuid,
                                slot: 20,
                            };
                            assert_eq!(
                                slot.insert(
                                    host,
                                    tx,
                                    &ItemResource::of(&stack("minecraft:gold_ingot")),
                                    1
                                ),
                                Ok(1)
                            );
                            close(host);
                            assert_eq!(menu.container_id(host), Some(event.window));
                            if commit { Ok(()) } else { Err(()) }
                        });
                        assert_eq!(result.is_ok(), commit);
                        return Flow::Cancel;
                    }
                    close(host);
                    assert_eq!(menu.container_id(host), None);
                }
                ClickAction::Replace { mod_menu } => {
                    let old = player.container_menu(host).unwrap();
                    let title = mod_api::Text::literal("Replacement menu");
                    let next = player
                        .open_mod_menu(host, mod_api::ModMenuOpen::new(mod_menu, title))
                        .unwrap()
                        .expect("open_mod_menu replaces the current menu directly");
                    assert_ne!(next.container_id(host).unwrap(), event.window);
                    assert_eq!(old.container_id(host), None);
                    assert_eq!(player.container_menu(host), Some(next));
                }
                ClickAction::InvalidateOpeningItem => {
                    host.transaction(|host, tx| {
                        let hand = ItemPlace::PlayerHand {
                            player: event.uuid,
                            hand: Hand::Off,
                        };
                        assert_eq!(
                            hand.extract(host, tx, &ItemResource::of(&stack("minecraft:paper")), 2),
                            Ok(2)
                        );
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                }
                ClickAction::PanicAfterCommit => {
                    let menu = player.container_menu(host).unwrap();
                    host.transaction(|host, tx| {
                        let target = ItemPlace::MenuSlot {
                            menu,
                            slot: usize::try_from(event.slot).unwrap(),
                        };
                        assert_eq!(
                            target.insert(
                                host,
                                tx,
                                &ItemResource::of(&stack("minecraft:stick")),
                                1
                            ),
                            Ok(1)
                        );
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                    panic!("a replacement click failed after committing its edit");
                }
            }
            return Flow::Continue;
        }
        if let Some(flow) = SCRIPTED_CLICK.get() {
            return flow;
        }
        if event.slot != 1 {
            return Flow::Continue;
        }
        let Some(menu) = host
            .player(event.uuid)
            .and_then(|player| player.container_menu(host))
            .filter(|menu| menu.container_id(host) == Some(event.window))
        else {
            return Flow::Continue;
        };
        let Some(cursor) = menu.carried(host).flatten() else {
            return Flow::Continue;
        };
        host.transaction(|host, tx| {
            let _ = ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                host,
                tx,
                &ItemResource::of(&cursor),
                1,
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        Flow::Cancel
    }

    fn menu_removed(&self, host: &mut dyn ServerHost, event: &mut MenuRemovedEvent<'_>) {
        CLOSE_NOTICES.set(CLOSE_NOTICES.get() + 1);
        if RECORD_CLOSE.get() {
            CLOSE_TRACE.with(|trace| trace.borrow_mut().push("menu_removed"));
            LAST_REMOVED.with(|slot| {
                *slot.borrow_mut() = Some((
                    event.player,
                    event.uuid,
                    event.window,
                    event.opening_hand,
                    event.owned_slots.to_vec(),
                ));
            });
        }
        if REOPEN_ON_REMOVED.replace(false) {
            let next = event
                .player
                .unwrap()
                .open_chest_menu(
                    host,
                    1,
                    mod_api::Text::literal("Replacement"),
                    Vec::new(),
                    MenuReplace::InPlace,
                )
                .unwrap();
            assert_eq!(next.container_id(host), None);
            CLOSE_TRACE.with(|trace| trace.borrow_mut().push("removed continued"));
        }
        if !self.return_contents {
            return;
        }
        assert_eq!(event.opening_hand, Some(Hand::Off));
        let Some(player) = event.player else {
            return;
        };
        let closing = player.container_menu(host).unwrap();
        assert_eq!(closing.container_id(host), Some(event.window));
        assert_eq!(closing.slot(host, 0), Some(None));
        host.transaction(|host, _| {
            for stack in event.owned_slots.iter().flatten() {
                assert!(player.give_stack(host, stack.clone()).unwrap().is_none());
            }
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}

struct Listener {
    return_contents: bool,
}

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(
            MOD_MENU,
            Part {
                return_contents: self.return_contents,
            },
        );
        r.register_menu_screen(
            MOD_MENU,
            std::sync::Arc::new(
                |_: &dyn mod_api::ClientHost, _: mod_api::MenuScreenOpen<'_>| {
                    Some(mod_api::MenuScreen::new(mod_api::MenuScreenLayout {
                        width: 176.0,
                        height: 166.0,
                        slots: vec![Some(mod_api::ScreenSlot::default()); 2],
                        ..Default::default()
                    }))
                },
            ),
        )
        .unwrap();
    }

    fn player_container_open(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::PlayerContainerOpenEvent<'_>,
    ) {
        if RECORD_CLOSE.get() {
            CLOSE_TRACE.with(|trace| trace.borrow_mut().push("player_container_open"));
        }
    }

    fn player_logged_out(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerLoggedOutEvent<'_>,
    ) {
        if RECORD_CLOSE.get() {
            let player = host.player(event.player).unwrap();
            let menu = player.container_menu(host).unwrap();
            assert_ne!(menu.container_id(host).unwrap(), 0);
            CLOSE_TRACE.with(|trace| trace.borrow_mut().push("player_logged_out"));
        }
    }

    fn player_container_close(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerContainerCloseEvent<'_>,
    ) {
        if RECORD_CLOSE.get() {
            CLOSE_TRACE.with(|trace| trace.borrow_mut().push("player_container_close"));
            let current = event.player.container_menu(host);
            LAST_CLOSE_DIMENSION.with(|dimension| {
                *dimension.borrow_mut() = event.player.dimension(host).map(|id| id.to_string());
            });
            let slot0 = event.menu.and_then(|menu| menu.slot(host, 0));
            LAST_CONTAINER_CLOSE.with(|slot| {
                *slot.borrow_mut() = Some((
                    event.player,
                    event.menu_type.to_owned(),
                    event.menu,
                    current,
                    slot0,
                ));
            });
        }
    }
}

fn with_owned_menu(
    return_contents: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, mpsc::Receiver<Out>, MenuKey),
) {
    with_owned_layout(return_contents, false, test);
}

fn with_owned_layout(
    return_contents: bool,
    player_inventory: bool,
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, mpsc::Receiver<Out>, MenuKey),
) {
    let mods: Mods = vec![ModEntry::new(
        "listen",
        Box::new(Listener { return_contents }),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        mod_api::__private::install_content_owners([], [], [], [(MOD_MENU, "listen")]);
        api_values::install_parts(&mods).unwrap();
        let _callbacks = sunrise_sim::native_client_hooks::scoped_menu_validity_hooks(
            api_values::menu_still_valid,
            |menu| mod_api::__private::menu_listens(mod_api::HookId::MenuStillValid, menu),
        );
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                2,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                player_inventory,
                Some(vec![1, 4]),
            )],
            || {
                with_game(|game| {
                    let (rx, key) = mod_menu_guest(game);
                    game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                        g.sender.menu_extra_rev = sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                        g.offhand = Some(ItemStack::new("minecraft:paper", 2));
                    });
                    test(game, rx, key);
                });
            },
        );
    });
}

fn open(game: &mut sunrise_gameplay::game::ServerGame<'_>, key: MenuKey) -> (i32, MenuRef) {
    let player = game.player(123).unwrap();
    // Open from the acting player's item action, when the hand lives in the game,
    // not in the guest's record.
    let menu = game
        .as_guest(1, |game| {
            player.open_mod_menu(game, {
                let mut request =
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Owned slots"));
                request.opening_hand = Some(Hand::Off);
                request
            })
        })
        .unwrap()
        .unwrap()
        .unwrap();
    (menu.container_id(game).unwrap(), menu)
}

fn click(game: &mut sunrise_gameplay::game::ServerGame<'_>, container: i32, slot: i16) {
    game.lan_guest_packet(
        1,
        S::ContainerClick(ContainerClick {
            container,
            state: 0,
            slot,
            button: 0,
            input: 0,
            changed: Vec::new(),
            carried: None,
        }),
    );
    game.run_native_menu_events();
}

#[test]
#[ignore = "daily"]
fn an_item_opened_menu_has_two_empty_owned_slots_with_individual_caps() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let packets = sent(&rx);
        assert!(packets.iter().any(|p| matches!(p,
            C::ContainerSetContent(content) if content.container == window
                && content.items == vec![None, None]
        )));
        assert_eq!(menu.slot_count(game), Some(2));
        assert_eq!(
            menu.back_slots(
                game,
                vec![MenuSlotPlace::HeldItem {
                    hand: Hand::Off,
                    slot: 0
                }],
            ),
            Err(MenuSlotBackRefusal::UnsupportedMenu),
        );
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let resource = ItemResource::of(&stick);
        for (slot, cap) in [1, 4].into_iter().enumerate() {
            assert_eq!(menu.slot(game, slot), Some(None));
            assert_eq!(
                menu.slot_backing(game, slot),
                Some(MenuSlotBacking::Owned(slot))
            );
            assert_eq!(menu.slot_holds(game, slot, &stick), Some(cap));
            let host: &mut dyn ServerHost = game;
            host.transaction(|host, tx| {
                let place = ItemPlace::MenuSlot { menu, slot };
                assert_eq!(place.insert(host, tx, &resource, 8), Ok(cap));
                assert_eq!(place.with_stack(host, |stack| stack.count()), Some(cap));
                Ok::<_, ()>(())
            })
            .unwrap();
        }
        let host: &mut dyn ServerHost = game;
        assert_eq!(
            menu.set_items(host, vec![]),
            Err(MenuItemsRefusal::OwnedSlots)
        );
        assert_eq!(menu.slot(host, 0), Some(Some(stick)));
    });
}

#[test]
#[ignore = "daily"]
fn an_open_menus_state_id_is_the_one_its_content_was_sent_with() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let state = sent(&rx)
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => Some(p.state),
                _ => None,
            })
            .unwrap();
        assert_eq!(menu.state_id(game), Some(state));
    });
}

#[test]
#[ignore = "daily"]
fn opening_an_owned_menu_with_crafting_keeps_the_old_window() {
    with_owned_menu(false, |game, rx, key| {
        let (_, menu) = open(game, key);
        sent(&rx);
        // Bypass package validation to exercise the binding boundary itself: owned_slots + crafting
        // is still refused, while owned_slots + results opens cleanly.
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                2,
                0,
                vec![],
                vec![],
                Some(sunrise_level::containers::ModCrafting {
                    grid: 0,
                    size: 1,
                    result: 1,
                }),
                None,
                false,
                Some(vec![1, 4]),
            )],
            || {
                let player = game.player(123).unwrap();
                assert_eq!(
                    player.open_mod_menu(
                        game,
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Invalid crafting")),
                    ),
                    Err(mod_api::OpenMenuError::OwnedSlotsWithCrafting)
                );
                assert_eq!(game.player_container_menu(player), Some(menu));
                assert!(sent(&rx).is_empty());
            },
        );
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                2,
                0,
                vec![1],
                vec![0],
                None,
                None,
                true,
                Some(vec![16, 4]),
            )],
            || {
                let player = game.player(123).unwrap();
                let opened = player
                    .open_mod_menu(
                        game,
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Valid results")),
                    )
                    .unwrap();
                assert!(opened.is_some());
                assert_ne!(game.player_container_menu(player), Some(menu));
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_cursor_cleared_outside_a_click_is_sent_once_on_the_next_broadcast() {
    with_owned_menu(false, |game, rx, key| {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stick", 2));
            });
        let (window, menu) = open(game, key);
        sent(&rx);
        let cursor = menu.cursor();
        let stick = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        let resource = ItemResource::of(&stick);
        game.storage_transaction(|host, tx| {
            assert_eq!(cursor.insert(host, tx, &resource, 3), Ok(3));
            Ok::<_, ()>(())
        })
        .unwrap();

        // Establish what the client sees through a full menu sync, not a click prediction.
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.resend_menu(1, window, &game.common.world);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerSetContent(content) if content.container == window
                && content.carried.as_ref().map(|item| item.count) == Some(3)
        )));
        hub.sync_containers(&game.common.world);
        assert!(
            sent(&rx).is_empty(),
            "the full sync already sent the cursor"
        );

        game.storage_transaction(|host, tx| {
            assert_eq!(cursor.extract(host, tx, &resource, 3), Ok(3));
            Ok::<_, ()>(())
        })
        .unwrap();
        assert!(sent(&rx).is_empty(), "nothing before the broadcast");

        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        // A repair of window 0 does not tell the open window about the clear.
        hub.send_inventory(0);
        assert!(matches!(&sent(&rx)[..], [C::ContainerSetContent(content)]
            if content.container == 0 && content.carried.as_ref().map(|item| item.count) == Some(2)));
        hub.sync_containers(&game.common.world);
        let packets = sent(&rx);
        assert!(
            matches!(
                &packets[..],
                [C::SetCursorItem(SetCursorItem { item: None })]
            ),
            "{packets:?}"
        );
        hub.sync_containers(&game.common.world);
        assert!(sent(&rx).is_empty(), "the empty cursor was already sent");
    });
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn owned_click_prediction_applies_caps_and_the_server_corrects_an_override() {
    use sunrise_gameplay_common::game::inventory_slots::MOD_MENU_SLOT_BASE;
    use sunrise_protocol::item::Hashed;
    use sunrise_sim::menu::Input;
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let opening = sent(&rx);
        let state = opening
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => Some(p.state),
                _ => None,
            })
            .unwrap();
        let stack = |count| Some(ItemStack::new("minecraft:stick", count));
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        let builtin = hub.sync_data().builtin.clone();
        let network = [1, 3, 4, 6, 7].map(|count| hub.stack(&stack(count)).unwrap());
        let [one, three, four, six, seven] = &network;
        let prediction = |slot, placed, left| {
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot,
                button: 0,
                input: 0,
                changed: vec![(slot, Some(Hashed::of(placed).unwrap()))],
                carried: Some(Hashed::of(left).unwrap()),
            })
        };
        let set_server_cursor = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = stack(7));
        };
        set_server_cursor(game);
        game.lan_guest_packet(1, prediction(0, one, six));
        assert_eq!(
            menu.slot(game, 0),
            Some(Some(stack(1).unwrap().try_into_api().unwrap()))
        );
        assert_eq!(
            menu.carried(game),
            Some(Some(stack(6).unwrap().try_into_api().unwrap()))
        );
        assert!(
            sent(&rx).is_empty(),
            "the server agreed with the ordinary click"
        );

        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot: 0,
                button: 0,
                input: sunrise_sim::lan::menu::input::QUICK_MOVE,
                changed: Vec::new(),
                carried: Some(Hashed::of(six).unwrap()),
            }),
        );
        assert_eq!(
            menu.slot(game, 0),
            Some(Some(stack(1).unwrap().try_into_api().unwrap())),
            "shift-click must not move an item to an omitted player inventory"
        );
        assert!(sent(&rx).is_empty());

        set_server_cursor(game);
        game.lan_guest_packet(1, prediction(1, four, three));
        assert_eq!(
            menu.slot(game, 1),
            Some(Some(stack(1).unwrap().try_into_api().unwrap()))
        );
        assert_eq!(
            menu.carried(game),
            Some(Some(stack(7).unwrap().try_into_api().unwrap()))
        );
        let corrections = sent(&rx);
        let corrected_state = corrections
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => {
                    assert_eq!(p.items, [Some(one.clone()), Some(one.clone())]);
                    assert_eq!(p.carried.as_ref(), Some(seven));
                    Some(p.state)
                }
                _ => None,
            })
            .unwrap();

        // The server's copy and cursor now differ only in components from what the
        // packet claims. Matching item IDs and counts is not enough to skip correction.
        let mut named = stack(7).unwrap();
        named.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Filter".into()));
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.guests.update(0, |g| {
            g.native_menu[1] = None;
            g.carried = Some(named.clone());
        });
        let named_one = ItemStack {
            count: 1,
            ..named.clone()
        };
        let named_one_net = hub.stack(&Some(named_one.clone()));
        let named_seven_net = hub.stack(&Some(named.clone()));
        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state: corrected_state,
                slot: 1,
                button: 0,
                input: 0,
                changed: vec![(1, Some(Hashed::of(one).unwrap()))],
                carried: Some(Hashed::of(seven).unwrap()),
            }),
        );
        let component_correction = sent(&rx);
        let state = component_correction
            .iter()
            .find_map(|p| match p {
                C::ContainerSetContent(p) if p.container == window => {
                    assert_eq!(p.items[1], named_one_net);
                    assert_eq!(p.carried, named_seven_net);
                    Some(p.state)
                }
                _ => None,
            })
            .unwrap();

        // No player slots means this quick move does nothing. The prediction has the correct
        // cursor, but claims the slot kept a named stick instead of an ordinary one.
        game.lan_guest_packet(
            1,
            S::ContainerClick(ContainerClick {
                container: window,
                state,
                slot: 0,
                button: 0,
                input: sunrise_sim::lan::menu::input::QUICK_MOVE,
                changed: vec![(0, named_one_net.as_ref().map(|s| Hashed::of(s).unwrap()))],
                carried: named_seven_net.as_ref().map(|s| Hashed::of(s).unwrap()),
            }),
        );
        let slot_correction = sent(&rx);
        assert!(matches!(&slot_correction[..], [C::ContainerSetSlot(p)]
            if p.container == window && p.slot == 0 && p.item.as_ref() == Some(one)
        ));

        // Run the real client click path and apply those authoritative packets. Both sides
        // know the declared caps; only the server executes the part's override.
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let _peer = sunrise_client_gameplay::game::client_side::native_client_session::tests::logged_client(game);
            game.client.remote.as_mut().unwrap().builtin = std::sync::Arc::new(builtin);
            for packet in opening {
                game.handle_packet(packet).unwrap();
            }
            assert_eq!(game.remote_menu_type(), Some(MOD_MENU));
            let [first, second] = [MOD_MENU_SLOT_BASE, MOD_MENU_SLOT_BASE + 1];
            assert_eq!(game.slots(), [first, second]);
            let cursor = || {
                C::SetCursorItem(SetCursorItem {
                    item: Some(seven.clone()),
                })
            };
            game.handle_packet(cursor()).unwrap();
            assert!(game.menu_ops(&[(Some(first), 0, Input::Pickup)]));
            assert_eq!(*game.slot_mut(first), stack(1));
            assert_eq!(game.carried, stack(6));
            assert!(game.menu_ops(&[(Some(first), 0, Input::QuickMove)]));
            assert_eq!(*game.slot_mut(first), stack(1));

            game.handle_packet(cursor()).unwrap();
            assert!(game.menu_ops(&[(Some(second), 0, Input::Pickup)]));
            assert_eq!(*game.slot_mut(second), stack(4));
            assert_eq!(game.carried, stack(3));
            for packet in corrections {
                game.handle_packet(packet).unwrap();
            }
            assert_eq!(*game.slot_mut(first), stack(1));
            assert_eq!(*game.slot_mut(second), stack(1));
            assert_eq!(game.carried, stack(7));
            for packet in component_correction {
                game.handle_packet(packet).unwrap();
            }
            assert_eq!(*game.slot_mut(second), Some(named_one));
            assert_eq!(game.carried, Some(named));
        });
    });
}

#[test]
#[ignore = "daily"]
fn closing_discards_owned_slots_unless_the_listener_returns_their_contents() {
    if run_fresh_cases(
        "cut_layout::game::server_side::native_menus::owned_slots_tests::closing_discards_owned_slots_unless_the_listener_returns_their_contents",
        "SUNRISE_OWNED_MENU_CLOSE_CASE",
        &["close"],
    ) {
        return;
    }
    for return_contents in [false, true] {
        with_owned_menu(return_contents, |game, _rx, key| {
            let (window, menu) = open(game, key);
            let stick =
                mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
            let host: &mut dyn ServerHost = game;
            host.transaction(|host, tx| {
                assert_eq!(
                    ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                        host,
                        tx,
                        &ItemResource::of(&stick),
                        3
                    ),
                    Ok(3)
                );
                Ok::<_, ()>(())
            })
            .unwrap();
            game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: window }));
            game.run_native_menu_events();
            assert_eq!(menu.slot(game, 1), None);
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .cloned()
                    .collect::<Vec<_>>(),
                if return_contents {
                    vec![ItemStack::new("minecraft:stick", 3)]
                } else {
                    vec![]
                }
            );
            if return_contents {
                // GiveCommand.giveItem: a fully fitting give leaves only the one-tick fake item.
                let [fake] = game.dropped.as_ref() else {
                    panic!("expected only the fake pickup, not the contents of the closed menu");
                };
                assert_eq!(fake.stack, ItemStack::new("minecraft:stick", 1));
                assert_eq!((fake.pickup_delay, fake.age), (32767, 5999));
                game.load_ticking_neighbourhood(0, 0);
                game.update_chunk_tickets();
                assert!(game.entity_ticking_area()(0, 0));
                game.server.ticked_entities_through = game.server.next_entity;
                game.tick_items();
            }
            assert!(game.dropped.is_empty());
            let (_, reopened) = open(game, key);
            assert_eq!(reopened.slot(game, 1), Some(None));
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_player_turn_closes_its_native_window_and_returns_both_kinds_of_stack_once() {
    with_owned_menu(true, |game, _rx, key| {
        let (_, menu) = open(game, key);
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let host: &mut dyn ServerHost = game;
        host.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                    host,
                    tx,
                    &ItemResource::of(&stick),
                    3
                ),
                Ok(3)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        let dirt = ItemStack::new("minecraft:dirt", 2);
        CLOSE_NOTICES.set(0);
        game.as_guest(1, |game| {
            game.common.carried = Some(dirt.clone());
            assert!(menu.close_container(game));
            assert!(!menu.close_container(game));
            assert!(game.common.carried.is_none());
        })
        .unwrap();
        game.run_native_menu_events();
        game.run_native_menu_events();
        assert_eq!(CLOSE_NOTICES.get(), 1);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        let mut returned: Vec<_> = guest.inventory.iter().flatten().cloned().collect();
        returned.sort_by(|a, b| a.item.cmp(&b.item));
        assert_eq!(returned, [dirt, ItemStack::new("minecraft:stick", 3)]);
    });
}

fn assert_inventory_cursor(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    rx: &mpsc::Receiver<Out>,
    cursor: &ItemStack,
) {
    let hub = &mut game.server.lan.as_mut().unwrap().hub;
    let expected = hub
        .stack(&Some(cursor.clone()))
        .expect("registered cursor item");
    hub.send_inventory(0);
    assert!(matches!(
        sent(rx).as_slice(),
        [C::ContainerSetContent(snapshot)]
            if snapshot.container == 0 && snapshot.carried.as_ref() == Some(&expected)
    ));
}

#[test]
#[ignore = "daily"]
fn a_transactional_replacement_keeps_the_old_menu_until_commit() {
    with_owned_menu(true, |game, rx, key| {
        let inventory_cursor = ItemStack::new("minecraft:dirt", 3);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.carried = Some(inventory_cursor.clone());
        });
        let (window, old) = open(game, key);
        let player = game.player(123).unwrap();
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let resource = ItemResource::of(&stick);
        let old_slot = ItemPlace::MenuSlot { menu: old, slot: 1 };
        let carried = Some(ItemStack::new("minecraft:stone", 2));
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.carried = carried.clone();
        });
        let host: &mut dyn ServerHost = game;
        host.transaction(|host, tx| {
            assert_eq!(old_slot.insert(host, tx, &resource, 2), Ok(2));
            Ok::<_, ()>(())
        })
        .unwrap();
        sent(&rx);
        CLOSE_NOTICES.set(0);

        let replace = |host: &mut dyn ServerHost, commit| {
            host.transaction(|host, tx| {
                assert_eq!(old_slot.insert(host, tx, &resource, 1), Ok(1));
                let next = player
                    .open_mod_menu(host, {
                        let mut request =
                            mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Replacement"));
                        request.opening_hand = Some(Hand::Off);
                        request.replace = MenuReplace::InPlace;
                        request
                    })
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    player.container_menu(host).unwrap().container_id(host),
                    Some(window)
                );
                assert_eq!(next.container_id(host), None);
                assert_eq!(
                    ItemPlace::MenuSlot {
                        menu: next,
                        slot: 1
                    }
                    .insert(host, tx, &resource, 4),
                    Ok(0)
                );
                assert!(sent(&rx).is_empty());
                if commit { Ok(next) } else { Err(()) }
            })
        };
        assert_eq!(
            replace(game, false),
            Err(mod_api::TransactionError::Aborted(()))
        );
        assert_eq!(old.slot(game, 1).flatten().unwrap().count(), 2);
        assert_eq!(CLOSE_NOTICES.get(), 0);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.carried, carried);
        assert!(guest.inventory.iter().all(Option::is_none));
        assert!(sent(&rx).is_empty());
        assert_inventory_cursor(game, &rx, &inventory_cursor);

        let staged = replace(game, true).unwrap();
        game.run_native_menu_events();
        let replacement = player.container_menu(game).unwrap();
        let next = replacement.container_id(game).unwrap();
        assert_eq!(old.container_id(game), None);
        assert_eq!(staged.container_id(game), None);
        assert_ne!(next, window);
        assert_eq!(replacement.slot(game, 1), Some(None));
        assert_eq!(CLOSE_NOTICES.get(), 1);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert!(guest.carried.is_none());
        assert_eq!(
            guest
                .inventory
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            [
                ItemStack::new("minecraft:stone", 2),
                ItemStack::new("minecraft:stick", 3),
            ]
        );
        assert!(
            sent(&rx)
                .iter()
                .any(|p| matches!(p, C::OpenScreen(p) if p.container == next))
        );
        assert_inventory_cursor(game, &rx, &inventory_cursor);
    });
}

#[test]
#[ignore = "daily"]
fn the_part_closes_the_menu_when_the_opening_stack_leaves_its_hand() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        let change_hand = |game: &mut sunrise_gameplay::game::ServerGame<'_>,
                           change: fn(&mut ItemStack)| {
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                change(g.offhand.as_mut().unwrap());
            });
        };
        change_hand(game, |s| s.count = 3);
        click(game, window, 1);
        assert_eq!(
            menu.container_id(game),
            Some(window),
            "count does not affect the comparison"
        );

        // Same item, different components: the opening stack is no longer held.
        change_hand(game, |s| {
            s.data_mut().custom_name = Some(sunrise_core::nbt::Tag::String("Different".into()));
        });
        click(game, window, 1);
        assert_eq!(menu.container_id(game), None);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerClose(p) if p.container == window
        )));

        let (window, menu) = open(game, key);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.offhand = None);
        click(game, window, 1);
        assert_eq!(menu.container_id(game), None);
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerClose(p) if p.container == window
        )));
    });
}

#[test]
#[ignore = "daily"]
fn a_click_that_changes_the_opening_hand_cannot_run_the_default_pickup() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.native_menu[0] = Some(ItemStack::new("minecraft:stick", 1));
        });
        sent(&rx);
        CLICK_ACTION.set(Some(ClickAction::InvalidateOpeningItem));
        click(game, window, 0);
        CLICK_ACTION.set(None);
        assert_eq!(menu.container_id(game), None);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert!(guest.offhand.is_none(), "the callback's edit committed");
        assert!(
            guest.carried.is_none(),
            "the invalid menu could not be picked up from"
        );
        assert!(guest.inventory.iter().all(Option::is_none));
        assert!(sent(&rx).iter().any(|p| matches!(p,
            C::ContainerClose(p) if p.container == window
        )));
    });
}

#[test]
#[ignore = "daily"]
fn a_click_panic_keeps_a_committed_owned_slot_edit_without_running_default() {
    with_owned_menu(false, |game, rx, key| {
        let (window, menu) = open(game, key);
        sent(&rx);
        CLICK_ACTION.set(Some(ClickAction::PanicAfterCommit));
        click(game, window, 0);
        CLICK_ACTION.set(None);
        let expected = ItemStack::new("minecraft:stick", 1);
        assert_eq!(
            menu.slot(game, 0),
            Some(Some(expected.clone().try_into_api().unwrap()))
        );
        assert_eq!(menu.carried(game), Some(None));
        let expected = game.server.lan.as_mut().unwrap().hub.stack(&Some(expected));
        assert!(
            sent(&rx)
                .iter()
                .any(|p| matches!(p, C::ContainerSetContent(p)
                    if p.container == window && p.items[0] == expected && p.carried.is_none()
                ))
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_close_requested_in_a_click_transaction_preserves_both_cursors_on_abort() {
    with_owned_menu(false, |game, rx, key| {
        let inventory_cursor = ItemStack::new("minecraft:dirt", 3);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.carried = Some(inventory_cursor.clone());
        });
        let (window, menu) = open(game, key);
        let carried = ItemStack::new("minecraft:stone", 2);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.carried = Some(carried.clone());
        });
        sent(&rx);
        CLOSE_NOTICES.set(0);
        CLICK_ACTION.set(Some(ClickAction::Close {
            commit: Some(false),
        }));
        click(game, window, 0);
        CLICK_ACTION.set(None);
        assert_eq!(menu.container_id(game), Some(window));
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.carried.as_ref(), Some(&carried));
        assert!(guest.inventory.iter().all(Option::is_none));
        assert_eq!(CLOSE_NOTICES.get(), 0);
        assert!(!sent(&rx).iter().any(|p| matches!(p, C::ContainerClose(_))));
        assert_inventory_cursor(game, &rx, &inventory_cursor);

        CLICK_ACTION.set(Some(ClickAction::Close { commit: Some(true) }));
        click(game, window, 0);
        CLICK_ACTION.set(None);
        assert_eq!(menu.container_id(game), None);
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(guest.carried.as_ref(), Some(&inventory_cursor));
        assert_eq!(
            guest.inventory[20],
            Some(ItemStack::new("minecraft:gold_ingot", 1))
        );
        assert_eq!(
            guest
                .inventory
                .iter()
                .flatten()
                .filter(|stack| **stack == carried)
                .count(),
            1
        );
        assert_eq!(CLOSE_NOTICES.get(), 1);
        assert!(
            sent(&rx)
                .iter()
                .any(|p| matches!(p, C::ContainerClose(p) if p.container == window))
        );
        let cursors = game
            .server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.take_menu_cursors().collect::<Vec<_>>());
        assert_eq!(cursors, [inventory_cursor]);
    });
}

#[test]
#[ignore = "daily"]
fn scoped_owned_clicks_replace_or_continue_each_input_mode() {
    use sunrise_sim::lan::menu::input::*;
    with_owned_layout(false, true, |game, rx, key| {
        let stone = |count| Some(ItemStack::new("minecraft:stone", count));
        let dirt = Some(ItemStack::new("minecraft:dirt", 1));
        let player = game.player(123).unwrap();
        let mut cases: Vec<_> = (0..=8)
            .chain([40])
            .map(|button| (SWAP, button, 0))
            .collect();
        cases.extend([
            (CLONE, 2, 0),
            (THROW, 0, 0),
            (QUICK_MOVE, 0, 0),
            (PICKUP_ALL, 0, 0),
            (PICKUP, 0, -999),
        ]);
        for (input, button, slot) in cases {
            for flow in [Flow::Cancel, Flow::Continue] {
                let menu = player
                    .open_mod_menu(
                        game,
                        mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Click matrix")),
                    )
                    .unwrap()
                    .unwrap();
                let window = menu.container_id(game).unwrap();
                let hub = &mut game.server.lan.as_mut().unwrap().hub;
                hub.guests.update(0, |g| {
                    g.game_mode = i32::from(input == CLONE);
                    g.inventory.fill(None);
                    g.offhand = (button == 40).then(|| dirt.clone()).flatten();
                    if input == SWAP && button <= 8 {
                        g.inventory[button as usize] = dirt.clone();
                    }
                    g.native_menu[0] = (!matches!(input, PICKUP_ALL | PICKUP))
                        .then(|| stone(1))
                        .flatten();
                    g.native_menu[1] = (input == PICKUP_ALL).then(|| stone(2)).flatten();
                    g.carried = matches!(input, PICKUP_ALL | PICKUP)
                        .then(|| stone(2))
                        .flatten();
                });
                hub.resend_menu(1, window, &game.common.world);
                sent(&rx);
                let before = game.dropped.len();
                send_scoped_click(game, flow, window, slot, button, input);
                let packets = sent(&rx);
                let g = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                let expected = if matches!(flow, Flow::Cancel) {
                    assert!(
                        packets.iter().any(
                            |p| matches!(p, C::ContainerSetContent(p) if p.container == window)
                        ),
                        "{input}:{button}"
                    );
                    assert_eq!(
                        g.native_menu[0],
                        (!matches!(input, PICKUP_ALL | PICKUP))
                            .then(|| stone(1))
                            .flatten()
                    );
                    assert_eq!(
                        g.native_menu[1],
                        (input == PICKUP_ALL).then(|| stone(2)).flatten()
                    );
                    assert_eq!(
                        g.carried,
                        matches!(input, PICKUP_ALL | PICKUP)
                            .then(|| stone(2))
                            .flatten()
                    );
                    assert_eq!(game.dropped.len(), before);
                    dirt.clone()
                } else {
                    match input {
                        SWAP => assert_eq!(g.native_menu[0], dirt),
                        CLONE => {
                            assert_eq!(g.native_menu[0], stone(1));
                            assert_eq!(g.carried, stone(64));
                        }
                        THROW | PICKUP => {
                            assert_eq!(g.native_menu[0], None);
                            assert_eq!(g.carried, None);
                            assert_eq!(game.dropped.len(), before + 1);
                            assert_eq!(
                                game.dropped[before].stack.count,
                                if input == THROW { 1 } else { 2 }
                            );
                        }
                        QUICK_MOVE => {
                            assert_eq!(g.native_menu[0], None);
                            assert_eq!(g.inventory[8], stone(1));
                        }
                        PICKUP_ALL => {
                            assert_eq!(g.native_menu[1], None);
                            assert_eq!(g.carried, stone(4));
                        }
                        _ => unreachable!(),
                    }
                    stone(1)
                };
                if input == SWAP {
                    if button == 40 {
                        assert_eq!(g.offhand, expected);
                        assert_eq!(g.menu_sent[45], expected);
                        let expected = game.server.lan.as_mut().unwrap().hub.stack(&expected);
                        assert_eq!(packets.iter().filter(|p| matches!(p, C::SetPlayerInventory(p) if p.slot == 40 && p.item == expected)).count(), 1);
                    } else {
                        assert_eq!(g.inventory[button as usize], expected);
                    }
                }
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_replaced_or_aborted_drag_step_keeps_the_existing_drag() {
    use sunrise_sim::lan::menu::input::QUICK_CRAFT;
    with_owned_menu(false, |game, rx, key| {
        let stack = |count| Some(ItemStack::new("minecraft:stone", count));
        let (window, _) = open(game, key);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.carried = stack(6));
        sent(&rx);
        for (slot, step) in [(-999, 0), (0, 1), (1, 1), (-999, 2)] {
            send_scoped_click(game, Flow::Cancel, window, slot, step, QUICK_CRAFT);
            assert!(
                sent(&rx)
                    .iter()
                    .any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window))
            );
            let g = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(g.native_menu, [None, None]);
            assert_eq!(g.carried, stack(6));
            // An enclosing abort must discard this step and its publications as well.
            game.storage_scope(|game, _| {
                send_scoped_click(game, Flow::Continue, window, slot, step, QUICK_CRAFT);
                assert!(sent(&rx).is_empty());
                ((), false)
            });
            assert!(sent(&rx).is_empty());
            send_scoped_click(game, Flow::Continue, window, slot, step, QUICK_CRAFT);
            sent(&rx);
        }
        let g = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(g.native_menu, [stack(1), stack(3)]);
        assert_eq!(g.carried, stack(2));
    });
}

fn send_scoped_click(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    flow: Flow,
    window: i32,
    slot: i16,
    button: i8,
    input: i32,
) {
    let state = game
        .server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(1)
        .unwrap()
        .menu
        .as_ref()
        .unwrap()
        .state;
    SCRIPTED_CLICK.set(Some(flow));
    game.lan_guest_packet(
        1,
        S::ContainerClick(ContainerClick {
            container: window,
            state,
            slot,
            button,
            input,
            changed: Vec::new(),
            carried: None,
        }),
    );
    SCRIPTED_CLICK.set(None);
}

#[test]
#[ignore = "daily"]
fn a_click_can_close_or_replace_a_live_menu_without_a_transaction() {
    with_owned_menu(true, |game, rx, key| {
        {
            let (window, menu) = open(game, key);
            let stored = ItemStack::new("minecraft:stick", 3);
            let carried = ItemStack::new("minecraft:dirt", 2);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.update(0, |g| {
                g.inventory.fill(None);
                g.native_menu[1] = Some(stored.clone());
                g.carried = Some(carried.clone());
            });
            hub.resend_menu(1, window, &game.common.world);
            sent(&rx);
            CLOSE_NOTICES.set(0);
            CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
            RECORD_CLOSE.set(true);
            CLICK_ACTION.set(Some(ClickAction::Close { commit: None }));

            click(game, window, 0);
            CLICK_ACTION.set(None);
            assert_eq!(menu.container_id(game), None);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert!(guest.carried.is_none());
            assert!(guest.native_menu.is_empty());
            assert_eq!(
                guest.inventory.iter().flatten().collect::<Vec<_>>(),
                [&carried, &stored],
                "the close returned the cursor and detached owned inputs once"
            );
            assert_eq!(CLOSE_NOTICES.get(), 1);
            RECORD_CLOSE.set(false);
            CLOSE_TRACE.with(|trace| {
                assert_eq!(
                    trace.borrow().as_slice(),
                    ["menu_removed", "player_container_close"]
                )
            });
            let packets = sent(&rx);
            assert_eq!(
                packets
                    .iter()
                    .filter(|p| matches!(p, C::ContainerClose(p) if p.container == window))
                    .count(),
                1
            );
            assert!(
                !packets
                    .iter()
                    .any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window)),
                "no stale resend after closing the menu"
            );
        }

        // Replacing the menu inside `menu_clicked` opens the new window directly and skips
        // default click execution on both the old and replacement windows.
        let (window, menu) = open(game, key);
        let stored = ItemStack::new("minecraft:stick", 3);
        let carried = ItemStack::new("minecraft:dirt", 2);
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.guests.update(0, |g| {
            g.inventory.fill(None);
            g.native_menu[0] = Some(stored.clone());
            g.carried = Some(carried.clone());
        });
        hub.resend_menu(1, window, &game.common.world);
        sent(&rx);
        CLOSE_NOTICES.set(0);
        CLICK_ACTION.set(Some(ClickAction::Replace { mod_menu: key }));

        click(game, window, 0);
        CLICK_ACTION.set(None);
        assert_eq!(menu.container_id(game), None);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        let replacement = guest.menu.as_ref().expect("replacement menu is open");
        assert_ne!(replacement.window, window);
        assert!(guest.carried.is_none());
        assert_eq!(
            guest.native_menu,
            [None, None],
            "default click did not run on the replacement menu"
        );
        assert_eq!(
            guest.inventory.iter().flatten().collect::<Vec<_>>(),
            [&carried, &stored],
            "replacing the menu returned the old cursor and detached owned inputs"
        );
        assert_eq!(CLOSE_NOTICES.get(), 1);
        let packets = sent(&rx);
        assert!(
            packets
                .iter()
                .any(|p| matches!(p, C::OpenScreen(p) if p.container == replacement.window))
        );
        assert!(
            !packets
                .iter()
                .any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window)),
            "no stale resend for the replaced window"
        );
    });
}

#[test]
#[ignore = "daily"]
fn owned_slot_queries_use_the_rule_a_click_will_apply() {
    with_owned_menu(false, |game, _rx, key| {
        let (window, menu) = open(game, key);
        let stored = ItemStack::new("minecraft:stick", 1);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.native_menu[0] = Some(stored.clone()));
        LOCK_FIRST_SLOT.set(true);
        assert_eq!(
            menu.slot_holds(game, 0, &stored.clone().try_into_api().unwrap()),
            Some(0)
        );
        assert_eq!(menu.slot_gives(game, 0), Some(false));
        click(game, window, 0);
        LOCK_FIRST_SLOT.set(false);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(guest.native_menu[0], Some(stored));
        assert!(guest.carried.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn menu_removed_precedes_player_container_close_with_live_handles() {
    with_owned_menu(false, |game, _rx, key| {
        game.server.lan.as_mut().unwrap().play.entry(1).or_default();
        let player = game.player(123).unwrap();
        let (window, menu) = open(game, key);
        let stored = ItemStack::new("minecraft:stick", 1);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.native_menu[0] = Some(stored.clone()));

        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        LAST_REMOVED.with(|slot| *slot.borrow_mut() = None);
        LAST_CONTAINER_CLOSE.with(|slot| *slot.borrow_mut() = None);
        RECORD_CLOSE.set(true);
        game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: window }));
        RECORD_CLOSE.set(false);

        CLOSE_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                ["menu_removed", "player_container_close"]
            );
        });
        let removed = LAST_REMOVED.with(|slot| slot.borrow_mut().take()).unwrap();
        assert_eq!(removed.0, Some(player));
        assert_eq!(removed.1, 123);
        assert_eq!(removed.2, window);
        assert_eq!(removed.3, Some(Hand::Off));
        assert_eq!(removed.4, [Some(stored.try_into_api().unwrap()), None]);

        let closed = LAST_CONTAINER_CLOSE
            .with(|slot| slot.borrow_mut().take())
            .unwrap();
        assert_eq!(closed.0, player);
        assert_eq!(closed.0.uuid(), 123);
        assert_eq!(closed.1, MOD_MENU);
        assert_eq!(closed.2, Some(menu));
        assert_eq!(closed.3, Some(menu));
        assert_eq!(closed.4, Some(None));
        assert_eq!(
            player
                .container_menu(game)
                .and_then(|inventory| inventory.container_id(game)),
            Some(0)
        );
        assert_eq!(menu.slot(game, 0), None);
    });
}

#[test]
#[ignore = "daily"]
fn menu_removed_runs_with_no_player_when_player_is_already_gone() {
    with_owned_menu(false, |game, _rx, key| {
        let (window, _menu) = open(game, key);
        let stored = ItemStack::new("minecraft:diamond", 2);
        {
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            let mut guest = hub.guests.remove(0);
            guest.native_menu[1] = Some(stored.clone());
            if let Some(menu) = guest.menu.as_mut() {
                let closed = menu.native_removed(
                    guest.uuid,
                    guest.menu_generation,
                    &mut guest.native_menu,
                    false,
                );
                hub.native_menu_events.extend(closed);
            }
        }

        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        LAST_REMOVED.with(|slot| *slot.borrow_mut() = None);
        RECORD_CLOSE.set(true);
        game.run_native_menu_events();
        RECORD_CLOSE.set(false);

        CLOSE_TRACE.with(|trace| {
            assert_eq!(trace.borrow().as_slice(), ["menu_removed"]);
        });
        let removed = LAST_REMOVED.with(|slot| slot.borrow_mut().take()).unwrap();
        assert_eq!(removed.0, None);
        assert_eq!(removed.1, 123);
        assert_eq!(removed.2, window);
        assert_eq!(removed.3, Some(Hand::Off));
        assert_eq!(removed.4, [None, Some(stored.try_into_api().unwrap())]);
    });
}

#[test]
#[ignore = "daily"]
fn ordinary_transaction_publishes_the_whole_close_before_a_replacement() {
    with_owned_menu(true, |game, rx, key| {
        let (window, menu) = open(game, key);
        let stack = ItemStack::new("minecraft:stick", 1);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| guest.native_menu[0] = Some(stack.clone()));
        sent(&rx);
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        RECORD_CLOSE.set(true);
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        let abort = host.transaction(|host, _| {
            assert!(menu.close_container(host));
            assert_eq!(player.container_menu(host), Some(menu));
            assert!(sent(&rx).is_empty());
            Err::<(), _>(())
        });
        assert!(abort.is_err());
        assert_eq!(player.container_menu(host), Some(menu));
        CLOSE_TRACE.with(|trace| assert!(trace.borrow().is_empty()));
        let staged = host
            .transaction(|host, _| {
                assert!(menu.close_container(host));
                let next = player
                    .open_chest_menu(
                        host,
                        1,
                        mod_api::Text::literal("Next"),
                        Vec::new(),
                        MenuReplace::InPlace,
                    )
                    .unwrap();
                assert_eq!(player.container_menu(host), Some(menu));
                assert_eq!(next.container_id(host), None);
                assert!(sent(&rx).is_empty());
                CLOSE_TRACE.with(|trace| assert!(trace.borrow().is_empty()));
                Ok::<_, ()>(next)
            })
            .unwrap();
        RECORD_CLOSE.set(false);
        CLOSE_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                [
                    "menu_removed",
                    "player_container_close",
                    "player_container_open"
                ]
            )
        });
        assert_eq!(menu.container_id(game), None);
        assert_eq!(staged.container_id(game), None);
        assert_eq!(
            player
                .container_menu(game)
                .and_then(|menu| menu.container_id(game)),
            Some(window % 100 + 1)
        );
        assert!(
            game.server.lan.as_ref().unwrap().hub.guests[0]
                .inventory
                .contains(&Some(stack))
        );
    });
}

#[test]
#[ignore = "daily"]
fn owner_can_open_a_replacement_without_interrupting_teardown() {
    with_owned_menu(false, |game, rx, key| {
        for from_packet in [true, false] {
            let (window, menu) = open(game, key);
            sent(&rx);
            CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
            RECORD_CLOSE.set(true);
            REOPEN_ON_REMOVED.set(true);
            if from_packet {
                game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: window }));
            } else {
                assert!(menu.close_container(game));
            }
            RECORD_CLOSE.set(false);
            CLOSE_TRACE.with(|trace| {
                assert_eq!(
                    trace.borrow().as_slice(),
                    [
                        "menu_removed",
                        "removed continued",
                        "player_container_close",
                        "player_container_open"
                    ]
                )
            });
            let old = LAST_CONTAINER_CLOSE.with(|close| close.borrow_mut().take().unwrap());
            assert_eq!(old.2, Some(menu));
            assert_eq!(old.3, Some(menu));
            assert_eq!(menu.container_id(game), None);
            let packets = sent(&rx);
            let [next] = packets
                .iter()
                .filter_map(|packet| match packet {
                    C::OpenScreen(open) => Some(open.container),
                    _ => None,
                })
                .collect::<Vec<_>>()[..]
            else {
                panic!("expected one replacement screen")
            };
            let player = game.player(123).unwrap();
            assert_eq!(
                player
                    .container_menu(game)
                    .and_then(|menu| menu.container_id(game)),
                Some(next)
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn unsupported_crafting_close_does_not_report_the_inventory() {
    with_owned_menu(false, |game, rx, _| {
        game.lan_open_crafting(1, [0, 64, 0]);
        let window = sent(&rx)
            .iter()
            .find_map(|packet| match packet {
                C::OpenScreen(open) => Some(open.container),
                _ => None,
            })
            .unwrap();
        RECORD_CLOSE.set(true);
        game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: window }));
        RECORD_CLOSE.set(false);
        let closed = LAST_CONTAINER_CLOSE.with(|event| event.borrow_mut().take().unwrap());
        assert_eq!(closed.1, "minecraft:crafting");
        assert_eq!(closed.2, None);
        assert_eq!(closed.3, None);
        assert_eq!(
            closed.0.container_menu(game).unwrap().container_id(game),
            Some(0)
        );
    });
}

#[test]
#[ignore = "daily"]
fn late_container_sync_invalidation_closes_in_its_original_level() {
    use crate::game::Dim;
    use sunrise_level::chunk::block_entity::BlockEntity;
    use sunrise_sim::lan::menu::Slots;
    with_owned_menu(false, |game, rx, _| {
        let pos = (0, 64, 0);
        game.world.block_entities.insert(
            pos,
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:apple", 1)); 27]),
        );
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .block_entities
            .insert(pos, BlockEntity::Chest(vec![None; 27]));
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.update_guest(1, |guest| {
            guest.dimension = Dim::Nether.id().into();
            guest.position = [0.5, 64.0, 0.5];
        });
        hub.open_container(
            1,
            "minecraft:generic_9x3",
            sunrise_core::nbt::Tag::String("Chest".into()),
            pos,
            Slots::Block(pos),
            vec![None; 27],
            None,
        );
        game.player_menu_opening(1);
        let player = game.player(123).unwrap();
        let menu = player.container_menu(game).unwrap();
        let window = menu.container_id(game).unwrap();
        game.lan_containers_tick();
        assert_eq!(menu.slot(game, 0), Some(None));
        sent(&rx);
        game.world.block_entities.remove(&pos);
        let events = game
            .server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert_eq!(events.len(), 1);
        assert!(sent(&rx).is_empty());
        assert_eq!(menu.container_id(game), Some(window));
        game.enter_level(Dim::Overworld).unwrap();
        RECORD_CLOSE.set(true);
        let host = game.server.lan.take().unwrap();
        game.server.lan = game.lan_process_events(host, events, &[], [0.5, 64.0, 0.5]);
        RECORD_CLOSE.set(false);
        let closed = LAST_CONTAINER_CLOSE.with(|event| event.borrow_mut().take().unwrap());
        assert_eq!(closed.2, Some(menu));
        assert_eq!(
            closed.4, None,
            "did not read overworld backing at the same coordinates"
        );
        LAST_CLOSE_DIMENSION
            .with(|dimension| assert_eq!(dimension.borrow().as_deref(), Some(Dim::Nether.id())));
        assert_eq!(game.dim, Dim::Overworld);
        assert_eq!(menu.container_id(game), None);
        assert_eq!(
            sent(&rx)
                .iter()
                .filter(
                    |packet| matches!(packet, C::ContainerClose(close) if close.container == window)
                )
                .count(),
            1
        );
    });
}

#[test]
#[ignore = "daily"]
fn disconnect_reports_logout_then_removed_then_close_with_live_views() {
    with_owned_menu(false, |game, _rx, key| {
        let (window, menu) = open(game, key);
        let player = game.player(123).unwrap();
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        RECORD_CLOSE.set(true);
        game.server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .lan
            .shared
            .events
            .send(sunrise_sim::lan::NetEvent::Left(
                1,
                "test disconnect".into(),
            ))
            .unwrap();
        game.tick_lan_packets();
        RECORD_CLOSE.set(false);
        CLOSE_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                [
                    "player_logged_out",
                    "menu_removed",
                    "player_container_close"
                ]
            )
        });
        let removed = LAST_REMOVED.with(|event| event.borrow_mut().take().unwrap());
        assert_eq!(removed.0, Some(player));
        assert_eq!(removed.2, window);
        let closed = LAST_CONTAINER_CLOSE.with(|event| event.borrow_mut().take().unwrap());
        assert_eq!(closed.0, player);
        assert_eq!(closed.2, Some(menu));
        assert_eq!(closed.3, Some(menu));
        assert!(game.player(123).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn staging_replacement_keeps_nested_edits_on_the_live_owner_until_commit() {
    with_owned_menu(true, |game, rx, key| {
        let (_, old) = open(game, key);
        let player = game.player(123).unwrap();
        let stick =
            mod_api::ItemStack::of(mod_api::__private::item_key("minecraft:stick").unwrap());
        let resource = ItemResource::of(&stick);
        let old_slot = ItemPlace::MenuSlot { menu: old, slot: 1 };
        let host: &mut dyn ServerHost = game;
        host.transaction(|host, tx| {
            assert_eq!(old_slot.insert(host, tx, &resource, 2), Ok(2));
            Ok::<_, ()>(())
        })
        .unwrap();
        sent(&rx);
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        RECORD_CLOSE.set(true);
        let staged = host
            .transaction(|host, tx| {
                assert_eq!(old_slot.insert(host, tx, &resource, 1), Ok(1));
                let next = player
                    .open_mod_menu(host, {
                        let mut request =
                            mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Replacement"));
                        request.opening_hand = Some(Hand::Off);
                        request.replace = MenuReplace::InPlace;
                        request
                    })
                    .unwrap()
                    .unwrap();
                assert_eq!(player.container_menu(host), Some(old));
                assert_eq!(next.container_id(host), None);
                let nested: Result<(), mod_api::TransactionError<()>> =
                    host.transaction(|host, tx| {
                        assert_eq!(old_slot.insert(host, tx, &resource, 1), Ok(1));
                        Err(())
                    });
                assert!(nested.is_err());
                assert_eq!(
                    old.slot(host, 1),
                    Some(Some(
                        ItemStack::new("minecraft:stick", 3).try_into_api().unwrap()
                    ))
                );
                assert!(sent(&rx).is_empty());
                CLOSE_TRACE.with(|trace| assert!(trace.borrow().is_empty()));
                Ok::<_, ()>(next)
            })
            .unwrap();
        RECORD_CLOSE.set(false);
        CLOSE_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                [
                    "menu_removed",
                    "player_container_close",
                    "player_container_open"
                ]
            )
        });
        assert_eq!(old.container_id(game), None);
        assert_eq!(staged.container_id(game), None);
        let replacement = game.player_container_menu(player).unwrap();
        assert_ne!(replacement, old);
        assert_eq!(replacement.slot(game, 1), Some(None));
        let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
        assert_eq!(
            guest
                .inventory
                .iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>(),
            [ItemStack::new("minecraft:stick", 3)],
            "old teardown returned committed plus pre-replacement writes"
        );
    });
}

#[test]
#[ignore = "daily"]
fn aborted_window_reservations_do_not_collide_at_wraparound() {
    with_owned_menu(false, |game, rx, key| {
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(1, |g| g.next_window = 99);
        let (window, old) = open(game, key);
        assert_eq!(window, 100);
        sent(&rx);
        let expected_next = window % 100 + 1;
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        for _ in 0..99 {
            let aborted: Result<(), mod_api::TransactionError<()>> = host.transaction(|host, _| {
                let reserved = player
                    .open_chest_menu(
                        host,
                        1,
                        mod_api::Text::literal("Aborted"),
                        Vec::new(),
                        MenuReplace::InPlace,
                    )
                    .unwrap();
                assert_eq!(reserved.container_id(host), None);
                Err(())
            });
            assert!(aborted.is_err());
        }
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].next_window,
            window
        );
        assert_eq!(old.container_id(game), Some(window));
        let host: &mut dyn ServerHost = game;
        let next = host
            .transaction(|host, _| {
                let next = player
                    .open_chest_menu(
                        host,
                        1,
                        mod_api::Text::literal("Committed"),
                        Vec::new(),
                        MenuReplace::InPlace,
                    )
                    .unwrap();
                Ok::<_, ()>(next)
            })
            .unwrap();
        assert_eq!(next.container_id(game), None);
        assert_eq!(old.container_id(game), None);
        assert_eq!(
            player
                .container_menu(game)
                .and_then(|menu| menu.container_id(game)),
            Some(expected_next)
        );
        assert_eq!(
            game.server.lan.as_ref().unwrap().hub.guests[0].next_window,
            expected_next
        );
    });
}

#[test]
#[ignore = "daily"]
fn reentrant_open_follows_both_root_opens_and_ignores_a_stale_close() {
    with_owned_menu(false, |game, rx, key| {
        let (old, _) = open(game, key);
        let hub = &game.server.lan.as_ref().unwrap().hub;
        let [first_kind, second_kind, reentrant_kind] = [2, 3, 1].map(|rows| {
            hub.registry_id("minecraft:menu", &format!("minecraft:generic_9x{rows}"))
                .unwrap()
        });
        sent(&rx);
        REOPEN_ON_REMOVED.set(true);
        let player = game.player(123).unwrap();
        game.transaction(|host, _| {
            let [first, second] = [(2, "First"), (3, "Second")].map(|(rows, title)| {
                player
                    .open_chest_menu(
                        host,
                        rows,
                        mod_api::Text::literal(title),
                        Vec::new(),
                        MenuReplace::InPlace,
                    )
                    .unwrap()
            });
            assert_eq!(first.container_id(host), None);
            assert_eq!(second.container_id(host), None);
            assert_ne!(first, second);
            assert!(sent(&rx).is_empty());
            Ok::<_, ()>(())
        })
        .unwrap();

        let first = old % 100 + 1;
        let second = first % 100 + 1;
        let reentrant = second % 100 + 1;
        let packets = sent(&rx);
        let opens: Vec<_> = packets
            .iter()
            .filter_map(|packet| match packet {
                C::OpenScreen(open) => Some((open.container, open.menu)),
                _ => None,
            })
            .collect();
        assert_eq!(
            opens,
            [
                (first, first_kind),
                (second, second_kind),
                (reentrant, reentrant_kind)
            ]
        );
        assert!(!packets.iter().any(|p| matches!(p, C::ContainerClose(_))));
        let replacement = player.container_menu(game).unwrap();
        assert_eq!(replacement.container_id(game), Some(reentrant));
        game.lan_guest_packet(1, S::ContainerClose(ContainerClose { container: second }));
        assert_eq!(player.container_menu(game), Some(replacement));
    });
}

#[test]
#[ignore = "daily"]
fn disconnect_before_publication_preserves_a_stack_offered_to_the_pending_menu() {
    with_owned_menu(false, |game, rx, key| {
        let (window, current) = open(game, key);
        let stack = ItemStack::new("minecraft:stick", 2);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(1, |guest| {
                guest.inventory[0] = Some(stack.clone());
            });
        sent(&rx);

        let player = game.player(123).unwrap();
        let resource = ItemResource::of(&stack.clone().try_into_api().unwrap());
        game.transaction(|host, _| {
            let pending = player
                .open_mod_menu(
                    host,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Pending")),
                )
                .unwrap()
                .unwrap();
            // Use the ordinary journal to offer a real extraction to the replacement.
            // Until publication it has no installed owner, so the transfer rolls back.
            let moved = host.transaction(|host, tx| {
                let source = ItemPlace::PlayerSlot {
                    player: 123,
                    slot: 0,
                };
                assert_eq!(source.extract(host, tx, &resource, 2), Ok(2));
                let accepted = ItemPlace::MenuSlot {
                    menu: pending,
                    slot: 1,
                }
                .insert(host, tx, &resource, 2);
                if accepted == Ok(2) { Ok(2) } else { Err(()) }
            });
            assert_eq!(moved, Err(mod_api::TransactionError::Aborted(())));
            assert!(
                player
                    .disconnect(host, mod_api::Text::literal("Leaving before publication"))
                    .unwrap()
            );
            Ok::<_, ()>(())
        })
        .unwrap();

        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert!(guest.disconnect_pending);
        assert_eq!(guest.next_window, window);
        assert_eq!(guest.inventory[0], Some(stack));
        assert_eq!(player.container_menu(game), Some(current));
        assert!(!sent(&rx).iter().any(|p| matches!(p, C::OpenScreen(_))));
    });
}

#[test]
#[ignore = "daily"]
fn reentrant_open_during_travel_close_opens_after_change_level() {
    use crate::game::Dim;
    with_owned_menu(false, |game, rx, key| {
        let (_window, old) = open(game, key);
        sent(&rx);
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        RECORD_CLOSE.set(true);
        REOPEN_ON_REMOVED.set(true);
        game.start_server_trip(1, Dim::Nether, None);
        RECORD_CLOSE.set(false);
        CLOSE_TRACE.with(|trace| {
            assert_eq!(
                trace.borrow().as_slice(),
                [
                    "menu_removed",
                    "removed continued",
                    "player_container_close",
                    "player_container_open"
                ]
            )
        });
        let packets = sent(&rx);
        let respawn_pos = packets
            .iter()
            .position(|packet| matches!(packet, C::Respawn(_)))
            .expect("travel sent Respawn");
        let (open_pos, next) = packets
            .iter()
            .enumerate()
            .find_map(|(idx, packet)| match packet {
                C::OpenScreen(open) => Some((idx, open.container)),
                _ => None,
            })
            .expect("replacement opened after travel");
        assert!(
            respawn_pos < open_pos,
            "replacement OpenScreen must follow Respawn"
        );
        assert!(
            !packets
                .iter()
                .any(|packet| matches!(packet, C::ContainerClose(_))),
            "travel close relies on Respawn without a close packet"
        );
        assert_eq!(old.container_id(game), None);
        game.enter_level(Dim::Nether).unwrap();
        let player = game.player(123).unwrap();
        let replacement = player.container_menu(game).unwrap();
        assert_eq!(
            replacement.container_id(game),
            Some(next),
            "change_level must not retire the reentrant replacement"
        );
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        LAST_CLOSE_DIMENSION.with(|dimension| *dimension.borrow_mut() = None);
        RECORD_CLOSE.set(true);
        assert!(replacement.close_container(game));
        RECORD_CLOSE.set(false);
        CLOSE_TRACE.with(|trace| {
            assert_eq!(trace.borrow().as_slice(), ["player_container_close"]);
        });
        LAST_CLOSE_DIMENSION
            .with(|dimension| assert_eq!(dimension.borrow().as_deref(), Some(Dim::Nether.id())));
        assert_eq!(replacement.container_id(game), None);
    });
}

/// A replacement the closing listener asks for while the player's menus close for a save stays
/// unpublished through the save: nothing real can reach a transient slot the save omits. A
/// leaving player never gets it; otherwise it publishes once the save is done.
#[test]
#[ignore = "daily"]
fn an_open_asked_during_a_save_close_publishes_only_after_the_save_if_the_player_stays() {
    for leaving in [true, false] {
        with_owned_menu(false, |game, rx, key| {
            open(game, key);
            sent(&rx);
            REOPEN_ON_REMOVED.set(true);
            game.test_close_guest_menus_for_save(1, leaving, || {
                assert!(
                    !sent(&rx).iter().any(|p| matches!(p, C::OpenScreen(_))),
                    "the replacement stays unpublished through the save"
                );
            });
            let opened = sent(&rx)
                .iter()
                .filter(|p| matches!(p, C::OpenScreen(_)))
                .count();
            assert_eq!(opened, usize::from(!leaving));
        });
    }
}

#[test]
#[ignore = "daily"]
fn a_departure_from_an_unenterable_level_still_closes_the_heard_menu() {
    let previous_extra = sunrise_gameplay_common::game::EXTRA_DIMS
        .read()
        .unwrap()
        .clone();
    with_owned_menu(false, |game, rx, key| {
        open(game, key);
        sent(&rx);
        sunrise_gameplay_common::game::Dim::add_extra("test:unenterable");
        let mut host = game.server.lan.take().unwrap();
        let mut gone = host.hub.guests.remove(0);
        gone.dimension = "test:unenterable".to_owned();
        CLOSE_TRACE.with(|trace| trace.borrow_mut().clear());
        RECORD_CLOSE.set(true);
        game.server.lan = game.lan_process_events(
            host,
            [sunrise_sim::lan::hub::HostEvent::Left(Box::new(gone))],
            &[],
            [0.5, 64.0, 0.5],
        );
        RECORD_CLOSE.set(false);
        let trace = CLOSE_TRACE.with(|trace| trace.borrow().clone());
        assert_eq!(
            trace
                .iter()
                .filter(|t| **t == "player_container_close")
                .count(),
            1,
            "one close for the open that was heard: {trace:?}"
        );
    });
    *sunrise_gameplay_common::game::EXTRA_DIMS.write().unwrap() = previous_extra;
}

#[derive(Clone, Copy)]
enum ChangeMode {
    Normal,
    OverCap,
    PlayerContainer,
    OutOfRange,
    Panic,
    WriteItemPlaceThenInvalid,
    WriteItemPlaceThenPanic,
}

#[derive(Clone, Copy)]
enum CancelClickMode {
    Continue,
    InsertSlot0Stick,
}

thread_local! {
    static CHANGE_MODE: std::cell::Cell<ChangeMode> = const { std::cell::Cell::new(ChangeMode::Normal) };
    static CANCEL_CLICK_MODE: std::cell::Cell<CancelClickMode> = const { std::cell::Cell::new(CancelClickMode::Continue) };
    static CHANGE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LAST_CHANGED_SLOTS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
    static CLOSED_OWNED_LEN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

struct ProcessingPart;
#[mod_api::sunrise_part]
impl MenuPart for ProcessingPart {
    fn menu_clicked(&self, host: &mut dyn ServerHost, event: &mut MenuClickedEvent<'_>) -> Flow {
        match CANCEL_CLICK_MODE.get() {
            CancelClickMode::Continue => Flow::Continue,
            CancelClickMode::InsertSlot0Stick => {
                let player = host.player(event.uuid).unwrap();
                let menu = player.container_menu(host).unwrap();
                let stick = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stick").unwrap(),
                    1,
                )
                .unwrap();
                let inserted = host
                    .transaction(|host, tx| {
                        ItemPlace::MenuSlot { menu, slot: 0 }.insert(
                            host,
                            tx,
                            &ItemResource::of(&stick),
                            1,
                        )
                    })
                    .expect("click transaction");
                assert_eq!(inserted, 1);
                Flow::Cancel
            }
        }
    }

    fn menu_slot_changed(&self, host: &mut dyn ServerHost, event: &mut MenuSlotChangedEvent<'_>) {
        assert_eq!(event.menu, MOD_MENU);
        assert_eq!(event.context.player, Some(123));
        assert_eq!(event.context.pos, None);
        CHANGE_CALLS.set(CHANGE_CALLS.get() + 1);
        LAST_CHANGED_SLOTS.with(|slots| {
            *slots.borrow_mut() = event.changed.iter().map(|c| c.slot).collect();
        });
        let stone = |count| {
            mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:stone").unwrap(),
                count,
            )
            .unwrap()
        };
        // Writes that settle with the click's own transaction, so its abort undoes them.
        let write_side_effects = |host: &mut dyn ServerHost| {
            let player = host.player(123).unwrap();
            let reward = mod_api::ItemStack::new(
                mod_api::__private::item_key("minecraft:apple").unwrap(),
                1,
            )
            .unwrap();
            host.current_transaction(|host, tx| {
                assert_eq!(
                    ItemPlace::PlayerSlot {
                        player: 123,
                        slot: 0,
                    }
                    .insert(host, tx, &ItemResource::of(&reward), 1),
                    Ok(1)
                );
                let menu = player.container_menu(host).unwrap();
                assert_eq!(
                    ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                        host,
                        tx,
                        &ItemResource::of(&reward),
                        1
                    ),
                    Ok(1)
                );
            })
            .expect("active click transaction");
        };
        match CHANGE_MODE.get() {
            ChangeMode::Normal => {
                let a = event.context.slot(MenuContainer::Main, 0);
                let b = event.context.slot(MenuContainer::Main, 1);
                let out = match (a, b) {
                    (Some(_), Some(_)) => Some(stone(2)),
                    _ => None,
                };
                event
                    .outputs
                    .push(MenuSlotWrite::new(MenuContainer::Main, 2, out));
            }
            ChangeMode::OverCap => {
                // Result slot 2 has cap 4.
                event
                    .outputs
                    .push(MenuSlotWrite::new(MenuContainer::Main, 2, Some(stone(5))));
            }
            ChangeMode::PlayerContainer => {
                event
                    .outputs
                    .push(MenuSlotWrite::new(MenuContainer::Player, 0, Some(stone(1))));
            }
            ChangeMode::OutOfRange => {
                event
                    .outputs
                    .push(MenuSlotWrite::new(MenuContainer::Main, 3, Some(stone(1))));
            }
            ChangeMode::Panic => panic!("menu_slot_changed failed"),
            ChangeMode::WriteItemPlaceThenInvalid => {
                write_side_effects(host);
                event
                    .outputs
                    .push(MenuSlotWrite::new(MenuContainer::Main, 2, Some(stone(5))));
            }
            ChangeMode::WriteItemPlaceThenPanic => {
                write_side_effects(host);
                panic!("menu_slot_changed failed after ItemPlace writes");
            }
        }
    }

    fn menu_removed(&self, _: &mut dyn ServerHost, event: &mut MenuRemovedEvent<'_>) {
        CLOSED_OWNED_LEN.set(Some(event.owned_slots.len()));
    }
}

struct ProcessingMod;
#[mod_api::__subscriptions]
impl Hooks for ProcessingMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(MOD_MENU, ProcessingPart);
    }
}

fn with_processing_owned_menu(
    test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>, mpsc::Receiver<Out>, MenuKey),
) {
    let mods: Mods = vec![ModEntry::new("listen", Box::new(ProcessingMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        api_values::install_parts(&mods).unwrap();
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "listen".into(),
                3,
                0,
                vec![2],
                vec![0, 1],
                None,
                None,
                true,
                Some(vec![64, 64, 4]),
            )],
            || {
                with_game(|game| {
                    let (rx, key) = mod_menu_guest(game);
                    CHANGE_MODE.set(ChangeMode::Normal);
                    CANCEL_CLICK_MODE.set(CancelClickMode::Continue);
                    CHANGE_CALLS.set(0);
                    LAST_CHANGED_SLOTS.with(|slots| slots.borrow_mut().clear());
                    CLOSED_OWNED_LEN.set(None);
                    test(game, rx, key);
                });
            },
        );
    });
}

fn raw_click(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    conn: u32,
    window: i32,
    slot: i16,
    button: i8,
    input: i32,
) {
    let state = game
        .server
        .lan
        .as_ref()
        .unwrap()
        .hub
        .guest(conn)
        .unwrap()
        .menu
        .as_ref()
        .unwrap()
        .state;
    game.lan_guest_packet(
        conn,
        S::ContainerClick(ContainerClick {
            container: window,
            state,
            slot,
            button,
            input,
            changed: Vec::new(),
            carried: None,
        }),
    );
}

#[test]
#[ignore = "daily"]
fn processing_owned_menu_computes_results_via_slot_changed_and_rejects_invalid_writes() {
    with_processing_owned_menu(|game, rx, key| {
        let player = game.player(123).unwrap();
        let menu = game
            .as_guest(1, |game| {
                let mut req = mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor"));
                req.pos = Some([5, 64, 5]);
                player.open_mod_menu(game, req)
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        sent(&rx);

        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        let dirt = |n| Some(ItemStack::new("minecraft:dirt", n));
        let stone = |n| Some(ItemStack::new("minecraft:stone", n));

        // Put stick in slot 0: fires once, slot 2 stays empty.
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.carried = stick(3));
        click(game, window, 0);
        assert_eq!(CHANGE_CALLS.get(), 1);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [stick(3), None, None]
        );

        // Placing directly into result slot 2 is refused (and does not fire menu_slot_changed).
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.carried = dirt(2));
        click(game, window, 2);
        assert_eq!(CHANGE_CALLS.get(), 1);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu[2],
            None
        );

        // Put dirt in slot 1: fires once (not re-fired by result output), populates result slot 2.
        sent(&rx);
        click(game, window, 1);
        assert_eq!(
            CHANGE_CALLS.get(),
            2,
            "writing result slot via outputs must not re-fire menu_slot_changed"
        );
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(guest.native_menu, [stick(3), dirt(2), stone(2)]);
        assert_eq!(guest.carried, None);
        let packets = sent(&rx);
        assert!(packets.iter().any(|p| matches!(
            p,
            C::ContainerSetSlot(s) if s.container == window && s.slot == 2 && s.item.is_some()
        ) || matches!(
            p,
            C::ContainerSetContent(c) if c.container == window && c.items.get(2).is_some_and(Option::is_some)
        )));

        // Removing slot 0 clears result slot 2.
        click(game, window, 0);
        assert_eq!(CHANGE_CALLS.get(), 3);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(guest.native_menu, [None, dirt(2), None]);
        assert_eq!(guest.carried, stick(3));

        // Invalid output writes and panics roll back the whole click and resend the menu.
        for bad in [
            ChangeMode::OverCap,
            ChangeMode::PlayerContainer,
            ChangeMode::OutOfRange,
            ChangeMode::Panic,
        ] {
            CHANGE_MODE.set(bad);
            sent(&rx);
            click(game, window, 0);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(
                guest.native_menu,
                [None, dirt(2), None],
                "failed menu_slot_changed must roll back transient slots"
            );
            assert_eq!(
                guest.carried,
                stick(3),
                "failed menu_slot_changed must roll back cursor"
            );
            assert!(
                sent(&rx)
                    .iter()
                    .any(|p| matches!(p, C::ContainerSetContent(c) if c.container == window)),
                "aborted click must resend the menu"
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn owned_menu_slot_changed_fires_on_item_place_writes_and_deduplicates_per_transaction() {
    with_processing_owned_menu(|game, rx, key| {
        let player = game.player(123).unwrap();
        let menu = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        sent(&rx);

        let stick_api = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        let dirt_api = ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap();
        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        let dirt = |n| Some(ItemStack::new("minecraft:dirt", n));
        let stone = |n| Some(ItemStack::new("minecraft:stone", n));

        // 1. Aborted transaction writing via ItemPlace::MenuSlot must never fire menu_slot_changed.
        let _: Result<(), mod_api::TransactionError<()>> = game.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.insert(
                    host,
                    tx,
                    &ItemResource::of(&stick_api),
                    2
                ),
                Ok(2)
            );
            Err(())
        });
        assert_eq!(CHANGE_CALLS.get(), 0);
        assert!(sent(&rx).is_empty());

        // 2. Single committed transaction writing both input slots (slot 1 then slot 0) delivers
        // ONE deduplicated menu_slot_changed call with slots in ascending order [0, 1]; the next
        // broadcastChanges sends the viewer the listener's result once.
        game.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 1 }.insert(
                    host,
                    tx,
                    &ItemResource::of(&dirt_api),
                    2
                ),
                Ok(2)
            );
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.insert(
                    host,
                    tx,
                    &ItemResource::of(&stick_api),
                    3
                ),
                Ok(3)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(CHANGE_CALLS.get(), 1);
        assert_eq!(LAST_CHANGED_SLOTS.with(|s| s.borrow().clone()), vec![0, 1]);
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(guest.native_menu, [stick(3), dirt(2), stone(2)]);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
            sent(&rx)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == window && s.slot == 2 && s.item.is_some())),
            "broadcastChanges sends the listener's result"
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
            !sent(&rx)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == window)),
            "a second broadcastChanges sends nothing more"
        );

        // 3. Clearing slot 0 through a menu_clicked Flow::Cancel override -> first empty slot 0,
        // then let menu_clicked insert stick into slot 0 and return Flow::Cancel.
        game.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.extract(
                    host,
                    tx,
                    &ItemResource::of(&stick_api),
                    3
                ),
                Ok(3)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(CHANGE_CALLS.get(), 2);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [None, dirt(2), None]
        );
        sent(&rx);

        CANCEL_CLICK_MODE.set(CancelClickMode::InsertSlot0Stick);
        click(game, window, 0);
        assert_eq!(
            CHANGE_CALLS.get(),
            3,
            "Flow::Cancel menu_clicked edit must fire menu_slot_changed once"
        );
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [stick(1), dirt(2), stone(2)]
        );

        // 4. Invalid output write inside menu_slot_changed during Flow::Cancel rolls back
        // menu_clicked's ItemPlace write too.
        game.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.extract(
                    host,
                    tx,
                    &ItemResource::of(&stick_api),
                    1
                ),
                Ok(1)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        CHANGE_MODE.set(ChangeMode::OverCap);
        click(game, window, 0);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [stick(1), dirt(2), None],
            "failed menu_slot_changed aborts its own outputs while menu_clicked's committed write stands"
        );
    });
}

#[test]
#[ignore = "daily"]
fn owned_menu_slot_changed_batches_multi_slot_drag_and_covers_swap_throw_pickup_all() {
    with_processing_owned_menu(|game, _rx, key| {
        let player = game.player(123).unwrap();
        let menu = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        let dirt = |n| Some(ItemStack::new("minecraft:dirt", n));
        let stone = |n| Some(ItemStack::new("minecraft:stone", n));

        // 1. Multi-slot drag across slots 0 and 1 (left drag: mode 5, start=0, add=1, end=2):
        // intermediate add steps do not fire; drag-end fires ONCE with [0, 1] in menu order.
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.carried = stick(4));
        raw_click(game, 1, window, -999, 0, 5);
        raw_click(game, 1, window, 1, 1, 5);
        raw_click(game, 1, window, 0, 1, 5);
        assert_eq!(CHANGE_CALLS.get(), 0);
        raw_click(game, 1, window, -999, 2, 5);
        assert_eq!(CHANGE_CALLS.get(), 1);
        assert_eq!(LAST_CHANGED_SLOTS.with(|s| s.borrow().clone()), vec![0, 1]);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [stick(2), stick(2), stone(2)]
        );

        // 2. Number-key swap (mode 2, button 0 -> hotbar 0 into slot 0).
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |g| g.inventory[0] = dirt(3));
        raw_click(game, 1, window, 0, 0, 2);
        assert_eq!(CHANGE_CALLS.get(), 2);
        assert_eq!(LAST_CHANGED_SLOTS.with(|s| s.borrow().clone()), vec![0]);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [dirt(3), stick(2), stone(2)]
        );

        // 3. Throw single item from slot 0 (mode 4, button 0).
        raw_click(game, 1, window, 0, 0, 4);
        assert_eq!(CHANGE_CALLS.get(), 3);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [dirt(2), stick(2), stone(2)]
        );

        // 4. Pickup-all (mode 6) collecting dirt from slot 0 onto a matching cursor -> clears slot 0
        // and clears result slot 2. Like a real double-click it lands on an empty slot (menu slot 3,
        // the first main inventory slot): `doClick` collects nothing when the clicked slot still
        // holds a stack it may pick up.
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.carried = dirt(1);
            g.inventory[9] = None;
        });
        raw_click(game, 1, window, 3, 0, 6);
        assert_eq!(CHANGE_CALLS.get(), 4);
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .native_menu,
            [None, stick(2), None]
        );
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .carried,
            dirt(3)
        );
    });
}

#[test]
#[ignore = "daily"]
fn owned_menu_slot_changed_rolls_back_listener_item_place_writes_on_invalid_output_or_panic() {
    with_processing_owned_menu(|game, rx, key| {
        let player = game.player(123).unwrap();
        let menu = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        for bad in [
            ChangeMode::WriteItemPlaceThenInvalid,
            ChangeMode::WriteItemPlaceThenPanic,
        ] {
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.inventory.fill(None);
                g.carried = stick(3);
            });
            CHANGE_MODE.set(bad);
            sent(&rx);
            click(game, window, 0);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
            assert_eq!(
                guest.native_menu,
                [None, None, None],
                "listener MenuSlot writes must roll back with the click"
            );
            assert!(
                guest.inventory.iter().all(Option::is_none),
                "listener PlayerInventory writes must roll back with the click"
            );
            assert_eq!(guest.carried, stick(3));
        }
    });
}

const BLOCK_PROC_MENU: &str = "minecraft:generic_9x3";
const BLOCK_PROC_POS: (i32, i32, i32) = (1, 64, 1);

thread_local! {
    static BLOCK_CHANGE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static BLOCK_LAST_SLOTS: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
}

struct BlockProcessingPart;
#[mod_api::sunrise_part]
impl MenuPart for BlockProcessingPart {
    fn menu_slot_changed(&self, _: &mut dyn ServerHost, event: &mut MenuSlotChangedEvent<'_>) {
        assert_eq!(event.menu, BLOCK_PROC_MENU);
        assert_eq!(
            event.context.pos,
            Some([BLOCK_PROC_POS.0, BLOCK_PROC_POS.1, BLOCK_PROC_POS.2])
        );
        BLOCK_CHANGE_CALLS.set(BLOCK_CHANGE_CALLS.get() + 1);
        BLOCK_LAST_SLOTS.with(|s| {
            *s.borrow_mut() = event.changed.iter().map(|c| c.slot).collect();
        });
        let a = event.context.slot(MenuContainer::Main, 0);
        let b = event.context.slot(MenuContainer::Main, 1);
        let out = match (a, b) {
            (Some(a), Some(_)) => Some(
                mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:stone").unwrap(),
                    a.count().min(64),
                )
                .unwrap(),
            ),
            _ => None,
        };
        event
            .outputs
            .push(MenuSlotWrite::new(MenuContainer::Main, 2, out));
    }
}

struct BlockProcessingMod;
#[mod_api::__subscriptions]
impl Hooks for BlockProcessingMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(BLOCK_PROC_MENU, BlockProcessingPart);
    }
}

/// The world with `BlockProcessingMod` listening on `BLOCK_PROC_MENU` and an empty three-slot mod
/// processor block at `BLOCK_PROC_POS` (two inputs, one result), for guests 12 and 13 with no
/// menu open.
fn with_block_processor(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
    use std::sync::Arc;
    use sunrise_level::{
        chunk::Chunk,
        containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer},
    };
    let mods: Mods = vec![ModEntry::new("listen", Box::new(BlockProcessingMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        api_values::install_parts(&mods).unwrap();
        sunrise_level::containers::with_mod_menu_types(
            [(
                BLOCK_PROC_MENU.into(),
                "listen".into(),
                3,
                0,
                vec![2],
                vec![0, 1],
                None,
                None,
                true,
                None,
            )],
            || {
                sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                    BLOCK_CHANGE_CALLS.set(0);
                    BLOCK_LAST_SLOTS.with(|s| s.borrow_mut().clear());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let stone_state = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world
                        .set(
                            BLOCK_PROC_POS.0,
                            BLOCK_PROC_POS.1,
                            BLOCK_PROC_POS.2,
                            stone_state,
                        )
                        .unwrap();
                    // The hopper above inserts through the top face (`getSlotsForFace`).
                    let mut faces: [Vec<usize>; 6] = Default::default();
                    faces[sunrise_core::geometry::Direction::Up as usize] = vec![0, 1];
                    let container = Arc::new(ModContainer {
                        slots: 3,
                        faces,
                        inputs: vec![0, 1],
                        ..Default::default()
                    });
                    game.world
                        .block_entities
                        .set_mod_containers([(stone_state.0, container)].into_iter().collect());
                    game.world.block_entities.insert(
                        BLOCK_PROC_POS,
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "listen:proc".into(),
                            ModBlockEntityFields {
                                items: vec![None; 3],
                                ..Default::default()
                            },
                        ))),
                    );
                    test(game);
                });
            },
        );
    });
}

/// A hopper holding `stack` placed on top of `pos` moves once (`HopperBlockEntity.pushItemsTick`):
/// a write outside any transaction to the block below.
fn hopper_above(
    game: &mut sunrise_gameplay::game::ServerGame<'_>,
    pos: (i32, i32, i32),
    stack: ItemStack,
) {
    let hopper_pos = (pos.0, pos.1 + 1, pos.2);
    let hopper_state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:hopper")
        .unwrap();
    game.world
        .set(hopper_pos.0, hopper_pos.1, hopper_pos.2, hopper_state)
        .unwrap();
    game.world.block_entities.insert(
        hopper_pos,
        sunrise_level::containers::BlockEntity::Chest(vec![Some(stack), None, None, None, None]),
    );
    // A fresh hopper: no transfer cooldown from an earlier move.
    game.server.hoppers.remove(&hopper_pos);
    game.hopper_try_move_items(hopper_pos);
}

#[test]
#[ignore = "daily"]
fn block_menu_slot_changed_costs_one_index_lookup_at_an_unviewed_block() {
    use sunrise_gameplay::game::slot_changes::probe;
    use sunrise_sim::lan::menu::Slots;
    with_block_processor(|game| {
        let level = game.level("minecraft:overworld").unwrap();
        let pos = [BLOCK_PROC_POS.0, BLOCK_PROC_POS.1, BLOCK_PROC_POS.2];
        let stick = || ItemStack::new("minecraft:stick", 1);
        let place = ItemPlace::BlockEntity {
            level,
            pos,
            side: None,
            slot: 0,
        };
        let resource = ItemResource::of(&stick().try_into_api().unwrap());
        probe::take();
        // Nobody has the menu open: a committed write and a hopper's write each cost the
        // open-menu index lookup only, with no container snapshot and no menu context.
        game.transaction(|host, tx| {
            assert_eq!(place.insert(host, tx, &resource, 1), Ok(1));
            Ok::<_, ()>(())
        })
        .unwrap();
        hopper_above(game, BLOCK_PROC_POS, stick());
        assert_eq!(probe::take(), (0, 0));
        assert_eq!(BLOCK_CHANGE_CALLS.get(), 0);
        // With a listening viewer the same writes are reported, through the counted path.
        let host = game.server.lan.as_mut().unwrap();
        host.hub.open_container(
            12,
            BLOCK_PROC_MENU,
            sunrise_core::nbt::Tag::String("proc".into()),
            BLOCK_PROC_POS,
            Slots::Block(BLOCK_PROC_POS),
            vec![None; 3],
            None,
        );
        game.transaction(|host, tx| {
            assert_eq!(place.insert(host, tx, &resource, 1), Ok(1));
            Ok::<_, ()>(())
        })
        .unwrap();
        hopper_above(game, BLOCK_PROC_POS, stick());
        assert_eq!(BLOCK_CHANGE_CALLS.get(), 2);
        let (snapshots, contexts) = probe::take();
        assert!(snapshots >= 2, "{snapshots} container snapshots");
        assert_eq!(contexts, 2);
    });
}

#[test]
#[ignore = "daily"]
fn block_menu_slot_changed_fires_on_external_container_writes_and_syncs_lan_viewers() {
    use std::sync::{Arc, atomic::AtomicUsize};
    use sunrise_level::containers::BlockEntity;
    use sunrise_sim::lan::{PlayerSender, menu::Slots};
    with_block_processor(|game| {
        // Attach observable packet channels to both LAN guests (12 and 13) and open
        // the block-backed mod menu for both viewers.
        let (tx12, rx12) = mpsc::channel();
        let (tx13, rx13) = mpsc::channel();
        let host = game.server.lan.as_mut().unwrap();
        host.hub.guests.update(0, |g| {
            g.sender = PlayerSender::new(tx12, Arc::new(AtomicUsize::new(0)));
        });
        host.hub.guests.update(1, |g| {
            g.sender = PlayerSender::new(tx13, Arc::new(AtomicUsize::new(0)));
        });
        let title = sunrise_core::nbt::Tag::String("proc".into());
        host.hub.open_container(
            12,
            BLOCK_PROC_MENU,
            title.clone(),
            BLOCK_PROC_POS,
            Slots::Block(BLOCK_PROC_POS),
            vec![None; 3],
            None,
        );
        let w12 = host.hub.guest(12).unwrap().menu.as_ref().unwrap().window;
        host.hub.open_container(
            13,
            BLOCK_PROC_MENU,
            title,
            BLOCK_PROC_POS,
            Slots::Block(BLOCK_PROC_POS),
            vec![None; 3],
            None,
        );
        let w13 = host.hub.guest(13).unwrap().menu.as_ref().unwrap().window;
        sent(&rx12);
        sent(&rx13);

        let level = game.level("minecraft:overworld").unwrap();
        let entity = game
            .block_entity(
                level,
                [BLOCK_PROC_POS.0, BLOCK_PROC_POS.1, BLOCK_PROC_POS.2],
            )
            .unwrap();
        let stick_api = ItemStack::new("minecraft:stick", 1).try_into_api().unwrap();
        let dirt_api = ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap();

        // 1. BlockEntityRef::set_item_slot on slot 0 -> fires once across both viewers;
        // the next broadcastChanges sends both the change once.
        game.transaction(|host, tx| {
            assert_eq!(
                entity.set_item_slot(host, tx, 0, Some(stick_api.clone())),
                Ok(true)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(BLOCK_CHANGE_CALLS.get(), 1);
        assert_eq!(BLOCK_LAST_SLOTS.with(|s| s.borrow().clone()), vec![0]);
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
                        sent(&rx12)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12 && s.slot == 0 && s.item.is_some()))
                    );
        assert!(
                        sent(&rx13)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13 && s.slot == 0 && s.item.is_some()))
                    );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
            !sent(&rx12)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12))
        );
        assert!(
            !sent(&rx13)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13))
        );

        // 2. ItemPlace::BlockEntity on slot 1 -> fires once, computes result slot 2 =
        // stone(1), which the next broadcastChanges sends BOTH viewers once.
        game.transaction(|host, tx| {
            assert_eq!(
                ItemPlace::BlockEntity {
                    level,
                    pos: [BLOCK_PROC_POS.0, BLOCK_PROC_POS.1, BLOCK_PROC_POS.2],
                    side: None,
                    slot: 1,
                }
                .insert(host, tx, &ItemResource::of(&dirt_api), 1),
                Ok(1)
            );
            Ok::<_, ()>(())
        })
        .unwrap();
        assert_eq!(BLOCK_CHANGE_CALLS.get(), 2);
        let BlockEntity::Mod(be) = game.world.block_entities.get(&BLOCK_PROC_POS).unwrap() else {
            panic!("expected mod block entity");
        };
        assert_eq!(be.items[2], Some(ItemStack::new("minecraft:stone", 1)));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
                        sent(&rx12)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12 && s.slot == 2 && s.item.is_some()))
                    );
        assert!(
                        sent(&rx13)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13 && s.slot == 2 && s.item.is_some()))
                    );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
            !sent(&rx12)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12))
        );
        assert!(
            !sent(&rx13)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13))
        );

        // 3. Hopper ejecting another stick into slot 0 (a write outside any
        // transaction) -> fires once, updates result slot 2 = stone(2), which the next
        // broadcastChanges sends both viewers once.
        hopper_above(game, BLOCK_PROC_POS, ItemStack::new("minecraft:stick", 1));
        assert_eq!(BLOCK_CHANGE_CALLS.get(), 3);
        let BlockEntity::Mod(be) = game.world.block_entities.get(&BLOCK_PROC_POS).unwrap() else {
            panic!("expected mod block entity");
        };
        assert_eq!(be.items[0], Some(ItemStack::new("minecraft:stick", 2)));
        assert_eq!(be.items[2], Some(ItemStack::new("minecraft:stone", 2)));
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
                        sent(&rx12)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12 && s.slot == 2 && s.item.is_some()))
                    );
        assert!(
                        sent(&rx13)
                            .iter()
                            .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13 && s.slot == 2 && s.item.is_some()))
                    );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .sync_containers(&game.common.world);
        assert!(
            !sent(&rx12)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w12))
        );
        assert!(
            !sent(&rx13)
                .iter()
                .any(|p| matches!(p, C::ContainerSetSlot(s) if s.container == w13))
        );
    });
}

const BLOCK_CRAFT_MENU: &str = "minecraft:generic_3x3";
const BLOCK_CRAFT_POS: (i32, i32, i32) = (2, 64, 2);

#[derive(Clone, Debug, PartialEq)]
struct RecordedCraftContext {
    player: Option<u128>,
    pos: Option<[i32; 3]>,
    main: Vec<Option<mod_api::ItemStack>>,
    grid: Vec<Option<mod_api::ItemStack>>,
    result: Option<mod_api::ItemStack>,
}

thread_local! {
    static CRAFT_CONTEXT_LOG: std::cell::RefCell<
        Vec<(Vec<mod_api::MenuSlotChange>, RecordedCraftContext)>,
    > = const { std::cell::RefCell::new(Vec::new()) };
}

struct BlockCraftObserverPart;
#[mod_api::sunrise_part]
impl MenuPart for BlockCraftObserverPart {
    fn menu_slot_changed(&self, _: &mut dyn ServerHost, event: &mut MenuSlotChangedEvent<'_>) {
        assert_eq!(event.menu, BLOCK_CRAFT_MENU);
        let recorded = RecordedCraftContext {
            player: event.context.player,
            pos: event.context.pos,
            main: (0..6)
                .map(|i| event.context.slot(MenuContainer::Main, i))
                .collect(),
            grid: (0..4)
                .map(|i| event.context.slot(MenuContainer::Grid, i))
                .collect(),
            result: event.context.slot(MenuContainer::Result, 0),
        };
        CRAFT_CONTEXT_LOG.with(|log| {
            log.borrow_mut().push((event.changed.to_vec(), recorded));
        });
    }
}

struct BlockCraftObserverMod;
#[mod_api::__subscriptions]
impl Hooks for BlockCraftObserverMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.menu_part(BLOCK_CRAFT_MENU, BlockCraftObserverPart);
    }
}

#[test]
#[ignore = "daily"]
fn block_menu_slot_changed_gives_identical_context_for_click_and_hopper_writes() {
    use std::sync::Arc;
    use sunrise_level::{
        chunk::Chunk,
        containers::{
            BlockEntity, ModBlockEntity, ModBlockEntityFields, ModContainer, ModCrafting,
        },
    };
    use sunrise_sim::lan::menu::Slots;
    let mods: Mods = vec![ModEntry::new("listen", Box::new(BlockCraftObserverMod))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        api_values::install_parts(&mods).unwrap();
        sunrise_level::containers::with_mod_menu_types(
            [(
                BLOCK_CRAFT_MENU.into(),
                "listen".into(),
                6,
                0,
                Vec::new(),
                vec![5],
                Some(ModCrafting {
                    grid: 0,
                    size: 2,
                    result: 4,
                }),
                None,
                true,
                None,
            )],
            || {
                sunrise_gameplay::game::native_guest_turn_tests::with_guests(|game| {
                    CRAFT_CONTEXT_LOG.with(|log| log.borrow_mut().clear());
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    let stone_state = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world
                        .set(
                            BLOCK_CRAFT_POS.0,
                            BLOCK_CRAFT_POS.1,
                            BLOCK_CRAFT_POS.2,
                            stone_state,
                        )
                        .unwrap();
                    // The hopper above inserts through the top face (`getSlotsForFace`).
                    let mut faces: [Vec<usize>; 6] = Default::default();
                    faces[sunrise_core::geometry::Direction::Up as usize] = vec![0];
                    let container = Arc::new(ModContainer {
                        slots: 6,
                        faces,
                        inputs: vec![0, 1, 2, 3, 5],
                        ..Default::default()
                    });
                    game.world
                        .block_entities
                        .set_mod_containers([(stone_state.0, container)].into_iter().collect());
                    game.world.block_entities.insert(
                        BLOCK_CRAFT_POS,
                        BlockEntity::Mod(Box::new(ModBlockEntity::new(
                            "listen:crafter".into(),
                            ModBlockEntityFields {
                                items: vec![None; 6],
                                ..Default::default()
                            },
                        ))),
                    );
                    let host = game.server.lan.as_mut().unwrap();
                    host.hub.open_container(
                        12,
                        BLOCK_CRAFT_MENU,
                        sunrise_core::nbt::Tag::String("crafter".into()),
                        BLOCK_CRAFT_POS,
                        Slots::Block(BLOCK_CRAFT_POS),
                        vec![None; 6],
                        None,
                    );
                    let window = host.hub.guest(12).unwrap().menu.as_ref().unwrap().window;
                    let oak_log = ItemStack::new("minecraft:oak_log", 1);
                    let expected_log = oak_log.try_into_api().unwrap();
                    let expected_planks = ItemStack::new("minecraft:oak_planks", 4)
                        .try_into_api()
                        .unwrap();

                    // 1. Click places oak_log into crafting grid slot 0.
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.carried = Some(oak_log.clone()));
                    raw_click(game, 12, window, 0, 0, 0);
                    let (click_changed, click_context) =
                        CRAFT_CONTEXT_LOG.with(|log| log.borrow_mut().pop().unwrap());
                    assert_eq!(click_changed.len(), 1);
                    assert_eq!(click_changed[0].container, MenuContainer::Grid);
                    assert_eq!(click_changed[0].slot, 0);
                    assert_eq!(click_context.grid[0], Some(expected_log));
                    assert_eq!(
                        click_context.result,
                        Some(expected_planks),
                        "click path exposes the live crafting result"
                    );

                    // 2. Click removes oak_log back out so the grid is empty again.
                    raw_click(game, 12, window, 0, 0, 0);
                    CRAFT_CONTEXT_LOG.with(|log| log.borrow_mut().clear());

                    // 3. Hopper ejects oak_log into slot 0 -> must produce the exact same
                    // changed list and live MenuSlotContext (including Result = 4 oak planks).
                    hopper_above(game, BLOCK_CRAFT_POS, oak_log);
                    let (hopper_changed, hopper_context) =
                        CRAFT_CONTEXT_LOG.with(|log| log.borrow_mut().pop().unwrap());
                    assert_eq!(hopper_changed, click_changed);
                    assert_eq!(
                        hopper_context, click_context,
                        "hopper write must present the exact same live MenuSlotContext as a click"
                    );
                });
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn processing_owned_menu_returns_inputs_on_close_drops_overflow_and_discards_results() {
    with_processing_owned_menu(|game, rx, key| {
        let player = game.player(123).unwrap();
        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        let dirt = |n| Some(ItemStack::new("minecraft:dirt", n));
        let stone = |n| Some(ItemStack::new("minecraft:stone", n));

        // 1. Live close with room in inventory: returns non-result inputs, discards result slot 2,
        // and delivers empty owned_slots to menu_removed so items cannot be duplicated.
        let menu = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inventory.fill(None);
            g.carried = None;
            g.native_menu = vec![stick(3), dirt(2), stone(2)];
        });
        game.dropped.clear();
        CLOSED_OWNED_LEN.set(None);
        sent(&rx);
        game.lan_guest_packet(
            1,
            S::ContainerClose(sunrise_protocol::packets::play::ContainerClose {
                container: window,
            }),
        );
        game.run_native_menu_events();
        assert_eq!(CLOSED_OWNED_LEN.get(), Some(0));
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        let count_inv = |g: &sunrise_sim::lan::guest::Guest, id: &str| -> u32 {
            g.inventory
                .iter()
                .flatten()
                .filter(|s| &*s.item == id)
                .map(|s| s.count)
                .sum()
        };
        assert_eq!(count_inv(guest, "minecraft:stick"), 3);
        assert_eq!(count_inv(guest, "minecraft:dirt"), 2);
        assert_eq!(
            count_inv(guest, "minecraft:stone"),
            0,
            "result slot must be discarded, never returned"
        );
        assert!(game.dropped.is_empty());

        // 2. Live close with full inventory: drops overflow inputs at player and discards results.
        let menu = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            for slot in g.inventory.iter_mut() {
                *slot = Some(ItemStack::new("minecraft:cobblestone", 64));
            }
            g.carried = None;
            g.native_menu = vec![stick(4), dirt(5), stone(4)];
        });
        game.dropped.clear();
        CLOSED_OWNED_LEN.set(None);
        game.lan_guest_packet(
            1,
            S::ContainerClose(sunrise_protocol::packets::play::ContainerClose {
                container: window,
            }),
        );
        game.run_native_menu_events();
        assert_eq!(CLOSED_OWNED_LEN.get(), Some(0));
        let dropped_count = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| -> u32 {
            game.dropped
                .iter()
                .filter(|e| &*e.stack.item == id)
                .map(|e| e.stack.count)
                .sum()
        };
        assert_eq!(dropped_count(game, "minecraft:stick"), 4);
        assert_eq!(dropped_count(game, "minecraft:dirt"), 5);
        assert_eq!(dropped_count(game, "minecraft:stone"), 0);

        // 3. Disconnect drops non-result inputs directly even when inventory has space.
        let _window = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inventory.fill(None);
            g.carried = None;
            g.native_menu = vec![stick(2), dirt(1), stone(1)];
        });
        game.dropped.clear();
        game.guest_disconnected(1);
        game.run_native_menu_events();
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(count_inv(guest, "minecraft:stick"), 0);
        assert_eq!(count_inv(guest, "minecraft:dirt"), 0);
        assert_eq!(dropped_count(game, "minecraft:stick"), 2);
        assert_eq!(dropped_count(game, "minecraft:dirt"), 1);
        assert_eq!(dropped_count(game, "minecraft:stone"), 0);
    });
}

#[test]
#[ignore = "daily"]
fn processing_owned_menu_close_on_death_drops_inputs_unlike_live_close() {
    with_processing_owned_menu(|game, _rx, key| {
        let player = game.player(123).unwrap();
        let stick = |n| Some(ItemStack::new("minecraft:stick", n));
        let dirt = |n| Some(ItemStack::new("minecraft:dirt", n));
        let stone = |n| Some(ItemStack::new("minecraft:stone", n));
        let count_inv = |g: &sunrise_sim::lan::guest::Guest, id: &str| -> u32 {
            g.inventory
                .iter()
                .flatten()
                .filter(|s| &*s.item == id)
                .map(|s| s.count)
                .sum()
        };
        let dropped_count = |game: &sunrise_gameplay::game::ServerGame<'_>, id: &str| -> u32 {
            game.dropped
                .iter()
                .filter(|e| &*e.stack.item == id)
                .map(|e| e.stack.count)
                .sum()
        };

        // 1. A dead player's menu closes on its next tick (`lan_containers_tick`, as killed; its
        // own packets are no longer handled): non-result inputs drop instead of going into the
        // dead player's inventory, and results are discarded.
        let _window = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.inventory.fill(None);
            g.carried = None;
            g.dead = true;
            g.native_menu = vec![stick(3), dirt(2), stone(2)];
        });
        game.dropped.clear();
        game.tick_level_guests();
        game.run_native_menu_events();
        assert!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(1)
                .unwrap()
                .menu
                .is_none()
        );
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(count_inv(guest, "minecraft:stick"), 0);
        assert_eq!(count_inv(guest, "minecraft:dirt"), 0);
        assert_eq!(dropped_count(game, "minecraft:stick"), 3);
        assert_eq!(dropped_count(game, "minecraft:dirt"), 2);
        assert_eq!(dropped_count(game, "minecraft:stone"), 0);

        // 2. Hub-side close_container while dead (e.g. respawn path before dead flag clears)
        // records drop_instead = true so inputs still drop even if g.dead clears before
        // run_native_menu_events runs.
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.dead = false;
            g.inventory.fill(None);
        });
        let _window = game
            .as_guest(1, |game| {
                player.open_mod_menu(
                    game,
                    mod_api::ModMenuOpen::new(key, mod_api::Text::literal("Processor")),
                )
            })
            .unwrap()
            .unwrap()
            .unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.dead = true;
            g.native_menu = vec![stick(5), dirt(4), stone(4)];
        });
        game.dropped.clear();
        game.server.lan.as_mut().unwrap().hub.close_container(1);
        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.dead = false;
        });
        game.run_native_menu_events();
        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
        assert_eq!(count_inv(guest, "minecraft:stick"), 0);
        assert_eq!(count_inv(guest, "minecraft:dirt"), 0);
        assert_eq!(dropped_count(game, "minecraft:stick"), 5);
        assert_eq!(dropped_count(game, "minecraft:dirt"), 4);
        assert_eq!(dropped_count(game, "minecraft:stone"), 0);
    });
}

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;
