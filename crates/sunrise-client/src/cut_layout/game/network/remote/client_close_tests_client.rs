use crate::game::remote::tests::client_close_tests::chest;
use crate::game::remote::tests::client_close_tests::close;
use crate::game::remote::tests::client_close_tests::deliver;
use crate::game::remote::tests::client_close_tests::isolated;
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
};
use {
    sunrise_core::nbt::Tag, sunrise_gameplay_common::game::Screen, sunrise_items::items::ItemStack,
    sunrise_protocol::packets::play, sunrise_protocol::packets::play::Clientbound as C,
    sunrise_protocol::packets::play::Serverbound as S,
};

#[test]
#[ignore = "daily"]
fn server_close_keeps_shown_native_screen_alive_for_closing_and_veto() {
    isolated(
        "cut_layout::game::client_side::remote_client::tests::client_close_tests::server_close_keeps_shown_native_screen_alive_for_closing_and_veto",
        || {
            use mod_api::{
                ClientHost, Hooks, MenuScreen, MenuScreenFactory, MenuScreenLayout, MenuScreenOpen,
                ModEntry, Registrar, RegistrationContext, ScreenClosingEvent, ScreenOpening,
                ScreenOpeningEvent, ScreenSlot,
            };
            use std::sync::{Arc, Mutex};
            use sunrise_client::hud::HudBatch;
            use sunrise_protocol::{
                menu_extra::{self, CHANNEL, Message},
                packets::configuration::CustomPayload,
                registry::Ids,
            };
            use sunrise_sim::menu::Input;

            const MENU: &str = "test:close_native";
            const POS: [i32; 3] = [11, 64, 19];
            const SLOT_POS: [f32; 2] = [12.0, 24.0];

            #[derive(Default)]
            struct Probe {
                cancel_dead: bool,
                closing: Vec<(Option<String>, Option<[i32; 3]>, Option<[f32; 2]>)>,
                draws: Vec<Vec<i32>>,
            }
            type Shared = Arc<Mutex<Probe>>;

            struct Factory(Shared);
            impl MenuScreenFactory for Factory {
                fn create(&self, _: &dyn ClientHost, _: MenuScreenOpen<'_>) -> Option<MenuScreen> {
                    let draws = self.0.clone();
                    Some(
                        MenuScreen::new(MenuScreenLayout {
                            width: 176.0,
                            height: 166.0,
                            inventory_x: 8.0,
                            inventory_y: 84.0,
                            slots: vec![
                                Some(ScreenSlot {
                                    position: SLOT_POS,
                                    ..Default::default()
                                }),
                                None,
                            ],
                            own_labels: true,
                            own_background: true,
                            ..Default::default()
                        })
                        .with_draw(move |_, frame, _| {
                            draws.lock().unwrap().draws.push(frame.data.to_vec());
                        }),
                    )
                }
            }

            struct Listener(Shared);
            #[mod_api::__subscriptions]
            impl Hooks for Listener {
                fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                    r.register_menu_screen(MENU, Arc::new(Factory(self.0.clone())))
                        .unwrap();
                }
                fn screen_opening(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    (event.new.id == "dead" && self.0.lock().unwrap().cancel_dead)
                        .then_some(ScreenOpening::Cancel)
                }
                fn screen_closing(
                    &self,
                    host: &dyn ClientHost,
                    event: &mut ScreenClosingEvent<'_>,
                ) {
                    self.0.lock().unwrap().closing.push((
                        event.screen.menu.map(str::to_owned),
                        host.screen_pos(),
                        host.screen_slot(MENU, 0),
                    ));
                }
            }

            let probe = Shared::default();
            let mods: mod_api::Mods = vec![ModEntry::new(
                "close_native_probe",
                Box::new(Listener(probe.clone())),
            )]
            .into();
            sunrise_level::containers::with_mod_menu_types(
                [(
                    MENU.into(),
                    "close_native_probe".into(),
                    2,
                    2,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    false,
                    None,
                )],
                || {
                    mod_platform::testing::with_mods(mods.clone(), || {
                        mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                        with_client_game(|game| {
                            let mut reader = logged_in(game);
                            Arc::make_mut(&mut game.client.remote.as_mut().unwrap().builtin)
                                .registries
                                .insert(
                                    "minecraft:menu".into(),
                                    Ids::from_names(vec![MENU.to_owned()]),
                                );

                            let open_native =
                                |game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                                 window: i32| {
                                    deliver(
                                        game,
                                        C::CustomPayload(CustomPayload {
                                            channel: CHANNEL.into(),
                                            data: menu_extra::encode(&Message {
                                                window,
                                                pos: Some(POS),
                                                data: Vec::new(),
                                            })
                                            .unwrap(),
                                        }),
                                    );
                                    deliver(
                                        game,
                                        C::OpenScreen(play::OpenScreen {
                                            container: window,
                                            menu: 0,
                                            title: Tag::String("Native".into()),
                                        }),
                                    );
                                };

                            // 1. Decoded server ContainerClose keeps live reads valid during Closing.
                            open_native(game, 14);
                            assert_eq!(game.client.screen, Screen::Inventory);
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 14,
                                    id: 0,
                                    value: 42,
                                }),
                            );
                            let mut batch = HudBatch::scratch([320, 240]);
                            assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(probe.lock().unwrap().draws, [vec![42]]);

                            deliver(
                                game,
                                C::ContainerClose(play::ContainerClose { container: 14 }),
                            );
                            assert_eq!(game.client.screen, Screen::Playing);
                            assert_eq!(game.remote_menu().0, 0);
                            assert_eq!(
                                probe.lock().unwrap().closing,
                                [(Some(MENU.to_owned()), Some(POS), Some(SLOT_POS))]
                            );
                            assert!(!game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(game.screen_pos(), None);
                            assert_eq!(game.screen_slot(MENU, 0), None);

                            // 2. Vetoing a real destination (DeathScreen) keeps the shown screen
                            //    drawing and readable while rejecting stale window protocol ops.
                            probe.lock().unwrap().closing.clear();
                            probe.lock().unwrap().draws.clear();
                            probe.lock().unwrap().cancel_dead = true;
                            open_native(game, 15);
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 15,
                                    id: 0,
                                    value: 7,
                                }),
                            );
                            game.player
                                .as_mut()
                                .unwrap()
                                .set_health(0.0, false)
                                .unwrap();
                            game.client.remote.as_mut().unwrap().show_death_screen = true;
                            let _ = sent(&mut reader);
                            deliver(
                                game,
                                C::ContainerClose(play::ContainerClose { container: 15 }),
                            );

                            assert_eq!(game.client.screen, Screen::Inventory);
                            assert_eq!(game.remote_menu().0, 0);
                            assert!(probe.lock().unwrap().closing.is_empty());
                            assert_eq!(game.screen_pos(), Some(POS));
                            assert_eq!(game.screen_slot(MENU, 0), Some(SLOT_POS));
                            // Stale protocol/menu ops for the closed window are rejected.
                            deliver(
                                game,
                                C::ContainerSetData(play::ContainerSetData {
                                    container: 15,
                                    id: 0,
                                    value: 99,
                                }),
                            );
                            assert!(!game.remote_button_click(1));
                            assert!(!game.remote_menu_ops(&[(Some(0), 0, Input::Pickup)]));
                            assert!(!sent(&mut reader).iter().any(|p| matches!(
                                p,
                                S::ContainerClick(_) | S::ContainerButtonClick(_)
                            )));
                            // The displayed native screen still draws with its pre-close data.
                            assert!(game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert_eq!(probe.lock().unwrap().draws, [vec![7]]);

                            // Leaving the vetoed screen later closes it with live reads and no echo.
                            probe.lock().unwrap().cancel_dead = false;
                            game.set_screen(Screen::Playing);
                            assert_eq!(game.client.screen, Screen::Playing);
                            assert_eq!(
                                probe.lock().unwrap().closing,
                                [(Some(MENU.to_owned()), Some(POS), Some(SLOT_POS))]
                            );
                            assert!(!game.draw_native_screen(&mut batch, 0.0, 0.0));
                            assert!(
                                !sent(&mut reader)
                                    .iter()
                                    .any(|p| matches!(p, S::ContainerClose(_)))
                            );
                        });
                    });
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens() {
    isolated(
        "cut_layout::game::client_side::remote_client::tests::client_close_tests::server_close_notifies_the_outgoing_receiver_and_only_opens_real_screens",
        || {
            use mod_api::{
                ClientHost, Hooks, ModEntry, ScreenClosingEvent, ScreenOpening, ScreenOpeningEvent,
            };
            use std::sync::{Arc, Mutex};

            #[derive(Debug, PartialEq)]
            enum Event {
                Open(&'static str),
                Close {
                    id: &'static str,
                    menu: Option<String>,
                    title: Option<mod_api::Text>,
                    slots: usize,
                },
            }
            struct Observe(Arc<Mutex<Vec<Event>>>);
            #[mod_api::__subscriptions]
            impl Hooks for Observe {
                fn screen_opening(
                    &self,
                    _: &dyn ClientHost,
                    event: &mut ScreenOpeningEvent<'_>,
                ) -> Option<ScreenOpening> {
                    self.0.lock().unwrap().push(Event::Open(event.new.id));
                    // A spurious gameplay Opening would keep an already-deactivated menu.
                    (event.new.id == "playing").then_some(ScreenOpening::Cancel)
                }
                fn screen_closing(&self, _: &dyn ClientHost, event: &mut ScreenClosingEvent<'_>) {
                    self.0.lock().unwrap().push(Event::Close {
                        id: event.screen.id,
                        menu: event.screen.menu.map(str::to_owned),
                        title: event.screen.title.cloned(),
                        slots: event.screen.slots,
                    });
                }
            }
            let events = Arc::new(Mutex::new(Vec::new()));
            let take = || std::mem::take(&mut *events.lock().unwrap());
            let mods = vec![ModEntry::new(
                "screen_probe",
                Box::new(Observe(events.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_client_game(|game| {
                    let mut reader = logged_in(game);
                    let open = chest(game);
                    deliver(game, open.clone());
                    let slots = game.slot_count();
                    assert!(slots > 36);
                    let title = game.test_observed_title().unwrap().as_ref().clone();
                    take();
                    close(game);
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "inventory",
                            menu: Some("minecraft:generic_9x3".into()),
                            title: Some(title),
                            slots,
                        }]
                    );

                    deliver(game, open);
                    game.open_sign_editor((1, 65, 2), true);
                    assert_eq!(game.client.screen, Screen::Inventory);
                    take();
                    close(game);
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert_eq!(
                        take(),
                        [Event::Close {
                            id: "sign_editor",
                            menu: None,
                            title: None,
                            slots: 0
                        }]
                    );

                    game.set_screen(Screen::Paused);
                    game.player
                        .as_mut()
                        .unwrap()
                        .set_health(0.0, false)
                        .unwrap();
                    game.client.remote.as_mut().unwrap().show_death_screen = true;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.client.screen, Screen::Dead);
                    assert_eq!(take(), [Event::Open("dead"), paused]);

                    game.client.remote.as_mut().unwrap().show_death_screen = false;
                    let dead = Event::Close {
                        id: "dead",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    let _ = sent(&mut reader);
                    close(game);
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert_eq!(take(), [dead]);
                    assert!(
                        sent(&mut reader)
                            .iter()
                            .any(|p| matches!(p, S::ClientCommand(command) if command.action == 0))
                    );

                    game.set_screen(Screen::Paused);
                    game.client.remote.as_mut().unwrap().logged_in = false;
                    let paused = Event::Close {
                        id: "paused",
                        menu: None,
                        title: None,
                        slots: game.slot_count(),
                    };
                    take();
                    close(game);
                    assert_eq!(game.client.title.0.as_ref().unwrap().mode.id(), "title");
                    assert_eq!(take(), [Event::Open("title"), paused]);
                });
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_restores_inventory_under_chat_without_echo() {
    isolated(
        "cut_layout::game::client_side::remote_client::tests::client_close_tests::server_close_restores_inventory_under_chat_without_echo",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                let open_chest = chest(game);
                let inventory_carry = ItemStack::new("minecraft:stone", 2);
                for container in [12, 37] {
                    game.carried = Some(inventory_carry.clone());
                    deliver(game, open_chest.clone());
                    assert_eq!(game.client.screen, Screen::Inventory);
                    game.carried = Some(ItemStack::new("minecraft:dirt", 3));
                    game.begin_chat_screen("draft from a container", false);
                    assert_eq!(game.client.screen, Screen::Chat);
                    assert_eq!(game.remote_menu().0, 12);
                    game.client.inv_drag = Some((0, vec![1]));
                    deliver(game, C::ContainerClose(play::ContainerClose { container }));
                    assert_eq!(game.remote_menu().0, 0, "packet id {container}");
                    assert!(
                        game.carried
                            .as_ref()
                            .is_some_and(|stack| stack.item == inventory_carry.item
                                && stack.count == inventory_carry.count)
                    );
                    assert_eq!(game.client.screen, Screen::Playing);
                    assert!(game.client.chat_ui.chat_screen.is_none());
                    assert!(game.client.inv_drag.is_none());
                    game.open_chat_method("");
                    assert_eq!(
                        game.client.chat_ui.chat_screen.as_ref().unwrap().value(),
                        "draft from a container"
                    );
                    game.test_close_chat_intentionally();
                    assert!(
                        !sent(&mut reader)
                            .iter()
                            .any(|packet| matches!(packet, S::ContainerClose(_)))
                    );
                }

                game.set_screen(Screen::Paused);
                deliver(
                    game,
                    C::ContainerClose(play::ContainerClose { container: 0 }),
                );
                assert_eq!(game.client.screen, Screen::Playing);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::ContainerClose(_)))
                );

                deliver(game, open_chest);
                game.close_container_client();
                let closes: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|packet| match packet {
                        S::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect();
                assert_eq!(closes, [12]);
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn server_close_runs_sign_and_inventory_removal_without_echo() {
    isolated(
        "cut_layout::game::client_side::remote_client::tests::client_close_tests::server_close_runs_sign_and_inventory_removal_without_echo",
        || {
            with_client_game(|game| {
                use sunrise_level::containers::BlockEntity;
                use sunrise_level::signs_data::SignData;

                let mut reader = logged_in(game);
                game.world
                    .block_entities
                    .insert((1, 65, 2), BlockEntity::Sign(SignData::default()));
                deliver(
                    game,
                    C::OpenSignEditor(play::OpenSignEditor {
                        pos: [1, 65, 2],
                        front: false,
                    }),
                );
                assert!(game.client.sign_edit.is_some());
                game.sign_input(None, Some("unsent"));
                game.sign_input(Some(winit::keyboard::KeyCode::Enter), None);
                game.sign_input(None, Some("next line"));
                close(game);
                assert!(game.client.sign_edit.is_none());
                let packets = sent(&mut reader);
                let mut updates = packets.iter().filter_map(|packet| match packet {
                    S::SignUpdate(update) => Some(update),
                    _ => None,
                });
                let sign = updates.next().unwrap();
                assert_eq!(sign.pos, [1, 65, 2]);
                assert!(!sign.front);
                assert_eq!(sign.lines, ["unsent", "next line", "", ""]);
                assert!(updates.next().is_none());
                assert!(!packets.iter().any(|p| matches!(p, S::ContainerClose(_))));

                game.open_player_inventory();
                assert_eq!(game.client.screen, Screen::Inventory);
                assert_eq!(game.remote_menu().0, 0);
                game.client.remote.as_mut().unwrap().craft_result =
                    Some(ItemStack::new("minecraft:stick", 4));
                game.craft_grid[0] = Some(ItemStack::new("minecraft:oak_planks", 2));
                close(game);
                assert_eq!(game.client.screen, Screen::Playing);
                assert!(game.client.remote.as_ref().unwrap().craft_result.is_none());
                let ingredient = game.craft_grid[0].as_ref().unwrap();
                assert_eq!(ingredient.count, 2);
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::ContainerClose(_)))
                );
            });
        },
    );
}
