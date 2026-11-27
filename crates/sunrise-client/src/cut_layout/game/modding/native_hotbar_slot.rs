#![cfg(test)]
#[cfg(feature = "client")]
#[cfg(test)]
pub(crate) mod hotbar_slot_tests {

    use crate::game::remote::tests::connect_menu_test;
    use crate::game::server_side::native_entity_refs::tests::with_client_game;
    use mod_api::HotbarSlotError;
    use sunrise_command::args::GameMode;
    use sunrise_protocol::connection::ConnReader;
    use sunrise_protocol::packets::play::{self, Clientbound as C, Serverbound as S};

    /// What the window sent since the last call, as play packets.
    pub(crate) fn sent(reader: &mut ConnReader) -> Vec<S> {
        reader
            .set_read_timeout(Some(std::time::Duration::from_millis(200)))
            .unwrap();
        let mut packets = Vec::new();
        while let Ok((id, body)) = reader.recv() {
            packets.extend(S::decode(id, &body).ok());
        }
        packets
    }

    fn login_packet() -> play::Login {
        play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: play::SpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld".into(),
                seed: 1,
                game_type: 0,
                previous_game_type: None,
                debug: false,
                flat: false,
                last_death: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: false,
        }
    }

    /// A window logged in to the overworld at (0.5, 65, 0.5), its player made by the server's
    /// position packet, with everything the login sent already drained.
    pub(crate) fn logged_in(
        game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
    ) -> ConnReader {
        let (mut reader, _writer) = connect_menu_test(game);
        game.handle_packet(C::Login(login_packet())).unwrap();
        game.handle_packet(C::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [0.5, 65.0, 0.5],
            delta: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            relatives: 0,
        }))
        .unwrap();
        if let Some(remote) = game.client.remote.as_mut() {
            remote.joined = true;
        }
        sent(&mut reader);
        reader
    }

    fn carried(packets: &[S]) -> Vec<i16> {
        packets
            .iter()
            .filter_map(|packet| match packet {
                S::SetCarriedItem(item) => Some(item.slot),
                _ => None,
            })
            .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn a_selection_reaches_the_server_at_the_next_tick_and_only_once() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            assert_eq!(game.selected, 4);
            assert!(
                sent(&mut reader).is_empty(),
                "select_hotbar_slot sends nothing itself"
            );

            game.tick_remote_level().unwrap();
            assert_eq!(carried(&sent(&mut reader)), [4]);

            // The same slot twice makes no second packet.
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            game.tick_remote_level().unwrap();
            assert!(carried(&sent(&mut reader)).is_empty());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_selection_goes_out_before_an_attack_in_the_same_tick() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            assert!(game.remote_attack_entity(123, 1.0));
            let packets = sent(&mut reader);
            let carried_at = packets
                .iter()
                .position(|packet| matches!(packet, S::SetCarriedItem(item) if item.slot == 4))
                .expect("the carried-item packet goes out");
            let attack_at = packets
                .iter()
                .position(|packet| matches!(packet, S::Attack(_)))
                .expect("the attack goes out");
            assert!(
                carried_at < attack_at,
                "the carried-item packet precedes the attack: {packets:?}"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_out_of_range_slot_is_refused_and_keeps_the_selection() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            assert_eq!(game.select_hotbar_slot(2), Ok(()));
            assert_eq!(
                game.select_hotbar_slot(9),
                Err(HotbarSlotError::OutOfRange { slot: 9 })
            );
            assert_eq!(game.selected, 2, "the refused slot changes nothing");
            game.tick_remote_level().unwrap();
            assert_eq!(carried(&sent(&mut reader)), [2]);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_spectator_gets_no_hotbar_selection() {
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.select_hotbar_slot(2).unwrap();
            game.mode = GameMode::Spectator;
            assert_eq!(game.select_hotbar_slot(4), Err(HotbarSlotError::Refused));
            assert_eq!(game.selected, 2, "a spectator keeps its slot");
            game.tick_remote_level().unwrap();
            assert_eq!(
                carried(&sent(&mut reader)),
                [2],
                "only the earlier change goes out"
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn the_debug_requests_follow_the_carried_slot_in_the_ping_then_subscription_order() {
        // The game fixture replaces process-wide registries: a child of its own.
        const MODE: &str = "SUNRISE_HOTBAR_SLOT_THE_DEBUG_REQUESTS_FOLLOW_THE_CARRIED_SLOT_IN_THE_PING_THEN_SUBSCRIPTION_ORDER";
        if std::env::var_os(MODE).is_none() {
            sunrise_gameplay::game::mods::tests::run_in_child(
                "cut_layout::game::client_side::native_hotbar_slot::hotbar_slot_tests::the_debug_requests_follow_the_carried_slot_in_the_ping_then_subscription_order",
                MODE,
            );
            return;
        }
        with_client_game(|game| {
            let mut reader = logged_in(game);
            game.client.debug = true;
            game.client.debug_charts.fps = true;
            game.client.debug_charts.network = true;
            assert_eq!(game.select_hotbar_slot(4), Ok(()));
            game.tick_remote_level().unwrap();
            let names: Vec<_> = sent(&mut reader)
                .iter()
                .filter_map(|p| match p {
                    S::SetCarriedItem(_) => Some("carried"),
                    S::PingRequest(_) => Some("ping"),
                    S::DebugSubscriptionRequest(r) if r.subscriptions == [0] => Some("subscribe"),
                    _ => None,
                })
                .collect();
            assert_eq!(names, ["carried", "ping", "subscribe"]);
            // Unchanged, no second request; a new login asks again, the chart still on.
            game.tick_remote_level().unwrap();
            assert!(
                !sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::DebugSubscriptionRequest(_)))
            );
            game.handle_packet(C::Login(login_packet())).unwrap();
            game.tick_remote_level().unwrap();
            assert!(
                sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::DebugSubscriptionRequest(r) if r.subscriptions == [0]))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_received_tick_sample_draws_the_full_and_method_bars() {
        sunrise_gameplay_common::game::in_own_process(
            "cut_layout::game::client_side::native_hotbar_slot::hotbar_slot_tests::a_received_tick_sample_draws_the_full_and_method_bars",
            || {
                with_client_game(|game| {
                    use sunrise_client::hud_layout::GuiDraw;

                    let _reader = logged_in(game);
                    game.client.debug = true;
                    game.client.debug_charts.fps = true;
                    game.clock.set_rate(20.0);
                    game.handle_packet(C::DebugSample(play::DebugSample {
                        sample: vec![25_000_000, 10_000_000, 0, 15_000_000],
                        kind: play::DEBUG_SAMPLE_TICK_TIME,
                    }))
                    .unwrap();
                    let draws = game.client.debug_charts.draw(
                        &game.client.gui.atlas.font,
                        484,
                        100,
                        0,
                        &game.clock,
                        true,
                    );
                    // At 50 mspt the 60-pixel chart gives 30 px to the full 25 ms and 12 px
                    // to the 10 ms tick method. The first sample is just inside the right chart.
                    let heights: Vec<_> = draws
                        .iter()
                        .filter_map(|draw| match draw {
                            GuiDraw::Fill {
                                x0: 243,
                                x1: 244,
                                y0,
                                y1: 100,
                                ..
                            } if *y0 < 100 => Some(100 - y0),
                            _ => None,
                        })
                        .collect();
                    assert_eq!(heights, [30, 12]);
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn no_debug_request_goes_out_before_the_login() {
        // The game fixture replaces process-wide registries: a child of its own.
        const MODE: &str = "SUNRISE_HOTBAR_SLOT_NO_DEBUG_REQUEST_GOES_OUT_BEFORE_THE_LOGIN";
        if std::env::var_os(MODE).is_none() {
            sunrise_gameplay::game::mods::tests::run_in_child(
                "cut_layout::game::client_side::native_hotbar_slot::hotbar_slot_tests::no_debug_request_goes_out_before_the_login",
                MODE,
            );
            return;
        }
        with_client_game(|game| {
            let (mut reader, _writer) = connect_menu_test(game);
            game.client.debug = true;
            game.client.debug_charts.fps = true;
            game.tick_remote_level().unwrap();
            assert!(
                !sent(&mut reader)
                    .iter()
                    .any(|p| matches!(p, S::DebugSubscriptionRequest(_)))
            );
        });
    }
}
