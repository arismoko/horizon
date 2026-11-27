use crate::game::server_side::commands::enchant_tests::RIPTIDE;
use crate::game::server_side::commands::enchant_tests::TRIDENT;
use crate::game::server_side::commands::enchant_tests::riptide;
use std::sync::Arc;
use sunrise_client::creative::Creative;
use sunrise_game_data::scoreboard::tag_to_json;
use sunrise_items::items::ItemStack;
use sunrise_protocol::{
    buf::Reader,
    packets::play::{Clientbound as C, GameEvent, Serverbound as S},
    registry::Builtin,
};
use sunrise_sim::{
    lan::{Out, data::SyncData},
    menu::Input,
};
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::lan::LanHost,
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::server_side::lan_particles::tests::test_hub,
    sunrise_gameplay_common::game::Screen,
};

#[test]
#[ignore = "daily"]
fn integrated_enchant_updates_the_clients_selected_slot() {
    const CASE: &str = "SUNRISE_TEST_INTEGRATED_ENCHANT_TARGET";
    if run_fresh_cases(
        "cut_layout::game::client_side::commands_client::enchant_tests::integrated_enchant_updates_the_clients_selected_slot",
        CASE,
        &["NearPlayer", "@s", "@a[name=NearPlayer]", "console"],
    ) {
        return;
    }
    let target = std::env::var(CASE).unwrap();
    let console = target == "console";
    let target = if console { "NearPlayer" } else { &target };
    let mut sync = SyncData::empty();
    sync.builtin = Builtin::load(&Builtin::find(&fixture_root()).unwrap()).unwrap();
    sync.dynamic
        .registries
        .insert("minecraft:enchantment".into(), vec![(RIPTIDE.into(), None)]);
    let sync = Arc::new(sync);
    let selected: usize = 3;
    let command = format!("enchant {target} {RIPTIDE} 3");
    with_client_game(|client| {
        let mut wire = logged_in(client);
        client
            .set_registries(Arc::new(sync.dynamic.clone()))
            .unwrap();
        client
            .handle_packet(C::GameEvent(GameEvent {
                event: 3, // CHANGE_GAME_MODE
                param: 1.0,
            }))
            .unwrap();
        client.set_screen(Screen::Creative);
        // Supply the category's displayed stack, not a predicted or server slot.
        let category = client
            .client
            .creative_screen
            .0
            .tabs
            .iter()
            .position(|tab| !tab.search && !tab.inventory && !tab.hotbar)
            .unwrap();
        client.client.creative_screen.0.select(category);
        client.client.creative_screen.0.tabs[category].items = vec![ItemStack::new(TRIDENT, 1)];
        client.client.creative_screen.0.forget_items();
        let display = Creative::DISPLAY;
        sent(&mut wire);
        match target {
            "@s" => {
                client.creative_display_click(display, 0, Input::Pickup);
                assert!(client.menu_ops(&[(Some(selected), 0, Input::Pickup)]));
            }
            "@a[name=NearPlayer]" => {
                client.creative_display_click(display, 0, Input::Pickup);
                assert!(client.menu_ops(&[
                    (None, 0, Input::QuickCraft),
                    (Some(selected), 1, Input::QuickCraft),
                    (None, 2, Input::QuickCraft),
                ]));
            }
            _ => client.creative_display_click(display, selected as i32, Input::Swap),
        }
        assert_eq!(riptide(client.inventory[selected].as_ref()), 0);
        client.select_hotbar_slot(selected as u8).unwrap();
        client.set_screen(Screen::Playing);
        client.remote_send_carried();
        if !console {
            client.send_command(&command).unwrap();
        }
        let outgoing = sent(&mut wire);
        let hotbar_slot = 36 + selected as i16;
        assert!(
            matches!(outgoing.first(), Some(S::SetCreativeModeSlot(change)) if change.slot == hotbar_slot && change.item.is_some()),
            "{outgoing:?}"
        );
        assert!(
            matches!(outgoing.get(1), Some(S::SetCarriedItem(change)) if change.slot == selected as i16),
            "{outgoing:?}"
        );
        if !console {
            assert!(
                matches!(outgoing.get(2), Some(S::ChatCommand(chat)) if chat.command == command),
                "{outgoing:?}"
            );
        }
        let (mut hub, rx, _other) = test_hub("Enchant", Vec::new(), Some(sync));
        hub.config.integrated = true;
        let index = hub.guests.connection_index(1).unwrap();
        hub.guests.update(index, |g| {
            g.permission = 4;
            g.game_mode = 1;
        });
        let mut slot = None;
        with_game(|game| {
            game.player = None;
            game.build_registries();
            game.server.lan = Some(LanHost::for_test(hub));
            for packet in outgoing {
                game.lan_guest_packet(1, packet);
            }
            let player = &game.server.lan.as_ref().unwrap().hub.guests[index];
            assert_eq!(player.selected, selected);
            assert_eq!(riptide(player.inventory[selected].as_ref()), 0);
            if console {
                let (ok, result) = game.recording_command_outcome(|game| {
                    game.as_console(|game| game.run_command(&command))
                });
                assert!(ok, "{command}: server source");
                assert_eq!(result, Some((true, 1)));
            } else {
                game.run_guest_commands();
            }
            let hub = &mut game.server.lan.as_mut().unwrap().hub;
            hub.sync_guest_after_tick(1);
            let mut feedback = Vec::new();
            for out in rx.try_iter() {
                let bodies = match out {
                    Out::Packet(body) => vec![body],
                    Out::Packets(bodies) => bodies,
                    Out::Disconnect(reason) => panic!("player disconnected: {reason:?}"),
                };
                for body in bodies {
                    let mut reader = Reader::new(&body);
                    let id = reader.varint().unwrap();
                    match C::decode(id, reader.rest()).unwrap() {
                        C::SystemChat(chat) => feedback.push(tag_to_json(&chat.content)),
                        C::ContainerSetSlot(packet)
                            if packet.container == 0 && packet.slot == hotbar_slot =>
                        {
                            slot = Some(packet);
                        }
                        _ => {}
                    }
                }
            }
            if !console {
                assert!(
                    feedback
                        .iter()
                        .any(|line| line["translate"] == "commands.enchant.success.single"),
                    "{target}: {feedback:?}"
                );
            }
            assert_eq!(riptide(hub.guests[index].inventory[selected].as_ref()), 3);
        });
        client
            .handle_packet(C::ContainerSetSlot(
                slot.expect("the client must get its hotbar update"),
            ))
            .unwrap();
        assert_eq!(riptide(client.inventory[selected].as_ref()), 3);
    });
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
