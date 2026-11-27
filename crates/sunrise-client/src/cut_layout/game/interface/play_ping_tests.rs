//! Registered play-Ping regression with the production sender-flush completion boundary.
use std::sync::{Arc, atomic::AtomicI32, mpsc};
use sunrise_gameplay::game::lan::LanSettings;
use sunrise_protocol::{
    buf::Reader,
    packets::{
        configuration::ClientInformation,
        play::{Clientbound as C, Serverbound as S},
        status,
    },
};
use sunrise_sim::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};

fn decode(packet: S) -> (S, usize) {
    let bytes = packet.encode();
    let mut reader = Reader::new(&bytes);
    let id = reader.varint().unwrap();
    (S::decode(id, reader.rest()).unwrap(), bytes.len())
}

pub(crate) fn output(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(reason) => panic!("unexpected disconnect: {reason:?}"),
        })
        .map(|body| {
            println!("actual outbound play body={body:02x?}");
            let mut reader = Reader::new(&body);
            let id = reader.varint().unwrap();
            C::decode(id, reader.rest()).unwrap()
        })
        .collect()
}

fn pass(game: &mut sunrise_gameplay::game::ServerGame<'_>, conn: u32, packet: S) {
    let (packet, _bytes) = decode(packet);
    if let S::PingRequest(p) = &packet {
        println!(
            "actual decoded play PingRequest conn={conn} value={} bits={:016x}",
            p.time, p.time as u64
        );
    }
    let hub = &game.server.lan.as_ref().unwrap().hub;
    assert_eq!(hub.guests.len(), 2);
    assert_ne!(hub.guests[0].uuid, hub.guests[1].uuid);
    assert_ne!(hub.guests[0].conn, hub.guests[1].conn);
    let registered = hub.guest(conn).expect("origin connection registered");
    assert_eq!(registered.uuid, u128::from(conn));
    assert!(!registered.is_fake());
    assert!(registered.left_reason.is_empty());
    assert_eq!(registered.dimension, "minecraft:overworld");
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |guest| {
            guest
                .inbox
                .push_back(sunrise_sim::lan::guest::InboxItem::Packet(packet, None));
        })
        .unwrap();
    complete_sender_tick(game);
    let world_clock = game.world.game_time;
    println!("actual sender completion conn={conn} world_clock={world_clock}");
    assert!(
        game.server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(conn)
            .unwrap()
            .inbox
            .is_empty(),
        "decoded request was drained by the actual Hub"
    );
}

fn complete_sender_tick(game: &mut sunrise_gameplay::game::ServerGame<'_>) {
    // Already registered connections: this is a second level pass, without polling sockets.
    game.server.level_pass.first = false;
    game.server.level_pass.packets_drained = false;
    let clock_before = game.world.game_time;
    game.tick_lan_packets();
    // tick_lan_packets has ended its Holding scope; production flushes these senders later.
    sunrise_sim::lan::flush_all(
        game.server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .map(|g| &g.sender),
    );
    assert_eq!(
        game.world.game_time, clock_before,
        "sender completion adds no world tick"
    );
    assert!(
        game.server.level_pass.packets_drained,
        "real Game packet phase completed"
    );
}

/// Two registered guests (connections 1 and 2) on a LAN host, with their
/// outbound packets.
pub(crate) fn fixture(
    resources: &mut Option<std::sync::Arc<sunrise_gameplay_common::game::Resources>>,
) -> (
    sunrise_gameplay::game::ServerGame<'_>,
    [mpsc::Receiver<Out>; 2],
) {
    let mut game = sunrise_gameplay::game::game_tests::tests::server_game(resources);
    game.build_registries();
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "play-ping-test".into(), 2)
        },
        HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
            level_seeds: Default::default(),
        },
    )
    .unwrap();
    let receivers = [1, 2].map(|conn| {
        let (tx, rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("PingGuest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            [0.5, 80.0, 0.5],
            2,
        );
        guest.placed = true;
        guest.loaded = true;
        guest.loading = false;
        guest.load_timeout = 0;
        guest.last_action_tick = 7;
        game.server.lan.as_mut().unwrap().hub.guests.push(guest);
        rx
    });
    game.server.lan.as_mut().unwrap().hub.ticks = 123;
    (game, receivers)
}

#[test]
#[ignore = "daily"]
fn player_pong_observes_order_and_sends_typed_ping() {
    use mod_api::{
        EntityRef, Hooks, ModEntry, Mods, PlayerPingRefusal, PlayerPongEvent, ServerHost,
    };
    use std::sync::{Mutex, atomic::Ordering};
    use sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_protocol::packets::configuration::Ping;
    use sunrise_sim::lan::guest::InboxItem;

    const CHILD: &str = "SUNRISE_PLAYER_PONG_CHILD";
    if run_fresh_cases(
        "cut_layout::game::server_side::play_ping_tests::player_pong_observes_order_and_sends_typed_ping",
        CHILD,
        &["placed", "respawning"],
    ) {
        return;
    }

    type Observation = (u128, String, i32, bool);
    struct Observer(Arc<Mutex<Vec<Observation>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Observer {
        fn player_pong(&self, host: &mut dyn ServerHost, event: &mut PlayerPongEvent<'_>) {
            let mut seen = self.0.lock().unwrap();
            let player = event.player;
            if seen.is_empty() {
                assert_eq!(player.send_ping(host, -123), Ok(()));
                let leaving = host.player(2).unwrap();
                assert_eq!(
                    leaving.send_ping(host, 45),
                    Err(PlayerPingRefusal::NoConnection)
                );
                // Guest 2's UUID with guest 1's entity identity must not resolve by UUID alone.
                let stale = mod_api::__private::game::player_ref(
                    EntityRef::new(player.entity().epoch(), player.entity().id()),
                    leaving.uuid(),
                );
                assert_eq!(stale.send_ping(host, 46), Err(PlayerPingRefusal::Stale));
            }
            seen.push((
                player.uuid(),
                event.name.to_owned(),
                event.id,
                player.text_filtering(host).unwrap(),
            ));
        }
    }
    /// Runs first and rewrites its own event: the observer still hears what the client sent.
    struct Tamper;
    #[mod_api::__subscriptions]
    impl Hooks for Tamper {
        fn player_pong(&self, _: &mut dyn ServerHost, event: &mut PlayerPongEvent<'_>) {
            event.id = 99;
            event.name = "tampered";
        }
    }
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![
        ModEntry::new("tamper", Box::new(Tamper)),
        ModEntry::new("observer", Box::new(Observer(seen.clone()))),
    ]
    .into();
    mod_platform::__private::install(mods);

    let mut resources = None;
    let (mut game, receivers) = fixture(&mut resources);
    let guests = &mut game.server.lan.as_mut().unwrap().hub.guests;
    guests
        .update_connection(2, |guest| guest.disconnect_pending = true)
        .unwrap();
    guests
        .update_connection(1, |guest| {
            if std::env::var(CHILD).as_deref() == Ok("respawning") {
                guest.placed = false;
                guest.respawning = true;
            }
            guest.latency.store(37, Ordering::Relaxed);
            let mut filtering = ClientInformation::new(2);
            filtering.text_filtering = true;
            guest.inbox.extend(
                [
                    S::ClientInformation(filtering),
                    S::Pong(Ping { id: i32::MIN }),
                    S::PingRequest(status::Ping { time: i64::MIN }),
                    S::ClientInformation(ClientInformation::new(2)),
                    // An action between pong boundaries: a respawning player's is discarded.
                    S::SetCarriedItem(sunrise_protocol::packets::play::SetCarriedItem { slot: 4 }),
                    S::Pong(Ping { id: i32::MIN }),
                    S::Pong(Ping { id: -7 }),
                ]
                .into_iter()
                .map(|packet| InboxItem::Packet(decode(packet).0, None)),
            );
        })
        .unwrap();

    complete_sender_tick(&mut game);

    assert_eq!(
        *seen.lock().unwrap(),
        [
            (1, "PingGuest1".to_owned(), i32::MIN, true),
            (1, "PingGuest1".to_owned(), i32::MIN, false),
            (1, "PingGuest1".to_owned(), -7, false),
        ]
    );
    assert!(
        matches!(output(&receivers[0]).as_slice(), [C::Ping(ping), C::PongResponse(pong)]
        if ping.id == -123 && pong.time == i64::MIN)
    );
    assert!(output(&receivers[1]).is_empty());
    let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
    assert_eq!(guest.latency.load(Ordering::Relaxed), 37);
    let respawning = std::env::var(CHILD).as_deref() == Ok("respawning");
    assert_eq!(guest.selected, if respawning { 0 } else { 4 });
    assert_eq!(guest.last_action_tick, if respawning { 7 } else { 123 });
    assert!(guest.inbox.is_empty());
}

#[test]
#[ignore = "daily"]
fn registered_play_ping_echoes_exact_long_to_only_its_connection() {
    let mut resources = None;
    let (mut game, receivers) = fixture(&mut resources);
    for (turn, time) in [
        1,
        0,
        -1,
        i64::MIN,
        i64::MAX,
        0x0123_4567_89ab_cdef,
        0xfedc_ba98_7654_3210_u64 as i64,
    ]
    .into_iter()
    .enumerate()
    {
        let index = turn % 2;
        let conn = index as u32 + 1;
        pass(&mut game, conn, S::PingRequest(status::Ping { time }));
        let packets = output(&receivers[index]);
        let echoed: Vec<_> = packets
            .iter()
            .filter_map(|p| match p {
                C::PongResponse(p) => Some(p.time),
                _ => None,
            })
            .collect();
        assert_eq!(
            echoed,
            vec![time],
            "missing or incorrect registered Game play-Ping reply"
        );
        assert_eq!(packets.len(), 1, "one request produces exactly one reply");
        assert_eq!(
            game.server
                .lan
                .as_ref()
                .unwrap()
                .hub
                .guest(conn)
                .unwrap()
                .last_action_tick,
            7,
            "transport ping does not count as a player action"
        );
        assert!(
            output(&receivers[1 - index]).is_empty(),
            "no reply to the other connection"
        );
    }
}

#[test]
#[ignore = "daily"]
fn dead_guest_play_ping_is_transport_not_an_action() {
    let mut resources = None;
    let (mut game, receivers) = fixture(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(1, |guest| guest.dead = true)
        .unwrap();
    pass(
        &mut game,
        1,
        S::PingRequest(status::Ping { time: i64::MIN }),
    );
    let packets = output(&receivers[0]);
    assert!(matches!(packets.as_slice(), [C::PongResponse(p)] if p.time == i64::MIN));
    assert!(output(&receivers[1]).is_empty());
    assert_eq!(
        game.server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(1)
            .unwrap()
            .last_action_tick,
        7
    );
}

#[test]
#[ignore = "daily"]
fn held_guest_play_ping_survives_the_acknowledgement_filter() {
    let mut resources = None;
    let (mut game, receivers) = fixture(&mut resources);
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(1, |guest| {
            guest.placed = false;
            guest.held = true;
        })
        .unwrap();
    pass(&mut game, 1, S::PingRequest(status::Ping { time: -1 }));
    assert!(matches!(output(&receivers[0]).as_slice(), [C::PongResponse(p)] if p.time == -1));
    assert!(output(&receivers[1]).is_empty());
    assert_eq!(
        game.server
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guest(1)
            .unwrap()
            .last_action_tick,
        7
    );
}

#[test]
#[ignore = "daily"]
fn handover_guest_play_ping_survives_the_acknowledgement_filter() {
    let mut resources = None;
    let (mut game, receivers) = fixture(&mut resources);
    let handover = sunrise_sim::lan::arrivals::Handover {
        id: 0x1234,
        since: 123,
        record: serde_json::json!({"fixture": "handover-state-sentinel"}),
    };
    game.server
        .lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(1, |guest| guest.handover = Some(handover.clone()))
        .unwrap();
    pass(
        &mut game,
        1,
        S::PingRequest(status::Ping {
            time: 0xfedc_ba98_7654_3210_u64 as i64,
        }),
    );
    let packets = output(&receivers[0]);
    assert!(
        matches!(packets.as_slice(), [C::PongResponse(p)]
        if p.time == 0xfedc_ba98_7654_3210_u64 as i64),
        "missing matching handover play-Ping reply"
    );
    assert!(output(&receivers[1]).is_empty());
    let guest = game.server.lan.as_ref().unwrap().hub.guest(1).unwrap();
    assert_eq!(guest.last_action_tick, 7);
    assert_eq!(guest.handover.as_ref(), Some(&handover));
}

use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;
