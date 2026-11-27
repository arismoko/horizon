#![cfg(feature = "client")]
use std::sync::Arc;
use sunrise_gameplay::game::remote::recipe_properties::tests::cut_pack;
use sunrise_gameplay::game::remote::recipe_properties::tests::place_stonecutter;
use sunrise_items::items::ItemStack;
use sunrise_mod_package::content::RECIPE_PRIORITIES_PATH;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_protocol::{
    buf::Reader,
    packets::recipes::{HolderSet, SlotDisplay, UpdateRecipes},
};
use sunrise_sim::lan::{Out, PlayerSender};
use {
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    sunrise_gameplay_common::game::lan_actions::GuestCommand,
};

#[test]
#[ignore = "daily"]
fn guest_stonecutter_uses_packet_tags_components_and_authoritative_buttons() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let write = cut_pack(root);
    write(
        RECIPE_PRIORITIES_PATH,
        r#"{"entries":{"fixture:z_tag":10}}"#,
    );
    write(
        "data/fixture/tags/item/cuttable.json",
        r#"{"values":["minecraft:stick"]}"#,
    );
    write(
        "data/fixture/recipe/z_tag.json",
        r##"{
        "type":"minecraft:stonecutting","ingredient":"#fixture:cuttable",
        "result":{"id":"minecraft:leather_helmet","components":{"minecraft:dyed_color":1122867}}
    }"##,
    );
    write(
        "data/fixture/recipe/a_named.json",
        r#"{
        "type":"minecraft:stonecutting",
        "ingredient":{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_name":"special"}},
        "result":{"id":"minecraft:diamond"}
    }"#,
    );
    write(
        "data/fixture/recipe/b_plain.json",
        r#"{
        "type":"minecraft:stonecutting","ingredient":"minecraft:stick",
        "result":{"id":"minecraft:emerald","count":2}
    }"#,
    );
    with_guests(|server| {
        let _serving =
            sunrise_gameplay::game::server_side::integrated::tests::install_serving(server, false);
        server.server.world_root = root.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        let host = server.server.lan.as_mut().unwrap();
        host.play.entry(12).or_default();
        host.hub.update_guest(12, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default());
        });
        server.reload_datapacks(true, &[]).unwrap();
        let station = place_stonecutter(server);
        assert!(server.lan_open_station(12, &station));
        let window = server.server.lan.as_ref().unwrap().menus[&12]
            .station
            .as_ref()
            .unwrap()
            .window;
        let click = |slot, input| {
            S::ContainerClick(sunrise_protocol::packets::play::ContainerClick {
                state: 0,
                changed: Vec::new(),
                carried: None,
                container: window,
                slot,
                button: 0,
                input,
            })
        };
        // Opening the station returns anything already on the cursor to inventory.
        server
            .server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(12, |guest| {
                guest.carried = Some(ItemStack::new("minecraft:stick", 3));
            });
        server.lan_guest_packet(12, click(0, 0));
        with_client_game(|client| {
            let (mut peer, _writer) = super::super::tests::connect_menu_test(client);
            let deliver = |client: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                           expect_closed: bool| {
                let mut recipes = None;
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("guest disconnected"),
                    };
                    for body in bodies {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        match C::decode(id, reader.rest()).unwrap() {
                            packet @ (C::UpdateTags(_)
                            | C::OpenScreen(_)
                            | C::ContainerClose(_)
                            | C::ContainerSetContent(_)
                            | C::ContainerSetData(_)) => client.handle_packet(packet).unwrap(),
                            C::UpdateRecipes(packet) => {
                                if expect_closed {
                                    assert!(client.station.is_none(), "close before new recipes");
                                }
                                recipes = Some(packet.clone());
                                client.handle_packet(C::UpdateRecipes(packet)).unwrap();
                            }
                            _ => {}
                        }
                    }
                }
                recipes
            };
            let packet = deliver(client, false).expect("server recipe packet");
            let input = client.station.as_ref().unwrap().inputs[0]
                .as_ref()
                .expect("the guest placed its stack");
            assert_eq!((input.item.as_ref(), input.count), ("minecraft:stick", 3));
            assert!(
                client.client.remote.as_ref().unwrap().tags.is_none(),
                "no native tag view"
            );
            let shown = |client: &sunrise_client_gameplay::game::ClientGame<'_>| {
                client.station_info()["recipe_results"].clone()
            };
            assert_eq!(
                shown(client),
                serde_json::json!([
                    "minecraft:leather_helmet",
                    "minecraft:diamond",
                    "minecraft:emerald"
                ])
            );
            let first = client.station.as_ref().unwrap().stone.recipes[0];
            let preview = &client.station_recipes().stonecutting[first].result;
            assert_eq!(
                preview.data().and_then(|data| data.dyed_color),
                Some(0x112233)
            );

            let mut press = |client: &mut sunrise_client_gameplay::game::ClientGame<'_>,
                             server: &mut sunrise_gameplay::game::ServerGame<'_>,
                             button| {
                assert!(client.menu_session(|menu, env, _| Some(menu.button(env, button))));
                assert!(client.remote_button_click(button));
                let (id, payload) = peer.recv().unwrap();
                let packet = S::decode(id, &payload).unwrap();
                assert!(
                    matches!(&packet, S::ContainerButtonClick(p) if p.container == window && p.button == button)
                );
                server.lan_guest_packet(12, packet);
                deliver(client, false);
            };
            press(client, server, 1); // The packet's item matches, but the actual component does not.
            assert!(client.station.as_ref().unwrap().result.is_none());
            assert_eq!(client.station.as_ref().unwrap().selected, None);
            press(client, server, 2); // The ineligible row must not shift this one's number.
            assert_eq!(client.station.as_ref().unwrap().selected, Some(2));
            assert_eq!(
                client
                    .station
                    .as_ref()
                    .unwrap()
                    .result
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:emerald"
            );
            server.lan_guest_packet(12, click(1, 1)); // Quick-move all three cuts into inventory.
            deliver(client, false);
            assert_eq!(
                server.server.lan.as_ref().unwrap().hub.guests[0]
                    .inventory
                    .iter()
                    .flatten()
                    .filter(|s| s.item.as_ref() == "minecraft:emerald")
                    .map(|s| s.count)
                    .sum::<u32>(),
                6
            );

            server
                .server
                .lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.carried = Some(ItemStack::new("minecraft:stick", 1))
                });
            server.lan_guest_packet(12, click(0, 0));
            deliver(client, false);
            press(client, server, 2);
            // Other servers can replace UpdateRecipes before the screen closes. Supply a single
            // different row without changing the input; the cached old indices cannot survive.
            let replaced = UpdateRecipes {
                stonecutter: vec![packet.stonecutter[first].clone()],
                ..packet
            };
            client.handle_packet(C::UpdateRecipes(replaced)).unwrap();
            assert_eq!(
                shown(client),
                serde_json::json!(["minecraft:leather_helmet"])
            );
            assert_eq!(
                client.station.as_ref().unwrap().inputs[0]
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:stick"
            );
            assert!(client.station_info()["selected"].is_null());
            assert!(client.station.as_ref().unwrap().result.is_none());

            // A Sunrise reload uses the vanilla close packet before the new recipe generation.
            write(
                RECIPE_PRIORITIES_PATH,
                r#"{"entries":{"fixture:b_plain":10}}"#,
            );
            // Run the operator's real command handler, with their inventory and cursor
            // swapped into Game. Neither the outer player nor the other guest owns the input.
            server.inventory[19] = Some(ItemStack::new("minecraft:gold_ingot", 4));
            server.carried = Some(ItemStack::new("minecraft:apple", 2));
            let outer = (server.inventory.clone(), server.carried.clone());
            let hub = &mut server.server.lan.as_mut().unwrap().hub;
            hub.update_guest(12, |g| {
                g.carried = Some(ItemStack::new("minecraft:iron_ingot", 3));
            });
            hub.update_guest(13, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:bread", 5));
                g.carried = Some(ItemStack::new("minecraft:carrot", 6));
            });
            let other = hub.guest(13).unwrap();
            let other = (other.inventory.clone(), other.carried.clone());
            assert!(server.run_guest_command(GuestCommand {
                conn: 12,
                command: "reload".into(),
                permission: 4,
                integrated: true,
                quiet: false,
            }));
            let hub = &server.server.lan.as_ref().unwrap().hub;
            let operator = hub.guest(12).unwrap();
            assert_eq!(operator.carried, None);
            for (item, count) in [("minecraft:stick", 1), ("minecraft:iron_ingot", 3)] {
                assert_eq!(
                    operator
                        .inventory
                        .iter()
                        .flatten()
                        .filter(|stack| stack.item.as_ref() == item)
                        .map(|stack| stack.count)
                        .sum::<u32>(),
                    count,
                    "operator did not get back {item}"
                );
            }
            assert_eq!((&server.inventory, &server.carried), (&outer.0, &outer.1));
            let other_guest = hub.guest(13).unwrap();
            assert_eq!(
                (&other_guest.inventory, &other_guest.carried),
                (&other.0, &other.1)
            );
            deliver(client, true).expect("new server recipe generation");
            assert!(client.station.is_none());
            server.lan_guest_packet(
                12,
                S::ContainerButtonClick(sunrise_protocol::packets::play::ContainerButtonClick {
                    container: window,
                    button: 0,
                }),
            );
            assert!(
                server.server.lan.as_ref().unwrap().menus[&12]
                    .station
                    .is_none()
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn guest_stonecutter_waits_for_deferred_recipe_publication() {
    let _publication = sunrise_sim::datapack_net::isolate_server_publication();
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path();
    let write = cut_pack(root);
    for (name, result) in [("a", "minecraft:diamond"), ("b", "minecraft:emerald")] {
        write(
            &format!("data/fixture/recipe/{name}.json"),
            &format!(
                r#"{{"type":"minecraft:stonecutting","ingredient":"minecraft:stick","result":{{"id":"{result}"}}}}"#
            ),
        );
    }
    with_guests(|server| {
        let _serving =
            sunrise_gameplay::game::server_side::integrated::tests::install_serving(server, false);
        server.server.world_root = root.to_owned();
        let (tx, rx) = std::sync::mpsc::channel();
        let host = server.server.lan.as_mut().unwrap();
        host.play.entry(12).or_default();
        host.hub.update_guest(12, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default());
        });
        server.reload_datapacks(true, &[]).unwrap();
        let station = place_stonecutter(server);
        assert!(server.lan_open_station(12, &station));
        let menu = |server: &sunrise_gameplay::game::ServerGame<'_>| {
            server.server.lan.as_ref().unwrap().menus[&12]
                .station
                .as_ref()
                .map(|station| {
                    (
                        station.window,
                        station.menu.stone.server_buttons.clone().unwrap(),
                    )
                })
        };
        let buttons = |map: &sunrise_sim::menu::StonecutterButtons| {
            [0, 1].map(|i| map.recipe("minecraft:stick", i).unwrap().to_owned())
        };
        let (old_window, old_buttons) = menu(server).unwrap();
        assert_eq!(buttons(&old_buttons), ["fixture:a", "fixture:b"]);
        let packets = || {
            rx.try_iter()
                .flat_map(|out| match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(_) => panic!("guest disconnected"),
                })
                .filter_map(|body| {
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    let packet = C::decode(id, reader.rest()).unwrap();
                    matches!(
                        packet,
                        C::ContainerClose(_) | C::UpdateRecipes(_) | C::OpenScreen(_)
                    )
                    .then_some(packet)
                })
                .collect::<Vec<_>>()
        };
        let _ = packets();
        write(RECIPE_PRIORITIES_PATH, r#"{"entries":{"fixture:b":10}}"#);
        let resources = server.options.resources.clone();
        let stack = sunrise_gameplay::game::functions::pack_stack(
            root,
            &resources,
            &server.pack.data_source,
            &server.pack.data_changes,
            false,
            &[],
            &[],
        );
        let prepared = server.prepare_world_reload(&stack, &resources).unwrap();
        // Simulate the mod-loaded hub's two real phases without running the second yet.
        server.server.lan.as_mut().unwrap().hub.mods_loaded = true;
        server.install_world_tables(prepared);
        let hub = &server.server.lan.as_ref().unwrap().hub;
        assert!(hub.native_reload_pending);
        assert_eq!(
            buttons(&hub.sync_data().stonecutter_buttons),
            ["fixture:b", "fixture:a"]
        );
        assert!(server.lan_open_station(12, &station));
        assert!(menu(server).is_none(), "opened with unpublished buttons");
        assert!(matches!(
            packets().as_slice(),
            [C::ContainerClose(close)] if close.container == old_window
        ));

        server.run_native_advancement_grants();
        assert!(server.lan_open_station(12, &station));
        let (new_window, new_buttons) = menu(server).unwrap();
        assert_ne!(new_window, old_window);
        assert_eq!(buttons(&new_buttons), ["fixture:b", "fixture:a"]);
        let sent = packets();
        let [C::UpdateRecipes(recipes), C::OpenScreen(open)] = sent.as_slice() else {
            panic!("expected UpdateRecipes before OpenScreen");
        };
        assert_eq!(open.container, new_window);
        let items = server
            .server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .sync_data()
            .builtin
            .get("minecraft:item")
            .unwrap();
        let stick = items.id("minecraft:stick").unwrap();
        let shown: Vec<_> = recipes
            .stonecutter
            .iter()
            .filter_map(|(holder, display)| match (holder, display) {
                (HolderSet::Items(ids), SlotDisplay::ItemStack(stack)) if ids == &[stick] => {
                    Some(items.name(stack.item).unwrap())
                }
                _ => None,
            })
            .collect();
        assert_eq!(shown, ["minecraft:emerald", "minecraft:diamond"]);
    });
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
