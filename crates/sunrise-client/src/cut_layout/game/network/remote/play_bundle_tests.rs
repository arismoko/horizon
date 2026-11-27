use super::{connect_play_test, finish_play_test, offer_test_payloads, spawn_menu_test};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadHandleError, PayloadId, PayloadSendError, PayloadWork,
    PayloadWorkStatus, Registrar,
};
use std::{
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, Instant},
};
use sunrise_client::net::Event;
use sunrise_client_gameplay::remote::chunks::Received;
use sunrise_protocol::{
    connection::ConnReader,
    connection_groups::{self, Cell, Entry, Section},
    packets::{
        Empty, configuration as cfg,
        custom_payload::CustomPayload,
        handshake, login,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use sunrise_sim::raw_payloads;
use {
    crate::game::client_side::viewed_worlds::tests::column,
    crate::game::server_side::native_entity_refs::tests::with_client_game,
    sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases,
};

pub(in crate::game) const MAIN: &str = "bundle_probe:main";
pub(in crate::game) const NETWORK: &str = "bundle_probe:network";
pub(in crate::game) const SERVER_ONLY: &str = "bundle_probe:server_only";
pub(in crate::game) const DROPPABLE: &str = "bundle_probe:droppable";
// Larger than the memory pipe's 64 KiB window: the writer cannot finish this
// frame until the inline drain witness starts reading.
pub(in crate::game) const DRAIN_LEN: usize = 128 * 1024;

pub(in crate::game) struct Witness {
    pub(in crate::game) main: ThreadId,
    pub(in crate::game) order: Mutex<Vec<&'static str>>,
    pub(in crate::game) transfer_started: Mutex<Option<mpsc::Receiver<()>>>,
    pub(in crate::game) parked_reader: Mutex<Option<JoinHandle<()>>>,
    pub(in crate::game) drain_reader: Mutex<Option<(ConnReader, Arc<raw_payloads::Local>)>>,
}

pub(in crate::game) struct Handler {
    pub(in crate::game) witness: Arc<Witness>,
    pub(in crate::game) network: bool,
}

impl IPayloadHandler for Handler {
    fn handle(
        &self,
        bytes: PayloadBytes,
        ctx: &mut dyn IPayloadContext,
    ) -> Result<(), PayloadHandleError> {
        if self.network {
            assert_eq!(thread::current().id(), self.witness.main);
            assert!(matches!(
                ctx.client_host(),
                Err(PayloadContextError::WrongThread)
            ));
            {
                if bytes.0 == [4] {
                    self.witness
                        .transfer_started
                        .lock()
                        .unwrap()
                        .take()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .expect("new connection starts before the next member");
                    self.witness
                        .order
                        .lock()
                        .unwrap()
                        .push("network after transfer");
                    return Ok(());
                }
                if bytes.0 == [7] {
                    assert_eq!(
                        ctx.reply(&PayloadId(DROPPABLE.into()), bytes),
                        Err(PayloadSendError::Disconnected)
                    );
                    self.witness.order.lock().unwrap().push("droppable refused");
                    return Ok(());
                }
                if bytes.0 == [8] {
                    let (mut reader, local) =
                        self.witness.drain_reader.lock().unwrap().take().unwrap();
                    let mut earlier = false;
                    loop {
                        match reader.recv() {
                            Ok((id, body)) => {
                                if let S::CustomPayload(p) =
                                    S::decode_with_payload_codecs(id, &body, &local.codecs).unwrap()
                                {
                                    assert_ne!(
                                        p.channel, DROPPABLE,
                                        "post-close reply joined the drain"
                                    );
                                    if p.channel == MAIN {
                                        assert!(!earlier);
                                        assert_eq!(p.data.len(), DRAIN_LEN);
                                        assert!(p.data.iter().all(|byte| *byte == 42));
                                        earlier = true;
                                    }
                                }
                            }
                            Err(error) => {
                                assert!(error.is_end_of_stream(), "expected EOF, got {error}");
                                break;
                            }
                        }
                    }
                    assert!(earlier, "pre-close write was lost");
                    self.witness
                        .order
                        .lock()
                        .unwrap()
                        .push("earlier write drained");
                    return Ok(());
                }
                if bytes.0 == [6] {
                    let reader = self.witness.parked_reader.lock().unwrap().take().unwrap();
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while !reader.is_finished() {
                        assert!(Instant::now() < deadline, "cutoff did not end the reader");
                        thread::yield_now();
                    }
                    reader.join().unwrap();
                    return Ok(());
                }
                if bytes.0 == [3] {
                    self.witness
                        .order
                        .lock()
                        .unwrap()
                        .push("network after close");
                    return Ok(());
                }
                let mut order = self.witness.order.lock().unwrap();
                assert_eq!(*order, ["main"]);
                order.push("network");
            }
            assert_eq!(
                ctx.enqueue_work(Arc::new(Work(self.witness.clone()))),
                Ok(PayloadWorkStatus::Queued)
            );
        } else {
            assert_eq!(thread::current().id(), self.witness.main);
            assert!(ctx.client_host().is_ok());
            self.witness.order.lock().unwrap().push("main");
        }
        let id = if self.network { NETWORK } else { MAIN };
        ctx.reply(&PayloadId(id.into()), bytes).unwrap();
        Ok(())
    }
}

pub(in crate::game) struct Work(pub(in crate::game) Arc<Witness>);
impl PayloadWork for Work {
    fn run(&self, ctx: &mut dyn IPayloadContext) -> Result<(), String> {
        assert_eq!(thread::current().id(), self.0.main);
        assert!(ctx.client_host().is_ok());
        self.0.order.lock().unwrap().push("work");
        ctx.reply(&PayloadId(NETWORK.into()), PayloadBytes(vec![9]))
            .unwrap();
        Ok(())
    }
}

pub(in crate::game) struct Probe(pub(in crate::game) Arc<Witness>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (id, network) in [(MAIN, false), (NETWORK, true)] {
            let handler = Arc::new(Handler {
                witness: self.0.clone(),
                network,
            });
            registrar
                .payload_handlers()
                .registrar("1")
                .executes_on(if network {
                    HandlerThread::Network
                } else {
                    HandlerThread::Main
                })
                .play_bidirectional(PayloadId(id.into()), handler.clone(), Some(handler));
        }
        let droppable = Arc::new(Handler {
            witness: self.0.clone(),
            network: true,
        });
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Network)
            .droppable()
            .play_bidirectional(
                PayloadId(DROPPABLE.into()),
                droppable.clone(),
                Some(droppable),
            );
        registrar.payload_handlers().registrar("1").play_to_server(
            PayloadId(SERVER_ONLY.into()),
            Arc::new(Handler {
                witness: self.0.clone(),
                network: false,
            }),
        );
    }
}

pub(in crate::game) fn wait_for_bundle(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) {
    game.wait_for_test_remote_event(|event| matches!(event, Event::Bundle(_)));
}

pub(in crate::game) fn with_probe(
    test: impl FnOnce(&mut sunrise_client_gameplay::game::ClientGame<'_>, Arc<Witness>),
) {
    let witness = Arc::new(Witness {
        main: thread::current().id(),
        order: Mutex::new(Vec::new()),
        transfer_started: Mutex::new(None),
        parked_reader: Mutex::new(None),
        drain_reader: Mutex::new(None),
    });
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "bundle_probe",
            Box::new(Probe(witness.clone())),
        )]
        .into(),
        || with_client_game(|game| test(game, witness)),
    );
}

pub(in crate::game) fn connect_probe(
    game: &mut sunrise_client_gameplay::game::ClientGame<'_>,
) -> (
    Arc<raw_payloads::Local>,
    sunrise_protocol::connection::ConnReader,
    sunrise_protocol::connection::ConnWriter,
) {
    let local = raw_payloads::Local::capture(raw_payloads::Side::Server).unwrap();
    let (reader, writer) = connect_play_test(game, |reader, writer| {
        offer_test_payloads(reader, writer, &local.manifest);
    });
    spawn_menu_test(game);
    (local, reader, writer)
}

#[test]
#[ignore = "daily"]
fn a_bundle_applies_its_column_and_orders_main_network_and_close() {
    with_probe(|game, witness| {
        let (local, mut reader, mut writer) = connect_probe(game);
        let block = |x| {
            C::BlockUpdate(play::BlockUpdate {
                pos: [x, 70, 5],
                state: 0,
            })
        };
        // The same packets handed individually to the vanilla-order handlers.
        game.handle_packet(column(-1, 0)).unwrap();
        assert_eq!(game.world.block(-13, 70, 5).0, 1);
        game.handle_packet(block(-13)).unwrap();
        let expected = [game.world.block(-13, 70, 5), game.world.block(-13, 71, 5)];
        assert_eq!(expected.map(|state| state.0), [0, 1]);
        let delimiter = C::BundleDelimiter(Empty);
        let payload = |channel: &str, x| {
            C::CustomPayload(CustomPayload {
                channel: channel.into(),
                data: vec![x],
            })
        };
        for packet in [
            delimiter.clone(),
            column(0, 0),
            block(3),
            payload(MAIN, 1),
            payload(NETWORK, 2),
            delimiter.clone(),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }

        // Receiving the completed turn alone applies neither the column
        // nor any payload handler on the network thread.
        wait_for_bundle(game);
        assert!(witness.order.lock().unwrap().is_empty());
        assert!(!game.world.loaded(0, 0));
        game.poll_remote_events(true).unwrap();
        assert_eq!(
            [game.world.block(3, 70, 5), game.world.block(3, 71, 5)],
            expected
        );
        assert_eq!(*witness.order.lock().unwrap(), ["main", "network", "work"]);
        for (channel, value) in [(MAIN, 1), (NETWORK, 2), (NETWORK, 9)] {
            loop {
                let (id, body) = reader.recv().unwrap();
                if let S::CustomPayload(p) =
                    S::decode_with_payload_codecs(id, &body, &local.codecs).unwrap()
                {
                    assert_eq!(
                        (p.channel.as_str(), p.data.as_slice()),
                        (channel, [value].as_slice())
                    );
                    break;
                }
            }
        }

        // A Disconnect member closes the connection, not the iteration.
        // A cookie and a typed handler still run before the listener is removed.
        let cookies = game.client.remote.as_ref().unwrap().session.cookies.clone();
        let cookie = "bundle_probe:after_close";
        for packet in [
            delimiter.clone(),
            C::Disconnect(cfg::Disconnect {
                reason: sunrise_core::nbt::Tag::String("bundle closed".into()),
            }),
            C::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![17],
            }),
            C::Ping(cfg::Ping { id: 179 }),
            payload(NETWORK, 3),
            delimiter,
            payload(MAIN, 5),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        // Have the separate MAIN payload queued behind the bundle before
        // processing either one: only the inline handlers survive the close.
        game.wait_for_test_remote_event(|event| matches!(event, Event::RawPayload(_)));
        game.poll_remote_events(true).unwrap();
        assert!(game.client.remote.is_none());
        assert_eq!(cookies.lock().unwrap().get(cookie), Some(&vec![17]));
        assert_eq!(
            *witness.order.lock().unwrap(),
            ["main", "network", "work", "network after close"]
        );
        loop {
            match reader.recv() {
                Ok((id, body)) => assert!(!matches!(
                    S::decode_with_payload_codecs(id, &body, &local.codecs).unwrap(),
                    S::Pong(pong) if pong.id == 179
                )),
                Err(error) => {
                    assert!(error.is_end_of_stream(), "expected EOF, got {error}");
                    break;
                }
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_post_disconnect_droppable_reply_cannot_join_the_writer_drain() {
    with_probe(|game, witness| {
        let (local, reader, mut writer) = connect_probe(game);
        let session = &game.client.remote.as_ref().unwrap().session;
        let endpoint = session.payload_endpoint();
        let epoch = session.payload_transport().epoch().unwrap();
        endpoint
            .prepare(
                &epoch,
                &PayloadId(MAIN.into()),
                PayloadBytes(vec![42; DRAIN_LEN]),
            )
            .unwrap()
            .publish()
            .unwrap();
        *witness.drain_reader.lock().unwrap() = Some((reader, local.clone()));
        for packet in [
            C::BundleDelimiter(Empty),
            C::Disconnect(cfg::Disconnect {
                reason: sunrise_core::nbt::Tag::String("bundle closed".into()),
            }),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![7],
            }),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![8],
            }),
            C::BundleDelimiter(Empty),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        wait_for_bundle(game);
        game.poll_remote_events(true).unwrap();
        assert!(game.client.remote.is_none());
        assert_eq!(
            *witness.order.lock().unwrap(),
            ["droppable refused", "earlier write drained"]
        );
    });
}

pub(in crate::game) fn accept_transfer_login(
    listener: std::net::TcpListener,
) -> sunrise_protocol::connection::Connection {
    let mut connection = accept_tcp(listener);
    let (id, body) = connection.recv().unwrap();
    let handshake::Serverbound::Intention(packet) =
        handshake::Serverbound::decode(id, &body).unwrap()
    else {
        panic!("expected transfer handshake")
    };
    assert_eq!(packet.intent, handshake::TRANSFER);
    let (id, body) = connection.recv().unwrap();
    assert!(matches!(
        login::Serverbound::decode(id, &body).unwrap(),
        login::Serverbound::Hello(_)
    ));
    connection
}

pub(in crate::game) fn login_cookie(
    connection: &mut sunrise_protocol::connection::Connection,
    key: &str,
) -> Option<Vec<u8>> {
    connection
        .send(&login::Clientbound::CookieRequest(login::CookieRequest { key: key.into() }).encode())
        .unwrap();
    let (id, body) = connection.recv().unwrap();
    let login::Serverbound::CookieResponse(response) =
        login::Serverbound::decode(id, &body).unwrap()
    else {
        panic!("expected transfer login cookie reply")
    };
    assert_eq!(response.key, key);
    response.payload
}

pub(in crate::game) fn transfer_to(target: std::net::SocketAddr) -> C {
    C::Transfer(cfg::Transfer {
        host: target.ip().to_string(),
        port: target.port().into(),
    })
}

#[test]
#[ignore = "daily"]
fn a_grouped_column_split_across_tcp_reads_is_adopted_as_one_bundle() {
    use std::sync::atomic::Ordering;
    with_probe(|game, _| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        game.connect_remote_session(&address, "BundleProbe", None, None, false, false, None)
            .unwrap();
        let local = raw_payloads::Local::capture(raw_payloads::Side::Server).unwrap();
        let (_reader, mut writer) =
            finish_play_test(game, accept_tcp(listener), |reader, writer| {
                offer_test_payloads(reader, writer, &local.manifest);
            });
        spawn_menu_test(game);
        let counters = game
            .client
            .remote
            .as_ref()
            .unwrap()
            .session
            .counters
            .lock()
            .unwrap()
            .clone()
            .unwrap();
        let group = C::CustomPayload(CustomPayload {
            channel: connection_groups::CHANNEL.into(),
            data: Section {
                pos: [0, 4, 0],
                entries: vec![Entry {
                    cell: Cell::of([3, 70, 5]),
                    group: Some(78),
                }],
            }
            .encode()
            .unwrap(),
        });
        let delimiter = C::BundleDelimiter(Empty).encode();
        let mut frames = Vec::new();
        for body in [
            delimiter.clone(),
            column(0, 0).encode(),
            group.try_encode_with_payload_codecs(&local.codecs).unwrap(),
        ] {
            writer.frame_into(&writer.contents(&body).unwrap(), &mut frames);
        }
        let before = counters.bytes_in.load(Ordering::Relaxed);
        let (prefix, suffix) = frames.split_at(frames.len() - 1);
        let wait_bytes = |sent| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while counters.bytes_in.load(Ordering::Relaxed) < before + sent as i64 {
                assert!(
                    Instant::now() < deadline,
                    "client did not read the fragment"
                );
                thread::yield_now();
            }
        };
        writer.write_raw(prefix).unwrap();
        wait_bytes(prefix.len());
        game.poll_remote_events(false).unwrap();
        assert!(!game.world.loaded(0, 0));

        writer.write_raw(suffix).unwrap();
        wait_bytes(frames.len());
        game.poll_remote_events(false).unwrap();
        assert!(
            !game.world.loaded(0, 0),
            "no adoption before the closing delimiter"
        );

        writer.send(&delimiter).unwrap();
        wait_for_bundle(game);
        game.poll_remote_events(true).unwrap();
        assert!(game.world.loaded(0, 0));
        assert_eq!(game.world.block(3, 70, 5).0, 1);
        assert_eq!(game.world.connection_group(3, 70, 5), Some(78));
    });
}

pub(in crate::game) fn accept_tcp(
    listener: std::net::TcpListener,
) -> sunrise_protocol::connection::Connection {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "no test connection");
                thread::yield_now();
            }
            Err(error) => panic!("test connection accept: {error}"),
        }
    };
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    sunrise_protocol::connection::Connection::from_stream(stream).unwrap()
}

#[test]
#[ignore = "daily"]
fn a_bundled_transfer_starts_the_next_session_before_later_members() {
    with_probe(|game, witness| {
        let (local, _reader, mut writer) = connect_probe(game);
        // The in-memory fixture represents a multiplayer source here.
        let source = &mut game.client.remote.as_mut().unwrap().session;
        source.address = "source.invalid".into();
        let old = source.cookies.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let destination = listener.local_addr().unwrap();
        let unexpected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        unexpected.set_nonblocking(true).unwrap();
        let (started, observed) = mpsc::channel();
        *witness.transfer_started.lock().unwrap() = Some(observed);
        let queued = "bundle_probe:after_bundle";
        let rejected = CustomPayload {
            channel: SERVER_ONLY.into(),
            data: vec![0],
        };
        assert!(matches!(
            raw_payloads::Incoming::classify(
                source.payload_endpoint().transport(),
                sunrise_protocol::payload_negotiation::Phase::Play,
                rejected.clone(),
            ),
            Err(raw_payloads::Failure {
                error: mod_api::PayloadSendError::WrongDirection(_),
                ..
            })
        ));
        let cookie = "bundle_probe:transfer";
        old.lock().unwrap().insert(cookie.into(), vec![16]);
        let target = thread::spawn(move || {
            let target = accept_transfer_login(listener);
            started.send(()).unwrap();
            target
        });
        for packet in [
            C::BundleDelimiter(Empty),
            transfer_to(destination),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![4],
            }),
            C::CustomPayload(rejected),
            C::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![27],
            }),
            transfer_to(unexpected.local_addr().unwrap()),
            C::BundleDelimiter(Empty),
            C::StoreCookie(cfg::StoreCookie {
                key: queued.into(),
                payload: vec![42],
            }),
            C::Ping(cfg::Ping { id: 33 }),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        game.wait_for_test_remote_event(|event| {
            matches!(event, Event::Packet(packet) if matches!(&**packet, C::Ping(p) if p.id == 33))
        });
        game.poll_remote_events(true).unwrap();
        assert_eq!(*witness.order.lock().unwrap(), ["network after transfer"]);
        let next = &game.client.remote.as_ref().unwrap().session;
        assert_eq!(next.address, destination.to_string());
        assert_eq!(old.lock().unwrap().get(cookie), Some(&vec![27]));
        assert_eq!(old.lock().unwrap().get(queued), Some(&vec![42]));
        // Probe the same accepted socket after the old listener's cookie writes.
        let mut target = target.join().unwrap();
        assert_eq!(login_cookie(&mut target, cookie), Some(vec![16]));
        assert!(
            matches!(unexpected.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
    });
}

#[test]
#[ignore = "daily"]
fn standalone_transfer_keeps_its_earlier_cookie_even_after_source_eof() {
    const NAME: &str = "cut_layout::game::remote::tests::play_bundle_tests::standalone_transfer_keeps_its_earlier_cookie_even_after_source_eof";
    if run_fresh_cases(NAME, "SUNRISE_TRANSFER_COOKIE_CHILD", &["cookie"]) {
        return;
    }
    with_client_game(|game| {
        for eof in [false, true] {
            let (mut reader, mut writer) = connect_play_test(game, |_, _| {});
            spawn_menu_test(game);
            let source = &mut game.client.remote.as_mut().unwrap().session;
            source.address = "source.invalid".into();
            let old = source.cookies.clone();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let destination = listener.local_addr().unwrap();
            let bundled = "bundle_probe:bundled";
            let standalone = "bundle_probe:standalone";
            let late = "bundle_probe:after_transfer";
            let target = thread::spawn(move || accept_transfer_login(listener));
            for packet in [
                C::BundleDelimiter(Empty),
                C::StoreCookie(cfg::StoreCookie {
                    key: bundled.into(),
                    payload: vec![1],
                }),
                C::StoreCookie(cfg::StoreCookie {
                    key: standalone.into(),
                    payload: vec![0],
                }),
                C::BundleDelimiter(Empty),
                C::StoreCookie(cfg::StoreCookie {
                    key: standalone.into(),
                    payload: vec![2],
                }),
                C::CookieRequest(cfg::CookieRequest {
                    key: standalone.into(),
                }),
                transfer_to(destination),
                C::StoreCookie(cfg::StoreCookie {
                    key: late.into(),
                    payload: vec![43],
                }),
                C::Ping(cfg::Ping { id: 34 }),
            ] {
                writer.send(&packet.encode()).unwrap();
            }
            game.wait_for_test_remote_event(|event| {
                matches!(event, Event::Packet(packet) if matches!(&**packet, C::Ping(p) if p.id == 34))
            });
            assert!(
                old.lock().unwrap().is_empty(),
                "cookies run on main, not receive"
            );
            if eof {
                writer.shutdown();
                game.wait_for_test_remote_event(|event| {
                    matches!(event, Event::Disconnected { .. })
                });
            }
            game.poll_remote_events(true).unwrap();
            assert_eq!(
                game.client.remote.as_ref().unwrap().session.address,
                destination.to_string()
            );
            // The login listener has already copied the cookies when it sends Hello.
            let mut target = target.join().unwrap();
            assert_eq!(login_cookie(&mut target, bundled), (!eof).then(|| vec![1]));
            assert_eq!(login_cookie(&mut target, standalone), Some(vec![2]));
            // The admitted tail can drain before or after the login listener takes its copy.
            // Write a separate cookie after Hello to distinguish a snapshot from a shared map.
            let after_hello = "bundle_probe:after_hello";
            old.lock().unwrap().insert(after_hello.into(), vec![44]);
            assert_eq!(login_cookie(&mut target, after_hello), None);
            assert_eq!(old.lock().unwrap().get(standalone), Some(&vec![2]));
            assert_eq!(old.lock().unwrap().get(late), Some(&vec![43]));
            if !eof {
                loop {
                    let (id, body) = reader.recv().unwrap();
                    if let S::CookieResponse(response) = S::decode(id, &body).unwrap() {
                        assert_eq!(response.key, standalone);
                        assert_eq!(response.payload, Some(vec![2]));
                        break;
                    }
                }
            }
            drop(target);
            game.leave_server(None).unwrap();
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_second_admitted_standalone_transfer_keeps_the_first_opening() {
    const NAME: &str = "cut_layout::game::remote::tests::play_bundle_tests::a_second_admitted_standalone_transfer_keeps_the_first_opening";
    if run_fresh_cases(NAME, "SUNRISE_TRANSFER_FIRST_OPENING_CHILD", &["opening"]) {
        return;
    }
    with_client_game(|game| {
        let (_reader, mut writer) = connect_play_test(game, |_, _| {});
        spawn_menu_test(game);
        game.client.remote.as_mut().unwrap().session.address = "source.invalid".into();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let first = listener.local_addr().unwrap();
        let unexpected = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        unexpected.set_nonblocking(true).unwrap();
        let target = thread::spawn(move || accept_transfer_login(listener));
        for packet in [
            transfer_to(first),
            transfer_to(unexpected.local_addr().unwrap()),
            C::Ping(cfg::Ping { id: 35 }),
        ] {
            writer.send(&packet.encode()).unwrap();
        }
        game.wait_for_test_remote_event(|event| {
            matches!(event, Event::Packet(packet) if matches!(&**packet, C::Ping(p) if p.id == 35))
        });
        writer.shutdown();
        game.wait_for_test_remote_event(|event| matches!(event, Event::Disconnected { .. }));
        let source = game.client.remote.as_ref().unwrap();
        assert!(!source.session.accepts_packets());
        let admitted = source.backlog.iter().filter_map(|item| match item {
            Received::Event(Event::Transfer(task), _) => Some(task.is_eligible()),
            _ => None,
        });
        assert_eq!(admitted.collect::<Vec<_>>(), [true, true]);
        game.poll_remote_events(true).unwrap();
        let target = target.join().unwrap();
        assert_eq!(
            game.client.remote.as_ref().unwrap().session.address,
            first.to_string()
        );
        assert!(
            matches!(unexpected.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        drop(target);
    });
}

#[test]
#[ignore = "daily"]
fn a_parked_main_payload_does_not_retire_the_bundle_listener() {
    with_probe(|game, witness| {
        let (local, _reader, mut writer) = connect_probe(game);
        let old = game.client.remote.as_ref().unwrap().session.cookies.clone();
        let key = "bundle_probe:after_refusal";
        for packet in [
            C::BundleDelimiter(Empty),
            C::CustomPayload(CustomPayload {
                channel: SERVER_ONLY.into(),
                data: vec![0],
            }),
            C::StoreCookie(cfg::StoreCookie {
                key: key.into(),
                payload: vec![45],
            }),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![6],
            }),
            C::CustomPayload(CustomPayload {
                channel: NETWORK.into(),
                data: vec![3],
            }),
            C::BundleDelimiter(Empty),
        ] {
            writer
                .send(
                    &packet
                        .try_encode_with_payload_codecs(&local.codecs)
                        .unwrap(),
                )
                .unwrap();
        }
        wait_for_bundle(game);
        let source = &mut game.client.remote.as_mut().unwrap().session;
        let held = source.hold_test_ingress_credit();
        writer
            .send(
                &C::CustomPayload(CustomPayload {
                    channel: MAIN.into(),
                    data: vec![20],
                })
                .try_encode_with_payload_codecs(&local.codecs)
                .unwrap(),
            )
            .unwrap();
        *witness.parked_reader.lock().unwrap() = Some(source.take_parked_test_reader());
        // The first inline NETWORK handler joins the awakened reader. Only
        // then can the second one classify and dispatch against the old epoch.
        game.poll_remote_events(true).unwrap();
        drop(held);
        assert!(game.client.remote.is_none());
        assert_eq!(old.lock().unwrap().get(key), Some(&vec![45]));
        assert_eq!(*witness.order.lock().unwrap(), ["network after close"]);
    });
}
