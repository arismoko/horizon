use super::*;

/// `ItemFrame.interact`, `AbstractBoat.interact`, and `Minecart.interact`:
/// interactions that pass leave the click to the held item without swinging;
/// vehicle boarding answers CONSUME and ends the click without a predicted swing (#9120).
#[test]
fn object_interactions_pass_and_swing_vanilla() {
    // ItemFrame: fixed frame or empty frame + empty hand passes.
    assert!(item_frame_interact_passes(true, true, true));
    assert!(item_frame_interact_passes(true, false, false));
    assert!(item_frame_interact_passes(false, false, false));
    // ItemFrame: inserting or rotating item does not pass.
    assert!(!item_frame_interact_passes(false, true, false)); // rotates
    assert!(!item_frame_interact_passes(false, false, true)); // inserts
    assert!(!item_frame_interact_passes(false, true, true)); // rotates

    // Boat: sneaking or out of control passes.
    assert!(boat_interact_passes(false, true, false, true)); // sneaking
    assert!(boat_interact_passes(false, false, true, true)); // out of control
    // Boat: the client answers SUCCESS, room or not (startRiding is the server's).
    assert!(!boat_interact_passes(false, false, false, true));
    assert!(!boat_interact_passes(false, false, false, false));

    // Chest boat: sneaking opens the chest; boarding answers SUCCESS.
    assert!(!boat_interact_passes(true, true, false, true)); // opens chest
    assert!(!boat_interact_passes(true, false, false, false)); // boards (client)
    // Chest boat out of control: passes with room, else opens the chest.
    assert!(boat_interact_passes(true, false, true, true));
    assert!(!boat_interact_passes(true, false, true, false));

    // Swings: boarding a rideable cart answers CONSUME (no swing).
    assert!(!remote_object_swings(None, true));
    // Swings: other object interactions, a boat's boarding too, answer SUCCESS.
    assert!(remote_object_swings(None, false));
    // Interaction entities honor response.
    assert!(remote_object_swings(Some(true), false));
    assert!(!remote_object_swings(Some(false), false));
}

#[test]
#[ignore = "daily"]
fn transactional_bucket_hands_reach_the_owning_client() {
    use mod_api::{
        HostTransaction, ITEM_FLUIDS, ItemPlace, ServerHost, resource_util::move_resources,
    };
    use sunrise_items::items::ItemStack;
    use sunrise_protocol::{buf::Reader, packets::play::Clientbound as C};
    use sunrise_sim::lan::{Out, PlayerSender};
    use {
        crate::game::server_side::native_entity_refs::tests::with_client_game,
        sunrise_gameplay::game::native_guest_turn_tests::with_guests,
    };

    with_guests(|server| {
        let (tx, rx) = std::sync::mpsc::channel();
        server
            .server
            .lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(0, |guest| {
                guest.sender = PlayerSender::new(tx, std::sync::Arc::default());
                guest.selected = 2;
                guest.inventory[2] = Some(ItemStack::new("minecraft:bucket", 1));
                guest.offhand = Some(ItemStack::new("minecraft:water_bucket", 1));
            });
        let main = ItemPlace::PlayerHand {
            player: 12,
            hand: sunrise_sim::first_person_sim::Hand::Main,
        };
        let off = ItemPlace::PlayerHand {
            player: 12,
            hand: sunrise_sim::first_person_sim::Hand::Off,
        };
        with_client_game(|client| {
            let (_reader, _writer) = connect_menu_test(client);
            spawn_menu_test(client);
            client.selected = 2;
            // Consume actual server packets through the client's ordinary handler.
            let deliver = |client: &mut sunrise_client_gameplay::game::ClientGame<'_>| {
                let mut slots = Vec::new();
                for out in rx.try_iter() {
                    let bodies = match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("owner disconnected"),
                    };
                    for body in bodies {
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        let packet = C::decode(id, reader.rest()).unwrap();
                        // Other tick packets concern entity/chunk state, not this contract.
                        if let C::ContainerSetSlot(ref slot) = packet {
                            slots.push(slot.slot);
                            client.handle_packet(packet).unwrap();
                        }
                    }
                }
                slots
            };
            server.tick_lan(true);
            deliver(client);
            assert_eq!(
                client.inventory[2],
                Some(ItemStack::new("minecraft:bucket", 1))
            );
            assert_eq!(
                client.offhand,
                Some(ItemStack::new("minecraft:water_bucket", 1))
            );

            for commit in [false, true] {
                let host: &mut dyn ServerHost = server;
                let result = host.transaction(|host, tx| {
                    let source = off.capability(host, &ITEM_FLUIDS).unwrap();
                    let target = main.capability(host, &ITEM_FLUIDS).unwrap();
                    assert_eq!(
                        move_resources(host, &source, &target, |_| true, 1000, Some(tx)).unwrap(),
                        1000
                    );
                    if commit { Ok(()) } else { Err(()) }
                });
                assert_eq!(result.is_ok(), commit);
                server.tick_lan(true);
                let slots = deliver(client);
                if commit {
                    assert!(slots.contains(&38), "selected hotbar slot published");
                    assert!(slots.contains(&45), "offhand published");
                } else {
                    assert!(slots.is_empty(), "abort publishes no tentative inventory");
                }
                let (held, other) = if commit {
                    ("minecraft:water_bucket", "minecraft:bucket")
                } else {
                    ("minecraft:bucket", "minecraft:water_bucket")
                };
                assert_eq!(
                    client.inventory[client.selected],
                    Some(ItemStack::new(held, 1))
                );
                assert_eq!(client.offhand, Some(ItemStack::new(other, 1)));
                let guest = &server.server.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(guest.inventory[2], client.inventory[2]);
                assert_eq!(guest.offhand, client.offhand);
            }
            server.tick_lan(true);
            assert!(
                deliver(client).is_empty(),
                "settled slots are not sent twice"
            );
        });
    });
}
