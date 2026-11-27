#![cfg(test)]
use sunrise_game_data::text::Text;
use sunrise_items::items::ItemStack;
use sunrise_sim::lan::data::component_nbt;
use sunrise_sim::lan::menu::Slots;
#[cfg(test)]
#[path = "native_menus/container_counter_tests.rs"]
mod container_counter_tests;

#[cfg(test)]
#[path = "native_menus/owned_slots_tests.rs"]
mod owned_slots_tests;

#[cfg(test)]
#[path = "native_menus/container_loot_open_tests.rs"]
mod container_loot_open_tests;

#[cfg(test)]
pub(crate) mod tests {

    use mod_api::__private::game::Fields;
    use sunrise_gameplay_common::game::TryIntoApi;
    use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1;
    use sunrise_gameplay_common::game::native_server::NativeServerGameExt1;
    use sunrise_gameplay_common::game::server_side::native_item_places::tests::guest;
    use sunrise_gameplay_common::game::transactions::TransactionsGameExt;

    use sunrise_gameplay_common::game::server_side::native_menus::tests::MOD_MENU;

    use sunrise_gameplay_common::game::server_side::native_menus::tests::sent;

    use sunrise_gameplay::game::server_side::native_menus::tests::mod_menu_guest;

    use sunrise_gameplay::game::server_side::native_menus::tests::open;

    use sunrise_gameplay::game::server_side::native_menus::tests::watched_guest;

    use super::super::native_entity_refs::tests::with_game;

    use super::*;

    use mod_api::{HostTransaction, ItemPlace, ItemResource, ServerHost, TransactionError};

    use mod_api::{MenuReplace, ModMenuOpen};

    use sunrise_gameplay::game::server_side::native_menus::open_menu_of;

    use std::sync::{Arc, mpsc};

    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    use sunrise_sim::lan::Out;

    #[test]
    #[ignore = "daily"]
    fn acting_menu_storage_extraction_rolls_back() {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let stack = ItemStack::new("minecraft:diamond", 5);
                let resource = ItemResource::of(&stack.clone().try_into_api().unwrap());
                let mut restored = Vec::new();
                for ender in [true, false] {
                    let host = game.server.lan.as_mut().unwrap();
                    if ender {
                        host.hub
                            .guests
                            .update(0, |g| g.ender_items[0] = Some(stack.clone()));
                        host.hub.open_container(
                            1,
                            "minecraft:generic_9x3",
                            sunrise_core::nbt::Tag::String("ender".into()),
                            (0, 64, 0),
                            sunrise_sim::lan::menu::Slots::Ender,
                            vec![Some(stack.clone())],
                            None,
                        );
                    } else {
                        host.hub
                            .open_native_menu(
                                1,
                                3,
                                sunrise_core::nbt::Tag::String("native".into()),
                                vec![Some(stack.clone())],
                            )
                            .unwrap();
                    }
                    game.as_guest(1, |game| {
                        let menu = game
                            .player_container_menu(game.player(123).unwrap())
                            .unwrap();
                        let place = ItemPlace::MenuSlot { menu, slot: 0 };
                        let mut inner_restored = None;
                        let host: &mut dyn ServerHost = game;
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            let inner = host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                                Err::<(), ()>(())
                            });
                            assert_eq!(inner, Err(TransactionError::Aborted(())));
                            inner_restored =
                                place.with_stack(host, |stack| stack.project()).flatten();
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        restored.push((
                            inner_restored,
                            place.with_stack(host, |stack| stack.project()).flatten(),
                        ));
                        // Player and menu writes share the ender snapshot, not competing copies.
                        let aborted = host.transaction(|host, tx| {
                            assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                            assert_eq!(menu.cursor().insert(host, tx, &resource, 1), Ok(1));
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        assert_eq!(
                            place.with_stack(host, |stack| stack.project()).flatten(),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                        assert_eq!(menu.carried(host), Some(None));
                        // A committed child lends its first snapshot to the parent.
                        let aborted = host.transaction(|host, _| {
                            host.transaction(|host, tx| {
                                assert_eq!(place.extract(host, tx, &resource, 1), Ok(1));
                                Ok::<_, ()>(())
                            })
                            .unwrap();
                            Err::<(), ()>(())
                        });
                        assert_eq!(aborted, Err(TransactionError::Aborted(())));
                        assert_eq!(
                            place.with_stack(host, |stack| stack.project()).flatten(),
                            Some(stack.clone().try_into_api().unwrap())
                        );
                    });
                    game.guest_close_all(1);
                }
                let expected = |n| {
                    Some(
                        ItemStack::new("minecraft:diamond", n)
                            .try_into_api()
                            .unwrap(),
                    )
                };
                assert_eq!(
                    restored,
                    vec![(expected(4), expected(5)), (expected(4), expected(5))]
                );
            })
        });
    }
    #[test]
    #[ignore = "daily"]
    fn container_open_handles_follow_supported_menus() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        struct Watch(Arc<Mutex<Vec<String>>>);
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                match event.menu_type {
                    "minecraft:furnace" => {
                        let menu = event.menu.expect("furnace has a handle during open");
                        assert_eq!(
                            menu.slot(host, 3),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ingot", 5)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                        assert_eq!(
                            menu.slot(host, 0),
                            Some(Some(
                                ItemStack::new("minecraft:iron_ore", 3)
                                    .try_into_api()
                                    .unwrap()
                            ))
                        );
                    }
                    "minecraft:crafting" => assert!(event.menu.is_none()),
                    "minecraft:generic_9x3" => assert!(event.menu.is_some()),
                    other => panic!("unexpected menu {other}"),
                }
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new("watch", Box::new(Watch(events.clone())))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _rx = watched_guest(game);
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| {
                        guest.inventory[9] = Some(ItemStack::new("minecraft:iron_ingot", 5));
                    });
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                for name in [
                    "minecraft:chest",
                    "minecraft:furnace",
                    "minecraft:crafting_table",
                ] {
                    let state = game.pack.assets.registry.default_state(name).unwrap();
                    assert_eq!(
                        game.level_set_block(
                            level,
                            [0, 64, 0],
                            state,
                            mod_api::BlockUpdateFlags::NEIGHBORS
                        ),
                        Ok(true)
                    );
                    if name == "minecraft:furnace" {
                        let Some(sunrise_level::containers::BlockEntity::Furnace(furnace)) =
                            game.world.block_entities.get_mut(&(0, 64, 0))
                        else {
                            panic!("furnace block entity")
                        };
                        furnace.items[0] = Some(ItemStack::new("minecraft:iron_ore", 3));
                    }
                    if name == "minecraft:crafting_table" {
                        game.lan_open_crafting(1, [0, 64, 0]);
                    } else {
                        assert_eq!(
                            game.lan_open_container(1, (0, 64, 0)),
                            sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened
                        );
                    }
                    // The same accessor works in any acting server turn, not only the notice.
                    game.as_guest(1, |game| {
                        let player = game.player(123).unwrap();
                        let menu = game.player_container_menu(player);
                        if name == "minecraft:crafting_table" {
                            assert!(menu.is_none());
                        } else {
                            let menu = menu.unwrap();
                            assert!(game.menu_slot_count(menu).is_some());
                            // The cursor is swapped out of Guest for this turn.
                            let cursor = Some(ItemStack::new("minecraft:stone", 2));
                            game.carried = cursor.clone();
                            assert_eq!(game.menu_carried(menu), Some(cursor));
                            game.carried = None;
                            let fields = menu.fields();
                            let stale = mod_api::__private::game::menu_ref(
                                player,
                                fields.window,
                                fields.generation + 1,
                            );
                            assert_eq!(game.menu_slot(stale, 0), None);
                            let wrong_window = mod_api::__private::game::menu_ref(
                                player,
                                fields.window + 1,
                                fields.generation,
                            );
                            assert_eq!(game.menu_slot(wrong_window, 0), None);
                        }
                    });
                    game.guest_close_all(1);
                }
                assert_eq!(
                    *events.lock().unwrap(),
                    [
                        "open minecraft:generic_9x3",
                        "close minecraft:generic_9x3",
                        "open minecraft:furnace",
                        "close minecraft:furnace",
                        "open minecraft:crafting",
                        "close minecraft:crafting",
                    ]
                );
            })
        });
    }
    #[test]
    #[ignore = "daily"]
    fn container_hooks_follow_actual_open_boundaries_without_a_tick() {
        use mod_api::{Hooks, ModEntry, Mods, PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_menus::tests::container_hooks_follow_actual_open_boundaries_without_a_tick",
            "SUNRISE_CONTAINER_BOUNDARY_CASE",
            &["same_tick", "replace", "mount", "guest"],
        ) {
            return;
        }
        struct Watch {
            events: Arc<Mutex<Vec<(bool, String)>>>,
            packets: Arc<Mutex<Option<mpsc::Receiver<Out>>>>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for Watch {
            fn player_container_open(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                // Both packets must precede the callback, not just the open's return.
                let packets = self.packets.lock().unwrap();
                let packets = sent(packets.as_ref().unwrap());
                let open = packets
                    .iter()
                    .find_map(|packet| match packet {
                        C::OpenScreen(p) => Some(p.container),
                        C::MountScreenOpen(p) => Some(p.container),
                        _ => None,
                    })
                    .unwrap();
                assert!(packets.iter().any(|packet| {
                    matches!(packet, C::ContainerSetContent(p) if p.container == open)
                }));
                self.events
                    .lock()
                    .unwrap()
                    .push((true, event.menu_type.to_string()));
            }
            fn player_container_close(
                &self,
                _host: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.events
                    .lock()
                    .unwrap()
                    .push((false, event.menu_type.to_string()));
            }
        }
        let events = Arc::new(Mutex::new(Vec::new()));
        let packets = Arc::new(Mutex::new(None));
        let mods: Mods = vec![ModEntry::new(
            "watch",
            Box::new(Watch {
                events: events.clone(),
                packets: packets.clone(),
            }),
        )]
        .into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        with_game(|game| {
            *packets.lock().unwrap() = Some(watched_guest(game));
            let case = std::env::var("SUNRISE_CONTAINER_BOUNDARY_CASE").unwrap();
            // Own inventory and unsuccessful construction have no open boundary.
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 0 }));
            game.lan_open_mount(1, u32::MAX);
            assert_eq!(
                game.lan_open_container(1, (0, 64, 0)),
                sunrise_gameplay_common::game::lan_actions::MenuOpen::NoProvider
            );
            assert!(events.lock().unwrap().is_empty());
            if case == "mount" {
                game.common.mobs.push(sunrise_sim::mobs::Mob::new(
                    31,
                    game.common
                        .res
                        .mob_types()
                        .get("minecraft:horse")
                        .unwrap()
                        .clone(),
                    sunrise_core::geometry::Vec3::new(0.0, 0.0, 0.0),
                    0.0,
                ));
                game.lan_open_mount(1, 31);
                assert_eq!(
                    open_menu_of(game.server.lan.as_ref().unwrap(), 1),
                    Some((1, "sunrise:mount_inventory", None))
                );
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "sunrise:mount_inventory".into()),
                        (false, "sunrise:mount_inventory".into()),
                    ]
                );
                return;
            }
            let first = if case == "guest" {
                let mut menu = None;
                game.as_guest(1, |game| {
                    menu = Some(open(game, 1, "first", MenuReplace::CloseFirst));
                });
                menu.unwrap()
            } else {
                open(game, 1, "first", MenuReplace::CloseFirst)
            };
            let first_window = first.container_id(game).unwrap();
            assert_eq!(first_window, 1);
            assert_eq!(
                open_menu_of(game.server.lan.as_ref().unwrap(), 1),
                Some((1, "minecraft:generic_9x1", None))
            );
            assert_eq!(
                *events.lock().unwrap(),
                vec![(true, "minecraft:generic_9x1".into())]
            );
            if case == "replace" {
                let second = open(game, 2, "second", MenuReplace::InPlace);
                assert_eq!(second.container_id(game), Some(2));
                // A stale close must not notify the replacement.
                game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                    ]
                );
                assert!(second.close_container(game));
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                        (true, "minecraft:generic_9x2".into()),
                        (false, "minecraft:generic_9x2".into()),
                    ]
                );
            } else {
                game.lan_guest_packet(
                    1,
                    S::ContainerClose(play::ContainerClose {
                        container: first_window,
                    }),
                );
                assert_eq!(
                    *events.lock().unwrap(),
                    vec![
                        (true, "minecraft:generic_9x1".into()),
                        (false, "minecraft:generic_9x1".into()),
                    ]
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_slot_written_outside_a_click_is_one_set_slot_at_the_next_sync() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
            let player = game.player(123).unwrap();
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(1));
            assert_eq!(player.container_menu(host), Some(menu));
            sent(&rx);

            let diamond = ItemStack::new("minecraft:diamond", 1)
                .try_into_api()
                .unwrap();
            let slot = ItemPlace::MenuSlot { menu, slot: 4 };
            let written =
                host.transaction(|host, tx| slot.insert(host, tx, &ItemResource::of(&diamond), 3));
            assert_eq!(written, Ok(3));
            assert!(sent(&rx).is_empty(), "nothing before the tick's sync");

            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_containers(&game.common.world);
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "{packets:?}");
            let C::ContainerSetSlot(set) = &packets[0] else {
                panic!("{packets:?}");
            };
            // The open's content took state 1; this is the next.
            assert_eq!((set.container, set.state, set.slot), (1, 2, 4));
            assert_eq!(set.item.as_ref().map(|item| item.count), Some(3));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .sync_containers(&game.common.world);
            assert!(sent(&rx).is_empty(), "sent once");

            // Closed, the menu answers nothing.
            assert!(menu.close_container(game));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            assert_ne!(player.container_menu(host), Some(menu));
        });
    }
    /// `menu_set_items` sends the whole contents of the open menu in one packet for its window,
    /// padded to its slots and cut at them, and the screen, the title and the cursor stay.
    #[test]
    #[ignore = "daily"]
    fn a_menus_items_are_replaced_in_one_packet_without_reopening() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
            let window = menu.container_id(game).unwrap();
            let stone = ItemStack::new("minecraft:stone", 3);
            let cursor = ItemStack::new("minecraft:dirt", 2);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(cursor.clone()));
            sent(&rx);
            let api = |stack: &ItemStack| stack.clone().try_into_api().unwrap();
            let host: &mut dyn ServerHost = game;
            // Two stacks, then enough empty slots to be cut at the menu's nine.
            let mut items = vec![None, Some(api(&stone))];
            items.resize(12, None);
            items[11] = Some(api(&stone));
            assert_eq!(menu.set_items(host, items), Ok(()));
            let guest = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(guest.native_menu.len(), 9, "cut at the menu's slots");
            assert_eq!(guest.native_menu[1], Some(stone.clone()));
            assert_eq!(
                guest.native_menu[8], None,
                "the tenth to twelfth are dropped"
            );
            assert_eq!(guest.carried, Some(cursor), "the cursor stays");
            let packets = sent(&rx);
            assert_eq!(packets.len(), 1, "one packet: {packets:?}");
            let C::ContainerSetContent(content) = &packets[0] else {
                panic!("{packets:?}");
            };
            assert_eq!(content.container, window);
            assert_eq!(content.items.len(), 9 + 36, "the menu, then the inventory");
            assert_eq!(content.items[1].as_ref().map(|item| item.count), Some(3));
            assert!(
                content.items[0].is_none(),
                "padded and empty slots are empty"
            );
            assert!(content.carried.is_some(), "the cursor is sent with it");
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(window), "still the open menu");
        });
    }
    /// A menu opened in the player's own turn, nested in another player's, clears and shows
    /// that player's own cursor and inventory: none of the outer player's stacks reach them.
    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_a_nested_turn_sends_the_openers_own_inventory_and_cursor() {
        with_game(|game| {
            let rx = watched_guest(game);
            open(game, 1, "old", MenuReplace::CloseFirst);
            let mut outer = guest(2);
            outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
            outer.carried = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.inventory[5] = Some(ItemStack::new("minecraft:gold_ingot", 2));
                g.carried = Some(ItemStack::new("minecraft:stone", 4));
            });
            sent(&rx);
            let menu = game
                .as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "pack", MenuReplace::CloseFirst))
                })
                .flatten()
                .unwrap();
            let window = menu.container_id(game).unwrap();
            let contents: Vec<_> = sent(&rx)
                .into_iter()
                .filter_map(|packet| match packet {
                    C::ContainerSetContent(content) => Some(content),
                    _ => None,
                })
                .collect();
            // The previous cursor's stone is back in the inventory of the new menu.
            let windows: Vec<_> = contents.iter().map(|c| c.container).collect();
            assert_eq!(windows, [window]);
            for content in &contents {
                let mut counts: Vec<_> = content.items.iter().flatten().map(|s| s.count).collect();
                counts.sort_unstable();
                assert_eq!(
                    counts,
                    [2, 4],
                    "window {}: their gold and stone",
                    content.container
                );
                assert!(content.carried.is_none(), "window {}", content.container);
            }
            let hub = &game.server.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried, None);
            assert_eq!(
                hub.guests[1].carried,
                Some(ItemStack::new("minecraft:dirt", 9)),
                "the outer player's cursor stays theirs"
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_block_entity_menu_parks_only_the_active_guests_cursor() {
        use mod_api::BlockUpdateFlags;
        use sunrise_level::chunk::Chunk;

        with_game(|game| {
            let rx = watched_guest(game);
            let opener_cursor = ItemStack::new("minecraft:stone", 4);
            let outer_cursor = ItemStack::new("minecraft:diamond", 7);
            let opener_slot = ItemStack::new("minecraft:gold_ingot", 2);
            let mut outer = guest(2);
            outer.carried = Some(outer_cursor.clone());
            outer.inventory[0] = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.carried = Some(opener_cursor.clone());
                g.inventory[0] = Some(opener_slot.clone());
            });
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, BlockUpdateFlags::NEIGHBORS),
                Ok(true)
            );
            let entity = game.block_entity(level, [0, 64, 0]).unwrap();
            let opener = game.player(123).unwrap();
            sent(&rx);
            game.as_guest(2, |game| {
                game.as_guest(1, |game| {
                    let host: &mut dyn ServerHost = game;
                    assert_eq!(opener.open_menu(host, entity), Ok(true));
                    assert_eq!(
                        game.carried, None,
                        "the new menu starts with an empty cursor"
                    );
                })
                .unwrap();
                assert_eq!(game.carried, Some(outer_cursor.clone()));
            })
            .unwrap();
            let hub = &game.server.lan.as_ref().unwrap().hub;
            let window = hub.guests[0].menu.as_ref().unwrap().window;
            let contents = sent(&rx)
                .into_iter()
                .find_map(|p| match p {
                    C::ContainerSetContent(content) if content.container == window => Some(content),
                    _ => None,
                })
                .unwrap();
            assert!(contents.carried.is_none());
            let shown: Vec<_> = contents
                .items
                .iter()
                .flatten()
                .map(|item| hub.stack_from_network(item))
                .collect();
            assert_eq!(shown, [opener_slot]);
            game.lan_guest_packet(
                1,
                S::ContainerClose(play::ContainerClose { container: window }),
            );
            let hub = &game.server.lan.as_ref().unwrap().hub;
            assert_eq!(hub.guests[0].carried, Some(opener_cursor));
            assert_eq!(hub.guests[1].carried, Some(outer_cursor));
        });
    }
    #[test]
    #[ignore = "daily"]
    fn shutdown_saves_current_and_parked_inventory_cursors() {
        with_game(|game| {
            let world = tempfile::tempdir().unwrap();
            game.server.world_root = world.path().to_owned();
            let _rx = watched_guest(game);
            let current = ItemStack::new("minecraft:lapis_lazuli", 3);
            let parked = ItemStack::new("minecraft:iron_ingot", 4);
            let open_cursor = ItemStack::new("minecraft:quartz", 5);
            let pending = ItemStack::new("minecraft:gold_ingot", 6);
            let mut other = guest(2);
            other.carried = Some(current.clone());
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(other);
            hub.guests.update(0, |g| g.carried = Some(parked.clone()));
            open(game, 1, "open", MenuReplace::CloseFirst);
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            assert!(hub.guests[0].inventory.iter().all(Option::is_none));
            hub.guests
                .update(0, |g| g.carried = Some(open_cursor.clone()));
            hub.guests.push(guest(3));
            hub.open_native_menu(
                3,
                1,
                component_nbt(&Text::literal("retired").component()),
                Vec::new(),
            )
            .unwrap();
            hub.guests.update(2, |g| g.carried = Some(pending.clone()));
            hub.close_container(3); // Shutdown wins the race with the queued return.

            game.save_leaving_guests();
            for (conn, expected) in [
                (1, vec![open_cursor, parked]),
                (2, vec![current]),
                (3, vec![pending]),
            ] {
                let mut reloaded = guest(conn);
                assert!(sunrise_sim::lan::playerdata::load(
                    &mut reloaded,
                    world.path()
                ));
                assert_eq!(
                    reloaded.inventory.into_iter().flatten().collect::<Vec<_>>(),
                    expected
                );
            }
        });
    }
    #[test]
    #[ignore = "daily"]
    fn cursor_overflow_listeners_see_and_write_only_the_openers_inventory() {
        use mod_api::{
            EntityJoinLevelEvent, Flow, PlayerContainerCloseEvent, PlayerContainerOpenEvent,
        };
        use std::sync::Mutex;

        struct Listen(Arc<Mutex<Vec<&'static str>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn entity_join_level(
                &self,
                host: &mut dyn ServerHost,
                event: &mut EntityJoinLevelEvent<'_>,
            ) -> Flow {
                if event.kind == "minecraft:item" {
                    let opener = host.player(123).unwrap();
                    let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
                    opener.with_inventory(host, |view| {
                        let mut slots = view.iter();
                        for expected in 0..36 {
                            let (slot, stack) = slots.next().unwrap();
                            assert_eq!((slot, stack.item(), stack.count()), (expected, stone, 64),
                                "the listener sees the opener's full inventory after cursor reinsertion");
                        }
                        assert!(slots.next().is_none());
                    }).unwrap();
                    opener.set_inventory_slot(host, 3, None).unwrap();
                    self.0.lock().unwrap().push("drop");
                }
                Flow::Continue
            }

            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerCloseEvent<'_>,
            ) {
                self.0.lock().unwrap().push("close");
            }

            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                _: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0.lock().unwrap().push("open");
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "overflow-listener",
            Box::new(Listen(heard.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                let _rx = watched_guest(game);
                let mut outer = guest(2);
                outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
                let outer_inventory = outer.inventory.clone();
                game.server.lan.as_mut().unwrap().hub.guests.push(outer);
                open(game, 1, "old", MenuReplace::CloseFirst);
                heard.lock().unwrap().clear();
                game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.inventory
                        .fill(Some(ItemStack::new("minecraft:stone", 64)));
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 62));
                    g.carried = Some(ItemStack::new("minecraft:stone", 4));
                });
                let dropped = game.dropped.len();
                game.as_guest(2, |game| {
                    game.as_guest(1, |game| open(game, 1, "new", MenuReplace::CloseFirst))
                        .unwrap();
                })
                .unwrap();
                assert_eq!(*heard.lock().unwrap(), ["drop", "close", "open"]);
                assert_eq!(game.dropped.len(), dropped + 1);
                assert_eq!(
                    game.dropped[dropped].stack,
                    ItemStack::new("minecraft:stone", 2)
                );
                let guests = &game.server.lan.as_ref().unwrap().hub.guests;
                assert_eq!(
                    guests[0].inventory[3], None,
                    "the listener's write belongs to the opener"
                );
                assert_eq!(guests[1].inventory, outer_inventory);
            });
        });
    }
    /// Each refusal has its own name and sends and changes nothing: another window or none
    /// open, no such player, a stack the game refuses, and a client.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn each_menu_items_refusal_has_its_own_name_and_sends_nothing() {
        use mod_api::{MenuItemsRefusal, StackWriteError};
        use sunrise_gameplay::game::server_side::native_menus::tests::watch_guest_in_hub;

        crate::game::integrated_pair::with_integrated_pair(|pair| {
            pair.call(|game| {
                let rx = watch_guest_in_hub(game);
                let player = game.player(123).unwrap();
                let api = |stack: ItemStack| stack.try_into_api().unwrap();
                let one = vec![Some(api(ItemStack::new("minecraft:stone", 1)))];
                let inventory = player.container_menu(game).unwrap();
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    inventory.set_items(host, one.clone()),
                    Err(MenuItemsRefusal::NotOpen),
                    "inventory menu"
                );
                let stale = open(game, 1, "old", MenuReplace::CloseFirst);
                let menu = open(game, 1, "shop", MenuReplace::CloseFirst);
                game.server.lan.as_mut().unwrap().hub.guests.push(guest(2));
                let other = game.player(124).unwrap();
                let offline = other
                    .open_chest_menu(
                        game,
                        1,
                        mod_api::Text::literal("other"),
                        Vec::new(),
                        MenuReplace::CloseFirst,
                    )
                    .unwrap();
                let other_index = game.guest_index(2).unwrap();
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .remove(other_index);
                let watched = game.guest_index(1).unwrap();
                sent(&rx);
                let before = game.server.lan.as_ref().unwrap().hub.guests[watched]
                    .native_menu
                    .clone();
                let host: &mut dyn ServerHost = game;
                assert_eq!(
                    stale.set_items(host, one.clone()),
                    Err(MenuItemsRefusal::NotOpen),
                    "stale menu"
                );
                assert_eq!(
                    offline.set_items(host, one.clone()),
                    Err(MenuItemsRefusal::NoPlayer)
                );
                let mut too_many = api(ItemStack::new("minecraft:stone", 1));
                too_many.set_count(65).unwrap();
                assert_eq!(
                    menu.set_items(host, vec![Some(too_many)]),
                    Err(MenuItemsRefusal::Stack(StackWriteError::CountExceeded {
                        maximum: 64
                    }))
                );
                assert!(sent(&rx).is_empty(), "nothing was sent");
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[watched].native_menu,
                    before,
                    "nothing was changed"
                );
                game.options.execution_role =
                    sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
                let host: &mut dyn ServerHost = game;
                assert_eq!(menu.set_items(host, one), Err(MenuItemsRefusal::Client));
            });
        });
    }
    #[test]
    #[ignore = "daily"]
    fn a_menu_opened_in_place_sends_no_close_returns_the_cursor_and_ignores_the_old_window() {
        with_game(|game| {
            let rx = watched_guest(game);
            let closes = |packets: &[C]| -> Vec<i32> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerClose(close) => Some(close.container),
                        _ => None,
                    })
                    .collect()
            };
            let opens = |packets: &[C]| -> Vec<(i32, sunrise_core::nbt::Tag)> {
                packets
                    .iter()
                    .filter_map(|p| match p {
                        C::OpenScreen(open) => Some((open.container, open.title.clone())),
                        _ => None,
                    })
                    .collect()
            };
            // From the inventory nothing is open, so nothing closes.
            let first = open(game, 1, "one", MenuReplace::CloseFirst);
            assert_eq!(first.container_id(game), Some(1));
            let packets = sent(&rx);
            assert_eq!(closes(&packets), Vec::<i32>::new());
            assert_eq!(opens(&packets).len(), 1);

            let stone = ItemStack::new("minecraft:stone", 4);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .native_menu_events
                .clear();
            let second = open(game, 2, "two", MenuReplace::InPlace);
            assert_eq!(second.container_id(game), Some(2));
            let packets = sent(&rx);
            assert_eq!(
                closes(&packets),
                Vec::<i32>::new(),
                "no close: the mouse stays"
            );
            assert_eq!(
                opens(&packets),
                vec![(2, component_nbt(&Text::literal("two").component()))]
            );
            let g = &game.server.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 18)));
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .native_menu_events
                    .is_empty(),
                "the old menu's teardown already ran synchronously"
            );

            // The old window's click and close land on nothing.
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .native_menu_events
                .clear();
            game.lan_guest_packet(
                1,
                S::ContainerClick(play::ContainerClick {
                    container: 1,
                    state: 1,
                    slot: 0,
                    button: 0,
                    input: 0,
                    changed: Vec::new(),
                    carried: None,
                }),
            );
            game.lan_guest_packet(1, S::ContainerClose(play::ContainerClose { container: 1 }));
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.native_menu_events,
                Vec::new()
            );
            let host: &mut dyn ServerHost = game;
            assert_eq!(first.container_id(host), None);
            assert_eq!(
                second.container_id(host),
                Some(2),
                "the new menu stays open"
            );

            // The default sends the close first.
            sent(&rx);
            let third = open(game, 3, "three", MenuReplace::CloseFirst);
            assert_eq!(third.container_id(game), Some(3));
            let packets = sent(&rx);
            assert_eq!(closes(&packets), vec![2]);
            assert_eq!(opens(&packets).len(), 1);
            let close_at = packets
                .iter()
                .position(|p| matches!(p, C::ContainerClose(_)))
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| matches!(p, C::OpenScreen(_)))
                .unwrap();
            assert!(close_at < open_at);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn bulk_menu_updates_send_the_live_owner_and_abort_without_packets() {
        with_game(|game| {
            let rx = watched_guest(game);
            let menu = open(game, 1, "display", MenuReplace::CloseFirst);
            let window = menu.container_id(game).unwrap();
            let mut outer = guest(2);
            outer.inventory[3] = Some(ItemStack::new("minecraft:diamond", 7));
            outer.carried = Some(ItemStack::new("minecraft:dirt", 9));
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.push(outer);
            hub.guests.update(0, |g| {
                g.inventory[5] = Some(ItemStack::new("minecraft:gold_ingot", 2));
                g.carried = Some(ItemStack::new("minecraft:stone", 4));
            });
            sent(&rx);
            let items = |count| {
                vec![Some(
                    ItemStack::new("minecraft:stick", count)
                        .try_into_api()
                        .unwrap(),
                )]
            };
            let check = |count| {
                let packets = sent(&rx);
                let contents: Vec<_> = packets
                    .iter()
                    .filter_map(|p| match p {
                        C::ContainerSetContent(content) => Some(content),
                        _ => None,
                    })
                    .collect();
                let [content] = contents.as_slice() else {
                    panic!("{packets:?}");
                };
                assert_eq!(content.container, window);
                assert_eq!(content.items[0].as_ref().map(|s| s.count), Some(count));
                assert_eq!(content.items[9 + 27 + 5].as_ref().map(|s| s.count), Some(2));
                assert!(content.items[9 + 27 + 3].is_none());
                assert_eq!(content.carried.as_ref().map(|s| s.count), Some(4));
            };
            game.as_guest(2, |game| {
                game.as_guest(1, |game| {
                    assert_eq!(menu.set_items(game, items(1)), Ok(()));
                    check(1);
                    let aborted: Result<(), mod_api::TransactionError<()>> =
                        game.transaction(|game, _| {
                            assert_eq!(menu.set_items(game, items(3)), Ok(()));
                            Err(())
                        });
                    assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                    assert!(sent(&rx).is_empty());
                    assert_eq!(
                        menu.slot(game, 0).flatten().map(|stack| stack.count()),
                        Some(1)
                    );
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        game.transaction(|game, _| {
                            assert_eq!(menu.set_items(game, items(5)), Ok(()));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    check(5);
                })
                .unwrap();
                assert_eq!(game.carried.as_ref().unwrap().count, 9);
            })
            .unwrap();
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .carried
                    .as_ref()
                    .unwrap()
                    .count,
                4
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn staged_same_level_open_spills_the_old_chest_before_its_veto_replaces_it() {
        use mod_api::{BlockState, BlockUpdateFlags, ContainerOpenedEvent, Flow, HostBlockStates};
        use sunrise_level::chunk::Chunk;

        const POS: [i32; 3] = [2, 64, 2];
        const CELL: (i32, i32, i32) = (POS[0], POS[1], POS[2]);
        const CONTENT: &str = "minecraft:gold_ingot";
        const COUNT: u32 = 7;
        struct Veto;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                event: &mut ContainerOpenedEvent<'_>,
            ) -> Flow {
                assert_eq!(event.kind, MOD_MENU);
                assert!(host.transaction_matches(None));
                let level = host.level("minecraft:overworld").unwrap();
                let chest = BlockState::of(
                    &host.block_states(),
                    mod_api::__private::block_key("minecraft:chest").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(
                    level.set_block(host, POS, chest, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                Flow::Cancel
            }
        }
        let mods = vec![mod_api::ModEntry::new("replace_chest_veto", Box::new(Veto))].into();
        mod_platform::testing::with_mods(mods, || {
            with_mod_menu_game(|game| {
                let (_rx, key) = mod_menu_guest(game);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let level = game.level("minecraft:overworld").unwrap();
                let chest = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                assert_eq!(
                    game.level_set_block(level, POS, chest, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                game.world
                    .block_entities
                    .get_mut(&CELL)
                    .unwrap()
                    .slots_mut()[0] = Some(ItemStack::new(CONTENT, COUNT));
                let player = game.player(123).unwrap();
                (game as &mut dyn ServerHost)
                    .transaction(|host, _| {
                        let staged = player
                            .open_mod_menu(
                                host,
                                ModMenuOpen::new(key, mod_api::Text::literal("vetoed")),
                            )
                            .unwrap()
                            .unwrap();
                        assert!(staged.fields().window < 0);
                        let air = BlockState::of(
                            &host.block_states(),
                            mod_api::__private::block_key("minecraft:air").unwrap(),
                            &[],
                        )
                        .unwrap();
                        assert_eq!(
                            level.set_block(host, POS, air, BlockUpdateFlags::NEIGHBORS),
                            Ok(true)
                        );
                        Ok::<_, ()>(())
                    })
                    .unwrap();

                assert_eq!(game.world.block(CELL.0, CELL.1, CELL.2), chest);
                let new_chest = game.world.block_entities.get(&CELL).unwrap();
                assert!(new_chest.slots().iter().all(Option::is_none));
                let dropped: u32 = game
                    .dropped
                    .iter()
                    .filter(|entity| &*entity.stack.item == CONTENT)
                    .map(|entity| entity.stack.count)
                    .sum();
                assert_eq!(
                    dropped, COUNT,
                    "the old chest must spill its entire inventory"
                );
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
            });
        });
    }

    /// [`with_game`] with [`MOD_MENU`] as the thread's menu types, so a test neither depends on
    /// registering first nor touches the one-shot process table.
    pub(super) fn with_mod_menu_game(
        test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>),
    ) {
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "status_package".into(),
                9,
                0,
                Vec::new(),
                Vec::new(),
                None,
                None,
                true,
                None,
            )],
            || with_game(test),
        );
    }
    #[test]
    #[ignore = "daily"]
    fn a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu() {
        use mod_api::{PlayerContainerCloseEvent, PlayerContainerOpenEvent};
        use std::sync::Mutex;
        const CASE: &str = "SUNRISE_MOD_MENU_REPLACE_CHILD";
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_menus::tests::a_mod_menu_opened_in_place_sends_no_close_and_still_closes_the_old_menu",
            CASE,
            &["close_first", "in_place"],
        ) {
            return;
        }
        // What the mods heard, in order.
        static HEARD: Mutex<Vec<String>> = Mutex::new(Vec::new());
        struct Listen;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Listen {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_menu_screen(
                    MOD_MENU,
                    Arc::new(
                        |_: &dyn mod_api::ClientHost, _: mod_api::MenuScreenOpen<'_>| {
                            HEARD.lock().unwrap().push("layout".to_owned());
                            None
                        },
                    ),
                )
                .unwrap();
            }
            fn player_container_open(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("open {}", event.menu_type));
            }
            fn player_container_close(
                &self,
                _: &mut dyn ServerHost,
                event: &mut PlayerContainerCloseEvent<'_>,
            ) {
                HEARD
                    .lock()
                    .unwrap()
                    .push(format!("close {}", event.menu_type));
            }
        }
        let heard = || std::mem::take(&mut *HEARD.lock().unwrap());
        let case = std::env::var(CASE).unwrap();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("listen", Box::new(Listen))].into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let replace = if case == "close_first" {
            MenuReplace::CloseFirst
        } else {
            MenuReplace::InPlace
        };
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            // Where the game keeps what the mods last heard the player open.
            game.server.lan.as_mut().unwrap().play.entry(1).or_default();
            let player = game.player(123).unwrap();
            let old = open(game, 3, "old", MenuReplace::CloseFirst);
            assert_eq!(old.container_id(game), Some(1));
            assert_eq!(heard(), ["open minecraft:generic_9x3"]);
            sent(&rx);
            let stone = ItemStack::new("minecraft:stone", 4);
            game.server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(0, |g| g.carried = Some(stone.clone()));

            let menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("new"));
                    request.extra = vec![7];
                    request.replace = replace;
                    request
                })
                .unwrap();
            assert_eq!(menu.unwrap().container_id(game), Some(2));
            // The old menu's close, heard once before the new one opens, with
            // no layout callback on the server.
            assert_eq!(
                heard(),
                [
                    "close minecraft:generic_9x3".to_owned(),
                    format!("open {MOD_MENU}")
                ]
            );
            let sent_packets = sent(&rx);
            let packets: Vec<String> = sent_packets
                .iter()
                .map(|p| match p {
                    C::ContainerClose(close) => format!("close {}", close.container),
                    C::CustomPayload(payload) => format!("payload {}", payload.channel),
                    C::OpenScreen(open) => format!("open {}", open.container),
                    C::ContainerSetContent(content) => format!(
                        "content {} state {} slots {}",
                        content.container,
                        content.state,
                        content.items.len()
                    ),
                    other => format!("{other:?}"),
                })
                .collect();
            // Only CloseFirst sends the old window's close. The returned cursor
            // is in the player slots of the new window's first contents.
            let mut expected = vec![
                "payload sunrise:menu_extra",
                "open 2",
                "content 2 state 1 slots 45",
            ];
            if replace == MenuReplace::CloseFirst {
                expected.insert(0, "close 1");
            }
            assert_eq!(packets, expected);
            let Some(C::ContainerSetContent(content)) = sent_packets.last() else {
                panic!("expected the new window's contents");
            };
            let hub = &game.server.lan.as_ref().unwrap().hub;
            let returned: Vec<_> = content.items[9..]
                .iter()
                .flatten()
                .map(|item| hub.stack_from_network(item))
                .collect();
            assert_eq!(returned.as_slice(), std::slice::from_ref(&stone));
            let g = &hub.guests[0];
            assert_eq!(g.carried, None, "the cursor stack went back");
            assert_eq!(
                g.inventory.iter().flatten().cloned().collect::<Vec<_>>(),
                vec![stone]
            );
            assert_eq!(g.menu.as_ref().map(|m| (m.window, m.size)), Some((2, 9)));
            assert!(
                game.server
                    .lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .native_menu_events
                    .is_empty(),
                "the old menu's teardown already ran synchronously"
            );

            // Both notifications were delivered inline, without a second close.
            assert!(heard().is_empty());
        });
    }
    #[test]
    #[ignore = "daily"]
    fn mod_menu_backend_payload_and_refusals() {
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_menus::tests::mod_menu_backend_payload_and_refusals",
            "SUNRISE_MOD_MENU_BACKEND_CHILD",
            &["backend"],
        ) {
            return;
        }
        with_mod_menu_game(|game| {
            let (rx, key) = mod_menu_guest(game);
            let player = game.player(123).unwrap();
            let old = open(game, 1, "old", MenuReplace::CloseFirst);
            let old_window = old.container_id(game).unwrap();
            sent(&rx);
            let bytes = vec![0, 255, 17];
            let menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("new"));
                    request.extra = bytes.clone();
                    request
                })
                .unwrap()
                .unwrap();
            let window = menu.container_id(game).unwrap();
            assert_eq!(window, old_window + 1);
            let packets = sent(&rx);
            let payload_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL
                    )
                })
                .unwrap();
            let open_at = packets
                .iter()
                .position(|p| {
                    matches!(p,
                        C::OpenScreen(p) if p.container == window
                    )
                })
                .unwrap();
            assert!(payload_at < open_at, "extra data must precede its window");
            let C::CustomPayload(payload) = &packets[payload_at] else {
                unreachable!()
            };
            assert_eq!(
                sunrise_protocol::menu_extra::decode(&payload.data).unwrap(),
                sunrise_protocol::menu_extra::Message {
                    window,
                    pos: None,
                    data: bytes
                }
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| (m.window, m.pos)),
                Some((window, None))
            );

            for (revision, extra, expected) in [
                (
                    1,
                    vec![0; sunrise_protocol::menu_extra::MAX_DATA + 1],
                    mod_api::OpenMenuError::TooLarge,
                ),
                (0, vec![1], mod_api::OpenMenuError::PlayerUnavailable),
            ] {
                game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.sender.menu_extra_rev = revision;
                });
                game.server
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .native_menu_events
                    .clear();
                assert_eq!(
                    game.player_open_mod_menu(player, {
                        let mut request = ModMenuOpen::new(key, mod_api::Text::literal("refused"));
                        request.extra = extra;
                        request.replace = MenuReplace::InPlace;
                        request
                    },),
                    Err(expected)
                );
                assert_eq!(
                    game.server.lan.as_ref().unwrap().hub.guests[0]
                        .menu
                        .as_ref()
                        .map(|m| m.window),
                    Some(window),
                    "refusal preserves the current menu"
                );
                assert!(
                    game.server
                        .lan
                        .as_ref()
                        .unwrap()
                        .hub
                        .native_menu_events
                        .is_empty()
                );
                assert!(sent(&rx).is_empty(), "refusal sends neither close nor open");
            }
            // Hub-level encode refusal also aborts before closing or emitting packets.
            let conn = game.server.lan.as_ref().unwrap().hub.guests[0].conn;
            assert_eq!(
                game.server.lan.as_mut().unwrap().hub.open_menu_window(
                    conn,
                    MOD_MENU,
                    sunrise_core::nbt::Tag::String("oversize".into()),
                    None,
                    Slots::Native,
                    vec![None; 9],
                    Some(vec![0; sunrise_protocol::menu_extra::MAX_DATA + 1]),
                ),
                None
            );
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| m.window),
                Some(window),
                "oversize hub refusal preserves the current menu"
            );
            assert!(sent(&rx).is_empty(), "oversize hub refusal emits nothing");
            // Payloads past the old 32 KiB limit (up to MAX_DATA) are accepted intact.
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender.menu_extra_rev = 1;
            });
            let large = vec![7; 33 * 1024];
            let large_menu = game
                .player_open_mod_menu(player, {
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("large"));
                    request.extra = large.clone();
                    request.replace = MenuReplace::InPlace;
                    request
                })
                .unwrap()
                .unwrap();
            let large_window = large_menu.container_id(game).unwrap();
            let large_packets = sent(&rx);
            let large_payload = large_packets
                .iter()
                .find_map(|p| match p {
                    C::CustomPayload(p) if p.channel == sunrise_protocol::menu_extra::CHANNEL => {
                        Some(&p.data)
                    }
                    _ => None,
                })
                .unwrap();
            assert_eq!(
                sunrise_protocol::menu_extra::decode(large_payload).unwrap(),
                sunrise_protocol::menu_extra::Message {
                    window: large_window,
                    pos: None,
                    data: large,
                }
            );

            // Block-backed mod menus also require the negotiated opening-format capability: a
            // Sunrise-branded peer that gave no revision answer (rev 0) is refused before closing
            // its active window or sending packets, while a negotiated peer opens it and a vanilla
            // peer keeps stock menus.
            use sunrise_gameplay_common::game::lan_actions::MenuOpen;
            use sunrise_level::containers::{BlockEntity, ModContainer};
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            let mod_pos = (0, 64, 0);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .set(mod_pos.0, mod_pos.1, mod_pos.2, stone)
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    stone.0,
                    std::sync::Arc::new(ModContainer {
                        slots: 9,
                        menu: Some(MOD_MENU.into()),
                        ..Default::default()
                    }),
                )]
                .into_iter()
                .collect(),
            );
            game.world
                .block_entities
                .insert(mod_pos, BlockEntity::Chest(vec![None; 9]));
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.brand = "sunrise".into();
                g.sender.menu_extra_rev = 0;
            });
            assert_eq!(game.open_block_menu(conn, mod_pos), MenuOpen::Refused);
            assert_eq!(
                game.server.lan.as_ref().unwrap().hub.guests[0]
                    .menu
                    .as_ref()
                    .map(|m| m.window),
                Some(large_window),
                "unnegotiated Sunrise peer keeps its current menu"
            );
            assert!(
                sent(&rx).is_empty(),
                "unnegotiated block-backed mod menu refusal sends neither close nor open"
            );
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.sender.menu_extra_rev = 1;
            });
            assert_eq!(game.open_block_menu(conn, mod_pos), MenuOpen::Opened);
            sent(&rx);
            // A stock menu remains available to a vanilla peer with no menu_extra negotiation.
            let chest_pos = (1, 64, 0);
            let chest = game
                .pack
                .assets
                .registry
                .default_state("minecraft:chest")
                .unwrap();
            game.world
                .set(chest_pos.0, chest_pos.1, chest_pos.2, chest)
                .unwrap();
            game.world
                .block_entities
                .insert(chest_pos, BlockEntity::Chest(vec![None; 27]));
            game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.brand = "vanilla".into();
                g.sender.menu_extra_rev = 0;
            });
            assert_eq!(game.open_block_menu(conn, chest_pos), MenuOpen::Opened);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn refused_swap_resends_the_menu_and_undisplayed_hotbar() {
        for (player_inventory, button, menu_slots) in [(false, 2, 1), (true, 40, 37)] {
            sunrise_level::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "status_package".into(),
                    1,
                    0,
                    vec![],
                    vec![],
                    None,
                    None,
                    player_inventory,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let held = ItemStack::new("minecraft:stone", 5);
                        let stored = ItemStack::new("minecraft:dirt", 2);
                        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.sender.menu_extra_rev =
                                sunrise_protocol::menu_extra::LIVE_DATA_REVISION;
                            if player_inventory {
                                g.offhand = Some(held.clone());
                            } else {
                                g.inventory[2] = Some(held.clone());
                            }
                        });
                        let player = game.player(123).unwrap();
                        let menu = player
                            .open_mod_menu(
                                game,
                                ModMenuOpen::new(key, mod_api::Text::literal("Panel")),
                            )
                            .unwrap()
                            .unwrap();
                        let window = menu.container_id(game).unwrap();
                        let host: &mut dyn ServerHost = game;
                        menu.set_items(host, vec![Some(stored.clone().try_into_api().unwrap())])
                            .unwrap();
                        sent(&rx);

                        // With no click listener the menu keeps its stack, but the client
                        // predicted a swap with the undisplayed hotbar or offhand.
                        let click = S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: 0,
                            slot: 0,
                            button,
                            input: sunrise_sim::lan::menu::input::SWAP,
                            changed: vec![],
                            carried: None,
                        });
                        let host = game.server.lan.take().unwrap();
                        let host = game
                            .lan_process_events(
                                host,
                                [sunrise_sim::lan::hub::HostEvent::Packet(1, Box::new(click))],
                                &[],
                                [0.0; 3],
                            )
                            .unwrap();
                        let packets = sent(&rx);
                        let [C::ContainerSetContent(menu), correction] = &packets[..] else {
                            panic!(
                                "swap button {button}: expected menu and backing inventory corrections, got {packets:?}"
                            );
                        };
                        assert_eq!((menu.container, menu.items.len()), (window, menu_slots));
                        assert_eq!(
                            host.hub.stack_from_network(menu.items[0].as_ref().unwrap()),
                            stored
                        );
                        let item = match (player_inventory, correction) {
                            (false, C::ContainerSetContent(inventory)) => {
                                assert_eq!((inventory.container, inventory.items.len()), (0, 46));
                                &inventory.items[38]
                            }
                            (true, C::SetPlayerInventory(offhand)) => {
                                assert_eq!(offhand.slot, 40);
                                &offhand.item
                            }
                            _ => {
                                panic!("swap button {button}: unexpected correction {correction:?}")
                            }
                        };
                        assert_eq!(host.hub.stack_from_network(item.as_ref().unwrap()), held);
                    });
                },
            );
        }
    }
    #[test]
    #[ignore = "daily"]
    fn blockless_menu_data_is_committed_to_only_its_live_generation() {
        use mod_api::{ContainerOpenedEvent, MenuDataRefusal, OpenMenuError};
        use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
        use sunrise_protocol::menu_extra::{DATA_CHANNEL, DataSlot, LIVE_DATA_REVISION};

        struct Veto {
            cancel: Arc<AtomicBool>,
            disconnect: Arc<AtomicBool>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                _: &mut ContainerOpenedEvent<'_>,
            ) -> mod_api::Flow {
                if self.disconnect.load(Relaxed) {
                    let player = host.players().acting_player().unwrap();
                    assert_eq!(
                        player.disconnect(host, mod_api::Text::literal("leaving")),
                        Ok(true)
                    );
                }
                if self.cancel.load(Relaxed) {
                    mod_api::Flow::Cancel
                } else {
                    mod_api::Flow::Continue
                }
            }
        }
        let veto = Arc::new(AtomicBool::new(false));
        let disconnect = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "veto",
            Box::new(Veto {
                cancel: veto.clone(),
                disconnect: disconnect.clone(),
            }),
        )]
        .into();
        let updates = |packets: Vec<C>| {
            packets
                .into_iter()
                .filter_map(|p| match p {
                    C::CustomPayload(p) if p.channel == DATA_CHANNEL => {
                        Some(DataSlot::decode(&p.data).unwrap())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        mod_platform::testing::with_mods(mods, || {
            sunrise_level::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "status_package".into(),
                    0,
                    2,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    false,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let old = open(game, 3, "old", MenuReplace::CloseFirst);
                        sent(&rx);
                        let player = game.player(123).unwrap();
                        let show = |host: &mut dyn ServerHost, data| {
                            player.open_mod_menu(host, {
                                let mut request =
                                    ModMenuOpen::new(key, mod_api::Text::literal("status"));
                                request.extra = vec![7];
                                request.data = data;
                                request.replace = MenuReplace::InPlace;
                                request
                            })
                        };
                        assert_eq!(show(game, vec![9]), Err(OpenMenuError::DataCountMismatch));
                        assert_eq!(
                            show(game, vec![-80_000, 0]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.sender.menu_extra_rev = LIVE_DATA_REVISION);
                        let network_title = mod_api::Text::literal("x".repeat(65_536));
                        assert_eq!(
                            player.open_mod_menu(game, {
                                let mut request = ModMenuOpen::new(key, network_title);
                                request.data = vec![1, 2];
                                request
                            }),
                            Err(OpenMenuError::TitleNotEncodable),
                        );
                        veto.store(true, Relaxed);
                        assert_eq!(show(game, vec![-80_000, 0]), Ok(None));
                        veto.store(false, Relaxed);
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.disconnect_pending = true);
                        assert_eq!(
                            show(game, vec![-80_000, 0]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.disconnect_pending = false);
                        game.server.lan.as_mut().unwrap().hub.guests.push(guest(2));
                        game.as_guest(1, |game| {
                            game.as_guest(2, |game| {
                                assert_eq!(
                                    show(game, vec![-80_000, 0]),
                                    Err(OpenMenuError::PlayerUnavailable)
                                );
                            })
                            .unwrap();
                        })
                        .unwrap();
                        assert_eq!(
                            game.menu_kind(old),
                            Some(Some("minecraft:generic_9x3".into()))
                        );
                        assert_eq!(
                            old.set_data_slot(game, 0, 1),
                            Err(MenuDataRefusal::Unsupported)
                        );
                        assert!(sent(&rx).is_empty());

                        let menu = show(game, vec![-80_000, 0]).unwrap().unwrap();
                        let window = menu.container_id(game).unwrap();
                        assert_eq!(player.container_menu(game), Some(menu));
                        assert_eq!(menu.slot_count(game), Some(0));
                        assert!(menu.slot(game, 0).is_none());
                        let packets = sent(&rx);
                        let open_at = packets
                            .iter()
                            .position(|p| matches!(p, C::OpenScreen(p) if p.container == window))
                            .unwrap();
                        assert!(packets.iter().any(|p| matches!(p, C::ContainerSetContent(p) if p.container == window && p.items.is_empty())));
                        assert!(!packets[..open_at].iter().any(
                            |p| matches!(p, C::CustomPayload(p) if p.channel == DATA_CHANNEL)
                        ));
                        assert_eq!(
                            updates(packets),
                            [
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: -80_000
                                },
                                DataSlot {
                                    window,
                                    index: 1,
                                    value: 0
                                },
                            ]
                        );
                        {
                            // Any server caller with the live MenuRef can update a declared
                            // data slot, not only the mod that registered the menu type.
                            let _guard = mod_api::content::crash::enter_native_mod(
                                mod_api::content::crash::ModCall {
                                    id: "listen",
                                    hook: "test",
                                    part: None,
                                },
                            );
                            assert_eq!(menu.set_data_slot(game, 0, 12), Ok(true));
                            assert_eq!(
                                updates(sent(&rx)),
                                [DataSlot {
                                    window,
                                    index: 0,
                                    value: 12,
                                }]
                            );
                            assert_eq!(menu.set_data_slot(game, 0, 12), Ok(false));
                            assert!(sent(&rx).is_empty());
                        }
                        // Updating without an active native mod caller succeeds as well.
                        assert_eq!(menu.set_data_slot(game, 0, -80_000), Ok(true));
                        assert_eq!(
                            updates(sent(&rx)),
                            [DataSlot {
                                window,
                                index: 0,
                                value: -80_000,
                            }]
                        );
                        // A full resend of this window has no hotbar slots, so a pending
                        // inventory update must remain pending.
                        let hub = &mut game.server.lan.as_mut().unwrap().hub;
                        hub.guests.update(0, |g| {
                            assert_eq!(g.menu_sent[36], None);
                            g.inventory[0] = Some(ItemStack::new("minecraft:stone", 1));
                        });
                        hub.resend_menu(1, window, &game.common.world);
                        assert_eq!(hub.guests[0].menu_sent[36], None);
                        let packets = sent(&rx);
                        assert!(matches!(packets.first(), Some(C::ContainerSetContent(p))
                            if p.container == window && p.items.is_empty()
                        ));
                        let snapshot = [
                            DataSlot {
                                window,
                                index: 0,
                                value: -80_000,
                            },
                            DataSlot {
                                window,
                                index: 1,
                                value: 0,
                            },
                        ];
                        assert_eq!(updates(packets), snapshot);
                        assert_eq!(
                            menu.set_data_slot(game, 2, 1),
                            Err(MenuDataRefusal::InvalidIndex)
                        );
                        let resend = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                            game.server.lan.as_mut().unwrap().hub.resend_menu(
                                1,
                                window,
                                &game.common.world,
                            );
                        };
                        game.storage_scope(|game, _| {
                            assert_eq!(menu.set_data_slot(game, 0, i32::MAX), Ok(true));
                            assert!(sent(&rx).is_empty());
                            resend(game);
                            assert_eq!(
                                updates(sent(&rx)),
                                snapshot,
                                "a resend cannot see the pending write"
                            );
                            ((), false)
                        });
                        assert_eq!(menu.set_data_slot(game, 0, -80_000), Ok(false));
                        assert!(sent(&rx).is_empty());

                        game.storage_scope(|game, _| {
                            game.transactions.last_mut().unwrap().packets =
                                Some(sunrise_sim::lan::conn::transaction_packets::Scope::begin());
                            assert_eq!(menu.set_data_slot(game, 0, 100_000), Ok(true));
                            resend(game);
                            assert!(sent(&rx).is_empty());
                            ((), true)
                        });
                        assert_eq!(
                            updates(sent(&rx)),
                            [
                                snapshot[0],
                                snapshot[1],
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: 100_000
                                },
                            ],
                            "the captured resend cannot roll back the committed delta"
                        );

                        let host = game as &mut dyn ServerHost;
                        let committed: Result<(), TransactionError<()>> =
                            host.transaction(|host, _| {
                                assert_eq!(menu.set_data_slot(host, 0, 1), Ok(true));
                                let nested: Result<(), TransactionError<()>> =
                                    host.transaction(|host, _| {
                                        assert_eq!(menu.set_data_slot(host, 0, i32::MIN), Ok(true));
                                        assert_eq!(menu.set_data_slot(host, 1, 5), Ok(true));
                                        Ok(())
                                    });
                                assert_eq!(nested, Ok(()));
                                assert_eq!(menu.set_data_slot(host, 1, 0), Ok(true));
                                assert!(sent(&rx).is_empty());
                                Ok(())
                            });
                        assert_eq!(committed, Ok(()));
                        assert_eq!(
                            updates(sent(&rx)),
                            [DataSlot {
                                window,
                                index: 0,
                                value: i32::MIN
                            }]
                        );
                        let state = game.server.lan.as_ref().unwrap().hub.guests[0]
                            .menu
                            .as_ref()
                            .unwrap()
                            .state;
                        game.lan_guest_packet(
                            1,
                            S::ContainerClick(play::ContainerClick {
                                container: window,
                                state: (state - 1) & 0x7FFF,
                                slot: -999,
                                button: 0,
                                input: sunrise_sim::lan::menu::input::PICKUP,
                                changed: vec![],
                                carried: None,
                            }),
                        );
                        let packets = sent(&rx);
                        assert!(matches!(packets.first(), Some(C::ContainerSetContent(p))
                            if p.container == window && p.items.is_empty() && p.state != state
                        ));
                        assert_eq!(
                            updates(packets),
                            [
                                DataSlot {
                                    value: i32::MIN,
                                    ..snapshot[0]
                                },
                                snapshot[1]
                            ],
                            "a stale click resends every committed data slot after the contents"
                        );

                        // Force the counter to reuse the old id, as it would after 100 opens.
                        game.server
                            .lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update(0, |g| g.next_window = window - 1);
                        let replaced: Result<(), TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                assert_eq!(menu.set_data_slot(host, 0, 900), Ok(true));
                                let staged = show(host, vec![5, 6]).unwrap().unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(staged.container_id(host), None);
                                Ok(())
                            });
                        assert_eq!(replaced, Ok(()));
                        assert_eq!(
                            player.container_menu(game).unwrap().container_id(game),
                            Some(window)
                        );
                        assert_eq!(
                            menu.set_data_slot(game, 0, 999),
                            Err(MenuDataRefusal::Stale)
                        );
                        // The old generation's commit can update the old screen, but its
                        // data must precede OpenScreen even though the protocol id is reused.
                        let mut packets = sent(&rx);
                        let opened = packets
                            .iter()
                            .position(|p| {
                                matches!(p,
                                    C::OpenScreen(p) if p.container == window
                                )
                            })
                            .unwrap();
                        let replacement = packets.split_off(opened);
                        assert_eq!(
                            updates(packets),
                            [DataSlot {
                                window,
                                index: 0,
                                value: 900
                            }]
                        );
                        assert_eq!(
                            updates(replacement),
                            [
                                DataSlot {
                                    window,
                                    index: 0,
                                    value: 5
                                },
                                DataSlot {
                                    window,
                                    index: 1,
                                    value: 6
                                },
                            ]
                        );
                        assert!(player.container_menu(game).unwrap().close_container(game));
                        assert!(updates(sent(&rx)).is_empty());
                        assert_eq!(
                            player
                                .container_menu(game)
                                .unwrap()
                                .set_data_slot(game, 0, 1),
                            Err(MenuDataRefusal::Unsupported)
                        );

                        let old = open(game, 3, "before disconnect", MenuReplace::CloseFirst);
                        let old_window = old.container_id(game).unwrap();
                        sent(&rx);
                        disconnect.store(true, Relaxed);
                        assert_eq!(
                            show(game, vec![1, 2]),
                            Err(OpenMenuError::PlayerUnavailable)
                        );
                        let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                        assert!(guest.disconnect_pending);
                        assert_eq!(guest.menu.as_ref().map(|m| m.window), Some(old_window));
                        assert!(!sent(&rx).iter().any(|packet| matches!(
                            packet,
                            C::OpenScreen(_) | C::ContainerClose(_)
                        )));
                    })
                },
            );
        });
    }
    #[test]
    #[ignore = "daily"]
    fn revision_two_blockless_menu_syncs_only_encodable_indices() {
        use sunrise_protocol::menu_extra::{DATA_CHANNEL, DataSlot};
        use sunrise_sim::lan::menu::input::PICKUP;

        const LAST: usize = u16::MAX as usize;
        const OVERFLOW: usize = LAST + 1;
        const FIRST_VALUE: i32 = 123_456;
        const LAST_VALUE: i32 = -80_000;
        const OVERFLOW_VALUE: i32 = 456_789;
        sunrise_level::containers::with_mod_menu_types(
            [(
                MOD_MENU.into(),
                "status_package".into(),
                0,
                OVERFLOW + 1,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
                None,
            )],
            || {
                with_game(|game| {
                    let (rx, key) = mod_menu_guest(game);
                    game.server
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |g| g.sender.menu_extra_rev = 2);
                    let player = game.player(123).unwrap();
                    let mut data = vec![0; OVERFLOW + 1];
                    data[0] = FIRST_VALUE;
                    data[LAST] = LAST_VALUE;
                    data[OVERFLOW] = OVERFLOW_VALUE;
                    let mut request = ModMenuOpen::new(key, mod_api::Text::literal("status"));
                    request.data = data;
                    request.replace = MenuReplace::CloseFirst;
                    let menu = player.open_mod_menu(game, request).unwrap().unwrap();
                    let window = menu.container_id(game).unwrap();
                    let updates = || {
                        sent(&rx).into_iter().filter_map(|packet| match packet {
                            C::CustomPayload(p) if p.channel == DATA_CHANNEL => {
                                Some(DataSlot::decode(&p.data).unwrap())
                            }
                            _ => None,
                        })
                    };
                    let assert_snapshot = |route, last_value| {
                        let expected = (0..=u16::MAX).map(|index| DataSlot {
                            window,
                            index,
                            value: match index {
                                0 => FIRST_VALUE,
                                u16::MAX => last_value,
                                _ => 0,
                            },
                        });
                        assert!(
                            updates().eq(expected),
                            "incorrect revision-2 snapshot on {route}"
                        );
                    };
                    assert_snapshot("open", LAST_VALUE);
                    let _guard = mod_api::content::crash::enter_native_mod(
                        mod_api::content::crash::ModCall {
                            id: "status_package",
                            hook: "test",
                            part: None,
                        },
                    );
                    assert_eq!(player.container_menu(game), Some(menu));
                    let host: &mut dyn ServerHost = game;
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        host.transaction(|host, _| {
                            assert_eq!(menu.set_data_slot(host, LAST, i32::MAX), Ok(true));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    assert!(updates().eq([DataSlot {
                        window,
                        index: u16::MAX,
                        value: i32::MAX,
                    }]));
                    let committed: Result<(), mod_api::TransactionError<()>> =
                        host.transaction(|host, _| {
                            assert_eq!(menu.set_data_slot(host, OVERFLOW, i32::MIN), Ok(true));
                            assert!(sent(&rx).is_empty());
                            Ok(())
                        });
                    assert_eq!(committed, Ok(()));
                    assert!(sent(&rx).is_empty(), "unencodable index sent a delta");

                    let hub = &mut game.server.lan.as_mut().unwrap().hub;
                    let open = hub.guests[0].menu.as_ref().unwrap();
                    assert_eq!(open.blockless_data[OVERFLOW], i32::MIN);
                    hub.resend_menu(1, window, &game.common.world);
                    assert_snapshot("explicit resend", i32::MAX);
                    let state = hub.guests[0].menu.as_ref().unwrap().state;
                    // The live native click route resends in prepare_click -> answer_click.
                    game.lan_guest_packet(
                        1,
                        S::ContainerClick(play::ContainerClick {
                            container: window,
                            state: (state - 1) & 0x7FFF,
                            slot: -999,
                            button: 0,
                            input: PICKUP,
                            changed: vec![],
                            carried: None,
                        }),
                    );
                    assert_snapshot("stale click", i32::MAX);
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn cross_dimension_mod_menu_open_in_transaction_isolates_veto_writes() {
        use mod_api::{BlockUpdateFlags, ContainerOpenedEvent, HostBlockStates};
        use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
        use sunrise_gameplay_common::game::Dim;
        use sunrise_level::chunk::Chunk;

        const POS: [i32; 3] = [2, 64, 2];
        struct Veto {
            calls: Arc<AtomicUsize>,
        }
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Veto {
            fn container_opened(
                &self,
                host: &mut dyn ServerHost,
                _: &mut ContainerOpenedEvent<'_>,
            ) -> mod_api::Flow {
                self.calls.fetch_add(1, Relaxed);
                assert!(
                    host.transaction_matches(None),
                    "deferred cross-level veto must not borrow the caller's transaction journal"
                );
                let nether = host.level("minecraft:the_nether").unwrap();
                let gold = mod_api::BlockState::of(
                    &host.block_states(),
                    mod_api::__private::block_key("minecraft:gold_block").unwrap(),
                    &[],
                )
                .unwrap();
                assert_eq!(
                    nether.set_block(host, POS, gold, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                mod_api::Flow::Continue
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "cross_veto",
            Box::new(Veto {
                calls: calls.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_level::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "cross_veto".into(),
                    1,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (_rx, key) = mod_menu_guest(game);
                        game.server.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                            g.dimension = Dim::Nether.id().to_owned();
                        });
                        let registry = game.pack.assets.registry;
                        let stone = registry.default_state("minecraft:stone").unwrap();
                        let netherrack = registry.default_state("minecraft:netherrack").unwrap();
                        let cobblestone = registry.default_state("minecraft:cobblestone").unwrap();
                        let gold_block = registry.default_state("minecraft:gold_block").unwrap();

                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.world.set(POS[0], POS[1], POS[2], stone).unwrap();
                        game.enter_level(Dim::Nether).unwrap();
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.world.set(POS[0], POS[1], POS[2], netherrack).unwrap();
                        game.enter_level(Dim::Overworld).unwrap();

                        let player = game.player(123).unwrap();
                        let aborted: Result<(), mod_api::TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                let overworld = host.level("minecraft:overworld").unwrap();
                                let planks = mod_api::BlockState::of(
                                    &host.block_states(),
                                    mod_api::__private::block_key("minecraft:oak_planks").unwrap(),
                                    &[],
                                )
                                .unwrap();
                                assert_eq!(
                                    overworld.set_block(
                                        host,
                                        POS,
                                        planks,
                                        BlockUpdateFlags::NEIGHBORS,
                                    ),
                                    Ok(true)
                                );
                                let staged = player
                                    .open_mod_menu(
                                        host,
                                        ModMenuOpen::new(key, mod_api::Text::literal("aborted")),
                                    )
                                    .unwrap()
                                    .unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(calls.load(Relaxed), 0);
                                Err(())
                            });
                        assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                        assert_eq!(game.dim, Dim::Overworld);
                        assert_eq!(calls.load(Relaxed), 0);
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), stone);
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
                        game.enter_level(Dim::Nether).unwrap();
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), netherrack);
                        game.enter_level(Dim::Overworld).unwrap();

                        let committed: Result<(), mod_api::TransactionError<()>> =
                            (game as &mut dyn ServerHost).transaction(|host, _| {
                                let overworld = host.level("minecraft:overworld").unwrap();
                                let cobble = mod_api::BlockState::of(
                                    &host.block_states(),
                                    mod_api::__private::block_key("minecraft:cobblestone").unwrap(),
                                    &[],
                                )
                                .unwrap();
                                assert_eq!(
                                    overworld.set_block(
                                        host,
                                        POS,
                                        cobble,
                                        BlockUpdateFlags::NEIGHBORS,
                                    ),
                                    Ok(true)
                                );
                                let staged = player
                                    .open_mod_menu(
                                        host,
                                        ModMenuOpen::new(key, mod_api::Text::literal("committed")),
                                    )
                                    .unwrap()
                                    .unwrap();
                                assert!(staged.fields().window < 0);
                                assert_eq!(calls.load(Relaxed), 0);
                                Ok(())
                            });
                        assert_eq!(committed, Ok(()));
                        assert_eq!(game.dim, Dim::Overworld);
                        assert_eq!(calls.load(Relaxed), 1);
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), cobblestone);
                        let live = game
                            .server
                            .lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(1)
                            .unwrap()
                            .menu
                            .as_ref()
                            .expect("committed cross-level open installed the menu")
                            .window;
                        assert!((1..=100).contains(&live));
                        game.enter_level(Dim::Nether).unwrap();
                        assert_eq!(game.world.block(POS[0], POS[1], POS[2]), gold_block);
                        game.enter_level(Dim::Overworld).unwrap();
                    });
                },
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn aborted_staged_menu_opens_never_wrap_onto_or_access_the_live_window() {
        use mod_api::{MenuItemsRefusal, MenuRef, MenuSlotBackRefusal, PlayerContainerOpenEvent};
        use std::sync::Mutex;

        struct Opened(Arc<Mutex<Vec<Option<i32>>>>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Opened {
            fn player_container_open(
                &self,
                host: &mut dyn ServerHost,
                event: &mut PlayerContainerOpenEvent<'_>,
            ) {
                self.0
                    .lock()
                    .unwrap()
                    .push(event.menu.and_then(|menu| menu.container_id(host)));
            }
        }
        let opened = Arc::new(Mutex::new(Vec::new()));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "opened",
            Box::new(Opened(opened.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            sunrise_level::containers::with_mod_menu_types(
                [(
                    MOD_MENU.into(),
                    "opened".into(),
                    1,
                    0,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    true,
                    None,
                )],
                || {
                    with_game(|game| {
                        let (rx, key) = mod_menu_guest(game);
                        let player = game.player(123).unwrap();
                        let stone = ItemStack::new("minecraft:stone", 1).try_into_api().unwrap();
                        let dirt = ItemStack::new("minecraft:dirt", 1).try_into_api().unwrap();

                        for start_counter in [0, 99] {
                            game.server
                                .lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .guests
                                .update(0, |g| g.next_window = start_counter);
                            let old = open(game, 1, "live", MenuReplace::CloseFirst);
                            let live = old.container_id(game).unwrap();
                            assert_eq!(live, start_counter % 100 + 1);
                            old.set_items(game, vec![Some(stone.clone())]).unwrap();
                            opened.lock().unwrap().clear();
                            sent(&rx);

                            for step in 0..105 {
                                let aborted: Result<(), mod_api::TransactionError<()>> =
                                    (game as &mut dyn ServerHost).transaction(|host, _| {
                                        let staged = if step % 2 == 0 {
                                            player
                                                .open_chest_menu(
                                                    host,
                                                    1,
                                                    mod_api::Text::literal("aborted"),
                                                    vec![Some(dirt.clone())],
                                                    MenuReplace::InPlace,
                                                )
                                                .unwrap()
                                        } else {
                                            let mut req = ModMenuOpen::new(
                                                key,
                                                mod_api::Text::literal("aborted mod"),
                                            );
                                            req.replace = MenuReplace::InPlace;
                                            player.open_mod_menu(host, req).unwrap().unwrap()
                                        };
                                        assert!(staged.fields().window < 0);
                                        assert_ne!(staged, old);
                                        assert_eq!(staged.container_id(host), None);
                                        assert_eq!(
                                            staged.set_items(host, vec![Some(dirt.clone())]),
                                            Err(MenuItemsRefusal::NotOpen)
                                        );
                                        assert_eq!(
                                            staged.back_slots(host, Vec::new()),
                                            Err(MenuSlotBackRefusal::NoPlayerOrWindow)
                                        );
                                        assert_eq!(player.container_menu(host), Some(old));
                                        Err(())
                                    });
                                assert_eq!(aborted, Err(mod_api::TransactionError::Aborted(())));
                            }

                            assert_eq!(
                                game.server
                                    .lan
                                    .as_ref()
                                    .unwrap()
                                    .hub
                                    .guest(1)
                                    .unwrap()
                                    .next_window,
                                live
                            );
                            assert_eq!(old.slot(game, 0), Some(Some(stone.clone())));
                            assert!(opened.lock().unwrap().is_empty());
                            assert!(sent(&rx).is_empty());

                            let committed: Result<MenuRef, mod_api::TransactionError<()>> =
                                (game as &mut dyn ServerHost).transaction(|host, _| {
                                    let staged = player
                                        .open_chest_menu(
                                            host,
                                            1,
                                            mod_api::Text::literal("replacement"),
                                            vec![Some(dirt.clone())],
                                            MenuReplace::InPlace,
                                        )
                                        .unwrap();
                                    assert!(staged.fields().window < 0);
                                    assert_eq!(staged.container_id(host), None);
                                    assert_eq!(
                                        staged.set_items(host, vec![Some(dirt.clone())]),
                                        Err(MenuItemsRefusal::NotOpen)
                                    );
                                    Ok(staged)
                                });
                            let staged = committed.unwrap();
                            let next = live % 100 + 1;
                            assert_ne!(next, live);
                            assert_eq!(*opened.lock().unwrap(), [Some(next)]);
                            let replacement = player.container_menu(game).unwrap();
                            assert_eq!(replacement.container_id(game), Some(next));
                            assert_eq!(old.container_id(game), None);
                            assert_eq!(staged.container_id(game), None);
                            assert_eq!(replacement.slot(game, 0), Some(Some(dirt.clone())));
                            let packets = sent(&rx);
                            assert!(
                                packets.iter().any(|p| {
                                    matches!(p, C::OpenScreen(p) if p.container == next)
                                })
                            );
                            assert!(!packets.iter().any(|p| matches!(p, C::ContainerClose(_))));

                            // An in-flight click for the outgoing window is ignored by the replacement.
                            game.lan_guest_packet(
                                1,
                                S::ContainerClick(play::ContainerClick {
                                    container: live,
                                    state: 0,
                                    slot: 0,
                                    button: 0,
                                    input: sunrise_sim::lan::menu::input::PICKUP,
                                    changed: vec![],
                                    carried: None,
                                }),
                            );
                            assert!(sent(&rx).is_empty());
                            assert_eq!(replacement.slot(game, 0), Some(Some(dirt.clone())));
                        }
                    });
                },
            );
        });
    }
}
