use crate::game::server_side::native_entity_refs::tests::with_client_game;
use mod_api::{
    HoldError, HoldId, Identifier, SendPacketError, ServerboundPlayPacket, content::crash::ModCall,
};
use mod_platform::content::crash::enter_native_mod;
use sunrise_client_gameplay::game::client_side::native_net::packet_hold::{Holds, PlaySend};
use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
use sunrise_protocol::{
    connection::Pipe,
    packets::{configuration::Ping, play::Serverbound as S},
};

fn state() -> Holds<&'static str> {
    let mut holds = Holds::default();
    holds.sync(Some((7, 3)));
    holds
}

#[test]
fn either_release_order_publishes_only_the_unblocked_prefix_in_send_order() {
    for newer_first in [false, true] {
        let mut holds = state();
        let first = holds.start("first").unwrap();
        holds.queue("A");
        let second = holds.start("second").unwrap();
        holds.queue("B");
        let mut sent = Vec::new();
        let mut publish = |packet| {
            sent.push(packet);
            Ok(())
        };
        if newer_first {
            assert_eq!(holds.release("second", second, &mut publish), Ok(0));
            holds.queue("C");
            assert_eq!(holds.release("first", first, publish), Ok(3));
        } else {
            assert_eq!(holds.release("first", first, &mut publish), Ok(1));
            holds.queue("C");
            assert_eq!(holds.release("second", second, publish), Ok(2));
        }
        assert_eq!(sent, ["A", "B", "C"]);
        assert_eq!(holds.discard("first", first), Err(HoldError::Unknown));
    }
}

#[test]
fn dropping_an_overlap_does_not_send_or_end_other_holds() {
    let mut holds = state();
    let first = holds.start("first").unwrap();
    holds.queue("A");
    let second = holds.start("second").unwrap();
    holds.queue("B");
    let third = holds.start("third").unwrap();
    holds.queue("C");
    assert_eq!(holds.discard("second", second), Ok(()));
    holds.queue("D");
    let mut sent = Vec::new();
    let mut publish = |packet| {
        sent.push(packet);
        Ok(())
    };
    assert_eq!(holds.release("first", first, &mut publish), Ok(1));
    assert_eq!(holds.release("third", third, publish), Ok(1));
    assert_eq!(sent, ["A", "D"]);
}

#[test]
fn other_mods_cannot_release_or_drop_a_live_hold() {
    let mut holds = state();
    let first = holds.start("first").unwrap();
    holds.queue("A");
    assert_eq!(
        holds.release("second", first, |_| panic!("foreign hold published")),
        Err(HoldError::WrongOwner)
    );
    assert_eq!(holds.discard("second", first), Err(HoldError::WrongOwner));
    let mut sent = Vec::new();
    assert_eq!(
        holds.release("first", first, |packet| {
            sent.push(packet);
            Ok(())
        }),
        Ok(1)
    );
    assert_eq!(sent, ["A"]);
}

#[test]
fn send_failure_reports_one_sent_and_discards_the_tail() {
    let mut holds = state();
    let id = holds.start("first").unwrap();
    for packet in ["A", "B", "C"] {
        holds.queue(packet);
    }
    let mut sent = Vec::new();
    assert_eq!(
        holds.release("first", id, |packet| {
            if packet == "B" {
                return Err(SendPacketError::Backpressure);
            }
            sent.push(packet);
            Ok(())
        }),
        Err(HoldError::Send {
            sent: 1,
            error: SendPacketError::Backpressure
        })
    );
    assert_eq!(sent, ["A"]);
    assert_eq!(holds.discard("first", id), Err(HoldError::Unknown));
}

#[test]
fn disconnect_and_protocol_incarnation_change_drop_and_invalidate() {
    let mut holds = state();
    let old = holds.start("first").unwrap();
    holds.queue("old");
    holds.sync(None);
    assert_eq!(holds.discard("first", old), Err(HoldError::Unknown));
    assert_eq!(holds.start("first"), Err(HoldError::NoWorld));
    holds.sync(Some((7, 5)));
    let next = holds.start("first").unwrap();
    holds.queue("new");
    holds.sync(Some((7, 7)));
    assert_eq!(holds.discard("first", next), Err(HoldError::Unknown));
    let last = holds.start("first").unwrap();
    assert_eq!(
        holds.release("first", last, |_| panic!("stale send")),
        Ok(0)
    );
}

#[test]
#[ignore = "daily"]
fn physical_hold_orders_engine_and_mod_packets_and_bypasses_network_keepalive() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_net::packet_hold::tests::physical_hold_orders_engine_and_mod_packets_and_bypasses_network_keepalive",
        "SUNRISE_PACKET_HOLD_CHILD",
        &["ordering"],
    ) {
        return;
    }
    mod_platform::__private::initialize().unwrap();
    with_client_game(|game| {
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HoldTest").unwrap();
        let (session, mut peer, keepalive) = sunrise_client::net::Session::packet_hold_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        assert_eq!(game.hold_packets(), Err(HoldError::NoOwner));
        assert_eq!(game.send_play(S::Pong(Ping { id: 0 })), PlaySend::Admitted);
        let (id, bytes) = peer.recv().unwrap();
        assert_eq!(S::decode(id, &bytes).unwrap(), S::Pong(Ping { id: 0 }));
        let call = |id| ModCall {
            id,
            hook: "client_tick",
            part: None,
        };
        let _first_mod = enter_native_mod(call("first"));
        let first = game.hold_packets().unwrap();
        assert_eq!(game.send_play(S::Pong(Ping { id: 1 })), PlaySend::Admitted);
        assert_eq!(game.release_held(HoldId::new(0)), Err(HoldError::Unknown));

        let second;
        {
            let _second_mod = enter_native_mod(call("second"));
            second = game.hold_packets().unwrap();
            assert_eq!(game.release_held(first), Err(HoldError::WrongOwner));
            assert_eq!(game.drop_held(first), Err(HoldError::WrongOwner));
            let pong = Identifier::parse("minecraft:pong").unwrap();
            game.storage_scope(|game, _| {
                game.send_packet(ServerboundPlayPacket::new(pong.clone(), vec![0, 0, 0, 2]))
                    .unwrap();
                // Invocation order is not commit order: the engine send joins first.
                assert_eq!(game.send_play(S::Pong(Ping { id: 3 })), PlaySend::Admitted);
                ((), true)
            });
            game.storage_scope(|game, _| {
                game.send_packet(ServerboundPlayPacket::new(pong, vec![0, 0, 0, 4]))
                    .unwrap();
                ((), false)
            });
        }
        keepalive(9);
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::KEEP_ALIVE,
                vec![0, 0, 0, 0, 0, 0, 0, 9]
            )
        );
        assert_eq!(game.release_held(first), Ok(1));
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::PONG,
                vec![0, 0, 0, 1]
            )
        );
        assert_eq!(game.send_play(S::Pong(Ping { id: 5 })), PlaySend::Admitted);
        {
            let _second_mod = enter_native_mod(call("second"));
            assert_eq!(game.release_held(second), Ok(3));
        }
        for n in [3, 2, 5] {
            assert_eq!(
                peer.recv().unwrap(),
                (
                    sunrise_protocol::ids::play::serverbound::PONG,
                    vec![0, 0, 0, n]
                )
            );
        }
        let disconnected = game.hold_packets().unwrap();
        game.send_play(S::Pong(Ping { id: 7 }));
        let mut old = game.client.remote.take().unwrap();
        old.session.disconnect();
        assert_eq!(game.release_held(disconnected), Err(HoldError::Unknown));
    });
}

#[test]
#[ignore = "daily"]
fn held_mod_packets_keep_real_writer_credit_until_drop() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_net::packet_hold::tests::held_mod_packets_keep_real_writer_credit_until_drop",
        "SUNRISE_PACKET_HOLD_PRESSURE_CHILD",
        &["pressure"],
    ) {
        return;
    }
    mod_platform::__private::initialize().unwrap();
    with_client_game(|game| {
        use sunrise_core::vanilla_bytes::Writer;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HoldTest").unwrap();
        let (session, mut peer, _keepalive) = sunrise_client::net::Session::packet_hold_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;
        let _mod = enter_native_mod(ModCall {
            id: "holder",
            hook: "client_tick",
            part: None,
        });
        let hold = game.hold_packets().unwrap();
        let mut data = Writer::new();
        data.string("example:unrelated");
        data.bytes(&vec![
            0x59;
            sunrise_protocol::packets::custom_payload::SERVERBOUND_LIMIT
        ]);
        let packet = ServerboundPlayPacket::new(
            Identifier::parse("minecraft:custom_payload").unwrap(),
            data.data,
        );
        let mut accepted = 0;
        for _ in 0..600 {
            match game.send_packet(packet.clone()) {
                Ok(()) => accepted += 1,
                Err(SendPacketError::Backpressure) => break,
                Err(error) => panic!("unexpected transport error: {error:?}"),
            }
        }
        assert!(accepted > 256, "the old hold limit is gone");
        assert!(
            accepted < 600,
            "the actual writer credit must refuse more sends"
        );
        assert_eq!(game.send_packet(packet), Err(SendPacketError::Backpressure));
        assert_eq!(game.drop_held(hold), Ok(()));
        game.send_packet(ServerboundPlayPacket::new(
            Identifier::parse("minecraft:pong").unwrap(),
            vec![0, 0, 0, 8],
        ))
        .unwrap();
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::PONG,
                vec![0, 0, 0, 8]
            )
        );
    });
}

#[test]
#[ignore = "daily"]
fn generation_change_after_a_drops_b_and_invalidates_the_hold() {
    let (session, mut peer, _keepalive) = sunrise_client::net::Session::packet_hold_probe();
    let incarnation = session.hold_incarnation().unwrap();
    let mut holds = Holds::default();
    holds.sync(Some(incarnation));
    let first = holds.start("first").unwrap();
    for id in [65, 66] {
        holds.queue(
            session
                .prepare_held_play(S::Pong(Ping { id }), incarnation.1)
                .unwrap(),
        );
    }
    let second = holds.start("second").unwrap();
    holds.queue(
        session
            .prepare_held_play(S::Pong(Ping { id: 67 }), incarnation.1)
            .unwrap(),
    );
    let mut published = 0;
    let result = holds.release("first", first, |packet| {
        packet.try_publish()?;
        published += 1;
        if published == 1 {
            session.packet_hold_rollover();
        }
        Ok(())
    });
    assert_eq!(
        result,
        Err(HoldError::Send {
            sent: 1,
            error: SendPacketError::NotInPlay
        })
    );
    assert_eq!(published, 1);
    assert_eq!(holds.discard("second", second), Err(HoldError::Unknown));
    assert_eq!(
        peer.recv().unwrap(),
        (
            sunrise_protocol::ids::play::serverbound::PONG,
            vec![0, 0, 0, 65]
        )
    );
    assert_eq!(peer.recv().unwrap(), (3, vec![]));
    assert_eq!(peer.recv().unwrap(), (3, vec![]));
    peer.reader
        .set_read_timeout(Some(std::time::Duration::from_millis(20)))
        .unwrap();
    assert!(
        peer.recv().is_err(),
        "old packets must not reach the new Play phase"
    );
}

#[test]
#[ignore = "daily"]
fn held_pressure_refuses_send_play_and_cross_passage_without_moving() {
    if run_fresh_cases(
        "cut_layout::game::client_side::native_net::packet_hold::tests::held_pressure_refuses_send_play_and_cross_passage_without_moving",
        "SUNRISE_PACKET_HOLD_CROSS_CHILD",
        &["cross"],
    ) {
        return;
    }
    mod_platform::__private::initialize().unwrap();
    with_client_game(|game| {
        use sunrise_core::vanilla_bytes::Writer;
        let (client, _server) = Pipe::pair();
        game.connect_local(client, "HoldCrossTest").unwrap();
        let (session, mut peer, keepalive) = sunrise_client::net::Session::packet_hold_probe();
        let remote = game.client.remote.as_mut().unwrap();
        remote.session = session;
        remote.joined = true;

        let start = sunrise_core::geometry::Vec3::new(0.5, 65.0, 8.25);
        let velocity = sunrise_core::geometry::Vec3::new(0.0, -0.0784, 0.25);
        let prev = sunrise_core::geometry::Vec3::new(0.5, 65.0, 8.0);
        let mut player = sunrise_sim::motion::PlayerMotion::new(game.motion, start).unwrap();
        player.velocity = velocity;
        game.player = Some(player);
        game.previous_position = prev;
        game.yaw = 0.0;
        game.passages.held.insert(
            mod_api::PassageId::from_raw(1),
            mod_api::Passage {
                origin: mod_api::Vec3::new(0.5, 65.5, 8.5),
                axis_w: mod_api::Vec3::new(1.0, 0.0, 0.0),
                axis_h: mod_api::Vec3::new(0.0, 1.0, 0.0),
                width: 2.0,
                height: 3.0,
                destination: mod_api::Vec3::new(24.5, 65.5, 8.5),
                turn: mod_api::PassageTurn::Clockwise90,
                destination_level: None,
            },
        );

        let _mod = enter_native_mod(ModCall {
            id: "holder",
            hook: "client_tick",
            part: None,
        });
        let hold = game.hold_packets().unwrap();
        let mut data = Writer::new();
        data.string("example:unrelated");
        data.bytes(&vec![
            0x59;
            sunrise_protocol::packets::custom_payload::SERVERBOUND_LIMIT
        ]);
        let large = ServerboundPlayPacket::new(
            Identifier::parse("minecraft:custom_payload").unwrap(),
            data.data,
        );
        while game.send_packet(large.clone()).is_ok() {}
        let small = ServerboundPlayPacket::new(
            Identifier::parse("minecraft:pong").unwrap(),
            vec![0, 0, 0, 1],
        );
        while game.send_packet(small.clone()).is_ok() {}

        // Excluded replies still drain under hold-only pressure.
        keepalive(42);
        assert_eq!(
            peer.recv().unwrap(),
            (
                sunrise_protocol::ids::play::serverbound::KEEP_ALIVE,
                vec![0, 0, 0, 0, 0, 0, 0, 42]
            )
        );
        game.client
            .remote
            .as_ref()
            .unwrap()
            .session
            .send_pack_status(7, 3);
        let (id, body) = peer.recv().unwrap();
        assert_eq!(
            S::decode(id, &body).unwrap(),
            S::ResourcePack(
                sunrise_protocol::packets::configuration::ResourcePackResponse { id: 7, action: 3 }
            )
        );

        // Held backpressure refuses the crossing payload, leaves the writer and the hold
        // alone, and leaves the player unmoved with no pending crossing.
        assert_eq!(
            game.cross_passage(mod_api::PassageId::from_raw(1)),
            Err(mod_api::PassageCrossError::Unavailable)
        );
        let player = game.player.as_ref().unwrap();
        assert_eq!(player.position, start);
        assert_eq!(player.velocity, velocity);
        assert_eq!(game.previous_position, prev);
        assert_eq!(game.yaw, 0.0);
        assert_eq!(game.passages.crossing, None);
        assert_eq!(
            game.send_play(S::Pong(Ping { id: 99 })),
            PlaySend::Unavailable
        );
        // The hold survived the refusals: dropping it frees the credit and sends flow again.
        assert_eq!(game.drop_held(hold), Ok(()));
        assert_eq!(
            game.send_play(S::Pong(Ping { id: 100 })),
            PlaySend::Admitted
        );
        let (id, body) = peer.recv().unwrap();
        assert_eq!(S::decode(id, &body).unwrap(), S::Pong(Ping { id: 100 }));
    });
}
