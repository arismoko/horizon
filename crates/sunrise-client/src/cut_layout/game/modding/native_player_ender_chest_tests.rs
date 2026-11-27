//! Reading and writing ender chest slots against real LAN guests through the public player handle.

use super::native_guest_turn_tests::with_guests;
use mod_api::{Host, InventoryView, ItemStack, PlayerDataRefusal, StackWriteError, prelude::*};

fn api_item(game: &sunrise_gameplay::game::ServerGame<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

fn contents(view: InventoryView<'_>) -> Vec<(u32, ItemStack)> {
    view.iter()
        .map(|(slot, stack)| (slot, stack.project().unwrap()))
        .collect()
}

const BOOK: &str = r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#;

/// Ender chest slots are written and read back by number, out of the player's turn and in
/// it, components and all, and each player has their own.
#[test]
#[ignore = "daily"]
fn ender_chest_slots_are_read_and_written() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let book = api_item(game, BOOK).unwrap();
        let pearls = api_item(game, r#"{id:"minecraft:ender_pearl",count:16}"#).unwrap();
        assert_eq!(
            b.with_ender_chest(game, |view| view.iter().next().is_none()),
            Some(true)
        );
        assert_eq!(b.set_ender_chest_slot(game, 26, Some(book.clone())), Ok(()));
        game.as_guest(12, |game| {
            assert_eq!(
                a.set_ender_chest_slot(game, 0, Some(pearls.clone())),
                Ok(())
            );
            assert_eq!(
                a.with_ender_chest(game, contents),
                Some(vec![(0, pearls.clone())])
            );
            assert_eq!(
                b.with_ender_chest(game, contents),
                Some(vec![(26, book.clone())])
            );

            game.open_ender_chest((1, 64, 1));
            game.server.ender_items[0] = None; // Only the open copy is authoritative now.
            assert_eq!(
                a.with_ender_chest(game, contents),
                Some(vec![(0, pearls.clone())])
            );
            game.close_ender_chest();
        })
        .unwrap();
        assert_eq!(b.with_ender_chest(game, contents), Some(vec![(26, book)]));
        assert_eq!(a.with_ender_chest(game, contents), Some(vec![(0, pearls)]));
        assert_eq!(b.set_ender_chest_slot(game, 26, None), Ok(()));
        assert_eq!(
            b.with_ender_chest(game, |view| view.iter().next().is_none()),
            Some(true)
        );
    });
}

/// Each refusal has its own name and writes nothing: a slot past 26 (never a panic), a stack
/// the item does not allow, and a handle whose player has left.
#[test]
#[ignore = "daily"]
fn each_ender_chest_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let book = api_item(game, BOOK).unwrap();
        assert_eq!(
            b.set_ender_chest_slot(game, 27, Some(book.clone())),
            Err(PlayerDataRefusal::SlotOutOfRange { slot: 27, last: 26 })
        );
        assert_eq!(
            b.set_ender_chest_slot(game, u32::MAX, None),
            Err(PlayerDataRefusal::SlotOutOfRange {
                slot: u32::MAX,
                last: 26
            })
        );
        let mut too_many = api_item(game, r#"{id:"minecraft:diamond_sword"}"#).unwrap();
        too_many.set_count(2).unwrap();
        assert_eq!(
            b.set_ender_chest_slot(game, 0, Some(too_many)),
            Err(PlayerDataRefusal::Stack(StackWriteError::CountExceeded {
                maximum: 1
            }))
        );
        assert_eq!(
            b.with_ender_chest(game, |view| view.iter().next().is_none()),
            Some(true)
        );
        game.server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| {
                g.dead = true;
                g.health = 0.0;
            });
        assert_eq!(b.set_ender_chest_slot(game, 0, Some(book.clone())), Ok(()));
        assert_eq!(
            b.with_ender_chest(game, |view| view.get(0).map(|s| s.count())),
            Some(Some(1))
        );
        let index = game
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .connection_index(13)
            .unwrap();
        game.server.lan.as_mut().unwrap().hub.guests.remove(index);
        assert_eq!(
            b.set_ender_chest_slot(game, 0, Some(book)),
            Err(PlayerDataRefusal::Stale)
        );
        assert_eq!(b.with_ender_chest(game, |_| ()), None);
    });
}

/// A stack written to an ender chest slot the player's open ender chest menu shows replaces
/// the old one in that menu slot, and clicking the slot picks up the new stack.
#[test]
#[ignore = "daily"]
fn a_written_slot_is_what_the_open_ender_chest_menu_shows_and_clicks() {
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound};
    use sunrise_sim::lan::menu::Slots;
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let old = api_item(game, r#"{id:"minecraft:ender_pearl",count:5}"#).unwrap();
        let new = api_item(game, BOOK).unwrap();
        let hub = &mut game.server.lan.as_mut().unwrap().hub;
        hub.open_container(
            13,
            "minecraft:generic_9x3",
            sunrise_core::nbt::Tag::String("test".into()),
            (1, 64, 1),
            Slots::Ender,
            vec![None; 27],
            None,
        );
        let window = hub.guest(13).unwrap().menu.as_ref().unwrap().window;
        assert_eq!(b.set_ender_chest_slot(game, 3, Some(old)), Ok(()));
        assert_eq!(b.set_ender_chest_slot(game, 3, Some(new)), Ok(()));
        let hub = &game.server.lan.as_ref().unwrap().hub;
        let guest = hub.guest(13).unwrap();
        let shown = Slots::Ender.get(&game.world, guest, 3).unwrap();
        assert_eq!(shown.item.as_ref(), "minecraft:written_book");
        assert_eq!(
            b.with_ender_chest(game, |view| {
                let stack = view.get(3).unwrap();
                stack.item()
            }),
            mod_api::__private::item_key("minecraft:written_book")
        );
        game.lan_guest_packet(
            13,
            Serverbound::ContainerClick(ContainerClick {
                container: window,
                state: 0,
                slot: 3,
                button: 0,
                input: 0,
                changed: Vec::new(),
                carried: None,
            }),
        );
        let guest = game.server.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(
            guest.carried,
            Some(shown),
            "the click picked up the written stack"
        );
        assert_eq!(guest.ender_items[3], None, "the old stack is gone");
    });
}

/// The local player's open ender chest menu holds a copy that closing writes back over the
/// chest: a write made while it is open goes into that copy and survives the close.
#[test]
#[ignore = "daily"]
fn a_write_while_the_local_ender_chest_is_open_survives_closing_it() {
    use sunrise_level::containers::BlockEntity;
    with_guests(|game| {
        let key = (1, 64, 1);
        let old = sunrise_items::items::ItemStack::new("minecraft:ender_pearl", 5);
        let new = sunrise_items::items::ItemStack::new("minecraft:diamond", 2);
        game.server.ender_items = vec![None; 27];
        game.server.ender_items[3] = Some(old);
        game.open_ender_chest(key);
        assert!(game.slot_set(
            &sunrise_gameplay::game::commands::items::Slot::Ender(3),
            Some(new.clone())
        ));
        let Some(BlockEntity::Chest(open)) = game.world.block_entities.get(&key) else {
            panic!("the open copy")
        };
        assert_eq!(
            open[3],
            Some(new.clone()),
            "the menu shows the written stack"
        );
        game.close_ender_chest();
        assert_eq!(game.server.ender_items[3], Some(new), "closing keeps it");
    });
}

/// A mod opens the player's ender chest from their own item use, as a satchel would: the
/// menu opens as the use ends, over their own ender chest with no block, and taking,
/// placing and shift-clicking move stacks between it and the inventory, none lost or copied.
#[test]
#[ignore = "daily"]
fn an_item_use_opens_the_players_own_ender_chest_and_its_clicks_keep_every_stack() {
    use mod_api::{
        Flow, Hooks, HostSide, InteractionResult, ModEntry, Mods, OpenMenuError,
        RightClickItemEvent, Text,
    };
    use std::sync::{Arc, Mutex};
    use sunrise_items::items::ItemStack as Stack;
    use sunrise_protocol::packets::play::{ContainerClick, Serverbound, UseItem};
    use sunrise_sim::lan::menu::Slots;
    struct Satchel(Arc<Mutex<Vec<Result<bool, OpenMenuError>>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Satchel {
        fn right_click_item(
            &self,
            host: HostSide<'_>,
            event: &mut RightClickItemEvent<'_>,
        ) -> Flow {
            let HostSide::Server(host) = host else {
                return Flow::Continue;
            };
            let player = host.player(event.click.player).unwrap();
            let opened = player.open_ender_chest(host, Text::literal("Void Satchel"));
            self.0.lock().unwrap().push(opened);
            *event.cancellation_result = InteractionResult::SuccessServer;
            Flow::Cancel
        }
    }
    let answers = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new("satchel", Box::new(Satchel(answers.clone())))].into();
    sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            // The outer state the record holds in the guest's turn: the window's own chest.
            game.server.ender_items = vec![Some(Stack::new("minecraft:dirt", 3)); 27];
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.guests.update_connection(12, |g| {
                g.inventory.fill(None);
                g.inventory[g.selected] = Some(Stack::new("minecraft:stick", 1));
                g.ender_items = vec![None; 27];
                g.ender_items[0] = Some(Stack::new("minecraft:stone", 10));
                g.ender_items[4] = Some(Stack::new("minecraft:diamond", 2));
            });
            game.lan_guest_packet(
                12,
                Serverbound::UseItem(UseItem {
                    hand: 0,
                    yaw: 0.0,
                    pitch: 0.0,
                    sequence: 1,
                }),
            );
            assert_eq!(*answers.lock().unwrap(), vec![Ok(true)]);
            let open = |game: &sunrise_gameplay::game::ServerGame<'_>| {
                let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                let menu = guest.menu.as_ref().expect("the ender chest is open");
                (
                    menu.window,
                    menu.registry_id.to_string(),
                    menu.pos,
                    menu.slots,
                )
            };
            let (window, kind, pos, slots) = open(game);
            assert_eq!(kind, "minecraft:generic_9x3");
            assert_eq!((pos, slots), (None, Slots::Ender));
            // The client was shown the guest's own chest, not the one their record held in the turn.
            let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
            let shown = &guest.menu.as_ref().unwrap().sent;
            assert_eq!(shown[0], Some(Stack::new("minecraft:stone", 10)));
            assert_eq!(shown[1], None);
            let count = |game: &sunrise_gameplay::game::ServerGame<'_>, item: &str| {
                let g = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
                g.ender_items
                    .iter()
                    .chain(&g.inventory)
                    .chain(std::iter::once(&g.carried))
                    .flatten()
                    .filter(|s| s.item.as_ref() == item)
                    .map(|s| s.count)
                    .sum::<u32>()
            };
            let click =
                |game: &mut sunrise_gameplay::game::ServerGame<'_>, slot: i16, input: i32| {
                    game.lan_guest_packet(
                        12,
                        Serverbound::ContainerClick(ContainerClick {
                            container: window,
                            state: 0,
                            slot,
                            button: 0,
                            input,
                            changed: Vec::new(),
                            carried: None,
                        }),
                    );
                    assert_eq!(count(game, "minecraft:stone"), 10);
                    assert_eq!(count(game, "minecraft:diamond"), 2);
                    assert_eq!(count(game, "minecraft:dirt"), 0);
                };
            // Shift-click the stone into the inventory, take the diamonds, put them in slot 9.
            click(game, 0, 1);
            click(game, 4, 0);
            click(game, 9, 0);
            let guest = game.server.lan.as_ref().unwrap().hub.guest(12).unwrap();
            assert_eq!(guest.ender_items[0], None);
            assert_eq!(guest.ender_items[4], None);
            assert_eq!(
                guest.ender_items[9],
                Some(Stack::new("minecraft:diamond", 2))
            );
            assert_eq!(guest.carried, None);
            assert!(
                guest
                    .inventory
                    .contains(&Some(Stack::new("minecraft:stone", 10)))
            );
            // The window's own chest was never shown nor touched.
            assert_eq!(
                game.server.ender_items[0],
                Some(Stack::new("minecraft:dirt", 3))
            );
            assert_eq!(open(game).0, window);
        });
    });
}
