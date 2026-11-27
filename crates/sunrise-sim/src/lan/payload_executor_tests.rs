//! Real network executor/provider tests. No World or substitute payload context.
use crate::raw_payloads::*;
use crate::{
    lan::{NetEvent, PlayerSender},
    raw_payloads::ingress::{Consumer, Producer},
};
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes,
    PayloadContextError, PayloadId, PayloadSendError, PayloadWork, PayloadWorkStatus, Registrar,
};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::time::Duration;
use sunrise_protocol::payload_negotiation::{self as negotiation};

const CHANNEL: &str = "executor_probe:message";
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Success,
    Pressure,
    PressureWithPanickingDrop,
    Error,
    Panic,
    Cancel,
    Reconfigure,
    PublicationClosed,
}
struct Probe {
    mode: Mode,
    events: Producer<NetEvent>,
    receiver: Mutex<Option<Consumer<NetEvent>>>,
    transport: Mutex<Weak<Transport>>,
    drops: Arc<AtomicUsize>,
    attributed_drops: Arc<AtomicUsize>,
    calls: AtomicUsize,
}
struct ReentrantWork {
    epoch: Arc<connection::Epoch>,
    drops: Arc<AtomicUsize>,
    attributed_drops: Arc<AtomicUsize>,
    panic_on_drop: bool,
}
impl PayloadWork for ReentrantWork {
    fn run(&self, _: &mut dyn IPayloadContext) -> Result<(), String> {
        panic!("network dispatch must not execute queued main work inline")
    }
}
impl Drop for ReentrantWork {
    fn drop(&mut self) {
        // A publication-gate Drop mutant deadlocks here. The bounded worker
        // completion assertion below fails without joining the stuck thread.
        let _ = self.epoch.with_active(|| ());
        self.drops.fetch_add(1, Ordering::SeqCst);
        if mod_api::content::crash::current_native_call()
            .is_some_and(|call| call.id == "executor_probe" && call.hook == "payload work drop")
        {
            self.attributed_drops.fetch_add(1, Ordering::SeqCst);
        }
        if self.panic_on_drop {
            panic!("deliberate work destructor panic");
        }
    }
}
impl IPayloadHandler for Probe {
    fn handle(
        &self,
        _: PayloadBytes,
        ctx: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert!(matches!(
            ctx.server_host(),
            Err(PayloadContextError::WrongThread)
        ));
        let transport = self.transport.lock().unwrap().upgrade().unwrap();
        let pressure = matches!(self.mode, Mode::Pressure | Mode::PressureWithPanickingDrop);
        let work_count = if self.mode == Mode::PressureWithPanickingDrop {
            1
        } else {
            2
        };
        for _ in 0..work_count {
            let before = self.events.stats().reserved_bytes;
            let result = ctx.enqueue_work(Arc::new(ReentrantWork {
                epoch: transport.epoch().unwrap(),
                drops: self.drops.clone(),
                attributed_drops: self.attributed_drops.clone(),
                panic_on_drop: matches!(self.mode, Mode::Panic | Mode::PressureWithPanickingDrop),
            }));
            if pressure {
                assert_eq!(result, Err(PayloadContextError::Backpressure));
                assert_eq!(self.events.stats().reserved_bytes, before);
            } else {
                assert_eq!(result, Ok(PayloadWorkStatus::Queued));
                assert!(
                    self.events.stats().reserved_bytes > before,
                    "Queued must already own main-provider credit"
                );
            }
            assert_eq!(
                self.events.stats().queued_items,
                0,
                "no publication during callback"
            );
        }
        if self.mode != Mode::Pressure {
            ctx.reply(&PayloadId(CHANNEL.into()), PayloadBytes(vec![1]))
                .unwrap();
        }
        // Also prove that no assertion was caught as a handler panic.
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Error => {
                return Err(mod_api::PayloadHandleError::Failed(
                    "deliberate handler error".into(),
                ));
            }
            Mode::Panic => panic!("deliberate handler panic after staging"),
            Mode::Cancel => transport.close(),
            Mode::Reconfigure => transport.begin_configuration(),
            Mode::PublicationClosed => drop(self.receiver.lock().unwrap().take()),
            _ => {}
        }
        Ok(())
    }
}
struct Registration(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for Registration {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(HandlerThread::Network)
            .play_bidirectional(
                PayloadId(CHANNEL.into()),
                self.0.clone(),
                Some(self.0.clone()),
            );
    }
}
fn with_case(
    mode: Mode,
    bytes: usize,
    run: impl FnOnce(Arc<Probe>, Incoming, Endpoint, PlayerSender, Box<dyn Fn() -> Vec<Vec<u8>>>),
) {
    let (events, receiver) = if matches!(mode, Mode::Pressure | Mode::PressureWithPanickingDrop) {
        crate::lan::event_channel_with_capacity(
            sunrise_mod_runtime::byte_channel::message_overhead::<NetEvent>(),
        )
    } else {
        crate::lan::event_channel()
    };
    let probe = Arc::new(Probe {
        mode,
        events,
        receiver: Mutex::new(Some(receiver)),
        transport: Mutex::new(Weak::new()),
        drops: Arc::default(),
        attributed_drops: Arc::default(),
        calls: AtomicUsize::new(0),
    });
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "executor_probe",
            Box::new(Registration(probe.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport =
                Transport::capture(Side::Server, crate::raw_payloads::TEST_RECORD_OVERHEAD)
                    .unwrap();
            assert!(transport.local.owns(CHANNEL));
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            *probe.transport.lock().unwrap() = Arc::downgrade(&transport);
            let (sender, out) = PlayerSender::payload_executor_test_sender(transport.clone());
            let endpoint = Endpoint::server(sender.clone());
            let incoming = Incoming::new(
                &transport,
                negotiation::Phase::Play,
                sunrise_protocol::packets::custom_payload::CustomPayload {
                    channel: CHANNEL.into(),
                    data: vec![0; bytes],
                },
            )
            .unwrap();
            run(probe, incoming, endpoint, sender, Box::new(out));
        },
    );
}
fn queue(probe: &Probe, endpoint: &Endpoint) -> ServerQueue<NetEvent> {
    ServerQueue::new(probe.events.clone(), endpoint.clone(), Some(1))
}

fn reply_count(snapshot: &dyn Fn() -> Vec<Vec<u8>>) -> usize {
    let reply = encode(
        negotiation::Phase::Play,
        Side::Server,
        &PayloadId(CHANNEL.into()),
        PayloadBytes(vec![1]),
    );
    snapshot().iter().filter(|body| *body == &reply).count()
}

#[test]
fn queued_work_reserves_before_acceptance_and_keeps_original_credit_until_last_drop() {
    mod_platform::__private::initialize().unwrap();

    with_case(
        Mode::Success,
        3 << 20,
        |probe, incoming, endpoint, _, out| {
            let transport = endpoint.transport().clone();
            let original = Arc::downgrade(&incoming.lease);
            dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(probe.events.stats().queued_items, 2);
            let receiver = probe.receiver.lock().unwrap().take().unwrap();
            let mut work: Vec<_> = receiver
                .take_turn()
                .into_iter()
                .map(|receipt| receipt.into_parts())
                .collect();
            assert_eq!(work.len(), 2);
            assert!(work.iter().all(|(event, _credit)| matches!(event, NetEvent::RawPayload(delivery) if matches!(&delivery.incoming.body, Body::Work(_)))));
            assert!(original.upgrade().is_some());
            assert!(transport.reserve_ingress(14 << 20).is_err());
            drop(work.pop());
            assert_eq!(probe.drops.load(Ordering::SeqCst), 1);
            assert!(original.upgrade().is_some());
            assert!(transport.reserve_ingress(14 << 20).is_err());
            drop(work);
            assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
            assert!(original.upgrade().is_none());
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            drop(
                transport
                    .reserve_ingress(14 << 20)
                    .expect("last work reference refunds physical credit"),
            );
            assert_eq!(reply_count(&*out), 1);
        },
    );
}

#[test]
fn provider_pressure_refuses_work_before_acceptance() {
    mod_platform::__private::initialize().unwrap();

    with_case(Mode::Pressure, 1, |probe, incoming, endpoint, _, out| {
        let original = Arc::downgrade(&incoming.lease);
        dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
        assert!(original.upgrade().is_none());
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
        assert_eq!(probe.events.stats().reserved_bytes, 0);
        assert_eq!(probe.events.stats().queued_items, 0);
        assert_eq!(reply_count(&*out), 0);
    });
}

#[test]
fn a_panicking_work_destructor_on_refused_enqueue_quarantines_only_its_registration() {
    mod_platform::__private::initialize().unwrap();

    with_case(
        Mode::PressureWithPanickingDrop,
        1,
        |probe, incoming, endpoint, _, out| {
            let original = Arc::downgrade(&incoming.lease);
            let receiver = incoming.receiver.clone();
            dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
            assert_eq!(
                probe.calls.load(Ordering::SeqCst),
                1,
                "the handler ignored the refusal and staged a reply"
            );
            assert_eq!(probe.attributed_drops.load(Ordering::SeqCst), 1);
            assert!(receiver.poisoned());
            assert!(!endpoint.transport().closed());
            assert!(original.upgrade().is_none());
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            assert!(out().is_empty());
        },
    );
}

#[test]
fn handler_error_or_epoch_cancellation_publishes_no_staged_work_or_replies() {
    mod_platform::__private::initialize().unwrap();

    for mode in [Mode::Error, Mode::Panic, Mode::Cancel, Mode::Reconfigure] {
        with_case(mode, 1, |probe, incoming, endpoint, _, out| {
            let original = Arc::downgrade(&incoming.lease);
            dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
            assert!(original.upgrade().is_none());
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(probe.drops.load(Ordering::SeqCst), 2);
            assert_eq!(probe.attributed_drops.load(Ordering::SeqCst), 2);
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            assert_eq!(probe.events.stats().queued_items, 0);
            assert_eq!(reply_count(&*out), 0);
            if mode == Mode::Error {
                // The player is told which mod's handler failed, never its reason.
                let sent = out();
                let has = |text: &[u8]| {
                    sent.iter()
                        .any(|packet| packet.windows(text.len()).any(|w| w == text))
                };
                assert!(has(b"a mod's network handler failed (executor_probe)"));
                assert!(!has(b"deliberate handler error"));
            } else if mode == Mode::Panic {
                let transport = endpoint.transport();
                assert!(!transport.closed());
                assert!(out().is_empty());
                let _pressure = transport.reserve_ingress(15 << 20).unwrap();
                let data = vec![0; 1 << 20];
                assert!(matches!(
                    transport.reserve_ingress(data.capacity()),
                    Err(PayloadSendError::Backpressure)
                ));
                assert!(matches!(
                    Incoming::classify(
                        transport,
                        negotiation::Phase::Play,
                        sunrise_protocol::packets::custom_payload::CustomPayload {
                            channel: CHANNEL.into(),
                            data,
                        },
                    ),
                    Ok(Classified::Skipped)
                ));
            }
        });
    }
}

#[test]
fn main_root_blocks_later_network_delivery_and_keeps_its_ingress_charge() {
    mod_platform::__private::initialize().unwrap();
    for cancel in [None, Some(Mode::Reconfigure), Some(Mode::Cancel)] {
        with_case(
            Mode::Success,
            3 << 20,
            |probe, mut incoming, endpoint, _, out| {
                let transport = endpoint.transport();
                let tail = Incoming::new(
                    transport,
                    negotiation::Phase::Play,
                    sunrise_protocol::packets::custom_payload::CustomPayload {
                        channel: CHANNEL.into(),
                        data: vec![2],
                    },
                )
                .unwrap();
                let head_lease = Arc::downgrade(&incoming.lease);
                let tail_lease = Arc::downgrade(&tail.lease);
                incoming.receiver.thread = HandlerThread::Main;
                dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
                dispatch_server(tail, endpoint.clone(), queue(&probe, &endpoint));
                let main = queue(&probe, &endpoint);
                dispatch_pending_server(&endpoint, &main);
                assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
                assert_eq!(reply_count(&*out), 0);
                assert_eq!(probe.events.stats().queued_items, 1);
                assert!(matches!(
                    transport.reserve_ingress(14 << 20),
                    Err(PayloadSendError::Backpressure)
                ));
                assert!(tail_lease.upgrade().is_some());
                match cancel {
                    Some(Mode::Reconfigure) => transport.begin_configuration(),
                    Some(Mode::Cancel) => transport.close(),
                    _ => {}
                }
                if cancel.is_some() {
                    assert!(tail_lease.upgrade().is_none());
                    assert!(
                        head_lease.upgrade().is_some(),
                        "the main provider still owns the head"
                    );
                }
                // Game consumes the root from the provider and returns its turn at completion.
                let receiver = probe.receiver.lock().unwrap().take().unwrap();
                drop(receiver.take_turn());
                assert!(head_lease.upgrade().is_none());
                dispatch_pending_server(&endpoint, &main);
                assert_eq!(
                    probe.calls.load(Ordering::SeqCst),
                    usize::from(cancel.is_none())
                );
                assert_eq!(reply_count(&*out), usize::from(cancel.is_none()));
                assert!(!transport.connection.as_ref().unwrap().payloads_pending());
                drop(receiver.take_turn());
                assert!(tail_lease.upgrade().is_none());
                assert_eq!(probe.events.stats().reserved_bytes, 0);
            },
        );
    }
}

#[test]
fn failed_publication_and_remaining_work_drop_outside_epoch_gate() {
    mod_platform::__private::initialize().unwrap();

    with_case(
        Mode::PublicationClosed,
        1,
        |probe, incoming, endpoint, _, out| {
            let original = Arc::downgrade(&incoming.lease);
            let main = queue(&probe, &endpoint);
            let (done, completion) = mpsc::channel();
            let worker = mod_api::testing::inherit_mods(move |_: &()| {
                dispatch_server(incoming, endpoint, main);
                done.send(()).unwrap();
            });
            let thread = std::thread::spawn(move || worker(&()));
            // Never join a stuck mutant: the timeout must fail this test rather
            // than hang the harness on opaque Work::drop reentering the gate.
            completion
                .recv_timeout(Duration::from_secs(5))
                .expect("dispatch deadlocked dropping opaque work under publication gate");
            thread.join().unwrap();
            assert!(original.upgrade().is_none());
            assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                probe.drops.load(Ordering::SeqCst),
                2,
                "both failed publication and unvisited staged work must be dropped"
            );
            assert_eq!(probe.events.stats().reserved_bytes, 0);
            assert_eq!(probe.events.stats().queued_items, 0);
            assert_eq!(reply_count(&*out), 0);
        },
    );
}

#[test]
fn a_retired_writer_is_reported_by_its_reservation_not_before_preflight() {
    with_case(Mode::Success, 1, |_, _, endpoint, sender, out| {
        let epoch = endpoint.transport().epoch().expect("a play epoch");
        let id = PayloadId(CHANNEL.into());
        // Retire the outbox writer while the connection and its epoch stay live.
        sender.retire_test_writer();
        // The captured writer still answers its capacity: a body past it is
        // refused for its size, not for the retired connection.
        let oversized = PayloadBytes(vec![0; super::io_pool::MAX_PENDING + 1]);
        assert!(matches!(
            endpoint.prepare(&epoch, &id, oversized),
            Err(PayloadSendError::Backpressure)
        ));
        // One that fits reaches the reservation, which is where the close shows.
        assert!(matches!(
            endpoint.prepare(&epoch, &id, PayloadBytes(vec![1; 4])),
            Err(PayloadSendError::Disconnected)
        ));
        assert_eq!(sender.pending.load(Ordering::SeqCst), 0);
        assert!(out().is_empty());
    });
}

#[test]
fn a_hosted_pairing_body_keeps_and_refunds_its_prepaid_reservation() {
    with_case(Mode::Success, 1, |_, _, endpoint, sender, out| {
        let epoch = endpoint.transport().epoch().expect("a play epoch");
        let before = sender.pending.load(Ordering::SeqCst);
        let pairing = HostedEndpoint::new(sender.clone())
            .prepare_pairing(&epoch, &PayloadId(CHANNEL.into()), PayloadBytes(vec![7; 4]))
            .expect("a hosted play sender prepares a pairing body");
        assert!(
            sender.pending.load(Ordering::SeqCst) > before,
            "the pairing body keeps its prepaid outbox credit"
        );
        assert!(out().is_empty(), "preparing publishes nothing");
        drop(pairing);
        assert_eq!(
            sender.pending.load(Ordering::SeqCst),
            before,
            "dropping an unpublished body refunds its credit"
        );
    });
}

#[test]
fn disabled_owner_refuses_before_entering_its_native_handler() {
    mod_platform::__private::initialize().unwrap();

    with_case(Mode::Success, 1, |probe, incoming, endpoint, _, out| {
        let original = Arc::downgrade(&incoming.lease);
        assert!(mod_api::__private::set_mod_enabled("executor_probe", false));
        dispatch_server(incoming, endpoint.clone(), queue(&probe, &endpoint));
        assert!(endpoint.transport().closed());
        assert!(original.upgrade().is_none());
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(probe.drops.load(Ordering::SeqCst), 0);
        assert_eq!(probe.events.stats().reserved_bytes, 0);
        assert_eq!(reply_count(&*out), 0);
    });
}

#[derive(Clone, Copy)]
enum ConfigFinishCase {
    Normal,
    EpochInvalidated,
    Disconnect,
    AdvanceSameType,
}

fn run_config_finish_case(
    case: ConfigFinishCase,
    task_types: Vec<mod_api::Identifier>,
) -> (
    Arc<crate::lan::configuration_tasks::Session>,
    Arc<Transport>,
    usize,
) {
    const CONFIG_CHANNEL: &str = "executor_probe:config";
    struct Handler {
        case: ConfigFinishCase,
        task_type: mod_api::Identifier,
        session: Mutex<Option<Arc<crate::lan::configuration_tasks::Session>>>,
        transport: Mutex<Weak<Transport>>,
        calls: AtomicUsize,
    }
    impl IPayloadHandler for Handler {
        fn handle(
            &self,
            _: PayloadBytes,
            ctx: &mut dyn IPayloadContext,
        ) -> Result<(), mod_api::PayloadHandleError> {
            let session = self.session.lock().unwrap().as_ref().unwrap().clone();
            let transport = self.transport.lock().unwrap().upgrade().unwrap();
            ctx.finish_configuration_task(&self.task_type).unwrap();
            assert!(
                !session.finished(),
                "network finish must stage until epoch-gated publication"
            );
            assert_eq!(
                ctx.finish_configuration_task(&self.task_type),
                Err(mod_api::ConfigurationTaskError::NotRunning)
            );
            match self.case {
                ConfigFinishCase::Normal => {}
                ConfigFinishCase::EpochInvalidated => {
                    // Replace the epoch on the same open transport. Finishing
                    // Configuration here would wait for this delivery to return.
                    transport.begin_configuration();
                    transport
                        .accept(transport.local.manifest.entries())
                        .unwrap();
                }
                ConfigFinishCase::Disconnect => {
                    ctx.disconnect(mod_api::Text::literal("stop"));
                }
                ConfigFinishCase::AdvanceSameType => {
                    // Simulate the server finishing ordinal 0 and starting ordinal 1
                    // of the exact same task type before network publication runs.
                    session.finish_current(0);
                    let (completed, next) = session.turn();
                    assert_eq!(completed, Some(0));
                    let next = next.unwrap();
                    assert!(next.start);
                    assert_eq!(next.ordinal, 1);
                    assert!(!session.finished());
                }
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    struct ConfigReg(Arc<Handler>);
    #[mod_api::__subscriptions]
    impl Hooks for ConfigReg {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            registrar
                .payload_handlers()
                .registrar("1")
                .executes_on(HandlerThread::Network)
                .configuration_bidirectional(
                    PayloadId(CONFIG_CHANNEL.into()),
                    self.0.clone(),
                    Some(self.0.clone()),
                );
        }
    }

    let task_type = task_types[0].clone();
    let handler = Arc::new(Handler {
        case,
        task_type,
        session: Mutex::new(None),
        transport: Mutex::new(Weak::new()),
        calls: AtomicUsize::new(0),
    });
    let mut result = None;
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "executor_probe",
            Box::new(ConfigReg(handler.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let transport =
                Transport::capture(Side::Server, crate::raw_payloads::TEST_RECORD_OVERHEAD)
                    .unwrap();
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            let (sender, _peer) =
                crate::lan::config_payloads::ConfigSender::test_sender(transport.clone());
            let session = crate::lan::configuration_tasks::Session::new(sender.clone());
            session.register(task_types);
            let (completed, turn) = session.turn();
            assert_eq!(completed, None);
            assert_eq!(turn.unwrap().ordinal, 0);

            *handler.session.lock().unwrap() = Some(session.clone());
            *handler.transport.lock().unwrap() = Arc::downgrade(&transport);

            let endpoint = Endpoint::configuration(
                sender,
                Some(Arc::downgrade(&session)
                    as std::sync::Weak<
                        dyn crate::raw_payloads::ConfigurationTask,
                    >),
            );
            let incoming = Incoming::new(
                &transport,
                negotiation::Phase::Configuration,
                sunrise_protocol::packets::custom_payload::CustomPayload {
                    channel: CONFIG_CHANNEL.into(),
                    data: vec![1],
                },
            )
            .unwrap();
            let (events, _receiver) = crate::lan::event_channel();
            let main = ServerQueue::new(events, endpoint.clone(), Some(1));
            dispatch_server(incoming, endpoint, main);
            result = Some((session, transport, handler.calls.load(Ordering::SeqCst)));
        },
    );
    result.unwrap()
}

#[test]
fn configuration_network_finish_respects_epoch_invalidation_and_disconnect() {
    let kind = mod_api::Identifier::parse("executor_probe:task").unwrap();

    let (session, transport, calls) =
        run_config_finish_case(ConfigFinishCase::EpochInvalidated, vec![kind.clone()]);
    assert_eq!(calls, 1);
    assert!(!transport.closed());
    assert!(
        !session.finished(),
        "epoch invalidation before publication must discard staged completion"
    );

    let (session, transport, calls) =
        run_config_finish_case(ConfigFinishCase::Disconnect, vec![kind.clone()]);
    assert_eq!(calls, 1);
    assert!(transport.closed());
    assert!(
        !session.finished(),
        "explicit disconnect must discard staged network completion"
    );

    let (session, transport, calls) = run_config_finish_case(ConfigFinishCase::Normal, vec![kind]);
    assert_eq!(calls, 1);
    assert!(!transport.closed());
    assert!(session.finished());
}

#[test]
fn configuration_network_finish_only_completes_the_validated_same_type_turn() {
    let kind = mod_api::Identifier::parse("executor_probe:same").unwrap();
    let (session, transport, calls) = run_config_finish_case(
        ConfigFinishCase::AdvanceSameType,
        vec![kind.clone(), kind.clone()],
    );
    assert_eq!(calls, 1);
    assert!(!transport.closed());
    assert!(
        !session.finished(),
        "staged completion for ordinal 0 must not finish ordinal 1 of the same type"
    );
    assert_eq!(session.running_ordinal_for_type(&kind), Ok(1));
    assert_eq!(session.result(), None);
}
