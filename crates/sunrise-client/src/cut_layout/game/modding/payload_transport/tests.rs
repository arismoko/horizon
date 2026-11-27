#[cfg(feature = "client")]
use mod_api::ModEntry;
#[cfg(feature = "client")]
use mod_api::{
    HandlerThread, Hooks, IPayloadContext, IPayloadHandler, PayloadBytes, PayloadContextError,
    PayloadDirection, PayloadId, PayloadProtocol, PayloadWork, PayloadWorkStatus, Registrar,
};
#[cfg(feature = "client")]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
#[cfg(feature = "client")]
use std::time::Duration;
use std::time::Instant;

#[cfg(feature = "client")]
const NAME: &str = "cut_layout::game::payload_transport::tests::integrated_memory_delivers_registered_raw_payload_and_reply";
#[cfg(feature = "client")]
const CHANNEL: &str = "raw_probe:message";
#[cfg(feature = "client")]
const WAIT_BUDGET: Duration = Duration::from_secs(60);

// Completion takes precedence when a poll itself crosses the deadline.
fn drive_until(deadline: Instant, mut poll: impl FnMut() -> bool) -> bool {
    loop {
        if poll() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
}

#[test]
fn completion_observed_on_last_poll_wins_over_deadline() {
    let expired = Instant::now();
    let mut polls = 0;
    assert!(drive_until(expired, || {
        polls += 1;
        true
    }));
    assert_eq!(polls, 1);
    assert!(!drive_until(expired, || false));
}

#[cfg(feature = "client")]
#[derive(Clone, Copy)]
struct Stamp {
    pub wall: Duration,
    cpu: Option<(u64, u64)>,
}
#[cfg(feature = "client")]
fn process_cpu_ticks() -> Option<(u64, u64)> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let mut fields = stat.rsplit_once(')')?.1.split_whitespace().skip(11);
    Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
}
#[cfg(feature = "client")]
struct Timeline {
    pub started: Instant,
    ticks_per_second: Option<u64>,
    marks: std::sync::Mutex<[Option<Stamp>; 7]>,
}
#[cfg(feature = "client")]
impl Timeline {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            ticks_per_second: std::process::Command::new("getconf")
                .arg("CLK_TCK")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .filter(|hz| *hz != 0),
            marks: std::sync::Mutex::new([None; 7]),
        }
    }
    fn mark(&self, index: usize) {
        self.marks.lock().unwrap()[index].get_or_insert_with(|| Stamp {
            wall: self.started.elapsed(),
            cpu: process_cpu_ticks(),
        });
    }
}
#[cfg(feature = "client")]
struct TimingReport {
    pub timeline: Arc<Timeline>,
    server: Arc<AtomicUsize>,
    client: Arc<AtomicUsize>,
    wire: Arc<AtomicUsize>,
    probe: sunrise_sim::raw_payloads::staging_probe::Watch,
    printed: std::sync::atomic::AtomicBool,
}
#[cfg(feature = "client")]
impl TimingReport {
    fn sample(&self) {
        eprintln!(
            "RAW_TRANSPORT_CREDIT_SAMPLE mark=5s test_elapsed_ms={} state={:?}",
            self.timeline.started.elapsed().as_millis(),
            self.probe.diagnostics()
        );
    }
    fn print(&self, capture: &str) {
        if self.printed.swap(true, Ordering::SeqCst) {
            return;
        }
        let marks = *self
            .timeline
            .marks
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let ms = marks.map(|mark| mark.map(|time| time.wall.as_millis()));
        let startup = marks[1]
            .zip(marks[0])
            .map(|(end, start)| end.wall.saturating_sub(start.wall).as_millis());
        let round_trip = marks[6]
            .zip(marks[2])
            .map(|(end, start)| end.wall.saturating_sub(start.wall).as_millis());
        let waiting = marks[2].map(|start| {
            self.timeline
                .started
                .elapsed()
                .saturating_sub(start.wall)
                .as_millis()
        });
        let cpu_ms = |ticks: Option<(u64, u64)>| {
            ticks
                .zip(self.timeline.ticks_per_second)
                .map(|((user, sys), hz)| (user * 1000 / hz, sys * 1000 / hz))
        };
        let phase_cpu = |begin: usize, end: usize| {
            marks[begin]
                .and_then(|s| s.cpu)
                .zip(marks[end].and_then(|s| s.cpu))
                .map(|((u0, s0), (u1, s1))| (u1.saturating_sub(u0), s1.saturating_sub(s0)))
        };
        eprintln!(
            "RAW_TRANSPORT_CPU case={} wall_ms={} cpu_user_sys_ms={:?} startup_cpu_user_sys_ms={:?} round_trip_cpu_user_sys_ms={:?}",
            std::env::var("SUNRISE_RAW_TRANSPORT_CHILD").unwrap(),
            self.timeline.started.elapsed().as_millis(),
            cpu_ms(process_cpu_ticks()),
            cpu_ms(phase_cpu(0, 1)),
            cpu_ms(phase_cpu(2, 6))
        );
        eprintln!(
            "RAW_TRANSPORT_TIMING capture={capture} case={} test_start_ms=0 open_ms={:?} join_ms={:?} request_started_ms={:?} request_queued_ms={:?} server_staged_ms={:?} wire_received_ms={:?} client_completed_ms={:?} startup_ms={startup:?} round_trip_ms={round_trip:?} request_elapsed_ms={waiting:?} server_hits={} client_hits={} wire_hits={} state={:?}",
            std::env::var("SUNRISE_RAW_TRANSPORT_CHILD").unwrap(),
            ms[0],
            ms[1],
            ms[2],
            ms[3],
            ms[4],
            ms[5],
            ms[6],
            self.server.load(Ordering::SeqCst),
            self.client.load(Ordering::SeqCst),
            self.wire.load(Ordering::SeqCst),
            self.probe.diagnostics()
        );
    }
}
#[cfg(feature = "client")]
struct Sampler {
    pub signal: std::sync::mpsc::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
#[cfg(feature = "client")]
impl Sampler {
    fn new(report: std::sync::Weak<TimingReport>) -> Self {
        let (signal, events) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            if events.recv() == Ok(true)
                && matches!(
                    events.recv_timeout(Duration::from_secs(5)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                )
                && let Some(report) = report.upgrade()
            {
                report.sample();
            }
        });
        Self {
            signal,
            thread: Some(thread),
        }
    }
    fn start(&self) {
        self.signal.send(true).unwrap();
    }
    fn stop(&self) {
        let _ = self.signal.send(false);
    }
}
#[cfg(feature = "client")]
impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop();
        let _ = self.thread.take().unwrap().join();
    }
}
#[cfg(feature = "client")]
impl Drop for TimingReport {
    fn drop(&mut self) {
        // Also report an unexpected panic before the normal completion/timeout path.
        self.print("drop_fallback_may_be_post_teardown");
    }
}
#[cfg(feature = "client")]
struct Probe {
    pub expected: Arc<Vec<u8>>,
    server: Arc<AtomicUsize>,
    client: Arc<AtomicUsize>,
    network: bool,
    timeline: Arc<Timeline>,
}
#[cfg(feature = "client")]
struct Handler {
    pub expected: Arc<Vec<u8>>,
    hits: Arc<AtomicUsize>,
    server: bool,
    network: bool,
    timeline: Arc<Timeline>,
}
#[cfg(feature = "client")]
struct Work {
    pub bytes: std::sync::Mutex<Option<PayloadBytes>>,
    hits: Arc<AtomicUsize>,
    server: bool,
    network_thread: std::thread::ThreadId,
    timeline: Arc<Timeline>,
}
#[cfg(feature = "client")]
impl PayloadWork for Work {
    fn run(&self, context: &mut dyn IPayloadContext) -> Result<(), String> {
        assert_ne!(
            std::thread::current().id(),
            self.network_thread,
            "work must actually switch executors"
        );
        assert_eq!(context.protocol(), PayloadProtocol::Play);
        assert!(context.player().is_ok());
        finish(
            self.bytes.lock().unwrap().take().unwrap(),
            context,
            self.server,
            &self.hits,
            &self.timeline,
        )
    }
}
#[cfg(feature = "client")]
fn finish(
    bytes: PayloadBytes,
    context: &mut dyn IPayloadContext,
    server: bool,
    hits: &AtomicUsize,
    timeline: &Timeline,
) -> Result<(), String> {
    if server {
        assert_eq!(std::thread::current().name(), Some("Server thread"));
        assert_eq!(context.flow(), PayloadDirection::ToServer);
        assert!(context.server_host().is_ok());
        assert!(matches!(
            context.client_host(),
            Err(PayloadContextError::WrongSide)
        ));
        context
            .reply(&PayloadId(CHANNEL.into()), bytes)
            .map_err(|error| format!("reply: {error:?}"))?;
    } else {
        assert_eq!(context.flow(), PayloadDirection::ToClient);
        assert!(context.client_host().is_ok());
        assert!(matches!(
            context.server_host(),
            Err(PayloadContextError::WrongSide)
        ));
    }
    timeline.mark(if server { 4 } else { 6 });
    hits.fetch_add(1, Ordering::SeqCst);
    Ok(())
}
#[cfg(feature = "client")]
impl IPayloadHandler for Handler {
    fn handle(
        &self,
        bytes: PayloadBytes,
        context: &mut dyn IPayloadContext,
    ) -> Result<(), mod_api::PayloadHandleError> {
        assert_eq!(
            &bytes.0,
            self.expected.as_ref(),
            "exact raw application bytes"
        );
        assert_eq!(context.protocol(), PayloadProtocol::Play);
        assert!(
            context.player().is_ok(),
            "a live connection supplies the player"
        );
        if self.network {
            if self.server {
                assert!(matches!(
                    context.server_host(),
                    Err(PayloadContextError::WrongThread)
                ));
                assert!(matches!(
                    context.client_host(),
                    Err(PayloadContextError::WrongSide)
                ));
            } else {
                assert!(matches!(
                    context.client_host(),
                    Err(PayloadContextError::WrongThread)
                ));
                assert!(matches!(
                    context.server_host(),
                    Err(PayloadContextError::WrongSide)
                ));
            }
            assert_eq!(
                context.enqueue_work(Arc::new(Work {
                    bytes: std::sync::Mutex::new(Some(bytes)),
                    hits: self.hits.clone(),
                    server: self.server,
                    network_thread: std::thread::current().id(),
                    timeline: self.timeline.clone(),
                })),
                Ok(PayloadWorkStatus::Queued)
            );
            Ok(())
        } else {
            finish(bytes, context, self.server, &self.hits, &self.timeline)
                .map_err(mod_api::PayloadHandleError::Failed)
        }
    }
}
#[cfg(feature = "client")]
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        registrar
            .payload_handlers()
            .registrar("1")
            .executes_on(if self.network {
                HandlerThread::Network
            } else {
                HandlerThread::Main
            })
            .play_bidirectional(
                PayloadId(CHANNEL.into()),
                Arc::new(Handler {
                    expected: self.expected.clone(),
                    hits: self.server.clone(),
                    server: true,
                    network: self.network,
                    timeline: self.timeline.clone(),
                }),
                Some(Arc::new(Handler {
                    expected: self.expected.clone(),
                    hits: self.client.clone(),
                    server: false,
                    network: self.network,
                    timeline: self.timeline.clone(),
                })),
            );
    }
}

#[cfg(feature = "client")]
#[test]
#[ignore = "daily"]
fn integrated_memory_delivers_registered_raw_payload_and_reply() {
    super::tests::require_resources();
    if sunrise_gameplay::game::native_entity_refs::tests::run_fresh_cases(
        NAME,
        "SUNRISE_RAW_TRANSPORT_CHILD",
        &["one_mib", "network", "nine_mib"],
    ) {
        return;
    }
    let (size, network) = match std::env::var("SUNRISE_RAW_TRANSPORT_CHILD")
        .unwrap()
        .as_str()
    {
        "one_mib" => (1 << 20, false),
        "network" => (1 << 20, true),
        "nine_mib" => (9 << 20, false),
        case => panic!("unknown transport fixture case {case}"),
    };
    let timeline = Arc::new(Timeline::new());
    let expected = Arc::new(
        (0..size)
            .map(|i| ((i * 31 + 7) % 251) as u8)
            .collect::<Vec<_>>(),
    );
    let server = Arc::new(AtomicUsize::new(0));
    let client = Arc::new(AtomicUsize::new(0));
    let wire_hits = Arc::new(AtomicUsize::new(0));
    let _report = Arc::new(TimingReport {
        timeline: timeline.clone(),
        server: server.clone(),
        client: client.clone(),
        wire: wire_hits.clone(),
        probe: sunrise_sim::raw_payloads::staging_probe::Watch::begin(CHANNEL),
        printed: std::sync::atomic::AtomicBool::new(false),
    });
    let sampler = Sampler::new(Arc::downgrade(&_report));
    let mods = vec![ModEntry::new(
        "raw_probe",
        Box::new(Probe {
            expected: expected.clone(),
            server: server.clone(),
            client: client.clone(),
            network,
            timeline: timeline.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::server_side::native_entity_refs::tests::with_client_game(|game| {
            game.client.settings.render_distance = 2;
            game.client.settings.simulation_distance = 2;
            let options = sunrise_sim::world_options::WorldOptions {
                world_type: sunrise_sim::save::WorldPreset::Flat,
                ..Default::default()
            };
            timeline.mark(0);
            game.open_integrated(
                "raw_transport",
                Some("Raw transport"),
                Some(1),
                true,
                Some(&options),
            )
            .unwrap();
            let ready = drive_until(Instant::now() + WAIT_BUDGET, || {
                game.poll_remote().unwrap();
                game.client.remote.as_ref().is_some_and(|remote| {
                    // The first position packet creates and binds the local player
                    // after Login. A NETWORK reply can run before the next main poll.
                    remote.logged_in
                        && remote.joined
                        && remote.session.payload_transport().player().is_some()
                })
            });
            if !ready {
                _report.print("before_teardown");
            }
            assert!(
                ready,
                "integrated play did not become ready; see timing report"
            );
            timeline.mark(1);
            assert!(
                game.client.integrated.is_some(),
                "production integrated server handle"
            );
            let wire_timeline = timeline.clone();
            game.client
                .remote
                .as_ref()
                .unwrap()
                .session
                .start_recording(
                    WireReceipt {
                        channel: CHANNEL,
                        expected: expected.clone(),
                        hits: wire_hits.clone(),
                        observed: Some(Arc::new(move || wire_timeline.mark(5))),
                    },
                    |_| {},
                );
            let deadline = Instant::now() + WAIT_BUDGET;
            timeline.mark(2);
            sampler.start();
            let host: &mut dyn mod_api::ClientHost = game;
            assert_eq!(
                host.send_payload_to_server(
                    &PayloadId(CHANNEL.into()),
                    PayloadBytes(expected.as_ref().clone())
                ),
                Ok(mod_api::PayloadAccepted::Queued)
            );
            timeline.mark(3);
            let completed = drive_until(deadline, || {
                game.poll_remote().unwrap();
                client.load(Ordering::SeqCst) != 0
            });
            sampler.stop();
            // Snapshot before unwinding or leave_server closes any connection.
            _report.print("before_teardown");
            assert!(
                completed,
                "registered round trip did not complete; server hits {}, client hits {}, wire hits {}; timing/queue report follows",
                server.load(Ordering::SeqCst),
                client.load(Ordering::SeqCst),
                wire_hits.load(Ordering::SeqCst)
            );
            assert_eq!(
                wire_hits.load(Ordering::SeqCst),
                1,
                "one independently inspected memory-transport reply"
            );
            assert_eq!(server.load(Ordering::SeqCst), 1);
            assert_eq!(client.load(Ordering::SeqCst), 1);
            game.leave_server(None).unwrap();
        });
    });
}

// These selectors are explicitly ignored, not silently optional once selected.
#[cfg(feature = "client")]
pub(super) fn require_resources() {
    let root =
        std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").expect(
            "missing SUNRISE_TEST_RESOURCES: set it to a private prepared 26.3 resource root",
        ));
    assert!(
        root.is_dir(),
        "missing SUNRISE_TEST_RESOURCES directory: {}",
        root.display()
    );
    for file in [
        "registries.json",
        "block-behaviours.json",
        "assets/minecraft/blockstates/stone.json",
    ] {
        let path = root.join(file);
        assert!(
            path.is_file(),
            "missing required prepared resource: {}",
            path.display()
        );
    }
}

// Inspect vanilla bytes independently of both the raw encoder and CustomPayload decoder.
// These fixed fixture channels have single-byte vanilla string lengths.
#[cfg(feature = "client")]
pub(super) fn assert_bare_body(body: &[u8], channel: &str, expected: &[u8]) {
    assert_eq!(
        body.len(),
        1 + channel.len() + expected.len(),
        "wire body must contain only the declared channel and application bytes"
    );
    assert_eq!(body[0], channel.len() as u8, "vanilla channel length");
    let prefix = 1 + channel.len();
    assert_eq!(
        &body[1..prefix],
        channel.as_bytes(),
        "declared wire channel"
    );
    assert!(
        body[prefix..] == *expected,
        "no envelope inside the declared channel"
    );
}

#[cfg(feature = "client")]
pub(super) struct WireReceipt {
    pub channel: &'static str,
    pub(super) expected: Arc<Vec<u8>>,
    pub(super) hits: Arc<AtomicUsize>,
    pub(super) observed: Option<Arc<dyn Fn() + Send + Sync>>,
}
#[cfg(feature = "client")]
impl sunrise_client::net::PacketRecorder for WireReceipt {
    fn is_active(&self) -> bool {
        true
    }
    fn received(
        &self,
        packet: &sunrise_protocol::packets::play::Clientbound,
        id: i32,
        body: &[u8],
    ) {
        if matches!(packet, sunrise_protocol::packets::play::Clientbound::CustomPayload(p) if p.channel == self.channel)
        {
            assert_eq!(id, sunrise_protocol::ids::play::clientbound::CUSTOM_PAYLOAD);
            assert_bare_body(body, self.channel, &self.expected);
            self.hits.fetch_add(1, Ordering::SeqCst);
            if let Some(observed) = &self.observed {
                observed();
            }
        }
    }
    fn record_packet(&self, _: i32, _: &[u8]) {}
    fn status(&self) -> sunrise_client::net::RecorderStatus {
        sunrise_client::net::RecorderStatus {
            active: true,
            path: Default::default(),
            elapsed: Duration::ZERO,
            packet_count: self.hits.load(Ordering::SeqCst) as u64,
            bytes_written: 0,
            error: None,
        }
    }
    fn stop(self: Box<Self>) -> std::io::Result<std::path::PathBuf> {
        Ok(Default::default())
    }
}
