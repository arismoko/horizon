use super::{connect_play_test, spawn_menu_test};
use mod_api::{ClientHost, ClientPacketSendEvent, Flow, Hooks, MenuClick, ModEntry};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sunrise_client_lib::net::Event;
use sunrise_command::args::GameMode;
use sunrise_gameplay_common::game::Screen;
use sunrise_items::items::ItemStack;
#[cfg(test)]
use sunrise_protocol::packets::play;
use sunrise_protocol::packets::play::Clientbound as C;
use sunrise_protocol::packets::play::Serverbound as S;
use sunrise_protocol::{packets::configuration as config, registry::Builtin};
use sunrise_sim::lan::data::SyncData;
use sunrise_sim::menu::Input;
use {
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::logged_in,
    crate::game::client_side::native_hotbar_slot::hotbar_slot_tests::sent,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::lan::LanHost,
    sunrise_gameplay::game::native_entity_refs::tests::fixture_root,
    sunrise_gameplay::game::native_entity_refs::tests::with_game,
    sunrise_gameplay::game::server_side::lan_particles::tests::test_hub,
};

#[test]
#[ignore = "daily"]
fn canceled_creative_drop_keeps_throttle_budget_for_next_take() {
    struct CancelFirstCreativeSlot(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl Hooks for CancelFirstCreativeSlot {
        fn client_packet_send(
            &self,
            _: &dyn ClientHost,
            event: &mut ClientPacketSendEvent<'_>,
        ) -> Flow {
            if event.packet.id.path() == "set_creative_mode_slot"
                && self.0.fetch_add(1, Ordering::SeqCst) == 0
            {
                return Flow::Cancel;
            }
            Flow::Continue
        }
    }

    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::container_drop_tests::canceled_creative_drop_keeps_throttle_budget_for_next_take",
        || {
            let seen = Arc::new(AtomicUsize::new(0));
            with_mods(
                vec![ModEntry::new(
                    "test:cancel_first_drop",
                    Box::new(CancelFirstCreativeSlot(Arc::clone(&seen))),
                )]
                .into(),
                || {
                    with_client_game(|game| {
                        let mut reader = logged_in(game);
                        game.set_local_mode(GameMode::Creative);
                        game.client.screen = Screen::Creative;
                        game.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                        game.send_creative_slots();
                        sent(&mut reader);
                        seen.store(0, Ordering::SeqCst);

                        // Room for one accepted drop before the 1280 cap.
                        for _ in 0..63 {
                            game.client.drop_spam_throttler.increment();
                        }
                        assert!(
                            game.menu_ops(&[
                                (Some(0), 0, Input::Throw),
                                (Some(0), 0, Input::Throw),
                            ])
                        );
                        assert_eq!(
                            game.inventory[0].as_ref().map(|s| s.count),
                            Some(1),
                            "second throw in the batch must still see available budget after the first drop is canceled"
                        );
                        assert!(!game.client.drop_spam_throttler.is_under_threshold());

                        let packets: Vec<_> = sent(&mut reader)
                            .into_iter()
                            .filter_map(|p| match p {
                                S::SetCreativeModeSlot(p) => {
                                    Some((p.slot, p.item.map(|s| s.count)))
                                }
                                _ => None,
                            })
                            .collect();
                        assert_eq!(packets, [(-1, Some(1)), (36, Some(1))]);
                    });
                },
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn creative_drop_guards_charge_only_sent_drops() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::container_drop_tests::creative_drop_guards_charge_only_sent_drops",
        || {
            with_client_game(|game| {
                let (mut reader, _writer) = connect_play_test(game, |_, server| {
                    server
                        .send(
                            &config::Clientbound::UpdateEnabledFeatures(
                                config::UpdateEnabledFeatures { features: vec![] },
                            )
                            .encode(),
                        )
                        .unwrap();
                });
                // Acknowledging configuration can already drain the feature update.
                // Wait only if the connection still has its default flags.
                if !game.enabled_feature_ids().is_empty() {
                    game.wait_for_test_remote_event(|event| matches!(event, Event::Features(_)));
                    game.poll_remote_events(true).unwrap();
                }
                for _ in 0..64 {
                    game.client.drop_spam_throttler.increment();
                }
                spawn_menu_test(game);
                assert!(game.client.drop_spam_throttler.is_under_threshold());
                game.handle_packet(C::PlayerPosition(play::PlayerPosition {
                    teleport: 1,
                    position: [0.5, 65.0, 0.5],
                    delta: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    relatives: 0,
                }))
                .unwrap();
                sent(&mut reader);
                game.set_local_mode(GameMode::Creative);
                let stone = || ItemStack::new("minecraft:stone", 1);
                let vanilla = ["minecraft:vanilla".to_owned()];
                let catalog = game.client.creative_screen.0.catalog();
                let items = game
                    .client
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap();
                assert!(!items.names.is_empty());
                assert!(game.enabled_feature_ids().is_empty());
                for name in &items.names {
                    let stack = ItemStack::new(name, 1);
                    assert!(catalog.item_enabled(&stack, &vanilla), "{name}");
                    assert!(!catalog.item_enabled(&stack, &[]), "{name}");
                }

                game.client.screen = Screen::Creative;
                game.inventory[0] = Some(stone());
                game.remote_creative_slot_add(0);
                game.inventory[1] = Some(stone());
                game.send_creative_slots();
                game.inventory[2] = Some(stone());
                game.remote_send_state(0);
                game.remote_creative_throw(stone());
                game.inventory[0] = None;
                game.remote_creative_slot_add(0);
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item)),
                        _ => None,
                    })
                    .collect();
                assert!(
                    packets.is_empty(),
                    "AIR requires minecraft:vanilla so an empty slot clear is withheld too: {packets:?}"
                );

                game.client.remote.as_mut().unwrap().features = vanilla.into();
                game.inventory[0] = None;
                game.remote_creative_slot_add(0);
                let clear_packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    clear_packets,
                    [(36, None)],
                    "empty slot clear goes out once minecraft:vanilla is enabled"
                );
                game.set_local_mode(GameMode::Survival);
                game.inventory[0] = Some(stone());
                game.remote_creative_slot_add(0);
                game.remote_creative_throw(stone());
                game.set_local_mode(GameMode::Creative);
                game.client.screen = Screen::Inventory;
                game.remote_creative_throw(stone());
                game.client.screen = Screen::Creative;
                game.remote_creative_throw(ItemStack::new("minecraft:air", 1));
                game.remote_creative_throw(ItemStack::new("minecraft:air", 0));
                game.remote_creative_throw(ItemStack::new("minecraft:stone", 0));
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_)))
                );

                // Any forbidden drop that charged the counter would move the burst boundary.
                for _ in 0..63 {
                    game.remote_creative_throw(stone());
                }
                assert!(game.client.drop_spam_throttler.is_under_threshold());
                game.remote_creative_throw(stone());
                assert!(!game.client.drop_spam_throttler.is_under_threshold());

                // With no screen the game-mode method may still send: only the click checks the cap.
                game.client.screen = Screen::Playing;
                game.remote_creative_throw(stone());
                game.remote_creative_slot_add(0);
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item.unwrap().count)),
                        _ => None,
                    })
                    .collect();
                let mut expected = vec![(-1, 1); 65];
                expected.push((36, 1));
                assert_eq!(packets, expected);

                // A refused publication (disconnected transport) must not charge the throttler.
                let spawn = spawn_menu_test(game);
                game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                    .unwrap();
                for _ in 0..63 {
                    game.client.drop_spam_throttler.increment();
                }
                assert!(game.client.drop_spam_throttler.is_under_threshold());
                game.client.remote.as_mut().unwrap().session.disconnect();
                game.remote_creative_throw(stone());
                assert!(
                    game.client.drop_spam_throttler.is_under_threshold(),
                    "refused send must not charge the drop throttler"
                );
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn creative_screen_throws_wait_for_loaded_player_tick() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::container_drop_tests::creative_screen_throws_wait_for_loaded_player_tick",
        || {
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.set_local_mode(GameMode::Creative);
                game.client.screen = Screen::Creative;
                let stone = ItemStack::new("minecraft:stone", 2);
                game.inventory[0] = Some(stone.clone());
                game.carried = Some(stone.clone());
                for _ in 0..64 {
                    game.client.drop_spam_throttler.increment();
                }
                game.client.remote.as_mut().unwrap().loaded_sent = false;
                assert_eq!(game.tick_remote_own_player().unwrap(), None);
                assert!(game.click_open_inventory(Some(0), 0, Input::Throw));
                game.throw_click(false, Input::Pickup);
                assert_eq!(game.inventory[0].as_ref(), Some(&stone));
                assert_eq!(game.carried.as_ref(), Some(&stone));
                assert!(
                    !sent(&mut reader)
                        .iter()
                        .any(|p| matches!(p, S::SetCreativeModeSlot(_)))
                );

                game.client.remote.as_mut().unwrap().loaded_sent = true;
                let _ = game.tick_remote_own_player().unwrap();
                game.throw_click(false, Input::Pickup);
                assert!(game.carried.is_none());
                assert!(!game.client.drop_spam_throttler.is_under_threshold());
                let packets: Vec<_> = sent(&mut reader)
                    .into_iter()
                    .filter_map(|p| match p {
                        S::SetCreativeModeSlot(p) => Some((p.slot, p.item.unwrap().count)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(packets, [(-1, 2)]);

                // A new screen doesn't reset the budget; a replacement LocalPlayer does.
                game.client.screen = Screen::Playing;
                game.client.screen = Screen::Creative;
                assert!(!game.client.drop_spam_throttler.is_under_threshold());
                let spawn = spawn_menu_test(game);
                assert!(
                    !game.client.drop_spam_throttler.is_under_threshold(),
                    "an existing player survives Login"
                );
                game.handle_packet(C::Respawn(play::Respawn { spawn, keep: 0 }))
                    .unwrap();
                assert!(game.client.drop_spam_throttler.is_under_threshold());
            });
        },
    );
}

#[test]
#[ignore = "daily"]
fn playing_menu_throw_uses_only_container_click_and_server_drops_once() {
    sunrise_gameplay_common::game::in_own_process(
        "cut_layout::game::client_side::remote_client::tests::container_drop_tests::playing_menu_throw_uses_only_container_click_and_server_drops_once",
        || {
            let mut outgoing = Vec::new();
            let mut sync = SyncData::empty();
            sync.builtin = Builtin::load(&Builtin::find(&fixture_root()).unwrap()).unwrap();
            with_client_game(|game| {
                let mut reader = logged_in(game);
                game.set_local_mode(GameMode::Creative);
                game.client.screen = Screen::Playing;
                game.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
                let menu = game
                    .local_player()
                    .and_then(|player| game.player_container_menu(player))
                    .unwrap();
                assert!(game.menu_click(
                    menu,
                    &[MenuClick::Throw {
                        slot: 36,
                        whole_stack: false,
                    }],
                ));
                assert_eq!(game.inventory[0].as_ref().unwrap().count, 2);
                let stone = game.item_id("minecraft:stone");
                outgoing = sent(&mut reader)
                    .into_iter()
                    .filter(|p| matches!(p, S::SetCreativeModeSlot(_) | S::ContainerClick(_)))
                    .collect();
                let [S::ContainerClick(click)] = outgoing.as_slice() else {
                    panic!(
                        "normal MenuClick::Throw in Screen::Playing must send only ContainerClick: {outgoing:?}"
                    );
                };
                assert_eq!(
                    (click.container, click.slot, click.button, click.input),
                    (0, 36, 0, 4)
                );
                let [(36, Some(changed))] = click.changed.as_slice() else {
                    panic!(
                        "only the clicked hotbar slot should change: {:?}",
                        click.changed
                    );
                };
                assert_eq!((changed.item, changed.count), (stone, 2));
                assert!(click.carried.is_none());
            });

            let (mut hub, _rx, _other) = test_hub("ThrowOnce", Vec::new(), Some(Arc::new(sync)));
            hub.config.integrated = true;
            let index = hub.guests.connection_index(1).unwrap();
            hub.guests.update(index, |g| {
                g.game_mode = 1;
                g.inventory[0] = Some(ItemStack::new("minecraft:stone", 3));
            });
            with_game(|server| {
                server.player = None;
                server.build_registries();
                server.server.lan = Some(LanHost::for_test(hub));
                for packet in outgoing {
                    server.lan_guest_packet(1, packet);
                }
                let guest = &server.server.lan.as_ref().unwrap().hub.guests[index];
                assert_eq!(
                    guest.inventory[0].as_ref().map(|s| (&*s.item, s.count)),
                    Some(("minecraft:stone", 2)),
                    "server must remove exactly one item from the slot"
                );
                assert_eq!(
                    server.dropped.len(),
                    1,
                    "server must spawn exactly one dropped stack"
                );
                assert_eq!(
                    (
                        &*server.dropped[0].stack.item,
                        server.dropped[0].stack.count
                    ),
                    ("minecraft:stone", 1)
                );
            });
        },
    );
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_menu_refs_common::NativeMenuRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;
