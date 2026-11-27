#![cfg(test)]
use mod_api::ItemPlace;
use mod_api::MenuRef;
use mod_api::MenuSlotPlace;
use mod_api::ServerHost;
use sunrise_sim::lan::menu::NativeMenuEvent;

use sunrise_gameplay_common::game::native_server::NativeServerGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game as with_base_game;
    use super::*;
    use mod_api::{Hand, MenuContainer, MenuSlotBackRefusal};
    use mod_api::{HostTransaction, MenuReplace, MenuSlotBacking};
    use sunrise_items::items::ItemStack;
    use sunrise_sim::lan::menu::Slots;
    use {
        sunrise_gameplay::game::server_side::native_item_places::tests::open_to_lan,
        sunrise_gameplay_common::game::server_side::native_item_places::tests::guest,
    };

    struct Backpack;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Backpack {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:shulker_box");
            r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
        }
    }

    fn with_game(test: impl FnOnce(&mut sunrise_gameplay::game::ServerGame<'_>)) {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "backpack_fixture",
            Box::new(Backpack),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || with_base_game(test));
    }

    /// Guest 1 (UUID 123) holding a shulker box in hotbar slot 0 with 64 stone in inventory
    /// slot 9.
    fn backpack_guest(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        open_to_lan(game);
        let mut player = guest(1);
        player.selected = 0;
        player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
        player.inventory[9] = Some(ItemStack::new("minecraft:stone", 64));
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
    }

    /// A 27-slot mod chest menu opened for guest 1: its handle.
    fn open_pack(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> MenuRef {
        let player = game.player(123).unwrap();
        let host: &mut dyn ServerHost = game;
        player
            .open_chest_menu(
                host,
                3,
                mod_api::Text::literal("pack"),
                Vec::new(),
                MenuReplace::CloseFirst,
            )
            .unwrap()
    }

    /// [`backpack_guest`] with the menu open: its handle.
    fn backpack_menu(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> MenuRef {
        backpack_guest(game);
        open_pack(game)
    }

    fn held_slot(n: usize) -> ItemPlace {
        ItemPlace::Nested {
            parent: Box::new(ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            }),
            slot: n,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_shift_click_into_a_held_backpack_writes_through_its_handler() {
        with_game(|game| {
            let stale = backpack_menu(game);
            let menu = open_pack(game);
            let window = menu.container_id(game).unwrap();
            let host: &mut dyn ServerHost = game;
            // Unbacked, the menu is the mod's own, as before.
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), None);

            let places: Vec<_> = (0..27)
                .map(|slot| MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot,
                })
                .collect();
            assert_eq!(
                stale.back_slots(game, places.clone()),
                Err(MenuSlotBackRefusal::NoPlayerOrWindow)
            );
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(
                menu.slot_backing(host, 0),
                Some(MenuSlotBacking::Place(MenuSlotPlace::HeldItem {
                    hand: Hand::Main,
                    slot: 0
                }))
            );

            // Inventory slot 9 is menu slot 27: a shift-click moves the stone into the box.
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 27 }.with_stack(host, |_| ()),
                None
            );
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 64);
            let stone = ItemPlace::MenuSlot { menu, slot: 0 }
                .resource(host)
                .unwrap();
            assert_eq!(menu.slot_gives(host, 0), None);
            assert_eq!(menu.probe_slot_gives(host, 0), Ok(Some(true)));
            assert_eq!(held_slot(0).amount(host), 64, "the pickup probe rolls back");
            assert_eq!(menu.slot_holds(host, 54, stone.as_stack()), Some(0));
            assert_eq!(menu.slot_gives(host, 54), Some(false));
            assert_eq!(
                menu.probe_slot_holds(host, 54, stone.as_stack()),
                Ok(Some(0))
            );
            assert_eq!(menu.probe_slot_gives(host, 54), Ok(Some(false)));

            // The box's own hotbar slot (menu slot 54) takes no click.
            assert_eq!(game.place_menu_click(123, window, 54, 0, 0), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.cursor().with_stack(host, |_| ()), None);
            let held = ItemPlace::PlayerHand {
                player: 123,
                hand: Hand::Main,
            };
            assert_eq!(held.amount(host), 1, "the box stays in the hand");

            // The box gone from the hand, the write's notice closes the menu once taken.
            let resource = held.resource(host).unwrap();
            let taken = host.transaction(|host, tx| held.extract(host, tx, &resource, 1));
            assert_eq!(taken, Ok(1));
            game.run_native_menu_events();
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
        });
    }

    /// The guest's record, to change their hands as a packet or a command would.
    fn edit_guest(
        game: &mut sunrise_gameplay::game::ServerGame<'_>,
        edit: impl FnOnce(&mut sunrise_sim::lan::guest::Guest),
    ) {
        let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
        let index = guests.connection_index(1).unwrap();
        guests.update(index, edit);
    }

    fn held_places(hand: Hand, slots: std::ops::Range<usize>) -> Vec<MenuSlotPlace> {
        slots
            .map(|slot| MenuSlotPlace::HeldItem { hand, slot })
            .collect()
    }

    /// Stone in the box, inventory slot 9 and the menu's own slots: un-backing keeps it at 64.
    fn stone(game: &mut sunrise_gameplay::game::ServerGame<'_>, menu: MenuRef) -> u32 {
        let host: &mut dyn ServerHost = game;
        let boxed: u32 = (0..27).map(|n| held_slot(n).amount(host)).sum();
        let own: u32 = (0..27)
            .map(|slot| match menu.slot_backing(host, slot) {
                // A backed slot is a view of the box, already counted above.
                Some(MenuSlotBacking::Native(_)) => ItemPlace::MenuSlot { menu, slot }.amount(host),
                _ => 0,
            })
            .sum();
        let carried = ItemPlace::PlayerSlot {
            player: 123,
            slot: 9,
        }
        .amount(host);
        boxed + own + carried
    }

    #[test]
    #[ignore = "daily"]
    fn un_backing_or_shortening_empties_the_mirrored_slots_and_keeps_the_items_once() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let window = menu.container_id(game).unwrap();
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            assert_eq!(stone(game, menu), 64);
            // Shortened to slot 1 of the box: menu slot 0 shows that empty slot, the rest nothing.
            let places = held_places(Hand::Main, 1..2);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(ItemPlace::MenuSlot { menu, slot: 0 }.amount(host), 0);
            assert_eq!(held_slot(0).amount(host), 64);
            assert_eq!(stone(game, menu), 64);
            // Un-backed: the menu's own slots hold no copy to take out.
            assert_eq!(menu.back_slots(game, Vec::new()), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.slot_backing(host, 0), Some(MenuSlotBacking::Native(0)));
            assert_eq!(
                ItemPlace::MenuSlot { menu, slot: 0 }.with_stack(host, |_| ()),
                None
            );
            assert_eq!(stone(game, menu), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_backing_names_why_it_refuses() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let back = |game: &mut sunrise_gameplay::game::ServerGame<'_>, places| {
                menu.back_slots(game, places)
            };
            assert_eq!(
                back(game, held_places(Hand::Main, 0..28)),
                Err(MenuSlotBackRefusal::TooManyPlaces)
            );
            // Nothing in the offhand: the second place names no storage.
            let mut places = held_places(Hand::Main, 0..1);
            places.extend(held_places(Hand::Off, 0..1));
            assert_eq!(
                back(game, places),
                Err(MenuSlotBackRefusal::PlaceUnresolved { index: 1 })
            );
            assert_eq!(back(game, held_places(Hand::Main, 0..1)), Ok(()));
            let host: &mut dyn ServerHost = game;
            assert!(matches!(
                menu.slot_backing(host, 0),
                Some(MenuSlotBacking::Place(_))
            ));
            assert_eq!(
                game.as_guest(1, |game| back(game, held_places(Hand::Main, 0..1))),
                Some(Ok(()))
            );
            // A block's menu cannot use a backing, even with a leftover record for the window.
            edit_guest(game, |g| {
                g.menu.as_mut().unwrap().slots = Slots::Block((0, 64, 0));
            });
            assert_eq!(
                back(game, held_places(Hand::Main, 0..1)),
                Err(MenuSlotBackRefusal::UnsupportedMenu)
            );
            assert_eq!(game.menu_places_of(1), None);
            edit_guest(game, |g| g.menu.as_mut().unwrap().slots = Slots::Native);
            assert_eq!(back(game, held_places(Hand::Main, 0..1)), Ok(()));
            assert_eq!(
                game.as_guest(1, |game| back(game, held_places(Hand::Main, 0..1))),
                Some(Ok(()))
            );
            // A closed/stale menu handle is refused, in its own turn too.
            let stale_menu = menu;
            let _fresh = open_pack(game);
            let refused = Err(MenuSlotBackRefusal::NoPlayerOrWindow);
            let stale = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
                stale_menu.back_slots(game, held_places(Hand::Main, 0..1))
            };
            assert_eq!(stale(game), refused);
            assert_eq!(game.as_guest(1, stale), Some(refused));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_write_into_the_box_is_mirrored_at_its_notice() {
        with_game(|game| {
            let menu = backpack_menu(game);
            let places = held_places(Hand::Main, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            let host: &mut dyn ServerHost = game;
            let custom_data = mod_api::ComponentId::new("minecraft:custom_data").unwrap();
            let data = mod_api::Tag::Compound(
                [("value".into(), mod_api::Tag::Int(42))]
                    .into_iter()
                    .collect(),
            );
            let stone = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            }
            .resource(host)
            .unwrap()
            .with_component(host, &custom_data, data.clone())
            .unwrap();
            let put = host.transaction(|host, tx| held_slot(4).insert(host, tx, &stone, 5));
            assert_eq!(put, Ok(5));
            let shown = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                guest.native_menu[4].as_ref().map(|s| s.count)
            };
            assert_eq!(shown(game), None, "mirrored after the write, not inside it");
            let place = ItemPlace::MenuSlot { menu, slot: 4 };
            let found = place.with_stack(game, |stack| (stack.count(), stack.project()));
            let (count, projected) = found.unwrap();
            assert_eq!(count, 5);
            let projected = projected.unwrap();
            assert_eq!(projected.count(), 5);
            assert_eq!(
                mod_api::ItemResource::of(&projected).component(&custom_data),
                Some(data)
            );
            game.run_native_menu_events();
            assert_eq!(shown(game), Some(5));

            let host: &mut dyn ServerHost = game;
            let taken = host.transaction(|host, tx| held_slot(4).extract(host, tx, &stone, 5));
            assert_eq!(taken, Ok(5));
            assert_eq!(shown(game), Some(5), "the mirror has not caught up yet");
            assert_eq!(
                place.with_stack(game, |_| ()),
                None,
                "no fallback to the display mirror"
            );
        });
    }

    /// An item's use opens its menu and backs it at once, inside the player's own turn.
    #[test]
    #[ignore = "daily"]
    fn a_menu_backed_in_the_players_own_turn_shows_the_held_box_after_it() {
        with_game(|game| {
            backpack_guest(game);
            let (menu, backed) = game
                .as_guest(1, |game| {
                    let host: &mut dyn ServerHost = game;
                    let carried = ItemPlace::PlayerSlot {
                        player: 123,
                        slot: 9,
                    };
                    let stone = carried.resource(host).unwrap();
                    let put = host.transaction(|host, tx| held_slot(4).insert(host, tx, &stone, 5));
                    assert_eq!(put, Ok(5));
                    let menu = open_pack(game);
                    let places = held_places(Hand::Main, 0..27);
                    (menu, menu.back_slots(game, places))
                })
                .unwrap();
            assert_eq!(backed, Ok(()));
            let window = menu.container_id(game).unwrap();
            let shown = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
                guest.native_menu[4].as_ref().map(|s| s.count)
            };
            assert_eq!(shown(game), None, "mirrored after the turn, not inside it");
            game.run_native_menu_events();
            assert_eq!(shown(game), Some(5));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), Some(window));

            // The box bound is the one the player held in that turn: another box selected, the
            // next click closes the menu and moves nothing.
            edit_guest(game, |g| {
                g.inventory[1] = Some(ItemStack::new("minecraft:shulker_box", 1));
                g.selected = 1;
            });
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn rebinding_coalesces_notices_until_the_right_window_is_processed() {
        with_game(|game| {
            backpack_guest(game);
            let (old, new) = game
                .as_guest(1, |game| {
                    let old = open_pack(game);
                    let old_window = old.container_id(game).unwrap();
                    old.back_slots(game, held_places(Hand::Main, 0..1)).unwrap();
                    let new = open_pack(game);
                    let new_window = new.container_id(game).unwrap();
                    for places in [
                        held_places(Hand::Main, 0..2),
                        Vec::new(),
                        held_places(Hand::Main, 1..2),
                    ] {
                        new.back_slots(game, places).unwrap();
                    }
                    (old_window, new_window)
                })
                .unwrap();
            // Deliver the old window's change as the event drain does, then write again.
            game.menu_places_changed(123, old);
            game.notice_menu_place_write(&held_slot(1));
            let windows: Vec<_> = game
                .server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .native_menu_events
                .iter()
                .filter_map(|event| match event {
                    NativeMenuEvent::PlacesChanged { uuid: 123, window } => Some(*window),
                    _ => None,
                })
                .collect();
            assert_eq!(
                windows,
                [old, new],
                "one notice per window, even across an unback"
            );
            game.run_native_menu_events();
            game.notice_menu_place_write(&held_slot(1));
            assert!(matches!(
                game.server.lan.as_ref().unwrap().hub.native_menu_events.as_slice(),
                [NativeMenuEvent::PlacesChanged { uuid: 123, window }] if *window == new
            ));
        });
    }

    /// The bound stack replaced by `replace`, then a shift-click: the menu closes and the click
    /// moves nothing.
    fn replaced_closes(hand: Hand, replace: impl FnOnce(&mut sunrise_sim::lan::guest::Guest)) {
        with_game(|game| {
            let menu = backpack_menu(game);
            let window = menu.container_id(game).unwrap();
            if hand == Hand::Off {
                edit_guest(game, |g| g.offhand = g.inventory[0].take());
            }
            let places = held_places(hand, 0..27);
            assert_eq!(menu.back_slots(game, places), Ok(()));
            edit_guest(game, replace);
            assert_eq!(game.place_menu_click(123, window, 27, 0, 1), Some(true));
            let host: &mut dyn ServerHost = game;
            assert_eq!(menu.container_id(host), None);
            let carried = ItemPlace::PlayerSlot {
                player: 123,
                slot: 9,
            };
            assert_eq!(carried.amount(host), 64);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_hand_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[0] = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_stack_in_the_offhand_closes_the_menu() {
        replaced_closes(Hand::Off, |g| {
            g.offhand = Some(ItemStack::new("minecraft:red_shulker_box", 1));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn another_hotbar_slot_selected_closes_the_menu() {
        replaced_closes(Hand::Main, |g| {
            g.inventory[1] = Some(ItemStack::new("minecraft:shulker_box", 1));
            g.selected = 1;
        });
    }

    #[test]
    #[ignore = "daily"]
    fn backed_slot_rules_ask_the_live_handler_not_the_display_copy() {
        use mod_api::{
            ItemResource, Overanswered, ResourceHandler, TransactionContext, TransactionError,
        };
        struct Handler;
        impl ResourceHandler<ItemResource> for Handler {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                2
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                Some(ItemResource::of(
                    &mod_api::ItemStack::new(
                        mod_api::__private::item_key("minecraft:stone").unwrap(),
                        1,
                    )
                    .unwrap(),
                ))
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                1
            }
            fn capacity(
                &self,
                _: &mut dyn ServerHost,
                slot: usize,
                _: Option<&ItemResource>,
            ) -> u32 {
                if slot == 0 { 7 } else { u32::MAX }
            }
            fn is_valid(&self, _: &mut dyn ServerHost, _: usize, stack: &ItemResource) -> bool {
                mod_api::__private::item_key_id(stack.as_stack().item()) == Some("minecraft:stone")
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(0)
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(0)
            }
        }
        struct ReadOnly;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ReadOnly {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.item_capability_before_game(
                    &mod_api::ITEM_ITEMS,
                    "minecraft:shulker_box",
                    |_, _| Some(Box::new(Handler)),
                );
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("readonly_pack", Box::new(ReadOnly))].into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_base_game(|game| {
                let menu = backpack_menu(game);
                assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..2)), Ok(()));
                let stone = menu.slot(game, 0).unwrap().unwrap();
                let dirt = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:dirt").unwrap(),
                    1,
                )
                .unwrap();
                let read: &dyn mod_api::Host = game;
                assert!(matches!(
                    menu.slot_backing(read, 0),
                    Some(MenuSlotBacking::Place(_))
                ));
                assert_eq!(menu.slot_holds(read, 0, &stone), None);
                assert_eq!(menu.slot_gives(read, 0), None);
                assert_eq!(menu.probe_slot_holds(game, 0, &stone), Ok(Some(7)));
                assert_eq!(menu.probe_slot_holds(game, 0, &dirt), Ok(Some(0)));
                assert_eq!(
                    menu.probe_slot_holds(game, 1, &stone),
                    Ok(Some(i32::MAX as u32)),
                    "ResourceHandlerSlot.getMaxStackSize clamps capacity to i32::MAX"
                );
                assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));
                // Hotbar zero backs the open menu, so its own inventory slot takes no move.
                let holding_slot = game
                    .native_menu_guest(menu)
                    .unwrap()
                    .menu
                    .as_ref()
                    .unwrap()
                    .size
                    + 27;
                assert_eq!(menu.slot_holds(game, holding_slot, &stone), Some(0));
                assert_eq!(menu.slot_gives(game, holding_slot), Some(false));
                assert_eq!(
                    menu.probe_slot_holds(game, holding_slot, &stone),
                    Ok(Some(0))
                );
                assert_eq!(menu.probe_slot_gives(game, holding_slot), Ok(Some(false)));
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn backed_clicks_enforce_handler_pickup_probe_before_taking_or_quick_moving() {
        use mod_api::{
            ItemResource, Overanswered, ResourceHandler, TransactionContext, TransactionError,
        };
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };
        struct BulkOnlyHandler(Arc<AtomicU32>);
        impl ResourceHandler<ItemResource> for BulkOnlyHandler {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                (self.0.load(Ordering::Relaxed) > 0).then(|| {
                    ItemResource::of(
                        &mod_api::ItemStack::new(
                            mod_api::__private::item_key("minecraft:stone").unwrap(),
                            1,
                        )
                        .unwrap(),
                    )
                })
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                self.0.load(Ordering::Relaxed)
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: Option<&ItemResource>) -> u32 {
                64
            }
            fn is_valid(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> bool {
                true
            }
            fn insert_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                Ok(amount)
            }
            fn extract_at(
                &self,
                _: &mut dyn ServerHost,
                _: &TransactionContext,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> Result<u32, TransactionError<Overanswered>> {
                // Refuse extracting 1 so ResourceHandlerSlot.mayPickup is false,
                // while still allowing larger explicit extracts.
                if amount <= 1 {
                    return Ok(0);
                }
                let held = self.0.load(Ordering::Relaxed);
                let taken = held.min(amount);
                self.0.store(held - taken, Ordering::Relaxed);
                Ok(taken)
            }
        }
        struct BulkPack(Arc<AtomicU32>);
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for BulkPack {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                let count = Arc::clone(&self.0);
                r.item_capability_before_game(
                    &mod_api::ITEM_ITEMS,
                    "minecraft:shulker_box",
                    move |_, _| Some(Box::new(BulkOnlyHandler(Arc::clone(&count)))),
                );
            }
        }
        let stored = Arc::new(AtomicU32::new(4));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "bulk_pack",
            Box::new(BulkPack(Arc::clone(&stored))),
        )]
        .into();
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_base_game(|game| {
                let menu = backpack_menu(game);
                let window = menu.container_id(game).unwrap();
                assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..1)), Ok(()));
                assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));

                // Empty-cursor pickup click refuses when mayPickup is false.
                assert_eq!(game.place_menu_click(123, window, 0, 0, 0), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);
                assert_eq!(menu.cursor().with_stack(game, |_| ()), None);

                // Quick-move click refuses when mayPickup is false.
                assert_eq!(game.place_menu_click(123, window, 0, 0, 1), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);

                // Non-empty cursor swap click refuses when mayPickup is false.
                edit_guest(game, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:dirt", 1));
                });
                assert_eq!(game.place_menu_click(123, window, 0, 0, 0), Some(true));
                assert_eq!(stored.load(Ordering::Relaxed), 4);
                let carried_id = menu
                    .cursor()
                    .resource(game)
                    .and_then(|r| mod_api::__private::item_key_id(r.as_stack().item()));
                assert_eq!(carried_id, Some("minecraft:dirt"));

                // Explicit ItemPlace::MenuSlot transfer still reaches the handler directly.
                let slot = ItemPlace::MenuSlot { menu, slot: 0 };
                let stone = slot.resource(game).unwrap();
                let host: &mut dyn ServerHost = game;
                let moved =
                    host.transaction(|host, tx| Ok::<_, ()>(slot.extract(host, tx, &stone, 4)));
                assert_eq!(moved, Ok(Ok(4)));
                assert_eq!(stored.load(Ordering::Relaxed), 0);
            })
        });
    }

    /// A held box's menu, opened from code at the mod block whose container
    /// opens its type, with backing slot 0 display-only under the menu rules.
    struct Satchel;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Satchel {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let item = r.item_key("minecraft:shulker_box");
            r.item_container(&item, |_: &dyn mod_api::Host| Some(27));
            let block = r.block_key("satchel:stand");
            r.block_container(
                &block,
                mod_api::ModContainer {
                    slots: 9,
                    menu: Some(SATCHEL_MENU.into()),
                    ..Default::default()
                },
            );
            r.menu_part(SATCHEL_MENU, SatchelSlotRules);
        }
        fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
            event
                .register(mod_api::Block::new("satchel:stand", "minecraft:barrel").with_item(false))
                .unwrap();
        }
    }
    struct SatchelSlotRules;
    #[mod_api::__subscriptions]
    impl mod_api::MenuPart for SatchelSlotRules {
        fn menu_slot_rules(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::MenuSlotRulesEvent<'_>,
        ) {
            if event.container == MenuContainer::Main && event.slot == 0 {
                event.may_place = false;
                event.may_pickup = false;
            }
        }
    }

    /// The content mod menu type the satchel opens: a prepared registry slot for a synthetic
    /// definition, as `native_menus`' tests use.
    const SATCHEL_MENU: &str = "satchel:menu";

    /// #1172: a menu `player_open_mod_menu` opens keeps its handle and backing, and a
    /// shift-click on the server passes over the slot its menu declares display-only.
    /// Neither pickup nor shift-click can move an item out of the display-only slot.
    #[test]
    #[ignore = "daily"]
    fn a_code_opened_backed_menu_respects_its_display_only_slot() {
        // Content registries are process-global.
        if super::super::native_entity_refs::tests::run_fresh_cases(
            "cut_layout::game::server_side::native_menu_places::tests::a_code_opened_backed_menu_respects_its_display_only_slot",
            "SUNRISE_MENU_PLACES_HIDDEN_CHILD",
            &["hidden"],
        ) {
            return;
        }
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("satchel", Box::new(Satchel))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_api::__private::mark_game_thread();
            let base =
                std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
            let root =
                sunrise_gameplay_common::game::mods::content_root(&base, &Default::default())
                    .unwrap();
            sunrise_level::containers::with_mod_menu_types(
                [(
                    SATCHEL_MENU.into(),
                    "satchel".into(),
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
                    super::super::native_entity_refs::tests::with_game_role(
                        root.prepared().to_owned(),
                        sunrise_gameplay_common::game::execution_role::ExecutionRole::IntegratedServer,
                        open_back_and_shift_click,
                    )
                },
            );
        });
    }

    fn open_back_and_shift_click(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
        let pos = [1, 64, 2];
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
        );
        let stand = game
            .pack
            .assets
            .registry
            .default_state("satchel:stand")
            .unwrap();
        game.world.set(pos[0], pos[1], pos[2], stand).unwrap();
        open_to_lan(game);
        let mut player = guest(1);
        player.selected = 0;
        player.brand = "sunrise".into();
        player.sender.menu_extra_rev = 1;
        player.inventory[0] = Some(ItemStack::new("minecraft:shulker_box", 1));
        player.inventory[9] = Some(ItemStack::new("minecraft:stone", 64));
        game.server.lan.as_mut().unwrap().hub.guests.push(player);
        let key = super::super::native_guest_turn_tests::register_test_menu(game, SATCHEL_MENU);
        let player = game.player(123).unwrap();
        let menu = game
            .player_open_mod_menu(player, {
                let mut request = mod_api::ModMenuOpen::new(key, mod_api::Text::literal("satchel"));
                request.pos = Some(pos);
                request
            })
            .unwrap()
            .unwrap();
        let window = menu.container_id(game).unwrap();
        // The handle and the backing stay usable.
        assert_eq!(menu.back_slots(game, held_places(Hand::Main, 0..9)), Ok(()));
        // Inventory slot 9 is menu slot 9: the stone passes over display-only slot 0.
        assert_eq!(game.place_menu_click(123, window, 9, 0, 1), Some(true));
        let host: &mut dyn ServerHost = game;
        assert_eq!(held_slot(0).amount(host), 0);
        assert_eq!(held_slot(1).amount(host), 64);
        assert_eq!(ItemPlace::MenuSlot { menu, slot: 1 }.amount(host), 64);

        // The menu slot obeys its rules even when addressed through a handle.
        // The storage itself can still be stocked directly for display.
        let hidden = ItemPlace::MenuSlot { menu, slot: 0 };
        let stone = ItemPlace::MenuSlot { menu, slot: 1 }
            .resource(host)
            .unwrap();
        assert_eq!(
            host.transaction(|host, tx| hidden.insert(host, tx, &stone, 5)),
            Ok(0)
        );
        assert_eq!(
            host.transaction(|host, tx| held_slot(0).insert(host, tx, &stone, 5)),
            Ok(5)
        );
        assert_eq!(hidden.amount(host), 5);
        assert_eq!(
            host.transaction(|host, tx| hidden.extract(host, tx, &stone, 5)),
            Ok(0)
        );
        // The probes answer what a click would: the handler permits both, the menu's rules veto.
        let stone_stack =
            mod_api::ItemStack::new(mod_api::__private::item_key("minecraft:stone").unwrap(), 1)
                .unwrap();
        assert_eq!(menu.probe_slot_holds(game, 0, &stone_stack), Ok(Some(0)));
        assert_eq!(menu.probe_slot_gives(game, 0), Ok(Some(false)));
        let contents = |game: &mut sunrise_gameplay::game::ServerGame<'_>| {
            (0..45)
                .map(|slot| {
                    ItemPlace::MenuSlot { menu, slot }
                        .with_stack(game, |stack| stack.project())
                        .flatten()
                })
                .collect::<Vec<_>>()
        };
        let before = contents(game);
        for mode in [0, 1] {
            assert_eq!(game.place_menu_click(123, window, 0, 0, mode), Some(true));
            assert_eq!(contents(game), before);
            assert_eq!(menu.cursor().with_stack(game, |_| ()), None);
            assert_eq!(held_slot(0).amount(game), 5);
        }
    }
}
