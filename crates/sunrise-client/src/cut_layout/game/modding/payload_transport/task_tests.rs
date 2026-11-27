#![cfg(any())]

//! Configuration admission through the serving Game and its payload executors.
#[cfg(test)]
use mod_api::ModEntry;
use mod_api::{
    ConfigurationTask, ConfigurationTaskContext, ConfigurationTaskError, Hooks, IPayloadContext,
    IPayloadHandler, Identifier, PayloadBytes, PayloadContextError, PayloadId, Registrar,
    ServerHost,
};
#[cfg(test)]
use std::time::Instant;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
const CHANNEL: &str = "task_fixture:value";
const PEER_NAMES: [&str; 2] = ["ConfigPeerA", "ConfigPeerB"];
fn kind(name: &str) -> Identifier {
    Identifier::parse(&format!("task_fixture:{name}")).unwrap()
}
#[derive(Clone, Copy)]
enum Case {
    Reply,
    NetworkReply,
    Stale,
    Failure,
    Vanilla,
    Tick,
    Panic,
    Duplicates,
    SendRefusal,
    SendRefusalPanic,
    ConcurrentPeers,
    RetainedConnection,
}
struct Fixture {
    case: Case,
    connection: Mutex<Option<mod_api::ConfigurationConnection>>,
    starts: Mutex<Vec<Identifier>>,
    joins: Mutex<Vec<String>>,
    type_reads: AtomicUsize,
    panic_staged_payload: AtomicBool,
}
struct Mod(Arc<Fixture>);
struct Reply(Arc<Fixture>);
impl IPayloadHandler for Reply {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        if matches!(self.0.case, Case::NetworkReply) {
            assert_ne!(std::thread::current().name(), Some("Server thread"));
            assert!(matches!(
                context.server_host(),
                Err(PayloadContextError::WrongThread)
            ));
        } else {
            assert_eq!(std::thread::current().name(), Some("Server thread"));
        }
        let reply = match bytes.0.as_slice() {
            [7] => {
                assert_eq!(*self.0.starts.lock().unwrap(), [kind("first")]);
                let connection = self.0.connection.lock().unwrap().unwrap();
                // Main-thread completion takes effect directly in the session without
                // waiting for handler return or transaction close, so it is neither
                // deferred nor rolled back when an inner child transaction aborts.
                let _: Result<(), _> = context.transaction(|context, _| {
                    context.finish_configuration_task(&kind("first")).unwrap();
                    Err(())
                });
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::NotRunning)
                );
                // Sending is scoped to the live configuration connection, not
                // to whether a task is running on it.
                context
                    .server_host()
                    .unwrap()
                    .configuration_send_payload(
                        connection,
                        &PayloadId(CHANNEL.into()),
                        PayloadBytes(vec![77]),
                    )
                    .unwrap();
                8
            }
            [9] => {
                assert_eq!(*self.0.starts.lock().unwrap(), [kind("first")]);
                assert_eq!(
                    context.finish_configuration_task(&kind("second")),
                    Err(ConfigurationTaskError::WrongType)
                );
                context.finish_configuration_task(&kind("first")).unwrap();
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::NotRunning)
                );
                10
            }
            [11] => {
                assert_eq!(
                    *self.0.starts.lock().unwrap(),
                    [kind("first"), kind("second")]
                );
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::WrongType)
                );
                12
            }
            [12] => {
                context.finish_configuration_task(&kind("second")).unwrap();
                assert_eq!(
                    context.finish_configuration_task(&kind("second")),
                    Err(ConfigurationTaskError::NotRunning)
                );
                13
            }
            [14] => {
                // B is running `second` and attempts to finish A's running `first` task.
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::WrongType)
                );
                22
            }
            [15] => {
                context.finish_configuration_task(&kind("first")).unwrap();
                17
            }
            [16] => {
                context.finish_configuration_task(&kind("second")).unwrap();
                17
            }
            [18] => {
                assert_eq!(*self.0.starts.lock().unwrap(), [kind("first")]);
                assert_eq!(
                    context.finish_configuration_task(&kind("second")),
                    Err(ConfigurationTaskError::WrongType)
                );
                context.finish_configuration_task(&kind("first")).unwrap();
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::NotRunning)
                );
                19
            }
            [20] => {
                assert_eq!(
                    *self.0.starts.lock().unwrap(),
                    [kind("first"), kind("second")]
                );
                assert_eq!(
                    context.finish_configuration_task(&kind("first")),
                    Err(ConfigurationTaskError::WrongType)
                );
                context.finish_configuration_task(&kind("second")).unwrap();
                assert_eq!(
                    context.finish_configuration_task(&kind("second")),
                    Err(ConfigurationTaskError::NotRunning)
                );
                21
            }
            other => panic!("unexpected configuration request: {other:?}"),
        };
        context
            .reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![reply]))
            .unwrap();
        Ok(())
    }
}
#[sunrise_mod_api_macros::__subscriptions]
impl Hooks for Mod {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        let thread = if matches!(self.0.case, Case::NetworkReply) {
            mod_api::HandlerThread::Network
        } else {
            mod_api::HandlerThread::Main
        };
        registrar
            .payload_handlers()
            .registrar("1")
            .optional()
            .executes_on(thread)
            .configuration_bidirectional(
                PayloadId(CHANNEL.into()),
                Arc::new(Reply(self.0.clone())),
                Some(Arc::new(Reply(self.0.clone()))),
            );
    }
    fn register_configuration_tasks(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::RegisterConfigurationTasksEvent<'_>,
    ) {
        let task = |id, slot| Task {
            fixture: self.0.clone(),
            id,
            slot,
            waiting: false,
        };
        if matches!(self.0.case, Case::ConcurrentPeers) {
            if let Some(slot) = PEER_NAMES.iter().position(|&name| name == event.name) {
                let id = if slot == 0 {
                    kind("first")
                } else {
                    kind("second")
                };
                event.register(task(id, slot));
            }
            return;
        }
        if event.name != "ConfigPeer" {
            return;
        }
        *self.0.connection.lock().unwrap() = Some(event.connection);
        if matches!(self.0.case, Case::Vanilla) {
            assert!(
                !event
                    .connection
                    .has_channel(host, &PayloadId(CHANNEL.into()))
            );
            return;
        }
        let first = task(kind("first"), 0);
        if matches!(self.0.case, Case::Panic) {
            event.register(PanickingTask(first));
        } else {
            event.register(first);
        }
        event.register(task(kind("second"), 1));
        if matches!(self.0.case, Case::Duplicates) {
            event.register(task(kind("first"), 0));
        }
    }
    fn player_logged_in(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerLoggedInEvent<'_>,
    ) {
        if PEER_NAMES.contains(&event.name) {
            self.0.joins.lock().unwrap().push(event.name.into());
        }
    }
}

struct Task {
    fixture: Arc<Fixture>,
    id: Identifier,
    slot: usize,
    waiting: bool,
}
impl ConfigurationTask for Task {
    fn task_type(&self) -> &Identifier {
        self.fixture.type_reads.fetch_add(1, Ordering::Relaxed);
        &self.id
    }
    fn start(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>) {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        self.fixture.starts.lock().unwrap().push(self.id.clone());
        match (self.fixture.case, self.slot) {
            (Case::Reply, 0)
            | (Case::NetworkReply, _)
            | (Case::Stale, _)
            | (Case::ConcurrentPeers, _) => {
                let channel = PayloadId(CHANNEL.into());
                assert!(context.has_channel(host, &channel));
                let challenge = match (self.fixture.case, self.slot) {
                    (Case::Reply, _) => 7,
                    (Case::Stale, 0) => 9,
                    (Case::Stale, _) => 11,
                    (Case::NetworkReply, 0) => 18,
                    (Case::NetworkReply, _) => 20,
                    (_, slot) => 15 + slot as u8,
                };
                context
                    .send_payload(host, &channel, PayloadBytes(vec![challenge]))
                    .unwrap();
            }
            (Case::Failure, 0) => {
                context.finish();
                context.fail("fixture failure");
                context.finish(); // Cannot overwrite failure.
            }
            (Case::RetainedConnection, 0) => {
                context
                    .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(vec![1]))
                    .unwrap();
            }
            (Case::Tick, 0) => self.waiting = true,
            (Case::Panic, 0) => {
                context.finish();
                context
                    .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(vec![99]))
                    .unwrap();
                self.fixture
                    .panic_staged_payload
                    .store(true, Ordering::Relaxed);
                panic!("fixture panic after staging completion and a payload");
            }
            (Case::SendRefusal, 0) => {
                let channel = PayloadId(CHANNEL.into());
                assert!(!context.has_channel(host, &channel));
                assert!(matches!(
                    context.send_payload(host, &channel, PayloadBytes(vec![42])),
                    Err(ConfigurationTaskError::Payload(_))
                ));
                context.finish();
            }
            (Case::SendRefusalPanic, 0) => {
                context
                    .send_payload(host, &PayloadId(CHANNEL.into()), PayloadBytes(vec![42]))
                    .unwrap();
                context.finish();
            }
            _ => context.finish(),
        }
    }
    fn tick(&mut self, _host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>) {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        if self.waiting {
            self.waiting = false;
            context.finish();
            context.finish();
        }
    }
}

// A distinct implementation lets the same mod's other task survive the panic latch.
struct PanickingTask(Task);
impl ConfigurationTask for PanickingTask {
    fn task_type(&self) -> &Identifier {
        self.0.task_type()
    }

    fn start(&mut self, host: &mut dyn ServerHost, context: &mut ConfigurationTaskContext<'_>) {
        self.0.start(host, context);
    }
}

fn vanilla_peer(
    port: u16,
) -> (
    sunrise_protocol::connection::Connection,
    sunrise_protocol::packets::configuration::SelectKnownPacks,
) {
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, handshake, login},
    };
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(10)).unwrap();
    c.reader
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: sunrise_protocol::PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .unwrap();
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "ConfigPeer".into(),
            uuid: sunrise_protocol::crypto::offline_uuid("ConfigPeer"),
        })
        .encode(),
    )
    .unwrap();
    loop {
        let (id, body) = c.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => break,
            p => panic!("unexpected login packet {p:?}"),
        }
    }
    c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();

    loop {
        let (id, body) = c.recv().unwrap();
        if let cfg::Clientbound::SelectKnownPacks(known) =
            cfg::Clientbound::decode(id, &body).unwrap()
        {
            return (c, known);
        }
    }
}

#[cfg(any())]
fn publish_task_server(game: &mut sunrise_client_gameplay::game::ClientGame<'_>) -> u16 {
    game.client.settings.render_distance = 2;
    game.client.settings.simulation_distance = 2;
    let options = sunrise_sim::world_options::WorldOptions {
        world_type: sunrise_sim::save::WorldPreset::Flat,
        ..Default::default()
    };
    game.open_integrated(
        "config_tasks",
        Some("Configuration tasks"),
        Some(1),
        true,
        Some(&options),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while !game
        .client
        .remote
        .as_ref()
        .is_some_and(|r| r.logged_in && r.joined)
    {
        game.poll_remote().unwrap();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    game.client
        .integrated
        .as_ref()
        .unwrap()
        .call(|server| {
            server.publish(sunrise_gameplay::game::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            })
        })
        .unwrap()
        .unwrap()
}

impl Fixture {
    fn new(case: Case) -> Self {
        Self {
            case,
            connection: Mutex::new(None),
            starts: Mutex::new(Vec::new()),
            joins: Mutex::new(Vec::new()),
            type_reads: AtomicUsize::new(0),
            panic_staged_payload: AtomicBool::new(false),
        }
    }
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn serving_game_drives_configuration_tasks() {
    const NAME: &str =
        "cut_layout::game::payload_transport::task_tests::serving_game_drives_configuration_tasks";
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TASK_CHILD",
        &[
            "reply",
            "network_reply",
            "stale",
            "failure",
            "vanilla",
            "tick",
            "panic",
            "duplicates",
            "send_refusal",
            "send_refusal_panic",
        ],
    ) {
        return;
    }
    let case = match std::env::var("SUNRISE_TASK_CHILD").unwrap().as_str() {
        "reply" => Case::Reply,
        "network_reply" => Case::NetworkReply,
        "stale" => Case::Stale,
        "failure" => Case::Failure,
        "vanilla" => Case::Vanilla,
        "tick" => Case::Tick,
        "panic" => Case::Panic,
        "duplicates" => Case::Duplicates,
        "send_refusal" => Case::SendRefusal,
        "send_refusal_panic" => Case::SendRefusalPanic,
        _ => unreachable!(),
    };
    // Each case is a fresh subprocess, so this installs the real process-wide crash hook.
    let crash_home =
        matches!(case, Case::Panic | Case::SendRefusalPanic).then(|| tempfile::tempdir().unwrap());
    if let Some(home) = &crash_home {
        sunrise_level::crash::install(home.path().to_owned(), "client", || None);
    }
    let fixture = Arc::new(Fixture::new(case));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "task_fixture",
            Box::new(Mod(fixture.clone())),
        )]
        .into(),
        || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                let port = publish_task_server(game);
                use sunrise_protocol::packets::{Empty, configuration as cfg, play};
                let (mut c, known) = if matches!(
                    case,
                    Case::Vanilla | Case::SendRefusal | Case::SendRefusalPanic
                ) {
                    vanilla_peer(port)
                } else {
                    let (c, _, known) = super::config_tests::configuration_peer(port, "ConfigPeer");
                    (c, known)
                };
                c.send(&cfg::Serverbound::SelectKnownPacks(known).encode())
                    .unwrap();
                let mut replies = match case {
                    Case::Reply => &[7_u8, 77, 8][..],
                    Case::NetworkReply => &[18, 19, 20, 21],
                    Case::Stale => &[9, 10, 11, 12, 13],
                    _ => &[],
                }
                .iter()
                .copied();
                loop {
                    let (id, body) = c.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                            let expected = replies.next().expect("too many configuration replies");
                            assert_eq!(p.data, [expected]);
                            if matches!(expected, 7 | 9 | 11 | 12 | 18 | 20) {
                                c.send(&cfg::Serverbound::CustomPayload(p).encode())
                                    .unwrap();
                            }
                        }
                        cfg::Clientbound::KeepAlive(p) => {
                            c.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                        }
                        cfg::Clientbound::Disconnect(reason) => {
                            let reason =
                                sunrise_protocol::text::plain(&reason.reason, &Default::default());
                            assert!(reason.contains("Configuration error"), "{reason}");
                            let starts = fixture.starts.lock().unwrap();
                            match case {
                                Case::Failure => assert_eq!(*starts, [kind("first")]),
                                Case::Panic | Case::SendRefusalPanic => {
                                    if matches!(case, Case::Panic) {
                                        assert!(
                                            fixture.panic_staged_payload.load(Ordering::Relaxed)
                                        );
                                    }
                                    assert!(
                                        reason.contains("mod task_fixture: configuration task callback panicked; it is disabled"),
                                        "{reason}"
                                    );
                                    if matches!(case, Case::Panic) {
                                        assert!(
                                            reason.contains("fixture panic after staging completion and a payload"),
                                            "{reason}"
                                        );
                                    }
                                    assert_eq!(*starts, [kind("first")]);
                                }
                                _ => panic!("unexpected configuration disconnect: {reason}"),
                            }
                            break;
                        }
                        cfg::Clientbound::FinishConfiguration(_) => {
                            assert!(!matches!(
                                case,
                                Case::Failure | Case::Panic | Case::SendRefusalPanic
                            ));
                            assert_eq!(
                                replies.next(),
                                None,
                                "FinishConfiguration overtook a reply"
                            );
                            let expected = match case {
                                Case::Vanilla => vec![],
                                Case::Duplicates => {
                                    vec![kind("first"), kind("second"), kind("first")]
                                }
                                _ => vec![kind("first"), kind("second")],
                            };
                            assert_eq!(*fixture.starts.lock().unwrap(), expected);
                            c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                                .unwrap();
                            loop {
                                let (id, body) = c.recv().unwrap();
                                if matches!(
                                    play::Clientbound::decode(id, &body).unwrap(),
                                    play::Clientbound::Login(_)
                                ) {
                                    break;
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
                if let Some(home) = &crash_home {
                    assert!(!sunrise_level::crash::server_crashed());
                    assert!(!home.path().join("crash-reports").exists());
                    assert!(mod_api::__private::take_hook_panics().contains(
                        &mod_api::__private::HookPanicReport {
                            mod_id: "task_fixture",
                            hook: "ConfigurationTask::start",
                        }
                    ));
                }
                assert_eq!(
                    fixture.type_reads.load(Ordering::Relaxed),
                    match case {
                        Case::Vanilla => 0,
                        Case::Duplicates => 3,
                        _ => 2,
                    }
                );
                if matches!(case, Case::Panic) {
                    // The same serving Game admits another peer without calling the
                    // disabled task again. Another task of that mod still runs.
                    let (mut peer, _, known) =
                        super::config_tests::configuration_peer(port, "ConfigPeer");
                    peer.send(&cfg::Serverbound::SelectKnownPacks(known).encode())
                        .unwrap();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match cfg::Clientbound::decode(id, &body).unwrap() {
                            cfg::Clientbound::FinishConfiguration(_) => break,
                            cfg::Clientbound::Disconnect(reason) => {
                                panic!("second peer refused: {reason:?}")
                            }
                            cfg::Clientbound::CustomPayload(p) => assert_ne!(p.channel, CHANNEL),
                            _ => {}
                        }
                    }
                    assert_eq!(
                        *fixture.starts.lock().unwrap(),
                        [kind("first"), kind("second")]
                    );
                    assert_eq!(
                        fixture.type_reads.load(Ordering::Relaxed),
                        3,
                        "the disabled implementation is skipped before reading its type again"
                    );
                }
                game.leave_server(None).unwrap();
            });
        },
    );
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn a_panicking_registration_does_not_fail_the_peer_or_leave_its_task() {
    use mod_api::RegisterConfigurationTasksEvent;
    use sunrise_protocol::packets::{Empty, configuration as cfg, play};
    const NAME: &str = "cut_layout::game::payload_transport::task_tests::a_panicking_registration_does_not_fail_the_peer_or_leave_its_task";
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_TASK_REGISTRATION_PANIC_CHILD",
        &["panic"],
    ) {
        return;
    }

    struct Listener {
        panics: bool,
        calls: Arc<AtomicUsize>,
        started: Arc<AtomicUsize>,
        discarded: Arc<AtomicUsize>,
    }
    struct RegistrationTask {
        id: Identifier,
        started: Arc<AtomicUsize>,
        discard_probe: Option<Arc<AtomicUsize>>,
    }
    impl ConfigurationTask for RegistrationTask {
        fn task_type(&self) -> &Identifier {
            &self.id
        }
        fn start(&mut self, _: &mut dyn ServerHost, ctx: &mut ConfigurationTaskContext<'_>) {
            self.started.fetch_add(1, Ordering::Relaxed);
            ctx.finish();
        }
    }
    impl Drop for RegistrationTask {
        fn drop(&mut self) {
            if let Some(discarded) = &self.discard_probe {
                discarded.fetch_add(1, Ordering::Relaxed);
                panic!("failed task also panics on disposal");
            }
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn register_configuration_tasks(
            &self,
            host: &mut dyn ServerHost,
            event: &mut RegisterConfigurationTasksEvent<'_>,
        ) {
            if !PEER_NAMES.contains(&event.name) {
                return;
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            event.register(RegistrationTask {
                id: kind("shared"),
                started: self.started.clone(),
                discard_probe: self.panics.then(|| self.discarded.clone()),
            });
            host.configuration_send_payload(
                event.connection,
                &PayloadId(CHANNEL.into()),
                PayloadBytes(vec![if self.panics { 99 } else { 1 }]),
            )
            .unwrap();
            assert!(!self.panics, "registration fails after appending a task");
        }
    }

    let scratch = tempfile::tempdir().unwrap();
    sunrise_level::crash::install(scratch.path().to_path_buf(), "client", || None);
    let failed = Arc::new(AtomicUsize::new(0));
    let healthy = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let discarded = Arc::new(AtomicUsize::new(0));
    let mods = vec![
        ModEntry::new(
            "broken_tasks",
            Box::new(Listener {
                panics: true,
                calls: failed.clone(),
                started: started.clone(),
                discarded: discarded.clone(),
            }),
        ),
        ModEntry::new(
            "healthy_tasks",
            Box::new(Listener {
                panics: false,
                calls: healthy.clone(),
                started: started.clone(),
                discarded: discarded.clone(),
            }),
        ),
        // Registers the same negotiated channel as the other task fixtures.
        ModEntry::new(
            "task_fixture",
            Box::new(Mod(Arc::new(Fixture::new(Case::Vanilla)))),
        ),
    ]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let port = publish_task_server(game);
            for name in PEER_NAMES {
                let (mut peer, _, packs) = super::config_tests::configuration_peer(port, name);
                peer.send(&cfg::Serverbound::SelectKnownPacks(packs).encode())
                    .unwrap();
                let mut payloads = Vec::new();
                loop {
                    let (id, body) = peer.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                            payloads.push(p.data);
                        }
                        cfg::Clientbound::KeepAlive(p) => {
                            peer.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                        }
                        cfg::Clientbound::FinishConfiguration(_) => break,
                        cfg::Clientbound::Disconnect(p) => {
                            panic!("healthy tasks could not finish admission: {p:?}")
                        }
                        _ => {}
                    }
                }
                assert_eq!(
                    payloads,
                    [vec![1]],
                    "only the healthy listener's queued send commits"
                );
                peer.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                loop {
                    let (id, body) = peer.recv().unwrap();
                    match play::Clientbound::decode(id, &body).unwrap() {
                        play::Clientbound::Login(_) => break,
                        play::Clientbound::Disconnect(p) => {
                            panic!("disconnected before Play: {p:?}")
                        }
                        _ => {}
                    }
                }
            }
            assert_eq!(failed.load(Ordering::Relaxed), 1);
            assert_eq!(healthy.load(Ordering::Relaxed), 2);
            assert_eq!(started.load(Ordering::Relaxed), 2);
            assert_eq!(discarded.load(Ordering::Relaxed), 1);
            assert!(!sunrise_level::crash::server_crashed());
            assert!(!scratch.path().join("crash-reports").exists());
            assert_eq!(
                mod_api::__private::take_hook_panics().as_slice(),
                [
                    mod_api::__private::HookPanicReport {
                        mod_id: "broken_tasks",
                        hook: "register_configuration_tasks",
                    },
                    mod_api::__private::HookPanicReport {
                        mod_id: "broken_tasks",
                        hook: "ConfigurationTask::drop",
                    },
                ],
                "registration and disposal panics are reported separately"
            );
            game.leave_server(None).unwrap();
        });
    });
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn peer_completion_rejects_other_connections() {
    use sunrise_protocol::{
        connection::Connection,
        packets::{Empty, configuration as cfg, play},
    };
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::payload_transport::task_tests::peer_completion_rejects_other_connections",
        "SUNRISE_TASK_PEER_CHILD",
        &["cross_peer"],
    ) {
        return;
    }
    let fixture = Arc::new(Fixture::new(Case::ConcurrentPeers));
    let mods = vec![ModEntry::new(
        "task_fixture",
        Box::new(Mod(fixture.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            let port = publish_task_server(game);
            let [(mut a, _, packs_a), (mut b, _, packs_b)] =
                PEER_NAMES.map(|name| super::config_tests::configuration_peer(port, name));
            a.send(&cfg::Serverbound::SelectKnownPacks(packs_a).encode())
                .unwrap();
            b.send(&cfg::Serverbound::SelectKnownPacks(packs_b).encode())
                .unwrap();

            let next = |c: &mut Connection| loop {
                let (id, body) = c.recv().unwrap();
                match cfg::Clientbound::decode(id, &body).unwrap() {
                    cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                        break Some(p.data);
                    }
                    cfg::Clientbound::KeepAlive(p) => {
                        c.send(&cfg::Serverbound::KeepAlive(p).encode()).unwrap();
                    }
                    cfg::Clientbound::FinishConfiguration(_) => break None,
                    cfg::Clientbound::Disconnect(p) => {
                        panic!("disconnected in configuration: {p:?}")
                    }
                    _ => {}
                }
            };
            let reply = |c: &mut Connection, step| {
                c.send(
                    &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: vec![step],
                    })
                    .encode(),
                )
                .unwrap();
            };

            // Both challenges are outstanding: A runs `first`, B runs `second`.
            assert_eq!(next(&mut a), Some(vec![15]));
            assert_eq!(next(&mut b), Some(vec![16]));
            {
                let starts = fixture.starts.lock().unwrap();
                assert_eq!(starts.len(), 2);
                assert!(starts.contains(&kind("first")));
                assert!(starts.contains(&kind("second")));
            }
            reply(&mut b, 14);
            assert_eq!(next(&mut b), Some(vec![22]), "B did not report WrongType");

            // A premature advance of either connection would put FinishConfiguration
            // before its own reply, or make the handler's own completion fail.
            for (peer, step) in [(&mut a, 15), (&mut b, 16)] {
                reply(peer, step);
                assert_eq!(next(peer), Some(vec![17]), "own task must still be running");
                assert_eq!(next(peer), None, "own task must finish configuration");
                peer.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                loop {
                    let (id, body) = peer.recv().unwrap();
                    match play::Clientbound::decode(id, &body).unwrap() {
                        play::Clientbound::Login(_) => break,
                        play::Clientbound::Disconnect(p) => {
                            panic!("disconnected before play login: {p:?}")
                        }
                        _ => {}
                    }
                }
            }
            // The play Login is sent in the same tick as the join announcement.
            // Read the hook's record between ticks, while both connections are still open.
            let joins = game
                .client
                .integrated
                .as_ref()
                .unwrap()
                .call(move |_| fixture.joins.lock().unwrap().clone())
                .unwrap();
            assert_eq!(joins, PEER_NAMES.map(str::to_owned));
            game.leave_server(None).unwrap();
        });
    });
}

#[cfg(any())]
#[test]
#[ignore = "daily"]
fn retained_configuration_host_send_refuses_play_while_session_is_registered() {
    use sunrise_protocol::{
        attachment_sync, packets::configuration as cfg, payload_negotiation::Phase,
    };
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        "cut_layout::game::payload_transport::task_tests::retained_configuration_host_send_refuses_play_while_session_is_registered",
        "SUNRISE_TASK_RETAINED_CONNECTION_CHILD",
        &["play"],
    ) {
        return;
    }
    let fixture = Arc::new(Fixture::new(Case::RetainedConnection));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "task_fixture",
            Box::new(Mod(fixture.clone())),
        )]
        .into(),
        || {
            crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
                let port = publish_task_server(game);
                let (mut peer, _, known) =
                    super::config_tests::configuration_peer(port, "ConfigPeer");
                peer.send(&cfg::Serverbound::SelectKnownPacks(known).encode())
                    .unwrap();
                loop {
                    let (id, body) = peer.recv().unwrap();
                    match cfg::Clientbound::decode(id, &body).unwrap() {
                        cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => break,
                        cfg::Clientbound::Disconnect(reason) => panic!("peer refused: {reason:?}"),
                        _ => {}
                    }
                }
                let connection = fixture.connection.lock().unwrap().unwrap();
                game.client
                    .integrated
                    .as_ref()
                    .unwrap()
                    .call(move |server| {
                        let session = server.configuration_session(connection).unwrap().clone();
                        let transport = session.transport();
                        assert_eq!(transport.epoch().unwrap().phase, Phase::Configuration);
                        // Do these two I/O transition steps between ticks while the
                        // configuration writer is still open and the handle is in the map.
                        session.close();
                        transport.finish_configuration().unwrap();
                        let channel = PayloadId(attachment_sync::CHANNEL.into());
                        assert_eq!(
                            transport
                                .epoch()
                                .unwrap()
                                .peer
                                .can_send(Phase::Play, &channel),
                            Ok(())
                        );
                        let (result, queued) = server.storage_scope(|server, _| {
                            let host: &mut dyn ServerHost = server;
                            let result = host.configuration_send_payload(
                                connection,
                                &channel,
                                PayloadBytes(vec![1]),
                            );
                            let queued = server.pending_transaction_payloads();
                            ((result, queued), false)
                        });
                        assert_eq!(result, Err(ConfigurationTaskError::Disconnected));
                        assert_eq!(queued, 0);
                    })
                    .unwrap();
                drop(peer);
                game.leave_server(None).unwrap();
            });
        },
    );
}

#[cfg(test)]
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;
