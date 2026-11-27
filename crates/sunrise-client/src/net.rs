//! Multiplayer session: a network thread that connects to a server, logs in,
//! runs the configuration phase and then forwards decoded play packets to the
//! game thread. Keep-alives are answered on the network thread; pings wait for
//! the game thread, like ClientCommonPacketListenerImpl.handlePing, so a pong
//! cannot overtake an earlier world update or teleport acknowledgement.
//! Online-mode servers need the Microsoft account the launcher signed in
//! (crate::auth): the login joins the server through the session service and
//! encrypts the connection (ClientHandshakePacketListenerImpl.handleHello).
use crate::auth::AuthSession;
mod client_inbox;

#[cfg(test)]
mod client_payload_test_support {
    use crate::raw_payloads::connection::record_overhead;
    use crate::raw_payloads::{Side, Transport};
    use mod_api::{
        Hooks, IPayloadContext, IPayloadHandler, ModEntry, PayloadBytes, PayloadId, Registrar,
    };
    use std::sync::Arc;
    pub(super) const CHANNEL: &str = "raw_hub_probe:message";
    struct Probe;
    impl IPayloadHandler for Probe {
        fn handle(
            &self,
            _: PayloadBytes,
            _: &mut dyn IPayloadContext,
        ) -> Result<(), mod_api::PayloadHandleError> {
            Ok(())
        }
    }
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn register(&self, registrar: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            registrar
                .payload_handlers()
                .registrar("1")
                .play_to_server(PayloadId(CHANNEL.into()), Arc::new(Probe));
        }
    }
    pub(super) fn with_peer<R>(run: impl FnOnce(Arc<Transport>) -> R) -> R {
        mod_platform::testing::with_mods(
            vec![ModEntry::new("raw_hub_probe", Box::new(Probe))].into(),
            || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                let transport = Transport::capture(Side::Server, record_overhead(&[])).unwrap();
                assert!(
                    transport.local.owns(CHANNEL),
                    "fixture declaration was not installed"
                );
                transport
                    .accept(transport.local.manifest.entries())
                    .unwrap();
                transport.finish_configuration().unwrap();
                run(transport)
            },
        )
    }
    pub(super) fn with_client<R>(run: impl FnOnce(Arc<Transport>) -> R) -> R {
        mod_platform::testing::with_mods(
            vec![ModEntry::new("raw_hub_probe", Box::new(Probe))].into(),
            || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                run(Transport::capture(Side::Client, record_overhead(&[])).unwrap())
            },
        )
    }
}
pub use client_inbox::{ClientTask, ClientTasks};
#[cfg(test)]
#[path = "net/bundle_tests.rs"]
mod bundle_tests;
mod configuration;
pub use configuration::ConfigurationTask;
use configuration::{Action as ConfigurationAction, Configuration};
mod held_packets;
pub use crate::raw_payloads::ingress;
mod known_packs;
mod play_bundle;
use known_packs::{fill_known_entries, select_known_packs};
pub use play_bundle::{BundleMember, BundleStep, CompletedBundle};
#[cfg(test)]
#[path = "net/mod_packet_tests.rs"]
mod mod_packet_tests;
use crate::raw_payloads::queued_writer::{PrepareRefusal, QueuedWriter, SendFailure};
pub use crate::raw_payloads::queued_writer::{PreparedPacket, PreparedPayload};
#[cfg(test)]
use std::sync::mpsc::channel;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_protocol::{
    PROTOCOL_VERSION,
    buf::Reader,
    connection::{ConnReader, Connection, Counters},
    crypto::{generate_secret, offline_uuid, server_hash},
    item::{Names, NoNames},
    packets::{Empty, configuration as cfg, handshake, login, play},
    registry::Dynamic,
    rsa::RsaPublicKey,
    servers::parse_address,
    text::{Language, plain, plain_with, translatable},
};
use {
    sunrise_core::nbt::{JavaString, Tag},
    sunrise_world::item_defaults::ItemData,
};

use sunrise_hooks::tunnel;

/// `CommonListenerCookie.serverCookies`: cookies the server asked the client
/// to store (`ClientboundStoreCookiePacket`), answered to
/// `ClientboundCookieRequestPacket`. A `transfer` carries them to the next
/// connection (`TransferState`), so a proxy's backend switch keeps them.
pub type Cookies = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// A recording of the session's packets: the tap a replay recorder plugs into. Replay recording is a
/// platform feature (docs/layers.md, "Controllers, replays and UI pages"); the engine only
/// hands it the packets: the network thread what it receives, the game thread what the client makes for
/// the recording itself (its own movement, pose and swings).
pub trait PacketRecorder: Send {
    /// Whether it still takes packets (false once stopped or failed).
    fn is_active(&self) -> bool;
    /// A play packet the network thread received, decoded, with its id and payload as they came.
    /// The recorder decides what it keeps.
    fn received(&self, packet: &play::Clientbound, id: i32, payload: &[u8]);
    /// A packet the client made for the recording, encoded, stamped now.
    fn record_packet(&self, id: i32, payload: &[u8]);
    fn status(&self) -> RecorderStatus;
    /// Finishes the recording; the file it wrote.
    fn stop(self: Box<Self>) -> std::io::Result<PathBuf>;
}

/// Where a recording stands.
#[derive(Clone, Debug, PartialEq)]
pub struct RecorderStatus {
    pub active: bool,
    pub path: PathBuf,
    pub elapsed: Duration,
    pub packet_count: u64,
    pub bytes_written: u64,
    pub error: Option<String>,
}

/// The session's recorder, shared with its network thread.
type RecorderSlot = Arc<Mutex<Option<Box<dyn PacketRecorder>>>>;
type PingSamples = Arc<Mutex<VecDeque<i64>>>;

/// Monotonic clock shared by `PingDebugMonitor.tick` and its receive handler.
pub fn ping_millis() -> i64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_millis() as i64
}

/// What the network thread reports to the game.
pub enum Event {
    /// Progress text for the connecting screen.
    Status(String),
    /// The server's `minecraft:brand` custom payload.
    Brand(String),
    /// The server runs mods' quiet commands (`sunrise_protocol::quiet`).
    QuietCommands,
    /// The server supports private custom recipe-book synchronization.
    CustomBooks,
    /// The server samples far terrain for this client (`lod_net`).
    FarTerrain(crate::lod_net::Offer),
    /// ClientConfigurationPacketListenerImpl.handleEnabledFeatures.
    Features(Vec<String>),
    /// Configuration finished: the play state begins with these registries.
    Joined {
        registries: Arc<Dynamic>,
        datapacks: crate::datapack_net::Receiver,
        uuid: u128,
        name: String,
    },
    Packet(Box<play::Clientbound>),
    /// The reader already handled this packet. Offer only mod observation on main.
    ObservePacket(Box<play::Clientbound>),
    /// A completed vanilla bundle, charged as one ingress record.
    Bundle(Box<CompletedBundle>),
    /// Attachment sync decoded at ingress, with the original bytes for `client_packet`.
    AttachmentSync(Box<sunrise_protocol::attachment_sync::StagedMessage>),
    /// An application payload or scheduled work, to run in a main transaction.
    RawPayload(Box<crate::raw_payloads::Incoming>),
    /// A standalone play packet that failed to decode and was skipped.
    DecodeError(String),
    /// The server sent us back to configuration (`ClientboundStartConfigurationPacket`):
    /// the game shows `ServerReconfigScreen` until the server logs it in again.
    Reconfiguring(Box<PhaseChange>),
    /// A standalone cookie handler on its original common listener.
    Cookie(Box<CookieTask>),
    /// A main-affine configuration action on the current listener.
    Configuration(Box<ConfigurationTask>),
    /// Configuration finish commits the ACK and Play phase on main.
    FinishConfiguration(Box<configuration::FinishConfiguration>),
    /// The server sent us back to configuration and finished it again.
    Reconfigured {
        registries: Arc<Dynamic>,
        datapacks: crate::datapack_net::Receiver,
    },
    /// ClientboundResourcePackPushPacket, in configuration or in play.
    PackPush(cfg::ResourcePackPush),
    /// ClientboundResourcePackPopPacket: one pack, or every pack.
    PackPop(Option<u128>),
    /// `show_dialog` in configuration: the dialog itself.
    ShowDialog {
        dialog: sunrise_core::nbt::Tag,
        registries: Arc<Dynamic>,
    },
    /// `clear_dialog` in configuration.
    ClearDialog,
    /// ClientboundCodeOfConductPacket: the game shows it and answers with
    /// `Session::accept_code_of_conduct` or a disconnect; until then the
    /// server waits in configuration.
    CodeOfConduct(String),
    /// The source listener marked itself transferring before queuing this task.
    /// Main commits the handoff even if the source has since closed.
    Transfer(Box<TransferTask>),
    /// `ClientboundServerLinksPacket`: the links the pause menu's "Server
    /// Links..." lists (ClientCommonPacketListenerImpl.handleServerLinks).
    ServerLinks(Vec<cfg::ServerLink>),
    /// The connection ended. `from_server`: the server gave the reason (or
    /// the connection was refused for good), rather than the connection
    /// failing. `after_login`: the protocol already left handshake/login for
    /// configuration or play (`ClientCommonPacketListenerImpl.onDisconnect`,
    /// which calls `Minecraft.disconnect`), rather than failing in
    /// `ConnectScreen` or `ClientHandshakePacketListenerImpl`.
    Disconnected {
        reason: String,
        /// The reason's translation key, when it had one.
        key: Option<String>,
        from_server: bool,
        after_login: bool,
    },
}

const EVENT_BUDGET: usize = 96 << 20;
fn event_channel() -> (ingress::Producer<Event>, ingress::Consumer<Event>) {
    ingress::channel(EVENT_BUDGET, Event::ingress_charge).expect("positive incoming byte budget")
}
impl Event {
    /// PacketProcessor checks ordinary standalone main tasks before calling
    /// their handler. Bundles and source-bound cookie/configuration tasks
    /// carry their own listener; Transfer was marked on receipt.
    pub fn requires_open_listener(&self) -> bool {
        match self {
            Self::Packet(_)
            | Self::ObservePacket(_)
            | Self::AttachmentSync(_)
            | Self::RawPayload(_)
            | Self::Brand(_)
            | Self::PackPush(_)
            | Self::PackPop(_)
            | Self::ShowDialog { .. }
            | Self::ClearDialog
            | Self::CodeOfConduct(_)
            | Self::ServerLinks(_)
            | Self::Reconfiguring(_)
            | Self::FinishConfiguration(_) => true,
            Self::Bundle(_)
            | Self::Cookie(_)
            | Self::Configuration(_)
            | Self::Transfer(_)
            | Self::Status(_)
            | Self::Joined { .. }
            | Self::Reconfigured { .. }
            | Self::Features(_)
            | Self::FarTerrain(_)
            | Self::CustomBooks
            | Self::QuietCommands
            | Self::DecodeError(_)
            | Self::Disconnected { .. } => false,
        }
    }

    /// Decode the engine payload at the play ingress boundary, before applying it.
    pub fn decode_attachment_sync(payload: cfg::CustomPayload) -> sunrise_protocol::Result<Self> {
        let message = sunrise_protocol::attachment_sync::decode_staged(payload)?;
        Ok(Self::AttachmentSync(Box::new(message)))
    }

    // Layout-derived allowance for a frozen receiving vector, actor backlog,
    // decoder marker and retained receipt, including vector spare capacity.
    const INGRESS_RECORD_OVERHEAD: usize =
        8 * std::mem::size_of::<(Self, ingress::ReceiveLease<Self>, usize, usize)>();

    fn ingress_charge(&self) -> usize {
        use std::mem::{align_of, size_of};
        use sunrise_protocol::retained::OwnedHeap;
        let dynamic = |value: &Dynamic| {
            size_of::<Dynamic>() + 2 * size_of::<usize>() + align_of::<Dynamic>() - 1
                + value.fresh_owned_heap_bytes()
        };
        let owned = match self {
            Self::Status(s) | Self::Brand(s) | Self::DecodeError(s) | Self::CodeOfConduct(s) => {
                s.capacity()
            }
            Self::Bundle(bundle) => bundle.owned_bytes(),
            Self::Reconfiguring(_) => size_of::<PhaseChange>(),
            Self::FinishConfiguration(task) => task.owned_bytes(),
            Self::Configuration(task) => task.owned_bytes(),
            Self::Cookie(task) => size_of::<CookieTask>() + task.cookie.owned_bytes(),
            Self::QuietCommands | Self::CustomBooks | Self::ClearDialog | Self::PackPop(_) => 0,
            Self::FarTerrain(crate::lod_net::Offer {
                distance: _,
                in_flight: _,
                per_second: _,
            }) => 0,
            Self::Features(values) => values.owned_heap_bytes(),
            // configure only inserts/replaces before freezing these snapshots.
            Self::Joined {
                registries,
                datapacks,
                name,
                ..
            } => dynamic(registries)
                .saturating_add(datapacks.owned_heap_bytes())
                .saturating_add(name.capacity()),
            Self::Reconfigured {
                registries,
                datapacks,
            } => dynamic(registries).saturating_add(datapacks.owned_heap_bytes()),
            Self::Packet(packet) | Self::ObservePacket(packet) => {
                size_of::<play::Clientbound>() + packet.owned_heap_bytes()
            }
            Self::RawPayload(incoming) => size_of::<crate::raw_payloads::Incoming>()
                .saturating_add(incoming.owned_heap_bytes()),
            Self::AttachmentSync(message) => {
                size_of_val(&**message).saturating_add(message.owned_heap_bytes())
            }
            Self::PackPush(packet) => packet.owned_heap_bytes(),
            Self::ShowDialog { dialog, registries } => dialog
                .owned_heap_bytes()
                .saturating_add(dynamic(registries)),
            Self::Transfer(task) => size_of::<TransferTask>() + task.packet.host.capacity(),
            Self::ServerLinks(links) => links.owned_heap_bytes(),
            Self::Disconnected {
                reason,
                key,
                from_server: _,
                after_login: _,
            } => reason.capacity().saturating_add(key.owned_heap_bytes()),
        };
        owned.saturating_add(Self::INGRESS_RECORD_OVERHEAD)
    }
}

/// Staging margin for the client's ordered event union and its delivery record.
pub(crate) const PAYLOAD_RECORD_OVERHEAD: usize =
    crate::raw_payloads::connection::record_overhead(&[std::mem::size_of::<Event>()]);

/// A client session's payload output: the queued writer and its ordered event
/// stream, in the raw-payload rules' terms.
pub(crate) struct PayloadOutput {
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    configuring: Arc<AtomicBool>,
    transport: Arc<crate::raw_payloads::Transport>,
    events: ingress::Producer<Event>,
}
impl crate::raw_payloads::Output for PayloadOutput {
    fn transport(&self) -> &Arc<crate::raw_payloads::Transport> {
        &self.transport
    }
    fn side(&self) -> crate::raw_payloads::Side {
        crate::raw_payloads::Side::Client
    }
    fn admit(
        &self,
        phase: sunrise_protocol::payload_negotiation::Phase,
        id: &mod_api::PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::Admission>, mod_api::PayloadSendError> {
        if self.configuring.load(Ordering::Acquire)
            != (phase == sunrise_protocol::payload_negotiation::Phase::Configuration)
        {
            return Err(mod_api::PayloadSendError::WrongProtocol(id.clone()));
        }
        let writer = self.writer.lock().unwrap().as_ref().cloned();
        Ok(Box::new(WriterAdmission(
            writer.ok_or(mod_api::PayloadSendError::Disconnected)?,
        )))
    }
    fn disconnect(&self, reason: &crate::text::Text) {
        let writer = self.writer.lock().unwrap();
        let _ = self.events.send_terminal(Event::Disconnected {
            reason: reason.plain(),
            key: None,
            from_server: false,
            after_login: true,
        });
        if let Some(writer) = writer.as_ref() {
            if self.configuring.load(Ordering::Acquire) {
                writer.shutdown();
            } else {
                writer.close_after_pending();
            }
        }
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    fn diagnostics(&self) -> Box<dyn Fn() -> String + Send + Sync> {
        let writer = Arc::downgrade(&self.writer);
        Box::new(move || {
            let Some(writer) = writer.upgrade() else {
                return "writer handle owner dropped".into();
            };
            match writer.try_lock() {
                Ok(w) => {
                    let current = w.clone();
                    drop(w);
                    current
                        .map(|w| w.diagnostics())
                        .unwrap_or_else(|| "writer absent".into())
                }
                Err(_) => "writer handle busy/poisoned".into(),
            }
        })
    }
}
/// One captured client writer: preflight and the reservation use the same one.
struct WriterAdmission(QueuedWriter);
impl crate::raw_payloads::Admission for WriterAdmission {
    fn framing(&self) -> sunrise_protocol::connection::Framing {
        self.0.framing()
    }
    fn capacity(&self) -> usize {
        self.0.payload_capacity()
    }
    fn accepts_packets(&self) -> bool {
        self.0.accepts_packets()
    }
    fn prepare(
        self: Box<Self>,
        body: Vec<u8>,
        id: &mod_api::PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::PreparedSend>, mod_api::PayloadSendError> {
        self.0
            .prepare_payload(body, id)
            .map(|prepared| Box::new(prepared) as Box<dyn crate::raw_payloads::PreparedSend>)
    }
}
impl crate::raw_payloads::ClientEvent for Event {
    fn raw_payload(incoming: crate::raw_payloads::Incoming) -> Self {
        Self::RawPayload(Box::new(incoming))
    }
    fn into_raw_payload(self) -> crate::raw_payloads::Incoming {
        let Self::RawPayload(incoming) = self else {
            unreachable!("only a raw payload event is taken back")
        };
        *incoming
    }
}

pub struct Session {
    events: ingress::Consumer<Event>,
    payload_events: ingress::Producer<Event>,
    payloads: Arc<crate::raw_payloads::Transport>,
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    /// The connection is in configuration (else in play). Changed only
    /// with `writer` locked, as the packet that switches is sent.
    configuring: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub address: String,
    pub name: String,
    pub counters: Arc<Mutex<Option<Arc<Counters>>>>,
    /// The cookies this server stored on us (`ClientboundStoreCookiePacket`).
    pub cookies: Cookies,
    /// SUNRISE_NET_DEBUG=1 logs every packet sent from the game thread.
    debug: bool,
    /// The far terrain requests waiting for the server (`lod_net`).
    far: Arc<crate::lod_net::Inbox>,
    /// The recording running, if any.
    recorder: RecorderSlot,
    pings: PingSamples,
    /// This connection's number, new for each one ([`Session::serial`]).
    serial: u64,
    /// The one payload output this session shares with every endpoint it hands out.
    payload_output: std::sync::OnceLock<Arc<dyn crate::raw_payloads::Output>>,
}
/// The next [`Session::serial`].
static NEXT_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn next_serial() -> u64 {
    NEXT_SERIAL.fetch_add(1, Ordering::Relaxed)
}
use std::path::PathBuf;

impl Session {
    /// A number of its own for each connection, never 0: entity ids are per connection, so what
    /// is kept by id goes when it changes.
    pub fn serial(&self) -> u64 {
        self.serial
    }
    /// Start connecting in the background, as `auth`'s account when there is one.
    pub fn connect(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
        reports: Arc<crate::chunk_io::Reports>,
    ) -> Self {
        Self::start(
            address, name, auth, info, lang, None, None, resources, reports, false,
        )
    }
    /// `ClientHandshakePacketListenerImpl.handleHello`: LAN entries can continue
    /// after session authentication fails; the server still decides whether to admit them.
    /// Only for entries selected from LAN discovery, not addresses classified by IP.
    pub fn connect_lan(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
        reports: Arc<crate::chunk_io::Reports>,
    ) -> Self {
        Self::start(
            address, name, auth, info, lang, None, None, resources, reports, true,
        )
    }
    /// `ConnectScreen.startConnecting` for a `transfer`: the cookies the
    /// server stored carry to the new connection (`TransferState`).
    pub fn connect_transferred(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        cookies: Cookies,
        resources: Arc<ItemData>,
        reports: Arc<crate::chunk_io::Reports>,
    ) -> Self {
        Self::start(
            address,
            name,
            auth,
            info,
            lang,
            None,
            Some(cookies),
            resources,
            reports,
            false,
        )
    }
    /// Join an integrated server through memory (`Connection.connectToLocalServer`). The
    /// hello still names the signed-in account (`Minecraft.handleClientLogin`'s
    /// `ServerboundHelloPacket(getUser().getName(), getUser().getProfileId())`), so the
    /// world's `players/data/<uuid>.dat` and `singleplayer_uuid` are the account's.
    pub fn connect_local(
        pipe: sunrise_protocol::connection::Pipe,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        resources: Arc<ItemData>,
        reports: Arc<crate::chunk_io::Reports>,
    ) -> Self {
        Self::start(
            "local",
            name,
            auth,
            info,
            lang,
            Some(pipe),
            None,
            resources,
            reports,
            false,
        )
    }
    /// A lightweight session for studio replay playback without background network IO.
    pub fn replay(name: &str) -> Self {
        let (_send, events) = event_channel();
        Self {
            payload_events: _send,
            events,
            writer: Arc::new(Mutex::new(None)),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            payloads: crate::raw_payloads::Transport::empty(),
            address: "studio-replay".to_owned(),
            name: name.to_owned(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Default::default(),
            debug: false,
            far: Arc::new(crate::lod_net::Inbox::default()),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        }
    }
    fn start(
        address: &str,
        name: &str,
        auth: Option<Arc<AuthSession>>,
        info: cfg::ClientInformation,
        lang: Arc<Language>,
        local: Option<sunrise_protocol::connection::Pipe>,
        transfer_from: Option<Cookies>,
        resources: Arc<ItemData>,
        reports: Arc<crate::chunk_io::Reports>,
        lan: bool,
    ) -> Self {
        let (send, events) = event_channel();
        let payload_events = send.clone();
        let captured_payloads = crate::raw_payloads::Transport::capture(
            crate::raw_payloads::Side::Client,
            crate::net::PAYLOAD_RECORD_OVERHEAD,
        );
        let payloads = captured_payloads
            .as_ref()
            .cloned()
            .unwrap_or_else(|_| crate::raw_payloads::Transport::empty());
        let thread_payloads = payloads.clone();
        let writer = Arc::new(Mutex::new(None));
        let configuring = Arc::new(AtomicBool::new(true));
        let counters = Arc::new(Mutex::new(None));
        let cookies = Cookies::default();
        let (thread_writer, thread_counters, thread_cookies) =
            (writer.clone(), counters.clone(), cookies.clone());
        let thread_configuring = configuring.clone();
        let far = Arc::new(crate::lod_net::Inbox::default());
        let thread_far = far.clone();
        let recorder = Arc::new(Mutex::new(None));
        let thread_recorder = recorder.clone();
        let pings = Arc::default();
        let thread_pings = Arc::clone(&pings);
        let (address_owned, name_owned) = (address.to_owned(), name.to_owned());
        // Standalone NETWORK payload handlers run on this thread.
        let thread = std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("sunrise-net".into())
            .spawn(move || {
                sunrise_threads::favour_current();
                let mut after_login = false;
                let result = captured_payloads.map_err(Reason::plain).and_then(|_| {
                    run(
                        local,
                        &address_owned,
                        &name_owned,
                        auth.as_deref(),
                        info,
                        &lang,
                        &send,
                        &thread_writer,
                        &thread_configuring,
                        &thread_counters,
                        &thread_cookies,
                        &thread_far,
                        transfer_from,
                        &resources,
                        lan,
                        &thread_recorder,
                        &thread_payloads,
                        &thread_pings,
                        &reports,
                        &mut after_login,
                    )
                });
                // The physical close is immediate. Session::disconnect retires
                // the payload epoch when main drops the listener, so a bundle
                // already in its member loop still has its handler table.
                report_connection_end(result, &thread_writer, &send, after_login);
                thread_far.cancel();
            })
            .inspect_err(|error| {
                // No reader runs to report the end: say it here, or the session waits forever.
                let _ = payload_events.send_terminal(Event::Disconnected {
                    reason: format!("network thread: {error}"),
                    key: None,
                    from_server: false,
                    after_login: false,
                });
            })
            .ok();
        let debug = std::env::var_os("SUNRISE_NET_DEBUG").is_some();
        Self {
            events,
            payload_events,
            payloads,
            writer,
            configuring,
            thread,
            address: address.to_owned(),
            name: name.to_owned(),
            counters,
            cookies,
            debug,
            far,
            recorder,
            pings,
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        }
    }
    /// Validate an untrusted mod packet once, reserve its physical writer credit,
    /// and return an invisible root-publication record. This is not Session::send:
    /// registered/engine channels must use their authority-aware service API.
    pub fn prepare_mod_packet(
        &self,
        packet: mod_api::ServerboundPlayPacket,
    ) -> std::result::Result<PreparedPacket, mod_api::SendPacketError> {
        use mod_api::SendPacketError;
        use sunrise_core::vanilla_bytes::Writer;
        use sunrise_protocol::{
            buf::{Codec, Ident, Reader},
            packets::custom_payload,
        };
        let id = match packet.id.namespace() {
            "minecraft" => sunrise_protocol::ids::play::serverbound::NAMES
                .iter()
                .position(|name| *name == packet.id.path()),
            _ => None,
        }
        .ok_or(SendPacketError::UnknownPacket { id: packet.id })? as i32;
        if id == sunrise_protocol::ids::play::serverbound::CUSTOM_PAYLOAD {
            // Check ownership before copying a potentially large application body.
            // Ident is the same identifier boundary used by the packet codec.
            let mut reader = Reader::new(&packet.body);
            let channel = Ident::read(&mut reader).map_err(|_| SendPacketError::InvalidBody)?;
            if self.payloads.local.owns(&channel) || custom_payload::is_reserved_channel(&channel) {
                return Err(SendPacketError::ProtectedPayload {
                    id: mod_api::PayloadId(channel),
                });
            }
            if reader.remaining() > custom_payload::SERVERBOUND_LIMIT {
                return Err(SendPacketError::PayloadTooLarge {
                    size: reader.remaining() as u64,
                    limit: custom_payload::SERVERBOUND_LIMIT as u64,
                });
            }
        } else {
            play::Serverbound::decode(id, &packet.body)
                .map_err(|_| SendPacketError::InvalidBody)?;
        }
        let mut encoded = Writer::new();
        encoded.varint(id);
        encoded.bytes(&packet.body);
        let writer = self
            .writer
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .ok_or(SendPacketError::NotConnected)?;
        writer
            .prepare_packet(encoded.data, self.serial(), None)
            .map_err(PrepareRefusal::error)
    }
    /// Publish into the real session FIFO for an actor test, without a reader thread.
    #[cfg(feature = "test-payload-probe")]
    #[doc(hidden)]
    pub fn received_event_probe(&self) -> impl Fn(Event) + Send + Sync + use<> {
        let sender = self.payload_events.clone();
        move |event| assert!(sender.send(event).is_ok())
    }

    /// The frozen receiving turn, retaining every input credit through its actor.
    #[sunrise_mod_api_macros::replaceable]
    pub fn poll(&self) -> Vec<ingress::OwnedReceipt<Event>> {
        self.events.take_turn()
    }
    #[cfg(feature = "test-payload-probe")]
    pub fn hold_test_ingress_credit(&self) -> impl Drop + use<> {
        self.payload_events
            .hold_test_credit(EVENT_BUDGET - self.events.stats().reserved_bytes)
    }
    #[cfg(feature = "test-payload-probe")]
    pub fn take_parked_test_reader(&mut self) -> JoinHandle<()> {
        self.events.wait_for_parked_reader();
        self.thread.take().expect("test Session has a reader")
    }
    /// Starts recording into `recorder`. `initialize` runs first, while the network thread
    /// can't record, so what it writes (a snapshot of the world) comes before any live packet.
    pub fn start_recording<R: PacketRecorder + 'static>(
        &self,
        recorder: R,
        initialize: impl FnOnce(&R),
    ) {
        let mut slot = self.recorder.lock().unwrap();
        initialize(&recorder);
        *slot = Some(Box::new(recorder));
    }
    /// Stop recording and finalize the `.mcpr` archive.
    pub fn stop_recording(&self) -> Option<std::io::Result<PathBuf>> {
        let rec = self.recorder.lock().unwrap().take()?;
        Some(rec.stop())
    }
    /// `Connection.isConnected`: the connection is open (a replay's never is).
    pub fn connected(&self) -> bool {
        self.writer.lock().is_ok_and(|writer| writer.is_some())
    }
    /// Connection/listener eligibility for ordinary queued main tasks.
    pub fn accepts_packets(&self) -> bool {
        self.writer
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(QueuedWriter::accepts_packets)
    }
    /// Present reader-side ping samples in their actual callback order.
    pub fn drain_debug_pings(&self, mut visit: impl FnMut(i64)) {
        for delay in self.pings.lock().unwrap().drain(..) {
            visit(delay);
        }
    }
    /// Stop output at a main-affine close but let preceding replies drain.
    pub fn close_after_pending(&self) {
        let writer = self.writer.lock().unwrap();
        self.events.close_reader_admission();
        if let Some(writer) = writer.as_ref() {
            writer.close_after_pending();
        }
    }
    /// Stop output, drain preceding replies, and wait for transport shutdown to complete.
    pub fn shutdown_and_wait(&self) {
        self.events.close_reader_admission();
        self.payloads.close();
        let writer = self.writer.lock().unwrap().take();
        if let Some(writer) = writer {
            writer.close_and_wait();
        }
    }
    /// Whether recording is currently active.
    pub fn is_recording(&self) -> bool {
        self.recorder
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.is_active())
    }
    /// Status of the active recording, if any.
    pub fn recorder_status(&self) -> Option<RecorderStatus> {
        self.recorder.lock().unwrap().as_ref().map(|r| r.status())
    }
    /// Record an encoded Clientbound packet into the active recording session at current elapsed time.
    pub fn record_clientbound(&self, packet: &sunrise_protocol::packets::play::Clientbound) {
        if self.is_recording() {
            let encoded = packet.encode();
            let mut reader = sunrise_protocol::buf::Reader::new(&encoded);
            if let Ok(id) = reader.varint()
                && let Some(rec) = self.recorder.lock().unwrap().as_ref()
            {
                rec.record_packet(id, reader.rest());
            }
        }
    }
    /// Physical reply target, preserving this session's ordered event stream.
    pub fn payload_endpoint(&self) -> crate::raw_payloads::Endpoint {
        let output = self.payload_output.get_or_init(|| {
            Arc::new(PayloadOutput {
                writer: self.writer.clone(),
                configuring: self.configuring.clone(),
                transport: self.payloads.clone(),
                events: self.payload_events.clone(),
            })
        });
        crate::raw_payloads::Endpoint::shared(output.clone())
    }
    /// This physical session's frozen declarations and current epoch.
    pub fn payload_transport(&self) -> &Arc<crate::raw_payloads::Transport> {
        &self.payloads
    }
    /// Prepare a registered payload without publishing it before root commit.
    /// The caller resolves its channel against the frozen peer table first.
    pub fn prepare_payload(
        &self,
        phase: sunrise_protocol::payload_negotiation::Phase,
        body: Vec<u8>,
        id: &mod_api::PayloadId,
    ) -> Result<PreparedPayload, mod_api::PayloadSendError> {
        let writer = self.writer.lock().unwrap();
        if self.configuring.load(Ordering::Relaxed)
            != (phase == sunrise_protocol::payload_negotiation::Phase::Configuration)
        {
            return Err(mod_api::PayloadSendError::WrongProtocol(id.clone()));
        }
        writer
            .as_ref()
            .ok_or(mod_api::PayloadSendError::Disconnected)?
            .prepare_payload(body, id)
    }
    /// Far terrain the server samples, as it offered (`Event::FarTerrain`):
    /// biomes named by this client's indices, block states below `states`.
    pub fn far_terrain(
        &self,
        offer: crate::lod_net::Offer,
        biomes: HashMap<String, u16>,
        states: u32,
    ) -> Arc<crate::lod_net::RemoteSource> {
        crate::lod_net::RemoteSource::new(self.far.clone(), offer, biomes, states)
    }
    /// The far terrain requests its samplers queued go to the server, from
    /// the game thread, among its other packets.
    pub fn send_far_terrain(&self) {
        for data in self.far.requests() {
            self.send(play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: crate::lod_net::CHANNEL.into(),
                data,
            }));
        }
    }
    /// The far terrain made with [`Self::far_terrain`] ends: its samplers
    /// stop waiting for the server.
    pub fn cancel_far_terrain(&self) {
        self.far.cancel();
    }
    /// The connection is configured again (a proxy's next server): requests
    /// the old server held never come back, so their slots are freed.
    pub fn reset_far_terrain(&self) {
        self.far.reset();
    }
    /// ServerboundResourcePackPacket, in whichever phase the connection is.
    pub fn send_pack_status(&self, id: u128, action: i32) {
        let status = cfg::ResourcePackResponse { id, action };
        let mut writer = self.writer.lock().unwrap();
        let body = if self.configuring.load(Ordering::Relaxed) {
            cfg::Serverbound::ResourcePack(status).encode()
        } else {
            play::Serverbound::ResourcePack(status).encode()
        };
        if let Some(w) = writer.as_mut()
            && w.send_bypassing_held(&body).is_err()
        {
            w.shutdown();
        }
    }
    /// ServerboundCustomClickActionPacket (a dialog's answer), in whichever
    /// phase the connection is.
    pub fn send_custom_click(&self, id: &str, payload: Option<sunrise_core::nbt::Tag>) {
        let click = cfg::CustomClickAction {
            id: id.to_owned(),
            payload,
        };
        let mut writer = self.writer.lock().unwrap();
        let body = if self.configuring.load(Ordering::Relaxed) {
            cfg::Serverbound::CustomClickAction(click).encode()
        } else {
            play::Serverbound::CustomClickAction(click).encode()
        };
        if let Some(w) = writer.as_mut()
            && w.send_bypassing_held(&body).is_err()
        {
            w.shutdown();
        }
    }
    /// ServerboundAcceptCodeOfConductPacket, sent in configuration.
    pub fn accept_code_of_conduct(&self) {
        let mut writer = self.writer.lock().unwrap();
        if let Some(w) = writer.as_mut()
            && w.send_bypassing_held(&cfg::Serverbound::AcceptCodeOfConduct(Empty).encode())
                .is_err()
        {
            w.shutdown();
        }
    }
    /// `Connection.disconnect`: the socket closes and the game goes on at once.
    /// The reader thread is left to end on its own: on Windows a shutdown
    /// doesn't wake a read already waiting, so joining it held the window until
    /// the server dropped the connection, many seconds later.
    pub fn disconnect(&mut self) {
        self.payloads.close();
        if let Some(w) = self.writer.lock().unwrap().take()
            && !w.is_terminal()
        {
            w.shutdown();
        }
        drop(self.thread.take());
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.disconnect();
    }
}

/// An address of the form `friend:K7QM-3XPD` joins a friend's world through
/// its invite code (Play with Friends, through a hook's [`tunnel::Resolver`]):
/// the session goes through a tunnel the client reaches on a loopback port, so
/// everything after is a plain join.
pub const FRIEND: &str = tunnel::PREFIX;

/// Whether `code` is an invite code; with `full`, only a full one (a short
/// relay code could also be a host name). Without a resolver nothing is one.
pub fn is_invite(code: &str, full: bool) -> bool {
    sunrise_hooks::engine::tunnel().is_some_and(|r| r.is_code(code, full))
}

/// Whether an invite code can be joined: a resolver is registered (sunrise-official registers
/// sunrise-friends'). The title screen's Friends icon, which opens Join a Friend, is inactive
/// without one, as the pause menu's is without a page to share from.
pub fn joins_friends() -> bool {
    #[cfg(test)]
    if JOINS_FRIENDS_IN_TEST.get() {
        return true;
    }
    sunrise_hooks::engine::tunnel().is_some()
}
#[cfg(test)]
thread_local! {
    /// A test's stand-in for a registered resolver: the registry is the process's and sealed.
    pub(crate) static JOINS_FRIENDS_IN_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// An address as players and logs see it: a friend's invite code stays
/// out of it (its secret is in there).
pub fn shown_address(address: &str) -> String {
    match address.strip_prefix(FRIEND) {
        Some(code) => match sunrise_hooks::engine::tunnel().and_then(|r| r.nameplate(code)) {
            Some(nameplate) => format!("Friend's world ({nameplate}-****)"),
            None => "Friend's world".to_owned(),
        },
        None => address.to_owned(),
    }
}

/// The relay the player set (`friendsRelay` in sunrise.txt, beside
/// options.txt), else the resolver's (`SUNRISE_RELAY_URL`). None by default.
pub fn friends_relay_setting() -> Option<String> {
    let dir = std::env::current_dir().ok()?;
    crate::ui_theme::preference(&dir, "friendsRelay")
        .filter(|r| !r.is_empty())
        .or_else(|| sunrise_hooks::engine::tunnel().and_then(|r| r.relay_from_env()))
}
/// Remember the relay typed on Join a friend (empty forgets it).
pub fn save_friends_relay(relay: &str) {
    if let Ok(dir) = std::env::current_dir() {
        crate::ui_theme::set_preference(&dir, "friendsRelay", (!relay.is_empty()).then_some(relay));
    }
}

/// Reach the friend's world from the code (directly, or through a relay)
/// and open the tunnel: the loopback address to connect to, and the token to send on it first.
/// Without a resolver (a build without Play with Friends) the address is refused, never retried.
fn join_friend(
    code: &str,
    lang: &Language,
    send: &ingress::Producer<Event>,
) -> Result<(String, u16, Vec<u8>), Reason> {
    let Some(resolver) = sunrise_hooks::engine::tunnel() else {
        return Err(Reason::keyed_arg(
            "sunrise.multiplayer.invalid_address",
            lang,
            &shown_address(&format!("{FRIEND}{code}")),
        ));
    };
    let mut progress = |p: tunnel::Progress| {
        let key = match p {
            tunnel::Progress::Finding => "sunrise.multiplayer.finding_friend",
            tunnel::Progress::Connected { direct: true } => {
                "sunrise.multiplayer.connected_friend_direct"
            }
            tunnel::Progress::Connected { direct: false } => {
                "sunrise.multiplayer.connected_friend_relay"
            }
        };
        status(send, &tr(lang, key));
    };
    let opened = resolver
        .open(code, friends_relay_setting(), &mut progress)
        .map_err(Reason::plain)?;
    Ok(("127.0.0.1".to_owned(), opened.port, opened.token))
}

fn status(send: &ingress::Producer<Event>, text: &str) {
    let _ = send.send_wait(Event::Status(text.to_owned()));
}

/// Shown when an online-mode server asks a client without an account to authenticate.
pub const NOT_SIGNED_IN: &str = "This server is in online mode. Sign in with your Microsoft account in the Sunrise launcher to join it.";

/// A translation, or the key itself (Language.getOrDefault).
fn tr(lang: &Language, key: &str) -> String {
    lang.get(key).cloned().unwrap_or_else(|| key.to_owned())
}

/// Why a connection ended: the shown text and, when the reason was a
/// translatable component or a known Sunrise error, its translation key.
#[derive(Clone, Debug)]
pub struct Reason {
    pub text: String,
    pub key: Option<String>,
    from_server: bool,
}
impl Reason {
    fn ended(self, from_server: bool, after_login: bool) -> Event {
        Event::Disconnected {
            reason: self.text,
            key: self.key,
            from_server,
            after_login,
        }
    }
    fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
            from_server: false,
        }
    }
    /// A vanilla key's text in the session's language, with the key.
    fn keyed(key: &str, lang: &Language) -> Self {
        Self {
            text: tr(lang, key),
            key: Some(key.to_owned()),
            from_server: false,
        }
    }
    /// A Sunrise key's text with a `%s` argument filled.
    fn keyed_arg(key: &str, lang: &Language, arg: &str) -> Self {
        let mut r = Self::keyed(key, lang);
        r.text = r.text.replacen("%s", arg, 1);
        r
    }
    /// A server's disconnect component: the resolved text and the
    /// `translate` key when the component named one (servers' literal
    /// kicks have none; their text is the server's own language).
    fn component(reason_json: &str, lang: &Language) -> Self {
        match serde_json::from_str::<serde_json::Value>(reason_json) {
            Ok(v) => Self {
                text: if crate::lang::has_mod_entries() {
                    let current = crate::lang::current();
                    json_text_with(&v, |key| {
                        current
                            .as_deref()
                            .and_then(|published| published.mod_entry(key))
                            .or_else(|| lang.get(key).map(String::as_str))
                    })
                } else {
                    json_text(&v, lang)
                },
                key: v["translate"].as_str().map(str::to_owned),
                from_server: true,
            },
            Err(_) => Self::plain(reason_json),
        }
    }
    /// `Connection.exceptionCaught`: `disconnect.genericReason` with `"Internal Exception: ..."`.
    fn generic_exception(detail: impl std::fmt::Display, lang: &Language) -> Self {
        Self {
            text: plain(
                &translatable(
                    "disconnect.genericReason",
                    &[&format!("Internal Exception: {detail}")],
                ),
                lang,
            ),
            key: Some("disconnect.genericReason".to_owned()),
            from_server: false,
        }
    }
    /// The play/configuration phases' disconnect: the reason arrives as
    /// an NBT component.
    fn tag(tag: &Tag, lang: &Language) -> Self {
        let key = match tag {
            Tag::Compound(map) => map
                .get(&JavaString::from("translate"))
                .and_then(|t| match t {
                    Tag::String(s) => s.to_string().ok(),
                    _ => None,
                }),
            _ => None,
        };
        Self {
            text: if crate::lang::has_mod_entries() {
                let current = crate::lang::current();
                plain_with(tag, |key| {
                    current
                        .as_deref()
                        .and_then(|published| published.mod_entry(key))
                        .or_else(|| lang.get(key).map(String::as_str))
                })
            } else {
                plain(tag, lang)
            },
            key,
            from_server: true,
        }
    }
}

/// `ConnectScreen.connect`'s log line, as vanilla writes it from its "Server Connector #N" thread:
/// the address as the player gave it, and the port (25565 when none was given), before any SRV
/// lookup. Other launchers read the last one from `latest.log` to offer the server again.
fn connecting_line(host: &str, port: u16) -> String {
    format!("Connecting to {host}, {port}")
}
fn log_connecting(host: &str, port: u16) {
    // ConnectScreen.UNIQUE_THREAD_ID: the first connector is #1.
    static CONNECTORS: AtomicU32 = AtomicU32::new(0);
    let n = CONNECTORS.fetch_add(1, Ordering::Relaxed) + 1;
    crate::logging::write_as(
        &format!("Server Connector #{n}"),
        log::Level::Info,
        &connecting_line(host, port),
    );
}

/// `Connection$1.initChannel`: a client socket's pipeline starts with
/// `ReadTimeoutHandler(30)`. A server sends a keep-alive every 15 s, so one
/// that sends nothing for 30 s has frozen or dropped off the network.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// `ConnectScreen.startConnecting`: the socket to a server, with `read_timeout`
/// ([`READ_TIMEOUT`]) on every read. The friends relay supplies a concrete
/// endpoint, not a Minecraft hostname, so it skips the SRV lookup. Returns the
/// resolved host and port for `ConnectScreen`'s login handshake.
fn connect_tcp(
    host: &str,
    port: u16,
    relayed: bool,
    read_timeout: Duration,
) -> Result<(Connection, String, u16), String> {
    let (c, host, port) = if relayed {
        Connection::connect(host, port, Duration::from_secs(10)).map(|c| (c, host.to_owned(), port))
    } else {
        Connection::connect_server_with_address(host, port, Duration::from_secs(10))
    }
    .map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(read_timeout))
        .map_err(|e| e.to_string())?;
    Ok((c, host, port))
}

/// `Connection.exceptionCaught`: a read that timed out ends with
/// `disconnect.timeout`, any other failure with `disconnect.genericReason`
/// ("Internal Exception: ..."); a stream the server closed ends with
/// `disconnect.endOfStream` (`Connection.channelInactive`).
fn connection_lost(e: sunrise_protocol::Error, lang: &Language) -> Reason {
    if e.is_timed_out() {
        Reason::keyed("disconnect.timeout", lang)
    } else if e.is_end_of_stream() {
        Reason::keyed("disconnect.endOfStream", lang)
    } else {
        Reason::generic_exception(e, lang)
    }
}

/// Vanilla `IdDispatchCodec` / `PacketDecoder` exception message wrapped by Netty's
/// `ByteToMessageDecoder` and reported via `Connection.exceptionCaught`.
fn packet_decode_reason(
    state: sunrise_protocol::State,
    id: i32,
    err: &sunrise_protocol::Error,
    lang: &Language,
) -> Reason {
    if err.is_unknown_packet_id() {
        return Reason::generic_exception(
            format!("io.netty.handler.codec.DecoderException: Received unknown packet id {id}"),
            lang,
        );
    }
    let name = sunrise_protocol::ids::name(state, sunrise_protocol::Direction::Clientbound, id)
        .unwrap_or("?");
    if let Some(extra) = err.extra_packet_bytes() {
        let protocol = match state {
            sunrise_protocol::State::Handshake => "handshake",
            sunrise_protocol::State::Status => "status",
            sunrise_protocol::State::Login => "login",
            sunrise_protocol::State::Configuration => "configuration",
            sunrise_protocol::State::Play => "play",
        };
        let class_name = vanilla_clientbound_class_name(state, id);
        return Reason::generic_exception(
            format!(
                "io.netty.handler.codec.DecoderException: java.io.IOException: Packet {protocol}/clientbound/minecraft:{name} ({class_name}) was larger than I expected, found {extra} bytes extra whilst reading packet clientbound/minecraft:{name}"
            ),
            lang,
        );
    }
    Reason::generic_exception(
        format!(
            "io.netty.handler.codec.DecoderException: Failed to decode packet 'clientbound/minecraft:{name}'"
        ),
        lang,
    )
}

/// Registered vanilla 26.3 simple Java class names (`Packet.getClass().getSimpleName()`),
/// indexed by protocol state and clientbound packet id (`StatusProtocols`, `LoginProtocols`,
/// `ConfigurationProtocols`, `GameProtocols`).
const STATUS_CLIENTBOUND_CLASSES: &[&str] = &[
    "ClientboundStatusResponsePacket",
    "ClientboundPongResponsePacket",
];
const LOGIN_CLIENTBOUND_CLASSES: &[&str] = &[
    "ClientboundLoginDisconnectPacket",
    "ClientboundHelloPacket",
    "ClientboundLoginFinishedPacket",
    "ClientboundLoginCompressionPacket",
    "ClientboundCustomQueryPacket",
    "ClientboundCookieRequestPacket",
];
const CONFIGURATION_CLIENTBOUND_CLASSES: &[&str] = &[
    "ClientboundCookieRequestPacket",
    "ClientboundCustomPayloadPacket",
    "ClientboundDisconnectPacket",
    "ClientboundFinishConfigurationPacket",
    "ClientboundKeepAlivePacket",
    "ClientboundPingPacket",
    "ClientboundResetChatPacket",
    "ClientboundRegistryDataPacket",
    "ClientboundResourcePackPopPacket",
    "ClientboundResourcePackPushPacket",
    "ClientboundPostEffectsPacket",
    "ClientboundStoreCookiePacket",
    "ClientboundTransferPacket",
    "ClientboundUpdateEnabledFeaturesPacket",
    "ClientboundUpdateTagsPacket",
    "ClientboundSelectKnownPacks",
    "ClientboundCustomReportDetailsPacket",
    "ClientboundServerLinksPacket",
    "ClientboundClearDialogPacket",
    "ClientboundShowDialogPacket",
    "ClientboundCodeOfConductPacket",
];
const PLAY_CLIENTBOUND_CLASSES: &[&str] = &[
    "ClientboundBundleDelimiterPacket",
    "ClientboundAddEntityPacket",
    "ClientboundAnimatePacket",
    "ClientboundAwardStatsPacket",
    "ClientboundBlockChangedAckPacket",
    "ClientboundBlockDestructionPacket",
    "ClientboundBlockEntityDataPacket",
    "ClientboundBlockEventPacket",
    "ClientboundBlockUpdatePacket",
    "ClientboundBossEventPacket",
    "ClientboundChangeDifficultyPacket",
    "ClientboundChunkBatchFinishedPacket",
    "ClientboundChunkBatchStartPacket",
    "ClientboundChunksBiomesPacket",
    "ClientboundClearTitlesPacket",
    "ClientboundCommandSuggestionsPacket",
    "ClientboundCommandsPacket",
    "ClientboundContainerClosePacket",
    "ClientboundContainerSetContentPacket",
    "ClientboundContainerSetDataPacket",
    "ClientboundContainerSetSlotPacket",
    "ClientboundCookieRequestPacket",
    "ClientboundCooldownPacket",
    "ClientboundCustomChatCompletionsPacket",
    "ClientboundCustomPayloadPacket",
    "ClientboundDamageEventPacket",
    "ClientboundDebugBlockValuePacket",
    "ClientboundDebugChunkValuePacket",
    "ClientboundDebugEntityValuePacket",
    "ClientboundDebugEventPacket",
    "ClientboundDebugSamplePacket",
    "ClientboundDeleteChatPacket",
    "ClientboundDisconnectPacket",
    "ClientboundDisguisedChatPacket",
    "ClientboundEntityEventPacket",
    "ClientboundEntityPositionSyncPacket",
    "ClientboundExplodePacket",
    "ClientboundAddTransientBlockPacket",
    "ClientboundForgetLevelChunkPacket",
    "ClientboundGameEventPacket",
    "ClientboundGameRuleValuesPacket",
    "ClientboundGameTestHighlightPosPacket",
    "ClientboundMountScreenOpenPacket",
    "ClientboundHurtAnimationPacket",
    "ClientboundInitializeBorderPacket",
    "ClientboundKeepAlivePacket",
    "ClientboundLevelChunkWithLightPacket",
    "ClientboundLevelEventPacket",
    "ClientboundLevelParticlesPacket",
    "ClientboundLightUpdatePacket",
    "ClientboundLoginPacket",
    "ClientboundLowDiskSpaceWarningPacket",
    "ClientboundMapItemDataPacket",
    "ClientboundMerchantOffersPacket",
    "Pos",
    "PosRot",
    "ClientboundMoveMinecartPacket",
    "Rot",
    "ClientboundMoveVehiclePacket",
    "ClientboundOpenBookPacket",
    "ClientboundOpenScreenPacket",
    "ClientboundOpenSignEditorPacket",
    "ClientboundPingPacket",
    "ClientboundPongResponsePacket",
    "ClientboundPlaceGhostRecipePacket",
    "ClientboundPlayerAbilitiesPacket",
    "ClientboundPlayerChatPacket",
    "ClientboundPlayerCombatEndPacket",
    "ClientboundPlayerCombatEnterPacket",
    "ClientboundPlayerCombatKillPacket",
    "ClientboundPlayerInfoRemovePacket",
    "ClientboundPlayerInfoUpdatePacket",
    "ClientboundPlayerLookAtPacket",
    "ClientboundPlayerPositionPacket",
    "ClientboundPlayerRotationPacket",
    "ClientboundRecipeBookAddPacket",
    "ClientboundRecipeBookRemovePacket",
    "ClientboundRecipeBookSettingsPacket",
    "ClientboundRemoveEntitiesPacket",
    "ClientboundRemoveMobEffectPacket",
    "ClientboundResetScorePacket",
    "ClientboundResourcePackPopPacket",
    "ClientboundResourcePackPushPacket",
    "ClientboundPostEffectsPacket",
    "ClientboundRespawnPacket",
    "ClientboundRotateHeadPacket",
    "ClientboundSectionBlocksUpdatePacket",
    "ClientboundSelectAdvancementsTabPacket",
    "ClientboundServerDataPacket",
    "ClientboundSetActionBarTextPacket",
    "ClientboundSetBorderCenterPacket",
    "ClientboundSetBorderLerpSizePacket",
    "ClientboundSetBorderSizePacket",
    "ClientboundSetBorderWarningDelayPacket",
    "ClientboundSetBorderWarningDistancePacket",
    "ClientboundSetCameraPacket",
    "ClientboundSetChunkCacheCenterPacket",
    "ClientboundSetChunkCacheRadiusPacket",
    "ClientboundSetCursorItemPacket",
    "ClientboundSetDefaultSpawnPositionPacket",
    "ClientboundSetDisplayObjectivePacket",
    "ClientboundSetEntityDataPacket",
    "ClientboundSetEntityLinkPacket",
    "ClientboundSetEntityMotionPacket",
    "ClientboundSetEquipmentPacket",
    "ClientboundSetExperiencePacket",
    "ClientboundSetHealthPacket",
    "ClientboundSetHeldSlotPacket",
    "ClientboundSetObjectivePacket",
    "ClientboundSetPassengersPacket",
    "ClientboundSetPlayerInventoryPacket",
    "ClientboundSetPlayerTeamPacket",
    "ClientboundSetScorePacket",
    "ClientboundSetSimulationDistancePacket",
    "ClientboundSetSubtitleTextPacket",
    "ClientboundSetTimePacket",
    "ClientboundSetTitleTextPacket",
    "ClientboundSetTitlesAnimationPacket",
    "ClientboundSoundEntityPacket",
    "ClientboundSoundPacket",
    "ClientboundStartConfigurationPacket",
    "ClientboundStopSoundPacket",
    "ClientboundStoreCookiePacket",
    "ClientboundSwingAnimationPacket",
    "ClientboundSystemChatPacket",
    "ClientboundTabListPacket",
    "ClientboundTagQueryPacket",
    "ClientboundTakeItemEntityPacket",
    "ClientboundTeleportEntityPacket",
    "ClientboundTestInstanceBlockStatus",
    "ClientboundTickingStatePacket",
    "ClientboundTickingStepPacket",
    "ClientboundTransferPacket",
    "ClientboundUpdateAdvancementsPacket",
    "ClientboundUpdateAttributesPacket",
    "ClientboundUpdateMobEffectPacket",
    "ClientboundUpdateRecipesPacket",
    "ClientboundUpdateTagsPacket",
    "ClientboundProjectilePowerPacket",
    "ClientboundCustomReportDetailsPacket",
    "ClientboundServerLinksPacket",
    "ClientboundTrackedWaypointPacket",
    "ClientboundClearDialogPacket",
    "ClientboundShowDialogPacket",
];

fn vanilla_clientbound_class_name(state: sunrise_protocol::State, id: i32) -> &'static str {
    let classes: &[&str] = match state {
        sunrise_protocol::State::Status => STATUS_CLIENTBOUND_CLASSES,
        sunrise_protocol::State::Login => LOGIN_CLIENTBOUND_CLASSES,
        sunrise_protocol::State::Configuration => CONFIGURATION_CLIENTBOUND_CLASSES,
        sunrise_protocol::State::Play => PLAY_CLIENTBOUND_CLASSES,
        sunrise_protocol::State::Handshake => &[],
    };
    usize::try_from(id)
        .ok()
        .and_then(|i| classes.get(i))
        .copied()
        .unwrap_or("?")
}

/// `ClientHandshakePacketListenerImpl.handleHello`: the whole connection.
/// Ok: why it ended for good (the server's reason, or
/// a refusal no retry fixes); Err: the connection failed.
fn run(
    local: Option<sunrise_protocol::connection::Pipe>,
    address: &str,
    name: &str,
    auth: Option<&AuthSession>,
    info: cfg::ClientInformation,
    lang: &Arc<Language>,
    send: &ingress::Producer<Event>,
    shared_writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    shared_counters: &Mutex<Option<Arc<Counters>>>,
    cookies: &Cookies,
    far: &Arc<crate::lod_net::Inbox>,
    transfer_from: Option<Cookies>,
    resources: &ItemData,
    lan: bool,
    recorder: &RecorderSlot,
    payloads: &Arc<crate::raw_payloads::Transport>,
    pings: &PingSamples,
    reports: &Arc<crate::chunk_io::Reports>,
    after_login: &mut bool,
) -> Result<Reason, Reason> {
    let singleplayer = local.is_some();
    let (host, port, mut c) = if let Some(pipe) = local {
        (address.to_owned(), 0, Connection::local(pipe))
    } else {
        let (host, port, token) = match address.strip_prefix(FRIEND) {
            Some(code) => join_friend(code, lang, send)?,
            None => {
                let Some(target) = parse_address(address) else {
                    return Err(Reason::keyed_arg(
                        "sunrise.multiplayer.invalid_address",
                        lang,
                        address,
                    ));
                };
                status(send, &tr(lang, "connect.connecting"));
                log_connecting(&target.0, target.1);
                (target.0, target.1, Vec::new())
            }
        };
        let (mut c, host, port) =
            connect_tcp(&host, port, address.starts_with(FRIEND), READ_TIMEOUT)
                .map_err(Reason::plain)?;
        // The tunnel's port takes only the client that sends its token first.
        if !token.is_empty() {
            c.writer
                .write_raw(&token)
                .map_err(|e| Reason::plain(e.to_string()))?;
        }
        (host, port, c)
    };
    *shared_counters.lock().unwrap() = Some(c.reader.counters.clone());
    let e = |e: sunrise_protocol::Error| connection_lost(e, lang);
    // ConnectScreen constructs ClientHandshakePacketListenerImpl only once
    // the socket connects. Its cookie map snapshots the old listener here.
    let transferred = transfer_from.is_some();
    if let Some(source) = transfer_from {
        let snapshot = source.lock().unwrap().clone();
        *cookies.lock().unwrap() = snapshot;
    }
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: if transferred {
                handshake::TRANSFER
            } else {
                handshake::LOGIN
            },
        })
        .encode(),
    )
    .map_err(e)?;
    // Vanilla's login sends the account's profile id, or the offline one.
    let uuid = auth.map_or_else(|| offline_uuid(name), |a| a.uuid);
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.to_owned(),
            uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    let profile = loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Clientbound::decode(id, &payload).map_err(e)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(p) => {
                // ClientHandshakePacketListenerImpl.handleLoginFinished installs
                // ClientConfigurationPacketListenerImpl (a ClientCommonPacketListenerImpl)
                // before sending LoginAcknowledged.
                *after_login = true;
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(e)?;
                break p.profile;
            }
            login::Clientbound::Disconnect(p) => {
                return Ok(Reason::component(&p.reason_json, lang));
            }
            login::Clientbound::Hello(hello) => {
                status(send, &tr(lang, "connect.authorizing"));
                let key = RsaPublicKey::from_der(&hello.public_key)
                    .map_err(|err| Reason::plain(format!("Protocol error: {err}")))?;
                let secret = generate_secret().map_err(e)?;
                if hello.should_authenticate {
                    let signed_in = auth.is_some();
                    let authenticated = auth
                        .ok_or_else(|| Reason::keyed("sunrise.multiplayer.not_signed_in", lang))
                        .and_then(|account| {
                            let hash = server_hash(&hello.server_id, &secret, &hello.public_key);
                            // The session service's refusal: the wrapped
                            // `disconnect.loginFailedInfo` text is shown, and
                            // the reason retains the inner key.
                            crate::auth::join_server(account, &hash).map_err(|err| {
                                let (reason, key) = match err.reason() {
                                    Ok(key) => (tr(lang, key), Some(key.to_owned())),
                                    Err(message) => (message.to_owned(), None),
                                };
                                Reason {
                                    text: plain(
                                        &translatable("disconnect.loginFailedInfo", &[&reason]),
                                        lang,
                                    ),
                                    key,
                                    from_server: false,
                                }
                            })
                        });
                    if let Err(reason) = authenticated {
                        if !continues_after_failed_join(lan, signed_in) {
                            return Err(reason);
                        }
                        log::warn!("Authentication failed: {}", reason.text);
                    }
                }
                status(send, &tr(lang, "connect.encrypting"));
                c.send(
                    &login::Serverbound::Key(login::Key {
                        key: key.encrypt(&secret).map_err(e)?,
                        challenge: key.encrypt(&hello.challenge).map_err(e)?,
                    })
                    .encode(),
                )
                .map_err(e)?;
                c.enable_encryption(&secret);
            }
            login::Clientbound::CustomQuery(q) => {
                c.send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: q.transaction,
                        payload: None,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            // ClientHandshakePacketListenerImpl.handleRequestCookie: the
            // cookies a transfer carried over (none on a plain join).
            login::Clientbound::CookieRequest(q) => {
                let payload = cookies.lock().unwrap().get(&q.key).cloned();
                c.send(
                    &login::Serverbound::CookieResponse(login::CookieResponse {
                        key: q.key,
                        payload,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            login::Clientbound::Unknown(..) => {}
        }
    };
    status(send, &tr(lang, "connect.joining"));
    // ClientHandshakePacketListenerImpl.handleLoginFinished: the brand, then
    // the client information.
    // The window's own server thread is no join to an address: it hears vanilla's identity.
    let network =
        (!singleplayer).then(|| (address, sunrise_hooks::engine::client_identity(address)));
    native_client_identity(&mut c, network, &payloads.local).map_err(e)?;
    c.send(&cfg::Serverbound::ClientInformation(info).encode())
        .map_err(e)?;
    let (mut reader, writer) = c.split();
    let writer = QueuedWriter::new(writer).map_err(|error| Reason::plain(error.to_string()))?;
    if let Some(queue) = payloads
        .connection
        .as_ref()
        .and_then(|connection| connection.droppable())
    {
        writer.attach_droppable(queue.clone());
    }
    *shared_writer.lock().unwrap() = Some(writer);
    let (registries, datapacks) = configure(
        &mut reader,
        shared_writer,
        configuring,
        lang,
        send,
        cookies,
        resources,
        payloads,
    )?;
    let registries = Arc::new(registries);
    send.send_wait(Event::Joined {
        registries: registries.clone(),
        datapacks,
        uuid: profile.uuid,
        name: profile.name,
    })
    .map_err(|error| Reason::plain(error.to_string()))?;
    receive_play(
        &mut reader,
        registries,
        shared_writer,
        configuring,
        lang,
        send,
        cookies,
        far,
        resources,
        recorder,
        payloads,
        pings,
        reports,
    )
}

fn report_connection_end(
    result: Result<Reason, Reason>,
    writer: &Mutex<Option<QueuedWriter>>,
    events: &ingress::Producer<Event>,
    after_login: bool,
) {
    let (why, from_server) = match result {
        Ok(why) => (why, true),
        Err(why) => {
            let from_server = why.from_server;
            (why, from_server)
        }
    };
    if let Some(w) = writer.lock().unwrap().take()
        && !w.is_terminal()
    {
        w.shutdown();
    }
    let _ = events.send_terminal(why.ended(from_server, after_login));
}

/// `Connection.setupOutboundProtocol`, run by the ordered main phase task.
fn switch_client_protocol(
    writer: &Mutex<Option<QueuedWriter>>,
    configuring: &AtomicBool,
    target: sunrise_protocol::payload_negotiation::Phase,
    ack: &[u8],
) -> sunrise_protocol::Result<()> {
    let writer = writer.lock().unwrap();
    writer
        .as_ref()
        .ok_or_else(|| sunrise_protocol::Error::new("Disconnected"))?
        .transition_protocol(target, ack)?;
    configuring.store(
        target == sunrise_protocol::payload_negotiation::Phase::Configuration,
        Ordering::Release,
    );
    Ok(())
}

/// The main Play → Configuration action switches the writer before resuming the decoder.
pub struct PhaseChange {
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    configuring: Arc<AtomicBool>,
    payloads: Arc<crate::raw_payloads::Transport>,
    resume: std::sync::mpsc::Sender<sunrise_protocol::Result<()>>,
}

impl PhaseChange {
    pub fn acknowledge(self) {
        self.payloads.begin_configuration();
        let result = switch_client_protocol(
            &self.writer,
            &self.configuring,
            sunrise_protocol::payload_negotiation::Phase::Configuration,
            &play::Serverbound::ConfigurationAcknowledged(Empty).encode(),
        );
        let _ = self.resume.send(result);
    }

    #[cfg(feature = "test-payload-probe")]
    pub fn dummy_for_test() -> Self {
        let (resume, _rx) = std::sync::mpsc::channel();
        Self {
            writer: Arc::new(Mutex::new(None)),
            configuring: Arc::new(AtomicBool::new(false)),
            payloads: crate::raw_payloads::Transport::empty(),
            resume,
        }
    }
}

enum Cookie {
    Request(cfg::CookieRequest),
    Store(cfg::StoreCookie),
}
impl Cookie {
    fn owned_bytes(&self) -> usize {
        match self {
            Self::Request(packet) => packet.key.capacity(),
            Self::Store(packet) => packet.key.capacity() + packet.payload.capacity(),
        }
    }
}

/// One vanilla common listener, not the physical connection. A later phase
/// inherits the cookies but has its own transfer eligibility.
struct CommonIo {
    writer: Arc<Mutex<Option<QueuedWriter>>>,
    cookies: Cookies,
    transferring: AtomicBool,
    phase: sunrise_protocol::payload_negotiation::Phase,
}

impl CommonIo {
    fn new(
        writer: &Arc<Mutex<Option<QueuedWriter>>>,
        cookies: &Cookies,
        phase: sunrise_protocol::payload_negotiation::Phase,
    ) -> Arc<Self> {
        Arc::new(Self {
            writer: writer.clone(),
            cookies: cookies.clone(),
            transferring: AtomicBool::new(false),
            phase,
        })
    }

    fn accepts_packets(&self) -> bool {
        self.writer
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(QueuedWriter::accepts_packets)
    }

    fn may_handle(&self, survives_transfer: bool) -> bool {
        self.accepts_packets() || (survives_transfer && self.transferring.load(Ordering::Acquire))
    }

    fn handle_cookie(&self, cookie: Cookie) {
        match cookie {
            Cookie::Request(q) => {
                let payload = self.cookies.lock().unwrap().get(&q.key).cloned();
                let response = cfg::CookieResponse {
                    key: q.key,
                    payload,
                };
                use sunrise_protocol::payload_negotiation::Phase;
                let body = match self.phase {
                    Phase::Configuration => cfg::Serverbound::CookieResponse(response).encode(),
                    Phase::Play => play::Serverbound::CookieResponse(response).encode(),
                };
                if let Some(writer) = self.writer.lock().unwrap().as_ref() {
                    let _ = writer.send_bypassing_held(&body);
                }
            }
            Cookie::Store(p) => {
                self.cookies.lock().unwrap().insert(p.key, p.payload);
            }
        }
    }

    fn queue_cookie(
        self: &Arc<Self>,
        send: &ingress::Producer<Event>,
        cookie: Cookie,
    ) -> Result<(), Reason> {
        send.send_wait(Event::Cookie(Box::new(CookieTask {
            io: self.clone(),
            cookie,
        })))
        .map_err(|error| Reason::plain(error.to_string()))
    }

    fn queue_transfer(
        self: &Arc<Self>,
        send: &ingress::Producer<Event>,
        packet: cfg::Transfer,
    ) -> Result<(), Reason> {
        // ClientCommonPacketListenerImpl sets this before handing off to main.
        self.transferring.store(true, Ordering::Release);
        send.send_wait(Event::Transfer(Box::new(TransferTask {
            io: self.clone(),
            packet,
        })))
        .map_err(|error| Reason::plain(error.to_string()))
    }
}

pub struct TransferTask {
    io: Arc<CommonIo>,
    packet: cfg::Transfer,
}

impl TransferTask {
    /// A queued Transfer retains its original listener and may run after it closes.
    pub fn into_eligible_packet(self) -> Option<cfg::Transfer> {
        self.is_eligible().then_some(self.packet)
    }

    /// Test the original listener's queued-packet predicate without consuming the task.
    pub fn is_eligible(&self) -> bool {
        self.io.may_handle(true)
    }
}

pub struct CookieTask {
    io: Arc<CommonIo>,
    cookie: Cookie,
}

impl CookieTask {
    pub fn handle(self) {
        // ClientCommonPacketListenerImpl.shouldHandleMessage: only StoreCookie
        // and Transfer are exempt after a transferring listener closes.
        if self.io.may_handle(matches!(self.cookie, Cookie::Store(_))) {
            self.io.handle_cookie(self.cookie);
        }
    }
}

enum PlayIoAction {
    KeepAlive(cfg::KeepAlive),
    ChunkBatchStart,
    ChunkBatchFinished(i32),
    FarTile(Vec<u8>),
}

pub struct PlayIo {
    common: Arc<CommonIo>,
    lang: Arc<Language>,
    far: Arc<crate::lod_net::Inbox>,
    pings: PingSamples,
    reports: Arc<crate::chunk_io::Reports>,
    // ClientPacketListener.chunkBatchSizeCalculator.
    batch: Mutex<sunrise_protocol::chunk_batch::ChunkBatchSize>,
}

impl PlayIo {
    fn close_after_pending(&self) {
        if let Some(writer) = self.common.writer.lock().unwrap().as_ref() {
            writer.close_after_pending();
        }
    }

    /// These Play handlers run on their caller: the standalone reader or
    /// the inline bundle, with only their mod observation deferred to main.
    fn run_reader_diagnostic(&self, packet: &play::Clientbound) -> bool {
        use sunrise_protocol::ids::play::clientbound as id;
        match packet {
            play::Clientbound::PongResponse(p) => {
                let mut pings = self.pings.lock().unwrap();
                if pings.len() == crate::debug_chart::CAPACITY {
                    pings.pop_front();
                }
                pings.push_back(ping_millis().wrapping_sub(p.time));
            }
            play::Clientbound::LowDiskSpaceWarning(_) => self.reports.report_low_disk_space(),
            // The combat messages have no effect; remote debug samples aren't
            // consumed by Sunrise. Keep only their main-thread mod observation.
            play::Clientbound::Unknown(
                id::PLAYER_COMBAT_ENTER | id::PLAYER_COMBAT_END | id::DEBUG_SAMPLE,
                _,
            ) => {}
            _ => return false,
        }
        true
    }

    fn handle(&self, action: PlayIoAction) {
        let reply = match action {
            PlayIoAction::KeepAlive(p) => play::Serverbound::KeepAlive(p),
            PlayIoAction::ChunkBatchStart => return self.batch.lock().unwrap().on_batch_start(),
            PlayIoAction::ChunkBatchFinished(size) => {
                let mut batch = self.batch.lock().unwrap();
                batch.on_batch_finished(size);
                play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: batch.desired_chunks_per_tick(),
                })
            }
            PlayIoAction::FarTile(data) => return self.far.deliver(data),
        };
        if let Some(writer) = self.common.writer.lock().unwrap().as_ref() {
            let _ = writer.send_bypassing_held(&reply.encode());
        }
    }
}

/// The loaded item table for `HoverEvent.ShowItem`, even when a packet's
/// component codec has no synchronized registry context (configuration).
struct ClientboundDecodeNames<'a> {
    registries: &'a dyn Names,
    items: &'a ItemData,
}
impl Names for ClientboundDecodeNames<'_> {
    fn name(&self, registry: &str, id: i32) -> Option<String> {
        self.registries.name(registry, id)
    }
    fn consume_effect_kind(
        &self,
        id: i32,
    ) -> Option<sunrise_protocol::item::consume_effects::Kind> {
        self.registries.consume_effect_kind(id)
    }
    fn registry_entry(&self, registry: &str, id: i32) -> Option<(&str, Option<&Tag>)> {
        self.registries.registry_entry(registry, id)
    }
    fn contains_entry(&self, registry: &str, name: &str) -> bool {
        if registry == "minecraft:item" {
            return self.items.defaults().entries().contains_key(name);
        }
        self.registries.contains_entry(registry, name)
    }
}

fn receive_play(
    reader: &mut ConnReader,
    mut registries: Arc<Dynamic>,
    shared_writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    lang: &Arc<Language>,
    send: &ingress::Producer<Event>,
    cookies: &Cookies,
    far: &Arc<crate::lod_net::Inbox>,
    resources: &ItemData,
    recorder: &RecorderSlot,
    payloads: &Arc<crate::raw_payloads::Transport>,
    pings: &PingSamples,
    reports: &Arc<crate::chunk_io::Reports>,
) -> Result<Reason, Reason> {
    use play_bundle::{PlayBundle, PlayTurn};
    use sunrise_protocol::payload_negotiation::Phase;
    let new_listener = || {
        Arc::new(PlayIo {
            common: CommonIo::new(shared_writer, cookies, Phase::Play),
            lang: lang.clone(),
            far: far.clone(),
            pings: pings.clone(),
            reports: reports.clone(),
            batch: Mutex::default(),
        })
    };
    let mut io = new_listener();
    let mut bundle = PlayBundle::default();
    loop {
        #[cfg(feature = "test-support")]
        if !bundle.assembling() {
            // Report only packets whose ingress work (including a complete bundle) has been published.
            let counters = &reader.counters;
            counters
                .play_ingress_received
                .store(counters.received.load(Ordering::Relaxed), Ordering::Release);
        }
        let (id, payload) = reader.recv().map_err(|e| connection_lost(e, lang))?;
        let decode_names = ClientboundDecodeNames {
            registries: registries.as_ref(),
            items: resources,
        };
        let packet = match play::Clientbound::decode_reader(
            id,
            Reader::with_payload_codecs(&payload, &payloads.local.codecs).with_names(&decode_names),
        ) {
            Ok(p) => p,
            Err(err) => {
                let name = sunrise_protocol::ids::name(
                    sunrise_protocol::State::Play,
                    sunrise_protocol::Direction::Clientbound,
                    id,
                )
                .unwrap_or("?");
                log::warn!("Sunrise: play packet decode failed ({name}, id={id}): {err}");
                return Err(packet_decode_reason(
                    sunrise_protocol::State::Play,
                    id,
                    &err,
                    lang,
                ));
            }
        };
        // Standalone handleDebugSample runs on the reader. Refuse its logger fault
        // before capture, but keep the fault in member order if packing a bundle.
        let oversized_sample = match &packet {
            play::Clientbound::DebugSample(p) if p.sample.len() > play::DEBUG_SAMPLE_DIMENSIONS => {
                Some(p.sample.len())
            }
            _ => None,
        };
        if let Some(size) = oversized_sample
            && !bundle.assembling()
        {
            let error = sunrise_protocol::Error::new(format!(
                "debug sample of {size} values, at most {}",
                play::DEBUG_SAMPLE_DIMENSIONS
            ));
            return Err(connection_lost(error, lang));
        }
        // A malformed sample is not a recordable packet in either path. In
        // a bundle only the ordered fault survives, never its body.
        if oversized_sample.is_none()
            && let Some(rec) = recorder.lock().unwrap().as_ref()
            && rec.is_active()
        {
            rec.received(&packet, id, &payload);
        }
        let member = match packet {
            _ if oversized_sample.is_some() => BundleMember::OversizedDebugSample,
            play::Clientbound::CustomPayload(payload)
                if payload.channel == sunrise_protocol::attachment_sync::CHANNEL =>
            {
                use crate::raw_payloads::{Classified, Incoming};
                let Classified::Other(payload) = Incoming::classify(payloads, Phase::Play, payload)
                    .map_err(|error| Reason::plain(error.to_string()))?
                else {
                    unreachable!("attachment sync is an engine declaration");
                };
                let staged = sunrise_protocol::attachment_sync::decode_staged(payload)
                    .map_err(|error| Reason::plain(format!("attachment sync in play: {error}")))?;
                BundleMember::AttachmentSync(Box::new(staged))
            }
            packet => BundleMember::Packet(packet),
        };
        let turn = bundle.accept(member).map_err(|error| {
            log::warn!("Sunrise: play bundle refused: {error}");
            Reason::generic_exception(error, lang)
        })?;
        let packet = match turn {
            None => continue,
            Some(PlayTurn::Single(BundleMember::AttachmentSync(staged))) => {
                if !io.common.accepts_packets() {
                    continue;
                }
                if send.send_wait(Event::AttachmentSync(staged)).is_err() {
                    return Ok(Reason::plain("Disconnected"));
                }
                continue;
            }
            Some(PlayTurn::Single(BundleMember::Packet(packet))) => packet,
            Some(PlayTurn::Single(BundleMember::OversizedDebugSample)) => {
                unreachable!("an oversized standalone sample was refused before packing")
            }
            Some(PlayTurn::Bundle(packets)) => {
                if io.accepts_bundle() {
                    send.send_wait(Event::Bundle(Box::new(CompletedBundle {
                        packets,
                        io: io.clone(),
                    })))
                    .map_err(|error| Reason::plain(error.to_string()))?;
                }
                continue;
            }
        };
        if !io.common.accepts_packets() {
            continue;
        }
        match packet {
            play::Clientbound::KeepAlive(k) => io.handle(PlayIoAction::KeepAlive(k)),
            // Outside a bundle, these two handlers run on the receive thread.
            play::Clientbound::ChunkBatchStart(_) => io.handle(PlayIoAction::ChunkBatchStart),
            play::Clientbound::ChunkBatchFinished(p) => {
                io.handle(PlayIoAction::ChunkBatchFinished(p.size))
            }
            packet if io.run_reader_diagnostic(&packet) => {
                if mod_api::__private::listened(mod_api::HookId::ClientPacket) {
                    let _ = send.send_wait(Event::ObservePacket(Box::new(packet)));
                }
            }
            play::Clientbound::ResourcePackPush(p) => {
                let _ = send.send_wait(Event::PackPush(p));
            }
            play::Clientbound::ResourcePackPop(p) => {
                let _ = send.send_wait(Event::PackPop(p.id));
            }
            play::Clientbound::StartConfiguration(_) => {
                // StartConfiguration is the real protocol barrier. The main
                // task runs behind preceding play handlers before the ACK.
                let (resume, ready) = std::sync::mpsc::channel();
                send.send_wait(Event::Reconfiguring(Box::new(PhaseChange {
                    writer: shared_writer.clone(),
                    configuring: configuring.clone(),
                    payloads: payloads.clone(),
                    resume,
                })))
                .map_err(|error| Reason::plain(error.to_string()))?;
                ready
                    .recv()
                    .map_err(|_| Reason::plain("Disconnected"))?
                    .map_err(|error| connection_lost(error, lang))?;
                let (snapshot, datapacks) = configure(
                    reader,
                    shared_writer,
                    configuring,
                    lang,
                    send,
                    cookies,
                    resources,
                    payloads,
                )?;
                registries = Arc::new(snapshot);
                io = new_listener();
                send.send_wait(Event::Reconfigured {
                    registries: registries.clone(),
                    datapacks,
                })
                .map_err(|error| Reason::plain(error.to_string()))?;
            }
            play::Clientbound::CookieRequest(q) => {
                io.common.queue_cookie(send, Cookie::Request(q))?;
            }
            play::Clientbound::StoreCookie(p) => {
                io.common.queue_cookie(send, Cookie::Store(p))?;
            }
            play::Clientbound::ServerLinks(p) => {
                let _ = send.send_wait(Event::ServerLinks(p.links));
            }
            play::Clientbound::Transfer(p) => io.common.queue_transfer(send, p)?,
            play::Clientbound::Disconnect(d) => {
                io.close_after_pending();
                return Ok(Reason::tag(&d.reason, lang));
            }
            play::Clientbound::CustomPayload(p) if p.channel == "minecraft:brand" => {
                receive_brand(&p, send);
            }
            // A far tile, straight to the sampler waiting for it.
            play::Clientbound::CustomPayload(p) if p.channel == crate::lod_net::CHANNEL => {
                io.handle(PlayIoAction::FarTile(p.data));
            }
            play::Clientbound::CustomPayload(payload) => {
                if let Some(payload) = receive_application(
                    payloads,
                    sunrise_protocol::payload_negotiation::Phase::Play,
                    payload,
                    send,
                    || {
                        crate::raw_payloads::Endpoint::new(PayloadOutput {
                            writer: shared_writer.clone(),
                            configuring: configuring.clone(),
                            transport: payloads.clone(),
                            events: send.clone(),
                        })
                    },
                )? {
                    send.send_wait(Event::Packet(Box::new(play::Clientbound::CustomPayload(
                        payload,
                    ))))
                    .map_err(|_| Reason::plain("Disconnected"))?;
                }
            }
            other => {
                if send.send_wait(Event::Packet(Box::new(other))).is_err() {
                    return Ok(Reason::plain("Disconnected"));
                }
            }
        }
    }
}

/// Classify before building the output: a passthrough payload never allocates one.
fn receive_application(
    payloads: &Arc<crate::raw_payloads::Transport>,
    phase: sunrise_protocol::payload_negotiation::Phase,
    payload: cfg::CustomPayload,
    send: &ingress::Producer<Event>,
    endpoint: impl FnOnce() -> crate::raw_payloads::Endpoint,
) -> Result<Option<cfg::CustomPayload>, Reason> {
    match crate::raw_payloads::Incoming::classify(payloads, phase, payload)
        .map_err(|error| Reason::plain(error.to_string()))?
    {
        crate::raw_payloads::Classified::Other(payload) => Ok(Some(payload)),
        crate::raw_payloads::Classified::Application(incoming) => {
            crate::raw_payloads::dispatch_client(
                incoming,
                endpoint(),
                crate::raw_payloads::ClientQueue::new(send.clone()),
            );
            Ok(None)
        }
        crate::raw_payloads::Classified::Skipped | crate::raw_payloads::Classified::Raw(_) => {
            Ok(None)
        }
    }
}

/// The configuration phase: registries, tags and known packs until the
/// server finishes it (ClientConfigurationPacketListenerImpl).
fn configure(
    reader: &mut ConnReader,
    writer: &Arc<Mutex<Option<QueuedWriter>>>,
    configuring: &Arc<AtomicBool>,
    lang: &Language,
    send: &ingress::Producer<Event>,
    cookies: &Cookies,
    resources: &ItemData,
    payloads: &Arc<crate::raw_payloads::Transport>,
) -> Result<(Dynamic, crate::datapack_net::Receiver), Reason> {
    use sunrise_protocol::payload_negotiation::Phase;
    let io = CommonIo::new(writer, cookies, Phase::Configuration);
    let config = Arc::new(Configuration::default());
    let mut menu_extra_revision = 0;
    let mut content_verified = false;
    let mut features = None;
    let mut negotiation =
        sunrise_protocol::payload_negotiation::Negotiation::new(payloads.local.manifest.clone());
    let mut datapacks = crate::datapack_net::Receiver::default();
    let reply = |p: cfg::Serverbound| -> Result<(), Reason> {
        match writer.lock().unwrap().as_mut() {
            Some(w) => p
                .try_encode_with_payload_codecs(&payloads.local.codecs)
                .and_then(|body| w.send(&body))
                .map_err(|e| connection_lost(e, lang)),
            None => Err(Reason::plain("Disconnected")),
        }
    };
    let decode_names = ClientboundDecodeNames {
        registries: &NoNames,
        items: resources,
    };
    // Nothing goes out before the server has spoken on the channel (its Query or Offer) or
    // named itself Sunrise by brand: a vanilla server is sent what vanilla's client sends,
    // whatever mods are installed.
    loop {
        let (id, payload) = match reader.recv() {
            Ok(frame) => frame,
            Err(e) => {
                if let Some(reason) = config.take_packet_error() {
                    return Err(reason);
                }
                return Err(connection_lost(e, lang));
            }
        };
        let packet = cfg::Clientbound::decode_reader(
            id,
            Reader::with_payload_codecs(&payload, &payloads.local.codecs).with_names(&decode_names),
        )
        .map_err(|err| {
            let name = sunrise_protocol::ids::name(
                sunrise_protocol::State::Configuration,
                sunrise_protocol::Direction::Clientbound,
                id,
            )
            .unwrap_or("?");
            log::warn!("Sunrise: configuration packet decode failed ({name}, id={id}): {err}");
            packet_decode_reason(sunrise_protocol::State::Configuration, id, &err, lang)
        })?;
        if !io.accepts_packets() {
            continue;
        }
        match packet {
            cfg::Clientbound::KeepAlive(k) => reply(cfg::Serverbound::KeepAlive(k))?,
            cfg::Clientbound::CustomPayload(p)
                if p.channel == sunrise_protocol::payload_negotiation::CHANNEL =>
            {
                let control = sunrise_protocol::payload_negotiation::Control::decode(&p.data)
                    .map_err(|error| Reason::plain(error.to_string()))?;
                if let Some(control) = negotiation
                    .receive(control)
                    .map_err(|error| Reason::plain(error.to_string()))?
                {
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
                        data: control
                            .encode()
                            .map_err(|error| Reason::plain(error.to_string()))?,
                    }))?;
                }
                if let Some(accepted) = negotiation.accepted() {
                    payloads.accept(accepted).map_err(Reason::plain)?;
                }
            }
            cfg::Clientbound::CustomPayload(p) if p.channel == crate::content_identity::CHANNEL => {
                let hash =
                    crate::content_identity::identity(resources.root(), resources.defaults())
                        .map_err(|_| {
                            Reason::plain(crate::content_identity::MISMATCH.to_string())
                        })?;
                content_verified = crate::content_identity::decode_payload(&p.data)
                    .map_err(|error| Reason::plain(error.to_string()))?
                    == Some(hash);
                reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: crate::content_identity::CHANNEL.into(),
                    data: hash.to_vec(),
                }))?;
            }
            cfg::Clientbound::CustomPayload(p) if crate::viewed_levels_net::asked(&p) => {
                reply(crate::viewed_levels_net::answer(&p.data).map_err(Reason::plain)?)?
            }
            cfg::Clientbound::CustomPayload(p) if crate::menu_extra_net::asked(&p) => {
                let (revision, answer) =
                    crate::menu_extra_net::answer(&p.data).map_err(Reason::plain)?;
                reply(answer)?;
                menu_extra_revision = revision;
            }
            cfg::Clientbound::CustomPayload(p) if p.channel == crate::datapack_net::CHANNEL => {
                datapacks.accept_chunk(&p.data).map_err(Reason::plain)?;
            }
            cfg::Clientbound::CustomPayload(p) => {
                let Some(p) = receive_application(
                    payloads,
                    sunrise_protocol::payload_negotiation::Phase::Configuration,
                    p,
                    send,
                    || {
                        crate::raw_payloads::Endpoint::new(PayloadOutput {
                            writer: writer.clone(),
                            configuring: configuring.clone(),
                            transport: payloads.clone(),
                            events: send.clone(),
                        })
                    },
                )?
                else {
                    continue;
                };
                // Sunrise servers use the client brand to negotiate their extensions.
                // Announce those only after the server identifies itself as Sunrise.
                if p.channel == "minecraft:brand"
                    && sunrise_protocol::buf::Reader::new(&p.data)
                        .string(32767)
                        .ok()
                        .as_deref()
                        == Some("sunrise")
                {
                    if let Some(control) = negotiation.require_protocol() {
                        reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
                            data: control
                                .encode()
                                .map_err(|error| Reason::plain(error.to_string()))?,
                        }))?;
                    }
                    let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
                    w.string("sunrise");
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: w.data,
                    }))?;
                    // Announce before replying to known packs, even when the
                    // server has no mods. Never disclose this to vanilla servers.
                    reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: sunrise_protocol::mod_list::CHANNEL.into(),
                        data: sunrise_protocol::mod_list::encode(
                            &sunrise_protocol::mod_list::local(),
                        ),
                    }))?;
                    // And the mods' datapack registries it has (#811).
                    if let Some(data) = crate::datapack_net::offer().map_err(Reason::plain)? {
                        reply(cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                            channel: crate::datapack_net::OFFER_CHANNEL.into(),
                            data,
                        }))?;
                    }
                }
                receive_brand(&p, send);
            }
            cfg::Clientbound::Ping(p) => config.queue(&io, send, ConfigurationAction::Ping(p))?,
            cfg::Clientbound::SelectKnownPacks(p) => {
                config.queue(&io, send, ConfigurationAction::SelectKnownPacks(p))?;
            }
            cfg::Clientbound::UpdateEnabledFeatures(p) => features = Some(p.features),
            cfg::Clientbound::RegistryData(mut r) => {
                let action = if r.registry == sunrise_protocol::item::consume_effects::REGISTRY {
                    ConfigurationAction::ConsumeEffectTypes(r)
                } else {
                    fill_known_entries(&mut r, resources.root());
                    ConfigurationAction::RegistryData(r)
                };
                config.queue(&io, send, action)?;
            }
            cfg::Clientbound::UpdateTags(t) => {
                config.queue(&io, send, ConfigurationAction::UpdateTags(t))?;
            }
            cfg::Clientbound::CodeOfConduct(c) => {
                let _ = send.send_wait(Event::CodeOfConduct(c.text));
            }
            // The game answers: it may prompt the player first.
            cfg::Clientbound::ResourcePackPush(p) => {
                let _ = send.send_wait(Event::PackPush(p));
            }
            cfg::Clientbound::ResourcePackPop(p) => {
                let _ = send.send_wait(Event::PackPop(p.id));
            }
            // ClientCommonPacketListenerImpl.handlePostEffects: queue in listener order so
            // earlier Ping/Pong replies drain before disconnect.packetError closes the connection.
            cfg::Clientbound::PostEffects(_) => {
                config.queue(
                    &io,
                    send,
                    ConfigurationAction::PostEffects(Reason::keyed("disconnect.packetError", lang)),
                )?;
            }
            // The game shows it immediately, before Joined/Reconfigured installs
            // the play registries. Only the dialog entries and tags are dynamic
            // lookups here; keep the snapshot for clicks from this screen too.
            cfg::Clientbound::ShowDialog(d) => {
                config.queue(&io, send, ConfigurationAction::ShowDialog(d))?;
            }
            cfg::Clientbound::ClearDialog(_) => {
                let _ = send.send_wait(Event::ClearDialog);
            }
            cfg::Clientbound::CookieRequest(q) => {
                io.queue_cookie(send, Cookie::Request(q))?;
            }
            cfg::Clientbound::StoreCookie(p) => {
                io.queue_cookie(send, Cookie::Store(p))?;
            }
            cfg::Clientbound::ServerLinks(p) => {
                let _ = send.send_wait(Event::ServerLinks(p.links));
            }
            cfg::Clientbound::Transfer(p) => {
                io.queue_transfer(send, p)?;
            }
            cfg::Clientbound::Disconnect(d) => return Err(Reason::tag(&d.reason, lang)),
            cfg::Clientbound::FinishConfiguration(_) => {
                use configuration::{FinishConfiguration, PlayPreparation};
                // A prior callback may still hold main work. Let the reader
                // wait for it, but hand off either outcome in FIFO order.
                let play = if negotiation.accepted().is_some() {
                    payloads.prepare_play().map(PlayPreparation::Ready)
                } else {
                    negotiation
                        .finish_without_offer()
                        .map_err(|e| e.to_string())
                        .map(|()| PlayPreparation::WithoutOffer(payloads.clone()))
                };
                let (resume, ready) = std::sync::mpsc::channel();
                send.send_wait(Event::FinishConfiguration(Box::new(FinishConfiguration {
                    listener: config.clone(),
                    writer: writer.clone(),
                    configuring: configuring.clone(),
                    play,
                    datapacks,
                    content_verified,
                    menu_extra_revision,
                    resume,
                })))
                .map_err(|error| Reason::plain(error.to_string()))?;
                let (registries, datapacks) = match ready.recv() {
                    Ok(result) => result?,
                    Err(_) => {
                        if let Some(reason) = config.take_packet_error() {
                            return Err(reason);
                        }
                        return Err(Reason::plain("Disconnected"));
                    }
                };
                if let Some(features) = features {
                    let _ = send.send_wait(Event::Features(features));
                }
                return Ok((registries, datapacks));
            }
            _ => {}
        }
    }
}

#[sunrise_mod_api_macros::replaceable]
fn receive_brand(payload: &cfg::CustomPayload, send: &ingress::Producer<Event>) {
    if let Some(event) = brand_event(payload) {
        let _ = send.send_wait(event);
    }
}

fn brand_event(payload: &cfg::CustomPayload) -> Option<Event> {
    match payload.channel.as_str() {
        "minecraft:brand" => sunrise_protocol::buf::Reader::new(&payload.data)
            .string(32767)
            .ok()
            .map(Event::Brand),
        sunrise_protocol::quiet::CHANNEL => Some(Event::QuietCommands),
        sunrise_protocol::custom_book::CHANNEL => Some(Event::CustomBooks),
        crate::lod_net::CHANNEL => {
            crate::lod_net::Offer::decode(&payload.data).map(Event::FarTerrain)
        }
        _ => None,
    }
}

/// Select the identity for initial configuration, before any client information is sent.
fn native_client_identity(
    c: &mut Connection,
    network: Option<(&str, sunrise_hooks::ClientIdentity)>,
    local_payloads: &crate::raw_payloads::Local,
) -> Result<(), sunrise_protocol::Error> {
    let Some((address, default)) = network else {
        return send_identity(c, &sunrise_hooks::ClientIdentity::vanilla());
    };
    struct Host<'a>(&'a str);
    impl mod_api::ClientIdentityHost for Host<'_> {
        fn server_address(&self) -> &str {
            self.0
        }
    }
    let mut answer = None;
    mod_api::dispatch_client_identity(&Host(address), &mut answer);
    let Some(mod_api::ClientIdentity {
        brand, payloads, ..
    }) = answer
    else {
        return send_identity(c, &default);
    };
    let payloads = payloads
        .into_iter()
        .map(|payload| {
            // An empty namespace still denotes `minecraft` on the wire. Check the
            // effective channel so `:brand` cannot bypass engine ownership.
            let channel = if payload.channel.namespace().is_empty() {
                format!("minecraft:{}", payload.channel.path())
            } else {
                payload.channel.to_string()
            };
            if sunrise_protocol::packets::custom_payload::is_reserved_channel(&channel)
                || local_payloads.owns(&channel)
            {
                return Err(sunrise_protocol::Error::new(format!(
                    "client identity payload {channel} is reserved or transport-owned"
                )));
            }
            // Identifier.STREAM_CODEC uses ByteBufCodecs.STRING_UTF8. Valid identifiers are ASCII.
            if channel.len() > 32_767 {
                return Err(sunrise_protocol::Error::new(
                    "client identity payload identifier is longer than 32767 characters",
                ));
            }
            Ok((channel, payload.bytes))
        })
        .collect::<Result<_, _>>()?;
    send_identity(c, &sunrise_hooks::ClientIdentity { brand, payloads })
}

/// `ClientBrandRetriever.getClientModName`, then the identity's configuration payloads, in order.
/// Every packet is encoded before any is sent, so an identity the codecs refuse sends nothing.
fn send_identity(
    c: &mut Connection,
    identity: &sunrise_hooks::ClientIdentity,
) -> Result<(), sunrise_protocol::Error> {
    let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
    brand.string(&identity.brand);
    let brand = ("minecraft:brand", brand.data);
    let packets = std::iter::once(brand)
        .chain(
            identity
                .payloads
                .iter()
                .map(|(channel, data)| (channel.as_str(), data.clone())),
        )
        .map(|(channel, data)| {
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: channel.into(),
                data,
            })
            .try_encode()
        })
        .collect::<Result<Vec<_>, _>>()?;
    for packet in &packets {
        c.send(packet)?;
    }
    Ok(())
}

/// Flatten a JSON chat component (login disconnects are JSON).
/// At most `sunrise_protocol::text::MAX_TEXT` bytes, as `plain`: a login-phase reason from a
/// server is any JSON it likes.
pub fn json_text(v: &serde_json::Value, lang: &Language) -> String {
    json_text_with(v, |key| lang.get(key).map(String::as_str))
}

fn json_text_with<'a>(v: &serde_json::Value, lookup: impl Fn(&str) -> Option<&'a str>) -> String {
    fn walk<'a, F: Fn(&str) -> Option<&'a str>>(v: &serde_json::Value, lookup: &F) -> String {
        use sunrise_protocol::text::{MAX_TEXT, push_capped};
        match v {
            serde_json::Value::String(s) => {
                let mut out = String::new();
                push_capped(&mut out, s);
                out
            }
            serde_json::Value::Array(a) => {
                let mut out = String::new();
                for e in a {
                    if out.len() >= MAX_TEXT {
                        break;
                    }
                    push_capped(&mut out, &walk(e, lookup));
                }
                out
            }
            serde_json::Value::Object(o) => {
                let mut out = if let Some(t) = o.get("text").and_then(|t| t.as_str()) {
                    let mut out = String::new();
                    push_capped(&mut out, t);
                    out
                } else if let Some(key) = o.get("translate").and_then(|t| t.as_str()) {
                    let args: Vec<String> = o
                        .get("with")
                        .and_then(|w| w.as_array())
                        .map(|a| a.iter().map(|e| walk(e, lookup)).collect())
                        .unwrap_or_default();
                    let pattern = lookup(key)
                        .map(str::to_owned)
                        .or_else(|| {
                            o.get("fallback")
                                .and_then(|f| f.as_str())
                                .map(str::to_owned)
                        })
                        .unwrap_or_else(|| key.to_owned());
                    sunrise_protocol::text::format_java(&pattern, &args)
                } else {
                    String::new()
                };
                if let Some(extra) = o.get("extra").and_then(|e| e.as_array()) {
                    for e in extra {
                        if out.len() >= MAX_TEXT {
                            break;
                        }
                        push_capped(&mut out, &walk(e, lookup));
                    }
                }
                out
            }
            other => other.to_string(),
        }
    }
    walk(v, &lookup)
}

pub use sunrise_sim::lang_load::load_language;

/// Ask a server for its status (ServerStatusPinger): MOTD, players and the
/// round trip time in milliseconds.
pub fn ping(address: &str, timeout: Duration) -> Result<(serde_json::Value, u128), String> {
    use sunrise_protocol::packets::status;
    let (host, port) = parse_address(address).ok_or("Invalid address")?;
    let mut c = Connection::connect_server(&host, port, timeout).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let e = |e: sunrise_protocol::Error| e.to_string();
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: host.clone(),
            port,
            intent: handshake::STATUS,
        })
        .encode(),
    )
    .map_err(e)?;
    c.send(&status::Serverbound::StatusRequest(Empty).encode())
        .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let json = match status::Clientbound::decode(id, &payload).map_err(e)? {
        status::Clientbound::StatusResponse(s) => {
            serde_json::from_str(&s.json).map_err(|e| e.to_string())?
        }
        _ => return Err("unexpected status reply".into()),
    };
    let start = std::time::Instant::now();
    c.send(&status::Serverbound::PingRequest(status::Ping { time: 1 }).encode())
        .map_err(e)?;
    let _ = c.recv().map_err(e)?;
    Ok((json, start.elapsed().as_millis()))
}

/// What a server that answers only the legacy ping reports
/// (`LegacyServerPinger.Output.handleResponse`).
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyStatus {
    pub protocol: i32,
    pub version: String,
    pub motd: String,
    pub players: i32,
    pub max_players: i32,
}

/// `Mth.getInt(str, default)`.
fn legacy_int(s: &str, default: i32) -> i32 {
    s.parse().unwrap_or(default)
}

/// `LegacyServerPinger.channelRead0` on the string after the 0xFF byte: split
/// on NUL (at most 6 parts) and the first part must be `§1`.
fn parse_legacy_status(text: &str) -> Option<LegacyStatus> {
    let parts: Vec<&str> = text.splitn(6, '\0').collect();
    if parts.first() != Some(&"\u{a7}1") || parts.len() < 6 {
        return None;
    }
    Some(LegacyStatus {
        protocol: legacy_int(parts[1], 0),
        version: parts[2].to_owned(),
        motd: parts[3].to_owned(),
        players: legacy_int(parts[4], -1),
        max_players: legacy_int(parts[5], -1),
    })
}

/// `LegacyProtocolUtils.writeLegacyString`: the UTF-16 length, then UTF-16BE.
fn write_legacy_string(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u16> = s.encode_utf16().collect();
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for unit in units {
        out.extend_from_slice(&unit.to_be_bytes());
    }
}

/// `LegacyServerPinger.channelActive`: the 1.6 `0xFE 0x01 0xFA MC|PingHost` request.
fn legacy_ping_request(host: &str, port: u16) -> Vec<u8> {
    let mut out = vec![254, 1, 250];
    write_legacy_string(&mut out, "MC|PingHost");
    let mut payload = vec![127];
    write_legacy_string(&mut payload, host);
    payload.extend_from_slice(&i32::from(port).to_be_bytes());
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend_from_slice(&payload);
    out
}

/// `ServerStatusPinger.pingLegacyServer`: ask a server that does not answer
/// the status handshake with the pre-1.7 ping.
pub fn ping_legacy(address: &str, timeout: Duration) -> Result<LegacyStatus, String> {
    let (host, port) = parse_address(address).ok_or("Invalid address")?;
    let mut c = Connection::connect_server(&host, port, timeout).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(timeout))
        .map_err(|e| e.to_string())?;
    let e = |e: sunrise_protocol::Error| e.to_string();
    c.writer
        .write_raw(&legacy_ping_request(&host, port))
        .map_err(e)?;
    let head = c.reader.read_raw(3).map_err(e)?;
    if head[0] != 255 {
        return Err("unexpected legacy reply".into());
    }
    let units = usize::from(u16::from_be_bytes([head[1], head[2]]));
    let bytes = c.reader.read_raw(units * 2).map_err(e)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes(*p))
        .collect();
    parse_legacy_status(&String::from_utf16_lossy(&units))
        .ok_or_else(|| "unexpected legacy reply".to_owned())
}

/// `ClientHandshakePacketListenerImpl.handleHello`: a failed session join goes
/// on to encryption only for an entry picked from LAN discovery
/// (`ServerData.isLan`), and the host decides. A vanilla client always has an
/// account; Sunrise without one never joins anything (Mojang's rules: no play
/// without an account that owns the game), LAN included.
fn continues_after_failed_join(lan: bool, signed_in: bool) -> bool {
    lan && signed_in
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunrise_world::native_load_context::DataLoadOrigin;

    pub(super) fn empty_item_data(name: &str) -> (PathBuf, Arc<ItemData>) {
        let root =
            std::env::temp_dir().join(format!("sunrise-net-data-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        (root, data)
    }

    // Drive the real reader and the test's main actor separately. Closing the
    // last producer ends the actor, even on a failed handshake.
    fn drive_configuration<T>(
        read: impl FnOnce(&ingress::Producer<Event>) -> T,
        mut effect: impl FnMut(Event) + Send,
    ) -> T {
        let (send, events) = event_channel();
        std::thread::scope(|scope| {
            scope.spawn(move || {
                while let Ok(receipt) = events.recv_timeout_owned(Duration::from_secs(10)) {
                    let (event, _credit) = receipt.into_parts();
                    match event {
                        Event::Configuration(task) => {
                            if let Some(event) = task.handle() {
                                effect(event);
                            }
                        }
                        Event::Cookie(task) => task.handle(),
                        Event::FinishConfiguration(change) => change.acknowledge(),
                        event => effect(event),
                    }
                }
            });
            let result = read(&send);
            drop(send);
            result
        })
    }

    #[test]
    fn sunrise_server_without_raw_protocol_is_refused_at_finish() {
        for wrong_version in [false, true] {
            let (root, data) = empty_item_data("raw-protocol-refusal");
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            let (mut reader, writer) = Connection::local(client).split();
            reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let thread = std::thread::spawn(move || {
                drive_configuration(
                    |send| {
                        configure(
                            &mut reader,
                            &Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
                            &Arc::new(AtomicBool::new(true)),
                            &Language::new(),
                            send,
                            &Cookies::default(),
                            &data,
                            &crate::raw_payloads::Transport::empty(),
                        )
                    },
                    |_| {},
                )
            });
            let mut server = Connection::local(server);
            let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
            brand.string("sunrise");
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: brand.data,
                    })
                    .encode(),
                )
                .unwrap();
            if wrong_version {
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
                            data: vec![0, sunrise_protocol::payload_negotiation::RAW_PROTOCOL + 1],
                        })
                        .encode(),
                    )
                    .unwrap();
            } else {
                server
                    .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                    .unwrap();
            }
            let Err(error) = thread.join().unwrap() else {
                panic!("old Sunrise server admitted");
            };
            assert!(error.text.contains(if wrong_version {
                "protocol version"
            } else {
                "missing response"
            }));
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// A client with a mod's payload declaration, joined to a server that offers nothing: the
    /// configuration phase carries no `sunrise:payloads` packet, as vanilla's carries none. When
    /// the server Queries the channel, the client answers with its Offer, mod declaration included.
    #[test]
    fn mod_client_offers_payloads_only_when_the_server_asks() {
        use sunrise_protocol::payload_negotiation::{CHANNEL, Control};
        for server_asks in [false, true] {
            let (root, data) = empty_item_data(&format!("mod-client-{server_asks}"));
            let sent = client_payload_test_support::with_client(|transport| {
                assert!(transport.local.has_mod_declarations());
                let (client, server) = sunrise_protocol::connection::Pipe::pair();
                let (mut reader, writer) = Connection::local(client).split();
                reader
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let data = data.clone();
                let thread = std::thread::spawn(move || {
                    drive_configuration(
                        |send| {
                            configure(
                                &mut reader,
                                &Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
                                &Arc::new(AtomicBool::new(true)),
                                &Language::new(),
                                send,
                                &Cookies::default(),
                                &data,
                                &transport,
                            )
                        },
                        |_| {},
                    )
                });
                let mut server = Connection::local(server);
                let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
                brand.string("vanilla");
                let mut to_client = vec![cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: brand.data,
                })];
                if server_asks {
                    to_client.push(cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: Control::Query.encode().unwrap(),
                    }));
                }
                to_client.push(cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks {
                    packs: vec![],
                }));
                for packet in to_client {
                    server.send(&packet.encode()).unwrap();
                }
                // The known packs reply comes after any offer: the server's Query is read first.
                let mut sent = Vec::new();
                loop {
                    let (id, body) = server.recv().unwrap();
                    let packet = cfg::Serverbound::decode(id, &body).unwrap();
                    let done = matches!(packet, cfg::Serverbound::SelectKnownPacks(_));
                    sent.push(packet);
                    if done {
                        break;
                    }
                }
                server
                    .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                // A required mod channel the server lacks refuses the join at the barrier
                // (`Missing`), as before: the join's end is not what this test pins.
                let _ = thread.join().unwrap();
                sent
            });
            let offers: Vec<_> = sent
                .iter()
                .filter_map(|packet| match packet {
                    cfg::Serverbound::CustomPayload(p) if p.channel == CHANNEL => {
                        Some(Control::decode(&p.data).unwrap())
                    }
                    _ => None,
                })
                .collect();
            if server_asks {
                let [Control::Offer(manifest)] = offers.as_slice() else {
                    panic!("one Offer expected, got {offers:?}");
                };
                assert!(
                    manifest
                        .entries()
                        .iter()
                        .any(|e| e.id() == client_payload_test_support::CHANNEL),
                    "the Offer lists the mod's channel"
                );
            } else {
                assert_eq!(offers, [], "nothing on the channel unasked");
                assert_eq!(
                    sent.len(),
                    1,
                    "only the known packs reply, as vanilla's client answers: {sent:?}"
                );
            }
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn identity_sends_the_brand_then_its_payloads_in_order() {
        for (payloads, expected) in [
            (vec![], vec![("minecraft:brand", b"\x07vanilla".to_vec())]),
            (
                vec![
                    ("a:one".to_owned(), vec![1, 1, 0]),
                    ("a:two".to_owned(), vec![]),
                ],
                vec![
                    ("minecraft:brand", b"\x03abc".to_vec()),
                    ("a:one", vec![1, 1, 0]),
                    ("a:two", vec![]),
                ],
            ),
        ] {
            let brand = if payloads.is_empty() {
                "vanilla"
            } else {
                "abc"
            };
            let identity = sunrise_hooks::ClientIdentity {
                brand: brand.to_owned(),
                payloads,
            };
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            let mut client = Connection::local(client);
            send_identity(&mut client, &identity).unwrap();
            // Closed before reading, so a packet too many shows as one more read, not a wait.
            drop(client);
            let mut server = Connection::local(server);
            for (channel, data) in expected {
                let (id, body) = server.recv().unwrap();
                let cfg::Serverbound::CustomPayload(p) =
                    cfg::Serverbound::decode(id, &body).unwrap()
                else {
                    panic!("expected a custom payload")
                };
                assert_eq!((p.channel.as_str(), p.data), (channel, data));
            }
            assert!(server.recv().is_err(), "a packet after the expected ones");
        }
    }

    struct JoinIdentityMod {
        answers: Vec<(&'static str, mod_api::ClientIdentity)>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    #[mod_api::__subscriptions]
    impl mod_api::Hooks for JoinIdentityMod {
        fn client_identity(
            &self,
            host: &dyn mod_api::ClientIdentityHost,
            _: &mut mod_api::ClientIdentityEvent,
        ) -> Option<mod_api::ClientIdentity> {
            let address = host.server_address();
            self.seen.lock().unwrap().push(address.to_owned());
            self.answers
                .iter()
                .find(|(target, _)| *target == address)
                .map(|(_, answer)| answer.clone())
        }
    }

    #[test]
    fn initial_identity_uses_the_mod_answer_for_each_network_join() {
        use mod_api::{ClientIdentity, ClientIdentityPayload, Identifier, ModEntry};
        use sunrise_protocol::connection::Pipe;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let later_seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![
            ModEntry::new(
                "first_identity",
                Box::new(JoinIdentityMod {
                    answers: vec![
                        (
                            "Join.Example:25572",
                            ClientIdentity::new(
                                "custom",
                                vec![
                                    ClientIdentityPayload::new(
                                        Identifier::parse("fixture:first").unwrap(),
                                        vec![5, 9],
                                    ),
                                    ClientIdentityPayload::new(
                                        Identifier::parse("fixture:second").unwrap(),
                                        vec![],
                                    ),
                                ],
                            ),
                        ),
                        ("Next.Example:25573", ClientIdentity::new("moved", vec![])),
                    ],
                    seen: seen.clone(),
                }),
            ),
            ModEntry::new(
                "later_identity",
                Box::new(JoinIdentityMod {
                    answers: vec![("Join.Example:25572", ClientIdentity::new("later", vec![]))],
                    seen: later_seen.clone(),
                }),
            ),
        ];
        mod_platform::testing::with_mods(mods.into(), || {
            let local = crate::raw_payloads::Local::empty();
            for (address, expected) in [
                (
                    Some("Join.Example:25572"),
                    vec![
                        ("minecraft:brand", b"\x06custom".as_slice()),
                        ("fixture:first", &[5, 9]),
                        ("fixture:second", &[]),
                    ],
                ),
                (
                    Some("Next.Example:25573"),
                    vec![("minecraft:brand", b"\x05moved".as_slice())],
                ),
                (
                    Some("Elsewhere.Example"),
                    vec![
                        ("minecraft:brand", b"\x05layer".as_slice()),
                        ("fixture:layer", &[3]),
                    ],
                ),
                (None, vec![("minecraft:brand", b"\x07vanilla".as_slice())]),
            ] {
                let network = address.map(|address| {
                    (
                        address,
                        sunrise_hooks::ClientIdentity {
                            brand: "layer".into(),
                            payloads: vec![("fixture:layer".into(), vec![3])],
                        },
                    )
                });
                let (client, server) = Pipe::pair();
                let mut client = Connection::local(client);
                native_client_identity(&mut client, network, &local).unwrap();
                drop(client);
                let mut server = Connection::local(server);
                for (channel, bytes) in expected {
                    let (id, body) = server.recv().unwrap();
                    let cfg::Serverbound::CustomPayload(payload) =
                        cfg::Serverbound::decode(id, &body).unwrap()
                    else {
                        panic!("expected a custom payload");
                    };
                    assert_eq!(
                        (payload.channel.as_str(), payload.data.as_slice()),
                        (channel, bytes)
                    );
                }
                assert!(server.recv().is_err(), "a packet after the expected ones");
            }
        });
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [
                "Join.Example:25572",
                "Next.Example:25573",
                "Elsewhere.Example"
            ]
        );
        assert_eq!(later_seen.lock().unwrap().as_slice(), ["Elsewhere.Example"]);
    }

    fn assert_refused_identity_channel(channel: mod_api::Identifier, diagnostic: &str) {
        use mod_api::{ClientIdentity, ClientIdentityPayload, ModEntry};
        use sunrise_protocol::connection::Pipe;

        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "refused_identity",
                Box::new(JoinIdentityMod {
                    answers: vec![(
                        "Join.Example",
                        ClientIdentity::new(
                            "custom",
                            vec![ClientIdentityPayload::new(channel, b"\x03two".to_vec())],
                        ),
                    )],
                    seen: Arc::default(),
                }),
            )]
            .into(),
            || {
                let (client, server) = Pipe::pair();
                let mut client = Connection::local(client);
                let error = native_client_identity(
                    &mut client,
                    Some(("Join.Example", sunrise_hooks::ClientIdentity::vanilla())),
                    &crate::raw_payloads::Local::empty(),
                )
                .unwrap_err();
                assert!(error.to_string().contains(diagnostic), "{error}");
                drop(client);
                assert!(
                    Connection::local(server).recv().is_err(),
                    "sent part of a refused identity"
                );
            },
        );
    }

    #[test]
    fn initial_identity_sends_nothing_unless_every_packet_fits() {
        use mod_api::{ClientIdentity, ClientIdentityPayload, Identifier, ModEntry};
        use sunrise_protocol::connection::Pipe;

        let payload = |len| {
            vec![ClientIdentityPayload::new(
                Identifier::parse("fixture:body").unwrap(),
                vec![7; len],
            )]
        };
        // ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE is 32767 bytes; BrandPayload.write
        // writes the brand with FriendlyByteBuf.writeUtf, at most 32767 characters.
        for (brand, payloads, sent) in [
            ("custom".to_owned(), payload(32_767), true),
            ("custom".to_owned(), payload(32_768), false),
            ("b".repeat(32_768), payload(0), false),
        ] {
            let answer = ClientIdentity::new(brand.clone(), payloads);
            mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "limits",
                    Box::new(JoinIdentityMod {
                        answers: vec![("Join.Example", answer)],
                        seen: Arc::default(),
                    }),
                )]
                .into(),
                || {
                    let (client, server) = Pipe::pair();
                    let mut client = Connection::local(client);
                    let result = native_client_identity(
                        &mut client,
                        Some(("Join.Example", sunrise_hooks::ClientIdentity::vanilla())),
                        &crate::raw_payloads::Local::empty(),
                    );
                    drop(client);
                    let mut server = Connection::local(server);
                    if sent {
                        result.unwrap();
                        let mut received = Vec::new();
                        while let Ok((id, body)) = server.recv() {
                            let cfg::Serverbound::CustomPayload(payload) =
                                cfg::Serverbound::decode(id, &body).unwrap()
                            else {
                                panic!("expected a custom payload");
                            };
                            received.push((payload.channel.to_string(), payload.data.len()));
                        }
                        assert_eq!(
                            received,
                            [
                                ("minecraft:brand".to_owned(), 7),
                                ("fixture:body".to_owned(), 32_767)
                            ]
                        );
                    } else {
                        assert!(result.is_err(), "brand of {} bytes", brand.len());
                        assert!(server.recv().is_err(), "sent part of a refused identity");
                    }
                },
            );
        }
    }

    #[test]
    fn initial_identity_refuses_brand_as_an_extra_payload() {
        for namespace in ["minecraft", ""] {
            assert_refused_identity_channel(
                mod_api::Identifier::from_parts(namespace, "brand").unwrap(),
                "minecraft:brand is reserved",
            );
        }
    }

    #[test]
    fn initial_identity_refuses_an_overlong_channel_identifier() {
        // The namespace and colon make this 32,768 characters (Identifier.STREAM_CODEC).
        assert_refused_identity_channel(
            mod_api::Identifier::from_parts("fixture", &"x".repeat(32_760)).unwrap(),
            "identifier is longer than 32767 characters",
        );
    }

    #[test]
    fn configuration_keeps_mod_lists_private_until_the_server_identifies_as_sunrise() {
        for server_brand in ["vanilla", "sunrise"] {
            let (root, data) = empty_item_data(&format!("mod-list-{server_brand}"));
            let (client, server) = sunrise_protocol::connection::Pipe::pair();
            let (mut reader, writer) = Connection::local(client).split();
            reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            // configure queues its final ACK asynchronously. Keep its writer alive
            // until the peer has received that ACK, just as the real connection
            // retains it for play; dropping the last owner closes the pipe.
            let shared_writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
            let client_writer = shared_writer.clone();
            let thread = std::thread::spawn(move || {
                drive_configuration(
                    |send| {
                        configure(
                            &mut reader,
                            &client_writer,
                            &Arc::new(AtomicBool::new(true)),
                            &Language::new(),
                            send,
                            &Cookies::default(),
                            &data,
                            &crate::raw_payloads::Transport::empty(),
                        )
                    },
                    |_| {},
                )
            });
            let mut server = Connection::local(server);
            server
                .reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            // A payload alone must not solicit private identities.
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: sunrise_protocol::mod_list::CHANNEL.into(),
                        data: sunrise_protocol::mod_list::encode(&[]),
                    })
                    .encode(),
                )
                .unwrap();
            server
                .send(&cfg::Clientbound::Ping(cfg::Ping { id: 17 }).encode())
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::Pong(cfg::Ping { id: 17 }),
                "an unidentified server received a mod announcement"
            );
            let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
            brand.string(server_brand);
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: brand.data,
                    })
                    .encode(),
                )
                .unwrap();
            if server_brand == "sunrise" {
                use sunrise_protocol::payload_negotiation::{CHANNEL, Control};
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Query.encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
            }
            server
                .send(
                    &cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap();
            let mut declared_mods = false;
            let mut saw_offer = false;
            loop {
                let (id, body) = server.recv().unwrap();
                match cfg::Serverbound::decode(id, &body).unwrap() {
                    cfg::Serverbound::CustomPayload(p)
                        if p.channel == sunrise_protocol::payload_negotiation::CHANNEL =>
                    {
                        use sunrise_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                        if Control::decode(&p.data).unwrap() == Control::Query {
                            server
                                .send(
                                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                                        channel: CHANNEL.into(),
                                        data: Control::Offer(Manifest::default()).encode().unwrap(),
                                    })
                                    .encode(),
                                )
                                .unwrap();
                        } else {
                            saw_offer = true;
                        }
                    }
                    cfg::Serverbound::CustomPayload(p)
                        if p.channel == sunrise_protocol::mod_list::CHANNEL =>
                    {
                        declared_mods = true;
                    }
                    cfg::Serverbound::SelectKnownPacks(_) => break,
                    _ => {}
                }
            }
            assert_eq!(declared_mods, server_brand == "sunrise");
            assert_eq!(saw_offer, server_brand == "sunrise");
            server
                .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::FinishConfiguration(Empty)
            );
            thread
                .join()
                .unwrap()
                .unwrap_or_else(|reason| panic!("configuration failed: {}", reason.text));
            drop(shared_writer);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn a_login_reason_repeating_its_argument_stays_bounded() {
        let pattern = "%1$s".repeat(16_000);
        let mut v = serde_json::json!({ "text": "A" });
        for _ in 0..3 {
            v = serde_json::json!({ "translate": "x.unknown", "fallback": pattern, "with": [v] });
        }
        let v = serde_json::json!({ "text": "", "extra": [v.clone(), v] });
        let text = json_text(&v, &Language::default());
        assert!(text.len() <= sunrise_protocol::text::MAX_TEXT);
    }

    #[test]
    fn disconnect_reasons_layer_runtime_overrides_over_pack_only_session_maps() {
        use crate::lang::{self, Language as ClientLanguage};
        use std::path::Path;

        // Unmodded mid-session locale change keeps resolving from the session's captured map.
        {
            let _isolated = lang::isolate_publication(ClientLanguage::of(
                "en_us",
                [("disconnect.test".into(), "English pack".into())],
            ));
            let session_lang = load_language(Path::new(""));
            lang::set(ClientLanguage::of(
                "de_de",
                [("disconnect.test".into(), "German pack".into())],
            ));
            assert_eq!(
                Reason::component(r#"{"translate":"disconnect.test"}"#, &session_lang).text,
                "English pack"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.test", &[]), &session_lang).text,
                "English pack"
            );
        }

        // A session opened during an override still captures the pack-only fallback.
        {
            let _isolated = lang::isolate_publication(ClientLanguage::of(
                "en_us",
                [
                    ("disconnect.test".into(), "Pack reason".into()),
                    ("disconnect.wrap".into(), "Wrap: %s".into()),
                ],
            ));
            lang::set_mod_entries(
                "disconnect_mod",
                "en_us",
                HashMap::from([
                    ("disconnect.test".into(), "Mod reason".into()),
                    ("disconnect.mod_only".into(), "Mod only".into()),
                ]),
            );
            let session_lang = load_language(Path::new(""));
            assert_eq!(
                Reason::component(r#"{"translate":"disconnect.test"}"#, &session_lang).text,
                "Mod reason"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.test", &[]), &session_lang).text,
                "Mod reason"
            );
            assert_eq!(
                Reason::component(
                    r#"{"translate":"disconnect.wrap","with":[{"translate":"disconnect.mod_only"}]}"#,
                    &session_lang,
                )
                .text,
                "Wrap: Mod only"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.mod_only", &[]), &session_lang).text,
                "Mod only"
            );

            lang::clear_mod_entries("disconnect_mod");
            assert_eq!(
                Reason::component(r#"{"translate":"disconnect.test"}"#, &session_lang).text,
                "Pack reason"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.test", &[]), &session_lang).text,
                "Pack reason"
            );
            assert_eq!(
                Reason::component(
                    r#"{"translate":"disconnect.mod_only","fallback":"Fallback"}"#,
                    &session_lang,
                )
                .text,
                "Fallback"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.mod_only", &[]), &session_lang).text,
                "disconnect.mod_only"
            );

            lang::set_mod_entries(
                "disconnect_mod",
                "en_us",
                HashMap::from([("disconnect.test".into(), "Late override".into())]),
            );
            assert_eq!(
                Reason::component(r#"{"translate":"disconnect.test"}"#, &session_lang).text,
                "Late override"
            );
            assert_eq!(
                Reason::tag(&translatable("disconnect.test", &[]), &session_lang).text,
                "Late override"
            );
        }
    }

    fn pack(namespace: &str, id: &str, version: &str) -> cfg::KnownPack {
        cfg::KnownPack {
            namespace: namespace.into(),
            id: id.into(),
            version: version.into(),
        }
    }

    #[test]
    fn the_client_knows_vanilla_core_pack_of_its_own_version_only() {
        let core = pack("minecraft", "core", sunrise_protocol::GAME_VERSION);
        let offered = vec![
            core.clone(),
            pack(
                "minecraft",
                "trade_rebalance",
                sunrise_protocol::GAME_VERSION,
            ),
            pack("example", "server_pack", "1"),
        ];
        assert_eq!(select_known_packs(offered), vec![core]);
        assert_eq!(
            select_known_packs(vec![pack("minecraft", "core", "1.21.11")]),
            vec![]
        );
        assert_eq!(select_known_packs(vec![]), vec![]);
    }

    #[test]
    fn a_known_entry_takes_its_data_from_the_clients_pack() {
        let root = std::env::temp_dir().join(format!("hz-known-entries-{}", std::process::id()));
        let dir = root.join("data/minecraft/damage_type");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("in_fire.json"),
            br#"{"message_id": "inFire", "exhaustion": 0.1, "scaling": "when_caused_by_living_non_player"}"#,
        )
        .unwrap();
        let sent = Tag::compound(vec![(
            "message_id",
            Tag::String(JavaString::from("custom")),
        )]);
        let mut r = cfg::RegistryData {
            registry: "minecraft:damage_type".into(),
            entries: vec![
                cfg::RegistryEntry {
                    id: "minecraft:in_fire".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "minecraft:custom".into(),
                    data: Some(sent.clone()),
                },
                cfg::RegistryEntry {
                    id: "minecraft:missing".into(),
                    data: None,
                },
            ],
        };
        fill_known_entries(&mut r, &root);
        std::fs::remove_dir_all(&root).unwrap();
        let Some(Tag::Compound(filled)) = &r.entries[0].data else {
            panic!("in_fire was not filled: {:?}", r.entries[0].data);
        };
        assert_eq!(
            filled.get(&JavaString::from("message_id")),
            Some(&Tag::String(JavaString::from("inFire")))
        );
        // Data the server sent stays; an entry the client has no file for stays empty.
        assert_eq!(r.entries[1].data, Some(sent));
        assert_eq!(r.entries[2].data, None);
    }

    /// Launchers find the last server joined in `latest.log` by vanilla's line.
    #[test]
    fn the_connecting_line_is_vanillas() {
        let line = |address: &str| {
            let (host, port) = parse_address(address).unwrap();
            connecting_line(&host, port)
        };
        assert_eq!(
            line("play.example.net"),
            "Connecting to play.example.net, 25565"
        );
        assert_eq!(
            line("play.example.net:25577"),
            "Connecting to play.example.net, 25577"
        );
        assert_eq!(line("[::1]:25566"), "Connecting to ::1, 25566");
    }

    #[test]
    #[ignore = "daily"]
    fn direct_login_preserves_hostname_and_negotiates_brand() {
        let (root, data) = empty_item_data("brand");
        for server_brand in [
            b"\x07vanilla".as_slice(),
            b"\x07sunrise",
            b"\x05Paper",
            b"\xff",
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let session = Session::connect(
                &format!("localhost:{port}"),
                "Probe",
                None,
                cfg::ClientInformation::new(8),
                Arc::new(Language::new()),
                Arc::clone(&data),
                Arc::default(),
            );
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut server = Connection::from_stream(stream).unwrap();
            let (id, body) = server.recv().unwrap();
            let handshake::Serverbound::Intention(packet) =
                handshake::Serverbound::decode(id, &body).unwrap()
            else {
                panic!("expected login handshake")
            };
            assert_eq!((packet.host.as_str(), packet.port), ("localhost", port));
            assert_eq!(packet.intent, handshake::LOGIN);
            server.recv().unwrap(); // login profile
            server
                .send(
                    &login::Clientbound::LoginFinished(login::LoginFinished {
                        profile: login::GameProfile {
                            uuid: 1,
                            name: "Probe".into(),
                            properties: vec![],
                        },
                        session: 1,
                    })
                    .encode(),
                )
                .unwrap();
            let (id, body) = server.recv().unwrap();
            assert!(matches!(
                login::Serverbound::decode(id, &body).unwrap(),
                login::Serverbound::LoginAcknowledged(_)
            ));
            // handleLoginFinished: the brand first, then the information.
            let (id, body) = server.recv().unwrap();
            assert_eq!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: b"\x07vanilla".to_vec(),
                })
            );
            let (id, body) = server.recv().unwrap();
            assert!(matches!(
                cfg::Serverbound::decode(id, &body).unwrap(),
                cfg::Serverbound::ClientInformation(_)
            ));
            server
                .send(
                    &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: server_brand.to_vec(),
                    })
                    .encode(),
                )
                .unwrap();
            if server_brand == b"\xff" {
                // A registered brand uses its own string codec: malformed
                // bytes fail the session rather than continuing negotiation.
                let error = server
                    .recv()
                    .expect_err("malformed brand must end the session");
                assert!(error.is_end_of_stream());
                drop(server);
                drop(session);
                continue;
            }
            if server_brand == b"\x07sunrise" {
                use sunrise_protocol::payload_negotiation::{CHANNEL, Control};
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Query.encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
            }
            server
                .send(
                    &cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] })
                        .encode(),
                )
                .unwrap();
            if server_brand == b"\x07sunrise" {
                use sunrise_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                let (id, body) = server.recv().unwrap();
                assert_eq!(
                    cfg::Serverbound::decode(id, &body).unwrap(),
                    cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data: Control::Query.encode().unwrap(),
                    })
                );
                server
                    .send(
                        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: CHANNEL.into(),
                            data: Control::Offer(Manifest::default()).encode().unwrap(),
                        })
                        .encode(),
                    )
                    .unwrap();
                let (id, body) = server.recv().unwrap();
                assert_eq!(
                    cfg::Serverbound::decode(id, &body).unwrap(),
                    cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: "minecraft:brand".into(),
                        data: b"\x07sunrise".to_vec(),
                    })
                );
            }
            loop {
                let (event, _credit) = session
                    .events
                    .recv_timeout_owned(Duration::from_secs(5))
                    .unwrap()
                    .into_parts();
                if let Event::Configuration(task) = event {
                    assert!(task.handle().is_none());
                    break;
                }
            }
            // Wait for the pack reply without pinning unrelated extension payloads.
            // The negotiated brand must not be announced a second time.
            let mut saw_protocol_offer = false;
            loop {
                let (id, body) = server.recv().unwrap();
                match cfg::Serverbound::decode(id, &body).unwrap() {
                    cfg::Serverbound::SelectKnownPacks(_) => break,
                    cfg::Serverbound::CustomPayload(p) => {
                        assert_ne!(p.channel, "minecraft:brand");
                        if p.channel == sunrise_protocol::payload_negotiation::CHANNEL {
                            use sunrise_protocol::payload_negotiation::Control;
                            // A client with no mods offers the engine's declarations.
                            let local = crate::raw_payloads::Local::capture(
                                crate::raw_payloads::Side::Client,
                            )
                            .unwrap();
                            assert!(!local.has_mod_declarations());
                            assert_eq!(
                                Control::decode(&p.data).unwrap(),
                                Control::Offer(local.manifest.clone())
                            );
                            assert!(!saw_protocol_offer, "duplicate protocol offer");
                            saw_protocol_offer = true;
                        }
                    }
                    packet => panic!("unexpected configuration reply: {packet:?}"),
                }
            }
            assert_eq!(saw_protocol_offer, server_brand == b"\x07sunrise");
            drop(server);
            drop(session);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_ping_request_is_the_1_6_ping_host_packet() {
        let bytes = legacy_ping_request("a.b", 25565);
        assert_eq!(&bytes[..3], &[254, 1, 250]);
        // "MC|PingHost": 11 chars.
        assert_eq!(&bytes[3..5], &[0, 11]);
        let after = 5 + 22;
        // payload: 127, string (2 + 6 bytes), port.
        assert_eq!(&bytes[after..after + 2], &[0, 1 + 2 + 6 + 4]);
        assert_eq!(bytes[after + 2], 127);
        assert_eq!(&bytes[after + 3..after + 5], &[0, 3]);
        assert_eq!(&bytes[bytes.len() - 4..], &25565i32.to_be_bytes());
    }

    #[test]
    fn legacy_status_is_split_on_nul_after_the_section_one_marker() {
        let status =
            parse_legacy_status("\u{a7}1\x00127\x001.6.4\x00A \u{a7}aserver\x003\x0020").unwrap();
        assert_eq!(
            status,
            LegacyStatus {
                protocol: 127,
                version: "1.6.4".into(),
                motd: "A \u{a7}aserver".into(),
                players: 3,
                max_players: 20,
            }
        );
        // Unparsable counts are -1, a wrong marker is no status.
        let status = parse_legacy_status("\u{a7}1\x00x\x00v\x00m\x00?\x00?").unwrap();
        assert_eq!(
            (status.protocol, status.players, status.max_players),
            (0, -1, -1)
        );
        assert!(parse_legacy_status("kick\x00127\x00v\x00m\x001\x002").is_none());
    }

    /// The en_us text of the disconnect reasons.
    fn disconnect_language() -> Language {
        [
            ("disconnect.timeout", "Timed out"),
            ("disconnect.endOfStream", "End of stream"),
            ("disconnect.genericReason", "%s"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    #[ignore = "daily"]
    fn a_server_that_stops_answering_times_out() {
        // The server accepts and then sends nothing, like a frozen server
        // or a network gone without a reset.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
        let (c, _, _) = connect_tcp("127.0.0.1", port, true, Duration::from_millis(300)).unwrap();
        let _server = held.join().unwrap().unwrap();
        let (mut reader, writer) = c.split();
        let (root, data) = empty_item_data("timeout");
        let (done, result) = channel();
        std::thread::spawn(move || {
            let (send, _events) = event_channel();
            let _ = done.send(receive_play(
                &mut reader,
                Arc::default(),
                &Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(disconnect_language()),
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            ));
        });
        let ended = result
            .recv_timeout(Duration::from_secs(10))
            .expect("the connection never timed out");
        assert_eq!(
            ended.map(|r| r.text).map_err(|r| r.text),
            Err("Timed out".to_owned())
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lost_connections_read_like_vanillas_disconnect_reasons() {
        let lang = disconnect_language();
        let lost = |e| {
            let reason = connection_lost(e, &lang);
            (reason.key, reason.text)
        };
        assert_eq!(
            lost(sunrise_protocol::Error::timed_out()),
            (
                Some("disconnect.timeout".to_owned()),
                "Timed out".to_owned()
            )
        );
        assert_eq!(
            lost(sunrise_protocol::Error::end_of_stream()),
            (
                Some("disconnect.endOfStream".to_owned()),
                "End of stream".to_owned(),
            )
        );
        assert_eq!(
            lost(sunrise_protocol::Error::new("Bad VarInt")),
            (
                Some("disconnect.genericReason".to_owned()),
                "Internal Exception: Bad VarInt".to_owned(),
            )
        );
    }

    /// #3716: a client asked about viewed levels answers in configuration, before its
    /// acknowledgement, that it views two levels besides its own.
    #[test]
    fn configuration_answers_the_viewed_levels_query_with_its_cap() {
        let (root, data) = empty_item_data("viewed-levels");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            drive_configuration(
                |send| {
                    configure(
                        &mut reader,
                        &client_writer,
                        &Arc::new(AtomicBool::new(true)),
                        &Language::new(),
                        send,
                        &Cookies::default(),
                        &data,
                        &crate::raw_payloads::Transport::empty(),
                    )
                },
                |_| {},
            )
        });
        let mut server = Connection::local(server);
        for packet in [
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: "sunrise:levels".into(),
                data: sunrise_protocol::viewed_levels::query(),
            }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        for expected in [
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "sunrise:levels".into(),
                data: sunrise_protocol::viewed_levels::answer(2),
            }),
            cfg::Serverbound::FinishConfiguration(Empty),
        ] {
            let (id, body) = server.recv().unwrap();
            assert_eq!(cfg::Serverbound::decode(id, &body).unwrap(), expected);
        }
        assert!(thread.join().unwrap().is_ok());
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_accepts_a_resource_pack_prompt_with_a_builtin_item_hover() {
        let root = TempDirGuard(std::env::temp_dir().join(format!(
            "sunrise-net-config-item-hover-{}",
            std::process::id()
        )));
        std::fs::create_dir_all(&root.0).unwrap();
        std::fs::write(root.0.join("items.json"), r#"{"minecraft:diamond":{}}"#).unwrap();
        let data =
            crate::test_resources::item_data(&root.0, DataLoadOrigin::ClientPreload).unwrap();
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (prompt, _) = sunrise_core::nbt::parse_snbt(
            r#"{text:"Pack prompt",hover_event:{action:show_item,id:"minecraft:diamond"}}"#,
        )
        .unwrap();
        let request = cfg::ResourcePackPush {
            id: 42,
            url: "https://example.invalid/pack.zip".into(),
            hash: String::new(),
            required: false,
            prompt: Some(prompt),
        };
        let mut server = Connection::local(server);
        for packet in [
            cfg::Clientbound::ResourcePackPush(request.clone()),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        let mut received = None;
        drive_configuration(
            |send| {
                configure(
                    &mut reader,
                    &writer,
                    &Arc::new(AtomicBool::new(true)),
                    &Language::new(),
                    send,
                    &Cookies::default(),
                    &data,
                    &crate::raw_payloads::Transport::empty(),
                )
            },
            |event| {
                if let Event::PackPush(request) = event {
                    received = Some(request);
                }
            },
        )
        .unwrap();
        assert_eq!(received, Some(request));
    }

    #[test]
    fn configuration_dialog_binds_holders_from_its_own_packets() {
        use crate::dialog::Dialog;
        use sunrise_core::nbt::parse_snbt;

        let (root, data) = empty_item_data("dialog-registry");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let mut effects = Vec::new();
        let mut server = Connection::local(server);
        let (leaf, _) = parse_snbt("{type:notice,title:Leaf}").unwrap();
        let (dialog, _) = parse_snbt(
            r##"{type:dialog_list,title:{text:Open,click_event:{action:show_dialog,dialog:"test:leaf"}},dialogs:"#test:list"}"##,
        )
        .unwrap();
        let tags = |tags| {
            cfg::Clientbound::UpdateTags(cfg::UpdateTags {
                registries: vec![cfg::RegistryTags {
                    registry: "minecraft:dialog".into(),
                    tags,
                }],
            })
        };
        for packet in [
            cfg::Clientbound::RegistryData(cfg::RegistryData {
                registry: "minecraft:dialog".into(),
                entries: vec![cfg::RegistryEntry {
                    id: "test:leaf".into(),
                    data: Some(leaf.clone()),
                }],
            }),
            tags(vec![cfg::TagEntry {
                name: "test:list".into(),
                ids: vec![0],
            }]),
            cfg::Clientbound::ShowDialog(cfg::ShowDialog {
                dialog: dialog.clone(),
            }),
            tags(vec![]),
            cfg::Clientbound::ShowDialog(cfg::ShowDialog { dialog }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        assert!(
            drive_configuration(
                |send| configure(
                    &mut reader,
                    &writer,
                    &Arc::new(AtomicBool::new(true)),
                    &Language::new(),
                    send,
                    &Cookies::default(),
                    &data,
                    &crate::raw_payloads::Transport::empty(),
                ),
                |event| effects.push(event)
            )
            .is_ok()
        );
        assert_eq!(effects.len(), 2);
        for (event, expected) in effects.into_iter().zip([true, false]) {
            let Event::ShowDialog { dialog, registries } = event else {
                panic!("expected a configuration dialog");
            };
            let parsed = Dialog::parse_in_registry(
                &dialog,
                &Default::default(),
                &|_| false,
                Some(&registries),
            );
            assert_eq!(parsed.is_some(), expected);
            if expected {
                let id = registries.tag("minecraft:dialog", "test:list").unwrap()[0];
                assert_eq!(registries.data("minecraft:dialog", id), Some(&leaf));
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_installs_the_server_consume_effect_ids() {
        use sunrise_core::nbt::Tag;
        use sunrise_protocol::item::{Hashed, Stack, component, consume_effects};
        use sunrise_protocol::registry::{Builtin, Ids, SyncedNames};

        #[derive(mod_api::Wire)]
        struct Charge {
            points: u16,
        }

        struct Effects;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for Effects {
            fn register(
                &self,
                registrar: &mut dyn mod_api::Registrar,
                _: &mod_api::RegistrationContext,
            ) {
                for key in ["effect_test:charge", "effect_test:other"] {
                    registrar
                        .register_consume_effect_type(
                            mod_api::Identifier::parse(key).unwrap(),
                            mod_api::ConsumeEffectType::new::<Charge>(
                                |fields| match fields.get(&"points".into()) {
                                    Some(Tag::Int(points)) => u16::try_from(*points)
                                        .map(|points| Charge { points })
                                        .map_err(|error| error.to_string()),
                                    _ => Err("expected points".into()),
                                },
                                |charge| {
                                    Ok([
                                        ("points".into(), Tag::Int(i32::from(charge.points))),
                                        ("samples".into(), Tag::IntArray(vec![3, 4])),
                                        ("flag".into(), Tag::Byte(7)),
                                    ]
                                    .into())
                                },
                            ),
                        )
                        .unwrap();
                }
            }
        }

        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("effect_test", Box::new(Effects))].into();
        sunrise_world::component_value::consume_effects::with_test_registry(|| {
            mod_platform::testing::with_mods(mods.clone(), || {
                mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
                crate::item_sync::install_consume_effect_codecs().unwrap();

                let (root, resources) = empty_item_data("consume-effects");
                let (client, server) = sunrise_protocol::connection::Pipe::pair();
                let (mut reader, writer) = Connection::local(client).split();
                let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
                let (send, events) = event_channel();
                let mut server = Connection::local(server);
                let mut entries: Vec<_> = consume_effects::configuration()
                    .map(|(id, schema)| cfg::RegistryEntry {
                        id: id.into(),
                        data: Some(Tag::Long(schema as i64)),
                    })
                    .collect();
                assert_eq!(entries[5].id, "effect_test:charge");
                // The server uses different IDs, including a built-in ID occupied by a native type.
                entries.swap(0, 5);
                entries.swap(5, 6);
                server
                    .send(
                        &cfg::Clientbound::RegistryData(cfg::RegistryData {
                            registry: consume_effects::REGISTRY.into(),
                            entries,
                        })
                        .encode(),
                    )
                    .unwrap();
                server
                    .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                let worker = std::thread::spawn(move || {
                    configure(
                        &mut reader,
                        &writer,
                        &Arc::new(AtomicBool::new(true)),
                        &Language::new(),
                        &send,
                        &Cookies::default(),
                        &resources,
                        &crate::raw_payloads::Transport::empty(),
                    )
                });
                // The configuration actor resolves codecs on the same thread that installed
                // the scoped fixture; the reader only queues the server's advertised IDs.
                let timeout = Duration::from_secs(5);
                let (event, _credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
                let Event::Configuration(task) = event else {
                    panic!("expected the registry update before finishing configuration");
                };
                assert!(task.handle().is_none());
                let (event, _credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
                let Event::FinishConfiguration(change) = event else {
                    panic!("expected the configuration finish task");
                };
                change.acknowledge();
                let (registries, _) = worker.join().unwrap().unwrap();
                assert_eq!(registries.consume_effect_id("effect_test:charge"), Some(0));
                assert_eq!(
                    registries.consume_effect_id("minecraft:apply_effects"),
                    Some(6)
                );

                let mut components = vec![String::new(); component::DEATH_PROTECTION as usize + 1];
                components[component::CONSUMABLE as usize] = "minecraft:consumable".into();
                components[component::DEATH_PROTECTION as usize] =
                    "minecraft:death_protection".into();
                let builtin = Builtin {
                    registries: [
                        (
                            "minecraft:item".into(),
                            Ids::from_names(vec!["minecraft:air".into(), "minecraft:stone".into()]),
                        ),
                        (
                            "minecraft:data_component_type".into(),
                            Ids::from_names(components),
                        ),
                        (
                            "minecraft:sound_event".into(),
                            Ids::from_names(vec![
                                String::new(),
                                "minecraft:entity.generic.eat".into(),
                            ]),
                        ),
                    ]
                    .into(),
                };
                let effect = Tag::compound([
                    ("type", Tag::String("effect_test:charge".into())),
                    ("points", Tag::Int(0x1234)),
                ]);
                let mut item = crate::items::ItemStack::new("minecraft:stone", 1);
                for (name, field) in [
                    ("minecraft:consumable", "on_consume_effects"),
                    ("minecraft:death_protection", "death_effects"),
                ] {
                    item.data_mut().set_other(
                        name,
                        Some(Tag::compound([(field, Tag::List(vec![effect.clone()]))])),
                    );
                }
                let encode = |item| {
                    crate::item_sync::net_stack::stack_to_network(
                        |_| 1,
                        |registry, name| {
                            if registry == consume_effects::REGISTRY {
                                registries.consume_effect_id(name)
                            } else {
                                builtin.get(registry)?.id(name)
                            }
                        },
                        &Some(item),
                    )
                    .unwrap()
                };
                let stack = encode(item);
                let Some(sunrise_protocol::item::Component::Raw(death)) =
                    stack.component(component::DEATH_PROTECTION)
                else {
                    panic!("expected a death protection stream");
                };
                assert_eq!(death, &[1, 0, 0x34, 0x12]); // one effect, server ID 0, Wire u16
                let body = play::Clientbound::ContainerSetContent(play::ContainerSetContent {
                    container: 0,
                    state: 1,
                    items: vec![Some(Stack::new(1, 3)), Some(stack.clone())],
                    carried: None,
                })
                .encode();

                // This layout is fixed for the connection, even when the gameplay listener is off.
                assert!(mod_api::__private::set_mod_enabled("effect_test", false));
                let mut reader = Reader::new(&body).with_names(&registries);
                let id = reader.varint().unwrap();
                let play::Clientbound::ContainerSetContent(update) =
                    play::Clientbound::decode_reader(id, reader).unwrap()
                else {
                    panic!("expected full container update");
                };
                assert_eq!(
                    update.items[0]
                        .as_ref()
                        .map(|stack| (stack.item, stack.count)),
                    Some((1, 3))
                );
                let received = update.items[1].as_ref().unwrap();
                let client = crate::item_sync::from_network(received, &builtin, &registries);
                for effects in [
                    &client.consumable(None).unwrap().effects[..],
                    client.death_protection(None).unwrap(),
                ] {
                    let [crate::items::ConsumeEffect::Native(effect)] = effects else {
                        panic!("expected custom effect: {effects:?}");
                    };
                    let data = &effect.data;
                    assert_eq!(effect.kind.as_ref(), "effect_test:charge");
                    assert_eq!(data.get("points"), Some(&Tag::Int(0x1234)));
                    assert_eq!(data.get("samples"), Some(&Tag::IntArray(vec![3, 4])));
                    assert_eq!(data.get("flag"), Some(&Tag::Byte(7)));
                }
                let names = SyncedNames {
                    builtin: &builtin,
                    dynamic: &registries,
                };
                let before = Hashed::create(&stack, &names).unwrap();
                assert_eq!(Hashed::create(received, &names).unwrap(), before);
                assert_eq!(Hashed::create(&encode(client), &names).unwrap(), before);
                // A connection without this registry never exposes local custom codecs.
                let vanilla = Dynamic::default();
                assert_eq!(vanilla.consume_effect_id("effect_test:charge"), None);
                let mut reader = Reader::new(&[5, 0x34, 0x12]).with_names(&vanilla);
                assert!(consume_effects::read_kind(&mut reader).is_err());
                std::fs::remove_dir_all(root).unwrap();
            })
        });
    }

    #[test]
    fn malformed_consume_effect_ids_refuse_play_after_pending_configuration_replies() {
        let (root, resources) = empty_item_data("bad-consume-effects");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let mut server = Connection::local(server);
        server
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let ping = cfg::Ping { id: 7 };
        for packet in [
            cfg::Clientbound::Ping(ping.clone()),
            cfg::Clientbound::RegistryData(cfg::RegistryData {
                registry: sunrise_protocol::item::consume_effects::REGISTRY.into(),
                entries: Vec::new(), // Missing the built-in codec rows.
            }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }

        let configuring = Arc::new(AtomicBool::new(true));
        let error = drive_configuration(
            |send| {
                configure(
                    &mut reader,
                    &writer,
                    &configuring,
                    &Language::new(),
                    send,
                    &Cookies::default(),
                    &resources,
                    &crate::raw_payloads::Transport::empty(),
                )
            },
            |_| {},
        )
        .unwrap_err();
        assert!(error.text.contains("missing built-in consume effect type"));
        assert!(configuring.load(Ordering::Acquire));
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::Pong(ping)
        );
        assert!(server.recv().unwrap_err().is_end_of_stream());
        std::fs::remove_dir_all(root).unwrap();
    }

    /// #3384: a client asked which `sunrise:menu_extra` revision it supports answers its revision
    /// in configuration and keeps it for play.
    #[test]
    fn configuration_answers_the_menu_extra_revision_query() {
        let (root, data) = empty_item_data("menu-extra-revision");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            drive_configuration(
                |send| {
                    configure(
                        &mut reader,
                        &client_writer,
                        &Arc::new(AtomicBool::new(true)),
                        &Language::new(),
                        send,
                        &Cookies::default(),
                        &data,
                        &crate::raw_payloads::Transport::empty(),
                    )
                },
                |_| {},
            )
        });
        let mut server = Connection::local(server);
        for packet in [
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: "sunrise:menu_extra".into(),
                data: sunrise_protocol::menu_extra::query(),
            }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        for expected in [
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "sunrise:menu_extra".into(),
                data: sunrise_protocol::menu_extra::answer(sunrise_protocol::menu_extra::REVISION),
            }),
            cfg::Serverbound::FinishConfiguration(Empty),
        ] {
            let (id, body) = server.recv().unwrap();
            assert_eq!(cfg::Serverbound::decode(id, &body).unwrap(), expected);
        }
        // A failure reads as 0 and fails the assertion below.
        let revision = thread
            .join()
            .unwrap()
            .map(|(registries, _)| registries.menu_extra_revision)
            .unwrap_or(0);
        drop(writer);
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(revision, sunrise_protocol::menu_extra::REVISION);
    }

    #[test]
    fn configuration_answers_the_content_identity_challenge() {
        let root =
            std::env::temp_dir().join(format!("sunrise-content-identity-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("facts.json"),
            br#"{"states":[["minecraft:stone",false,false,"Block{minecraft:stone}",{"id":"minecraft:stone"}]]}"#,
        )
        .unwrap();
        std::fs::write(root.join("items.json"), "{}").unwrap();
        let data = crate::test_resources::item_data(&root, DataLoadOrigin::ClientPreload).unwrap();
        let expected = crate::content_identity::identity(&root, data.defaults()).unwrap();
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        // Keep the connection owner alive until the peer receives the queued
        // finish ACK, as a real Session does when configure returns to play.
        let client_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            drive_configuration(
                |send| {
                    configure(
                        &mut reader,
                        &client_writer,
                        &Arc::new(AtomicBool::new(true)),
                        &Language::new(),
                        send,
                        &Cookies::default(),
                        &data,
                        &crate::raw_payloads::Transport::empty(),
                    )
                },
                |_| {},
            )
        });
        let mut server = Connection::local(server);
        server
            .send(
                &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                    channel: crate::content_identity::CHANNEL.into(),
                    data: vec![],
                })
                .encode(),
            )
            .unwrap();
        server
            .send(&cfg::Clientbound::Ping(cfg::Ping { id: 17 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: crate::content_identity::CHANNEL.into(),
                data: expected.to_vec(),
            })
        );
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::Pong(cfg::Ping { id: 17 })
        );
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );
        assert!(thread.join().unwrap().is_ok());
        drop(writer);
        std::fs::remove_dir_all(
            std::env::temp_dir().join(format!("sunrise-content-identity-{}", std::process::id())),
        )
        .unwrap();
    }

    #[test]
    fn disconnect_and_drop_do_not_wait_for_a_stalled_reader() {
        // A reader can remain stalled after Windows socket shutdown. It must
        // not delay either an explicit leave or Session's Drop on the UI thread.
        for explicit_disconnect in [true, false] {
            let (client, _server) = sunrise_protocol::connection::Pipe::pair();
            let (_reader, writer) = Connection::local(client).split();
            let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
            let (_send, events) = event_channel();
            let (release, stalled) = channel();
            let (finished, completed) = channel();
            let thread = std::thread::spawn(move || {
                let _ = stalled.recv_timeout(std::time::Duration::from_secs(3));
                let _ = finished.send(());
            });
            let mut session = Session {
                events,
                payload_events: event_channel().0,
                payloads: crate::raw_payloads::Transport::empty(),
                writer: writer.clone(),
                configuring: Arc::new(AtomicBool::new(false)),
                thread: Some(thread),
                address: "local".into(),
                name: "TestPlayer".into(),
                counters: Arc::new(Mutex::new(None)),
                cookies: Cookies::default(),
                debug: false,
                far: Arc::default(),
                recorder: Arc::new(Mutex::new(None)),
                pings: Arc::default(),
                serial: next_serial(),
                payload_output: std::sync::OnceLock::new(),
            };
            let started = std::time::Instant::now();
            if explicit_disconnect {
                session.disconnect();
                session.disconnect(); // Repeated cleanup is harmless.
            }
            drop(session);
            let elapsed = started.elapsed();
            let reader_still_pending = completed.try_recv().is_err();
            // Always release the synthetic reader before asserting, even if
            // a future regression restores the blocking join.
            let _ = release.send(());
            if reader_still_pending {
                completed
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap();
            }
            assert!(
                writer.lock().unwrap().is_none(),
                "connection was not closed"
            );
            assert!(
                reader_still_pending,
                "disconnect waited for the stalled reader"
            );
            assert!(
                elapsed < std::time::Duration::from_secs(1),
                "disconnect took {elapsed:?}"
            );
        }
    }

    #[test]
    fn pings_wait_behind_teleports_but_keepalives_do_not_wait_for_the_game() {
        let (root, data) = empty_item_data("pings");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = event_channel();
        let thread_writer = writer.clone();
        let pings: PingSamples = Arc::default();
        let thread_pings = pings.clone();
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &thread_writer,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(Language::new()),
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
                &thread_pings,
                &Arc::default(),
            )
        });
        let mut server = Connection::local(server);
        let teleport = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 42,
            position: [10.0, 64.0, 10.0],
            delta: [0.0; 3],
            yaw: 90.0,
            pitch: 0.0,
            relatives: 0,
        });
        server.send(&teleport.encode()).unwrap();
        server
            .send(&play::Clientbound::Ping(cfg::Ping { id: 7 }).encode())
            .unwrap();
        server
            .send(
                &play::Clientbound::PongResponse(sunrise_protocol::packets::status::Ping {
                    time: ping_millis(),
                })
                .encode(),
            )
            .unwrap();
        // A reply to this proves the receive loop has passed the ping. The
        // old implementation instead replies with Pong before this keepalive.
        server
            .send(&play::Clientbound::KeepAlive(cfg::KeepAlive { id: 9 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 9 })
        );
        assert_eq!(
            pings.lock().unwrap().len(),
            1,
            "debug Pong is sampled before main dispatch"
        );
        let timeout = Duration::from_secs(5);
        let (event, _teleport_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(event, Event::Packet(p) if *p == teleport));
        let (event, _ping_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(
            matches!(event, Event::Packet(p) if *p == play::Clientbound::Ping(cfg::Ping { id: 7 }))
        );
        writer.lock().unwrap().take().unwrap().shutdown();
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn bundle_pipe(
        name: &str,
        capacity: usize,
    ) -> (PathBuf, Connection, Session, std::thread::JoinHandle<()>) {
        let (root, data) = empty_item_data(name);
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let (send, events) = ingress::channel(capacity, Event::ingress_charge).unwrap();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: writer.clone(),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        };
        let (configuring, cookies) = (session.configuring.clone(), session.cookies.clone());
        let recorder = session.recorder.clone();
        let thread = std::thread::spawn(move || {
            let result = receive_play(
                &mut reader,
                Arc::default(),
                &writer,
                &configuring,
                &Arc::new(Language::new()),
                &send,
                &cookies,
                &Arc::default(),
                &data,
                &recorder,
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            );
            report_connection_end(result, &writer, &send, true);
        });
        let server = Connection::local(server);
        server
            .reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        (root, server, session, thread)
    }

    #[test]
    fn a_malformed_standalone_debug_sample_disconnects_without_waiting_for_the_peer() {
        for (sample, kind) in [(vec![0; 4], 1), (vec![0; 5], play::DEBUG_SAMPLE_TICK_TIME)] {
            let (root, mut server, session, reader) =
                bundle_pipe("malformed-debug-sample", 1 << 20);
            let recorder = TestRecorder::default();
            *session.recorder.lock().unwrap() = Some(Box::new(recorder.clone()));
            let valid = play::Clientbound::DebugSample(play::DebugSample {
                sample: vec![1],
                kind: play::DEBUG_SAMPLE_TICK_TIME,
            });
            server.send(&valid.encode()).unwrap();
            let (event, _credit) = session
                .events
                .recv_timeout_owned(Duration::from_secs(5))
                .unwrap()
                .into_parts();
            assert!(matches!(event, Event::Packet(packet) if *packet == valid));
            server
                .send(&play::Clientbound::DebugSample(play::DebugSample { sample, kind }).encode())
                .unwrap();
            let result = session.events.recv_timeout_owned(Duration::from_secs(5));
            drop(server);
            reader.join().unwrap();
            std::fs::remove_dir_all(root).unwrap();
            assert_eq!(recorder.decoded(), [valid]);
            let (event, _credit) = result.unwrap().into_parts();
            assert!(matches!(
                event,
                Event::Disconnected {
                    from_server: false,
                    ..
                }
            ));
        }
    }

    #[test]
    fn block_entity_data_with_a_non_compound_root_disconnects_before_ingress() {
        let (root, mut server, session, reader) = bundle_pipe("invalid-block-entity-root", 1 << 20);
        let recorder = TestRecorder::default();
        *session.recorder.lock().unwrap() = Some(Box::new(recorder.clone()));
        server
            .send(
                &play::Clientbound::BlockEntityData(play::BlockEntityData {
                    pos: [0; 3],
                    kind: 0,
                    tag: Tag::Int(1),
                })
                .encode(),
            )
            .unwrap();
        let event = session.events.recv_timeout_owned(Duration::from_secs(5));
        drop(server);
        reader.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(recorder.decoded().is_empty());
        let (event, _credit) = event.unwrap().into_parts();
        assert!(matches!(
            event,
            Event::Disconnected {
                from_server: false,
                ..
            }
        ));
    }

    #[test]
    fn an_oversized_bundle_waits_for_credit_but_still_accepts_exactly_4096_members() {
        let ping = |id| play::Clientbound::Ping(cfg::Ping { id });
        let capacity = 2
            * (Event::Packet(Box::new(ping(-1))).ingress_charge()
                + sunrise_mod_runtime::byte_channel::message_overhead::<Event>());
        let (root, mut server, session, thread) = bundle_pipe("bundle-pressure", capacity);
        let events = &session.events;
        let delimiter = play::Clientbound::BundleDelimiter(Empty);
        server.send(&ping(-1).encode()).unwrap();
        server.send(&delimiter.encode()).unwrap();
        for id in 0..4096 {
            server.send(&ping(id).encode()).unwrap();
        }
        server.send(&delimiter.encode()).unwrap();
        server.send(&ping(-2).encode()).unwrap();
        // A later truncated bundle never publishes its prefix.
        server.send(&delimiter.encode()).unwrap();
        server.send(&ping(4096).encode()).unwrap();
        server
            .send(&play::Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        let timeout = Duration::from_secs(5);
        let (event, baseline_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(event, Event::Packet(packet) if *packet == ping(-1)));
        assert!(events.try_recv_owned().is_err());
        drop(baseline_credit);
        let (event, credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        let Event::Bundle(bundle) = event else {
            panic!("missing bundle")
        };
        assert!(events.stats().reserved_bytes > capacity);
        assert_eq!(bundle.packets.len(), 4096);
        assert!(bundle.packets.iter().enumerate().all(
            |(id, p)| matches!(p, BundleMember::Packet(packet) if *packet == ping(id as i32))
        ));
        assert!(events.try_recv_owned().is_err());
        drop(bundle);
        drop(credit);
        let (next, next_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(next, Event::Packet(packet) if *packet == ping(-2)));
        let (end, end_credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(
            matches!(end, Event::Disconnected { from_server: false, key: Some(key), .. }
            if key == "disconnect.genericReason")
        );
        assert!(events.try_recv_owned().is_err());
        drop(end_credit);
        drop(next_credit);
        assert_eq!(events.stats().reserved_bytes, 0);
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_4097th_member_disconnects_before_a_closing_delimiter() {
        use play::Clientbound as C;

        let (root, mut server, session, thread) = bundle_pipe("bundle-member-limit", 1 << 20);
        let events = &session.events;
        server.send(&C::BundleDelimiter(Empty).encode()).unwrap();
        for id in 0..4097 {
            server.send(&C::Ping(cfg::Ping { id }).encode()).unwrap();
        }
        // Leave the wire open. The 4097th member itself fails; there is no
        // closing delimiter or EOF to finish (or reject) the partial bundle.
        let (event, _) = events
            .recv_timeout_owned(Duration::from_secs(5))
            .unwrap()
            .into_parts();
        assert!(matches!(
            event,
            Event::Disconnected { from_server: false, key: Some(key), .. }
                if key == "disconnect.genericReason"
        ));
        assert!(events.try_recv_owned().is_err());
        assert!(server.recv().is_err());
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn several_completed_bundles_wait_for_owned_credit_without_disconnecting() {
        use play::{Clientbound as C, Serverbound as S};
        let timeout = Duration::from_secs(5);
        let members = 64;
        let one = Event::INGRESS_RECORD_OVERHEAD
            + std::mem::size_of::<CompletedBundle>()
            + members * std::mem::size_of::<BundleMember>()
            + sunrise_mod_runtime::byte_channel::message_overhead::<Event>();
        let capacity = one * 2;
        let (root, mut server, session, thread) = bundle_pipe("bundle-aggregate", capacity);
        let events = &session.events;
        let delimiter = C::BundleDelimiter(Empty).encode();
        for id in 0..5 {
            server.send(&delimiter).unwrap();
            for _ in 0..members {
                server.send(&C::Ping(cfg::Ping { id }).encode()).unwrap();
            }
            server.send(&delimiter).unwrap();
        }
        let ping = C::Ping(cfg::Ping { id: 30 });
        server.send(&ping.encode()).unwrap();
        let keep_alive = cfg::KeepAlive { id: 31 };
        server
            .send(&C::KeepAlive(keep_alive.clone()).encode())
            .unwrap();
        let receive = |id| {
            let (event, lease) = events.recv_timeout_owned(timeout).unwrap().into_parts();
            let Event::Bundle(bundle) = event else {
                panic!("bundle {id} was refused")
            };
            assert_eq!(bundle.packets.len(), members);
            assert!(
                bundle
                    .packets
                    .iter()
                    .all(|packet| matches!(packet, BundleMember::Packet(p) if *p == C::Ping(cfg::Ping { id })))
            );
            lease
        };
        let mut first = receive(0);
        let mut second = receive(1);
        for id in 2..5 {
            assert!(matches!(
                events.recv_timeout_owned(Duration::from_millis(25)),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            ));
            assert_eq!(events.stats().reserved_bytes, capacity);
            drop(first);
            first = second;
            second = receive(id);
        }
        assert_eq!(events.stats().peak_reserved_bytes, capacity);
        assert!(matches!(
            events.recv_timeout_owned(Duration::from_millis(25)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        assert_eq!(events.stats().reserved_bytes, capacity);
        drop(first);
        let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(event, Event::Packet(packet) if *packet == ping));
        drop(second);
        let (id, body) = server.recv().unwrap();
        assert_eq!(S::decode(id, &body).unwrap(), S::KeepAlive(keep_alive));
        server.writer.shutdown();
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_store_cookie_waiting_for_credit_does_not_join_the_transfer_tail() {
        use play::Clientbound as C;
        let transfer = cfg::Transfer {
            host: "first.invalid".into(),
            port: 25565,
        };
        let capacity = Event::INGRESS_RECORD_OVERHEAD
            + size_of::<TransferTask>()
            + transfer.host.capacity()
            + sunrise_mod_runtime::byte_channel::message_overhead::<Event>();
        let (root, mut server, session, thread) = bundle_pipe("transfer-credit", capacity);
        let events = &session.events;
        server.send(&C::Transfer(transfer).encode()).unwrap();
        let receipt = events.recv_timeout_owned(Duration::from_secs(5)).unwrap();
        assert_eq!(events.stats().reserved_bytes, capacity);
        server
            .send(
                &C::StoreCookie(cfg::StoreCookie {
                    key: "test:waiting".into(),
                    payload: vec![1],
                })
                .encode(),
            )
            .unwrap();
        events.wait_for_parked_reader();
        assert_eq!(events.stats().queued_items, 0);
        let (Event::Transfer(task), receipt) = receipt.into_parts() else {
            panic!("missing transfer")
        };
        session.close_after_pending();
        assert_eq!(task.into_eligible_packet().unwrap().host, "first.invalid");
        drop(receipt);
        let (event, _credit) = events
            .recv_timeout_owned(Duration::from_secs(5))
            .unwrap()
            .into_parts();
        assert!(
            matches!(event, Event::Disconnected { .. }),
            "a cookie without admission credit must not appear after the cutoff"
        );
        assert!(session.cookies.lock().unwrap().is_empty());
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_waits_in_main_order_without_stalling_standalone_controls() {
        use play::{Clientbound as C, Serverbound as S};
        let timeout = Duration::from_secs(5);
        let recv = |server: &mut Connection| {
            let (id, body) = server.recv().unwrap();
            S::decode(id, &body).unwrap()
        };
        let (root, mut server, session, thread) = bundle_pipe("bundle-order", 1 << 20);
        let events = &session.events;
        let cookie = "test:session";
        let delimiter = C::BundleDelimiter(Empty);
        let request = C::CookieRequest(cfg::CookieRequest { key: cookie.into() });
        let store = |value| {
            C::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![value],
            })
        };
        let response = |value| {
            S::CookieResponse(cfg::CookieResponse {
                key: cookie.into(),
                payload: Some(vec![value]),
            })
        };
        let keep_alive = cfg::KeepAlive { id: 76 };
        for p in [
            &delimiter,
            &store(10),
            &request,
            &store(11),
            &delimiter,
            &C::KeepAlive(keep_alive.clone()),
            &C::ChunkBatchStart(Empty),
            &C::ChunkBatchFinished(play::ChunkBatchFinished { size: 2 }),
            &request,
            &C::StartConfiguration(Empty),
        ] {
            server.send(&p.encode()).unwrap();
        }
        // These happen on the receiver, before main has even dequeued the bundle.
        assert_eq!(recv(&mut server), S::KeepAlive(keep_alive));
        assert!(matches!(recv(&mut server), S::ChunkBatchReceived(_)));
        let (event, _credit) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        let Event::Bundle(bundle) = event else {
            panic!("missing bundle")
        };
        assert!(bundle.io.accepts_bundle());
        for packet in bundle.packets {
            assert!(matches!(
                bundle.io.bundle_member(packet),
                BundleStep::Continue
            ));
        }
        let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        let Event::Cookie(request) = event else {
            panic!("missing standalone cookie")
        };
        request.handle();
        let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        let Event::Reconfiguring(resume) = event else {
            panic!("missing configuration task")
        };
        let configuring = resume.configuring.clone();
        let config_writer = resume.writer.clone();
        resume.acknowledge();
        assert!(configuring.load(Ordering::Acquire));
        assert_eq!(
            config_writer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .play_incarnation(),
            None
        );
        assert_eq!(recv(&mut server), response(10));
        assert_eq!(recv(&mut server), response(11));
        assert_eq!(recv(&mut server), S::ConfigurationAcknowledged(Empty));
        for packet in [
            cfg::Clientbound::StoreCookie(cfg::StoreCookie {
                key: cookie.into(),
                payload: vec![12],
            }),
            cfg::Clientbound::CookieRequest(cfg::CookieRequest { key: cookie.into() }),
            cfg::Clientbound::Ping(cfg::Ping { id: 21 }),
            cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] }),
            cfg::Clientbound::FinishConfiguration(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        for _ in 0..2 {
            let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
            let Event::Cookie(task) = event else {
                panic!("missing configuration cookie task")
            };
            task.handle();
        }
        for _ in 0..2 {
            let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
            let Event::Configuration(task) = event else {
                panic!("missing main-affine configuration task")
            };
            assert!(task.handle().is_none());
        }
        let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        let Event::FinishConfiguration(change) = event else {
            panic!("missing configuration finish task")
        };
        assert!(configuring.load(Ordering::Acquire));
        change.acknowledge();
        assert!(!configuring.load(Ordering::Acquire));
        assert!(
            config_writer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .play_incarnation()
                .is_some()
        );
        for expected in [
            cfg::Serverbound::CookieResponse(cfg::CookieResponse {
                key: cookie.into(),
                payload: Some(vec![12]),
            }),
            cfg::Serverbound::Pong(cfg::Ping { id: 21 }),
            cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks { packs: vec![] }),
            cfg::Serverbound::FinishConfiguration(Empty),
        ] {
            let (id, body) = server.recv().unwrap();
            assert_eq!(cfg::Serverbound::decode(id, &body).unwrap(), expected);
        }
        let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
        assert!(matches!(event, Event::Reconfigured { .. }));
        server.writer.shutdown();
        thread.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn standalone_close_can_overtake_a_queued_bundle_and_eof_drops_a_partial_one() {
        use play::Clientbound as C;
        let timeout = Duration::from_secs(5);
        for (complete, disconnect) in [(true, true), (true, false), (false, false)] {
            let (root, mut server, session, thread) = bundle_pipe("bundle-read-close", 1 << 20);
            let events = &session.events;
            let delimiter = C::BundleDelimiter(Empty);
            server.send(&delimiter.encode()).unwrap();
            server.send(&C::Ping(cfg::Ping { id: 5 }).encode()).unwrap();
            let bundle = complete.then(|| {
                server.send(&delimiter.encode()).unwrap();
                let (event, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
                let Event::Bundle(bundle) = event else {
                    panic!("missing bundle")
                };
                bundle
            });
            if disconnect {
                server
                    .send(
                        &C::Disconnect(cfg::Disconnect {
                            reason: Tag::String("done".into()),
                        })
                        .encode(),
                    )
                    .unwrap();
            } else {
                server.writer.shutdown();
            }
            let (ended, _) = events.recv_timeout_owned(timeout).unwrap().into_parts();
            assert!(
                matches!(ended, Event::Disconnected { from_server, .. } if from_server == disconnect)
            );
            if let Some(bundle) = bundle {
                assert!(!bundle.io.accepts_bundle());
            }
            assert!(events.try_recv_owned().is_err());
            thread.join().unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// ClientPacketListener.handleChunkBatchStart and handleChunkBatchFinished: the
    /// receive loop answers a batch itself, with the game's queue never read.
    #[test]
    fn the_network_thread_answers_each_chunk_batch_once() {
        let (root, data) = empty_item_data("chunk-batch");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        // Held and never polled: nothing the game thread does can answer.
        let (send, _events) = event_channel();
        let thread_writer = writer.clone();
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &thread_writer,
                &Arc::new(AtomicBool::new(false)),
                &Arc::new(Language::new()),
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            )
        });
        let mut server = Connection::local(server);
        let keep_alive = |id| play::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode();
        let next = |server: &mut Connection| {
            let (id, body) = server.recv().unwrap();
            play::Serverbound::decode(id, &body).unwrap()
        };
        // The start sends nothing: the first answer is the keep-alive after it.
        server
            .send(&play::Clientbound::ChunkBatchStart(Empty).encode())
            .unwrap();
        server.send(&keep_alive(1)).unwrap();
        assert_eq!(
            next(&mut server),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 1 })
        );
        // An empty first batch leaves the starting average: 7e6 / 2e6 chunks a tick.
        let finished = play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished { size: 0 });
        server.send(&finished.encode()).unwrap();
        server.send(&keep_alive(2)).unwrap();
        assert_eq!(
            next(&mut server),
            play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                chunks_per_tick: 3.5
            })
        );
        // Exactly one answer: the next is the keep-alive sent after the finish.
        assert_eq!(
            next(&mut server),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 2 })
        );
        writer.lock().unwrap().take().unwrap().shutdown();
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn local_send_rejects_unknown_oversize_payload_before_writing() {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (_, writer) = Connection::local(client).split();
        let (_, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        };
        let mut server = Connection::local(server);
        let payload = |size| {
            play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "test:opaque".into(),
                data: vec![0x83; size],
            })
        };
        session.send(payload(32_767));
        let (id, bytes) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &bytes).unwrap(),
            payload(32_767)
        );
        session.send(payload(32_768));
        session.send(play::Serverbound::ClientTickEnd(Empty));
        let (id, bytes) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &bytes).unwrap(),
            play::Serverbound::ClientTickEnd(Empty)
        );
    }

    #[test]
    fn play_packets_wait_out_a_reconfiguration() {
        let (root, data) = empty_item_data("reconfiguration");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let (mut server_read, mut server_write) = Connection::local(server).split();
        let timeout = Duration::from_secs(5);
        server_read.set_read_timeout(Some(timeout)).unwrap();
        let writer = QueuedWriter::new(writer).unwrap();
        // Put the fixture's writer in Play the same way initial configuration does.
        writer
            .transition_protocol(
                sunrise_protocol::payload_negotiation::Phase::Play,
                &cfg::Serverbound::FinishConfiguration(Empty).encode(),
            )
            .unwrap();
        let (id, body) = server_read.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );
        let (send, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(writer))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        };
        let (thread_writer, thread_configuring, thread_recorder) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.recorder.clone(),
        );
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &thread_writer,
                &thread_configuring,
                &Arc::new(Language::new()),
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &thread_recorder,
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            )
        });
        let tick_end = play::Serverbound::ClientTickEnd(Empty);
        server_write
            .send(&play::Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        let (event, _) = session
            .events
            .recv_timeout_owned(timeout)
            .unwrap()
            .into_parts();
        let Event::Reconfiguring(resume) = event else {
            panic!("missing configuration task")
        };
        let configuring = resume.configuring.clone();
        let config_writer = resume.writer.clone();
        assert!(
            config_writer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .play_incarnation()
                .is_some()
        );
        resume.acknowledge();
        assert!(configuring.load(Ordering::Acquire));
        assert_eq!(
            config_writer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .play_incarnation(),
            None
        );
        let (id, body) = server_read.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::ConfigurationAcknowledged(Empty)
        );
        // The game thread still ticks the old level: nothing of it goes out.
        session.send(tick_end.clone());
        server_write
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (event, _credit) = session
            .events
            .recv_timeout_owned(timeout)
            .unwrap()
            .into_parts();
        let Event::FinishConfiguration(change) = event else {
            panic!("missing configuration finish task without a cookie prefix")
        };
        assert!(configuring.load(Ordering::Acquire));
        change.acknowledge();
        assert!(!configuring.load(Ordering::Acquire));
        assert!(
            config_writer
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .play_incarnation()
                .is_some()
        );
        let keep_alive = cfg::KeepAlive { id: 19 };
        server_write
            .send(&play::Clientbound::KeepAlive(keep_alive.clone()).encode())
            .unwrap();
        let (id, body) = server_read.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );
        let (id, body) = server_read.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::KeepAlive(keep_alive)
        );
        // Back in play, the game's packets flow again.
        session.send(tick_end.clone());
        let (id, body) = server_read.recv().unwrap();
        assert_eq!(play::Serverbound::decode(id, &body).unwrap(), tick_end);
        drop(session);
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn exhausted_held_play_credit_still_serves_replies_and_reconfiguration_ack() {
        use mod_api::{Identifier, SendPacketError, ServerboundPlayPacket};
        use sunrise_protocol::payload_negotiation::Phase;

        // Configuration clears the remembered datapack registries; keep that in a fresh process.
        const CHILD: &str = "SUNRISE_RECONFIG_HELD_PRESSURE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            crate::fresh_process::run_in_fresh_process(
                "net::tests::exhausted_held_play_credit_still_serves_replies_and_reconfiguration_ack",
                CHILD,
                "1",
            );
            return;
        }

        let (root, data) = empty_item_data("reconfig-held-pressure");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let writer = QueuedWriter::new(writer).unwrap();
        let mut server = Connection::local(server);
        server
            .reader
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writer.transition_protocol(Phase::Play, &[3]).unwrap();
        assert_eq!(server.recv().unwrap(), (3, vec![]));

        let (send, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(writer))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "Player".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        };
        let (thread_writer, thread_configuring, thread_recorder, thread_cookies) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.recorder.clone(),
            session.cookies.clone(),
        );
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &thread_writer,
                &thread_configuring,
                &Arc::new(Language::new()),
                &send,
                &thread_cookies,
                &Arc::default(),
                &data,
                &thread_recorder,
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            )
        });

        let mut held = {
            let writer = session.writer.lock().unwrap();
            writer
                .as_ref()
                .unwrap()
                .exhaust_play_credit(session.serial())
        };
        assert!(!held.is_empty());

        // Excluded replies bypass unpublished held credit without flushing held packets.
        server
            .send(&play::Clientbound::KeepAlive(cfg::KeepAlive { id: 11 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 11 })
        );

        server
            .send(&play::Clientbound::ChunkBatchStart(Empty).encode())
            .unwrap();
        server
            .send(
                &play::Clientbound::ChunkBatchFinished(play::ChunkBatchFinished { size: 0 })
                    .encode(),
            )
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert!(matches!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::ChunkBatchReceived(_)
        ));

        server
            .send(
                &play::Clientbound::CookieRequest(cfg::CookieRequest {
                    key: "proxy:held".into(),
                })
                .encode(),
            )
            .unwrap();
        let (event, _credit) = session
            .events
            .recv_timeout_owned(Duration::from_secs(2))
            .unwrap()
            .into_parts();
        let Event::Cookie(task) = event else {
            panic!("missing ordered cookie task")
        };
        task.handle();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::CookieResponse(cfg::CookieResponse {
                key: "proxy:held".into(),
                payload: None,
            })
        );

        session.send_pack_status(42, 3);
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::ResourcePack(cfg::ResourcePackResponse { id: 42, action: 3 })
        );

        // Acknowledge on the actor without dropping any of its held permits:
        // the phase barrier revokes old-play reservations before admitting the ACK.
        server
            .send(&play::Clientbound::StartConfiguration(Empty).encode())
            .unwrap();
        let (event, _credit) = session
            .events
            .recv_timeout_owned(Duration::from_secs(2))
            .unwrap()
            .into_parts();
        let Event::Reconfiguring(task) = event else {
            panic!("missing configuration task")
        };
        task.acknowledge();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::ConfigurationAcknowledged(Empty)
        );
        server
            .send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
            .unwrap();
        let (event, _credit) = session
            .events
            .recv_timeout_owned(Duration::from_secs(2))
            .unwrap()
            .into_parts();
        let Event::FinishConfiguration(task) = event else {
            panic!("missing configuration finish task")
        };
        task.acknowledge();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            cfg::Serverbound::decode(id, &body).unwrap(),
            cfg::Serverbound::FinishConfiguration(Empty)
        );

        assert_eq!(
            held.pop().unwrap().try_publish(),
            Err(SendPacketError::NotInPlay)
        );
        drop(held);

        // New play incarnation has full credit and accepts packets normally.
        session
            .prepare_mod_packet(ServerboundPlayPacket::new(
                Identifier::parse("minecraft:pong").unwrap(),
                vec![0, 0, 0, 7],
            ))
            .unwrap()
            .try_publish()
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::Pong(cfg::Ping { id: 7 })
        );

        drop(session);
        assert!(thread.join().unwrap().is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn server_brand_payload_is_decoded_and_invalid_or_unrelated_payloads_are_ignored() {
        let (send, events) = event_channel();
        let mut writer = ::sunrise_core::vanilla_bytes::Writer::new();
        writer.string("vanilla");
        let mut payload = cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: writer.data,
        };
        receive_brand(&payload, &send);
        let (event, _credit) = events.try_recv_owned().unwrap().into_parts();
        assert!(matches!(event, Event::Brand(brand) if brand == "vanilla"));
        payload.channel = "example:other".into();
        receive_brand(&payload, &send);
        assert!(events.try_recv_owned().is_err());
        payload.channel = "minecraft:brand".into();
        payload.data = vec![0xff];
        receive_brand(&payload, &send);
        assert!(events.try_recv_owned().is_err());
    }
    /// No account never joins; the signed-in rule is compared with the real
    /// client login listener by test_lan_auth_parity.py.
    #[test]
    #[ignore = "daily"]
    fn lan_login_continues_after_authentication_failure() {
        let (root, data) = empty_item_data("lan-auth");
        for lan in [true, false] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let worker = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                // Generous: a debug build's key generation on a busy CI runner takes seconds. A pass
                // never waits for these.
                stream
                    .set_read_timeout(Some(Duration::from_secs(60)))
                    .unwrap();
                let mut server = Connection::from_stream(stream).unwrap();
                server.recv().unwrap(); // intention
                server.recv().unwrap(); // profile
                let key = sunrise_protocol::rsa::RsaPrivateKey::generate(1024).unwrap();
                let challenge = vec![1, 2, 3, 4];
                server
                    .send(
                        &login::Clientbound::Hello(login::Hello {
                            server_id: String::new(),
                            public_key: key.public_der(),
                            challenge: challenge.clone(),
                            should_authenticate: true,
                        })
                        .encode(),
                    )
                    .unwrap();
                let Ok((id, body)) = server.recv() else {
                    return false;
                };
                let login::Serverbound::Key(reply) = login::Serverbound::decode(id, &body).unwrap()
                else {
                    panic!("client did not send encryption reply")
                };
                assert_eq!(key.decrypt(&reply.challenge).unwrap(), challenge);
                let secret: [u8; 16] = key.decrypt(&reply.key).unwrap().try_into().unwrap();
                server.enable_encryption(&secret);
                server
                    .send(
                        &login::Clientbound::Disconnect(login::Disconnect {
                            reason_json: r#"{"text":"LAN fixture accepted"}"#.into(),
                        })
                        .encode(),
                    )
                    .unwrap();
                true
            });
            let address = format!("127.0.0.1:{port}");
            let connect = if lan {
                Session::connect_lan
            } else {
                Session::connect
            };
            // The session's language carries Sunrise's bundled strings
            // (`load_language`), so the keyed refusal resolves.
            let lang: Language = [(
                "sunrise.multiplayer.not_signed_in".to_owned(),
                NOT_SIGNED_IN.to_owned(),
            )]
            .into_iter()
            .collect();
            let session = connect(
                &address,
                "Tester",
                None,
                cfg::ClientInformation::new(8),
                Arc::new(lang),
                Arc::clone(&data),
                Arc::default(),
            );
            let (reason, _credit) = loop {
                let (event, credit) = session
                    .events
                    .recv_timeout_owned(Duration::from_secs(60))
                    .unwrap()
                    .into_parts();
                if let Event::Disconnected { reason, .. } = event {
                    break (reason, credit);
                }
            };
            // Without an account nothing joins, LAN or not.
            let sent_key = worker.join().unwrap();
            assert!(!sent_key, "lan {lan}: joined without an account");
            assert_eq!(reason, NOT_SIGNED_IN);
            // A signed-in account whose join the session service rejects:
            // the case vanilla's listener is run with.
            let goes_on = continues_after_failed_join(lan, true);
            assert_eq!(goes_on, lan);
            println!(
                "lan_auth={}:{}:{}",
                if lan { "LAN" } else { "OTHER" },
                goes_on,
                !goes_on
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A recorder that keeps what reaches it, as (id, payload).
    #[derive(Clone, Default)]
    struct TestRecorder(Arc<Mutex<Vec<(i32, Vec<u8>)>>>);
    impl PacketRecorder for TestRecorder {
        fn is_active(&self) -> bool {
            true
        }
        fn received(&self, _packet: &play::Clientbound, id: i32, payload: &[u8]) {
            self.0.lock().unwrap().push((id, payload.to_vec()));
        }
        fn record_packet(&self, id: i32, payload: &[u8]) {
            self.0.lock().unwrap().push((id, payload.to_vec()));
        }
        fn status(&self) -> RecorderStatus {
            RecorderStatus {
                active: true,
                path: PathBuf::from("test.mcpr"),
                elapsed: Duration::ZERO,
                packet_count: self.0.lock().unwrap().len() as u64,
                bytes_written: 0,
                error: None,
            }
        }
        fn stop(self: Box<Self>) -> std::io::Result<PathBuf> {
            Ok(PathBuf::from("test.mcpr"))
        }
    }
    impl TestRecorder {
        fn decoded(&self) -> Vec<play::Clientbound> {
            let packets = self.0.lock().unwrap();
            packets
                .iter()
                .map(|(id, payload)| play::Clientbound::decode(*id, payload).unwrap())
                .collect()
        }
    }

    #[test]
    fn session_recorder_gets_the_play_packets_the_network_thread_receives() {
        let (root, data) = empty_item_data("studio_recording");
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        let (send, events) = event_channel();
        let session = Session {
            events,
            payload_events: event_channel().0,
            payloads: crate::raw_payloads::Transport::empty(),
            writer: Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap()))),
            configuring: Arc::new(AtomicBool::new(false)),
            thread: None,
            address: "local".into(),
            name: "StudioTester".into(),
            counters: Arc::new(Mutex::new(None)),
            cookies: Cookies::default(),
            debug: false,
            far: Arc::default(),
            recorder: Arc::new(Mutex::new(None)),
            pings: Arc::default(),
            serial: next_serial(),
            payload_output: std::sync::OnceLock::new(),
        };
        let recorder = TestRecorder::default();
        session.start_recording(recorder.clone(), |_| {});
        assert!(session.is_recording());

        let (thread_writer, thread_configuring, thread_recorder) = (
            session.writer.clone(),
            session.configuring.clone(),
            session.recorder.clone(),
        );
        let thread = std::thread::spawn(move || {
            receive_play(
                &mut reader,
                Arc::default(),
                &thread_writer,
                &thread_configuring,
                &Arc::new(Language::new()),
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &thread_recorder,
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            )
        });

        let mut server = Connection::local(server);
        let pos_pkt = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 1,
            position: [100.0, 64.0, -200.0],
            delta: [0.0; 3],
            yaw: 45.0,
            pitch: 10.0,
            relatives: 0,
        });
        server.send(&pos_pkt.encode()).unwrap();
        // Disconnect ends receive_play's loop.
        let disc_pkt = play::Clientbound::Disconnect(cfg::Disconnect {
            reason: Tag::String("Replay tap test completed".into()),
        });
        server.send(&disc_pkt.encode()).unwrap();
        let _ = thread.join().unwrap();

        assert_eq!(
            session.stop_recording().unwrap().unwrap(),
            PathBuf::from("test.mcpr")
        );
        assert!(!session.is_recording());
        // The recorder is handed both; what a recording keeps is the recorder's choice.
        assert_eq!(recorder.decoded(), [pos_pkt, disc_pkt]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_recorder_gets_the_packets_the_client_makes() {
        use sunrise_protocol::entity_data::{Value, serializer};

        let session = Session::replay("StudioTester");
        session.record_clientbound(&play::Clientbound::SwingAnimation(play::SwingAnimation {
            id: 1,
            hand: 0,
            kind: 0,
            duration: 6,
        }));
        let recorder = TestRecorder::default();
        // What `initialize` writes comes first.
        let snapshot = play::Clientbound::PlayerPosition(play::PlayerPosition {
            teleport: 0,
            position: [12.5, 64.0, -88.5],
            delta: [0.0; 3],
            yaw: 90.0,
            pitch: 15.0,
            relatives: 0,
        });
        session.start_recording(recorder.clone(), |rec| {
            let encoded = snapshot.encode();
            let mut reader = sunrise_protocol::buf::Reader::new(&encoded);
            let id = reader.varint().unwrap();
            rec.record_packet(id, reader.rest());
        });
        let swing_pkt = play::Clientbound::SwingAnimation(play::SwingAnimation {
            id: 42,
            hand: 0,
            kind: 1,
            duration: 6,
        });
        session.record_clientbound(&swing_pkt);
        let data_pkt = play::Clientbound::SetEntityData(play::SetEntityData {
            id: 42,
            values: vec![
                (0, serializer::BYTE, Value::Byte(0x02)),
                (6, serializer::POSE, Value::Id(5)),
            ],
        });
        session.record_clientbound(&data_pkt);
        assert_eq!(session.recorder_status().unwrap().packet_count, 3);
        session.stop_recording().unwrap().unwrap();
        // Nothing reaches a stopped recording.
        session.record_clientbound(&swing_pkt);

        assert_eq!(recorder.decoded(), [snapshot, swing_pkt, data_pkt]);
    }

    #[test]
    fn fill_known_entries_rejects_path_traversal_and_reads_valid_entries() {
        let temp_dir =
            std::env::temp_dir().join(format!("sunrise-net-reg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp_dir);
        let resources = temp_dir.join("res");
        let valid_dir = resources.join("data/minecraft/worldgen/biome");
        std::fs::create_dir_all(&valid_dir).unwrap();
        std::fs::write(
            valid_dir.join("plains.json"),
            br#"{"has_precipitation":true}"#,
        )
        .unwrap();
        std::fs::write(temp_dir.join("secret.json"), br#"{"leaked":true}"#).unwrap();

        let mut reg = cfg::RegistryData {
            registry: "minecraft:worldgen/biome".into(),
            entries: vec![
                cfg::RegistryEntry {
                    id: "minecraft:plains".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "minecraft:../../../../secret".into(),
                    data: None,
                },
                cfg::RegistryEntry {
                    id: "..:plains".into(),
                    data: None,
                },
            ],
        };
        fill_known_entries(&mut reg, &resources);
        assert!(reg.entries[0].data.is_some());
        assert!(reg.entries[1].data.is_none());
        assert!(reg.entries[2].data.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn network_witness(file: &str) -> serde_json::Value {
        let dir = std::env::var("SUNRISE_NETWORK_WITNESS_DIR").expect(
            "SUNRISE_NETWORK_WITNESS_DIR must be set (run via tools/test_network_parity.py)",
        );
        let path = std::path::Path::new(&dir).join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        serde_json::from_str(&text).unwrap()
    }

    fn network_hex(hex: &str) -> Vec<u8> {
        (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    }

    fn spawn_play_harness(
        name: &str,
        lang: Language,
    ) -> (
        TempDirGuard,
        Connection,
        ingress::Consumer<Event>,
        Arc<Mutex<Option<QueuedWriter>>>,
        std::sync::mpsc::Receiver<Result<Reason, Reason>>,
        JoinHandle<()>,
    ) {
        spawn_play_harness_with_registries(name, lang, Arc::default())
    }

    fn spawn_play_harness_with_registries(
        name: &str,
        lang: Language,
        registries: Arc<Dynamic>,
    ) -> (
        TempDirGuard,
        Connection,
        ingress::Consumer<Event>,
        Arc<Mutex<Option<QueuedWriter>>>,
        std::sync::mpsc::Receiver<Result<Reason, Reason>>,
        JoinHandle<()>,
    ) {
        spawn_play_harness_with_items(name, lang, registries, "{}")
    }

    fn spawn_play_harness_with_items(
        name: &str,
        lang: Language,
        registries: Arc<Dynamic>,
        items_json: &str,
    ) -> (
        TempDirGuard,
        Connection,
        ingress::Consumer<Event>,
        Arc<Mutex<Option<QueuedWriter>>>,
        std::sync::mpsc::Receiver<Result<Reason, Reason>>,
        JoinHandle<()>,
    ) {
        let root = std::env::temp_dir().join(format!(
            "sunrise-net-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("items.json"), items_json).unwrap();
        let inputs = sunrise_world::native_load_context::LoadInputs::new(
            Default::default(),
            root.join("settings"),
        );
        let data = Arc::new(
            ItemData::load(
                &root,
                sunrise_world::native_load_context::LoadContext::new(
                    sunrise_world::native_load_context::DataLoadOrigin::ClientPreload,
                    Arc::new(inputs),
                    [],
                ),
            )
            .unwrap(),
        );
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let (mut reader, writer) = Connection::local(client).split();
        reader
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let shared_writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
        let client_writer = shared_writer.clone();
        let (send, events) = event_channel();
        let (done_tx, done_rx) = channel();
        let lang = Arc::new(lang);
        let thread = std::thread::spawn(move || {
            let result = receive_play(
                &mut reader,
                registries,
                &client_writer,
                &Arc::new(AtomicBool::new(false)),
                &lang,
                &send,
                &Cookies::default(),
                &Arc::default(),
                &data,
                &Arc::new(Mutex::new(None)),
                &crate::raw_payloads::Transport::empty(),
                &Arc::default(),
                &Arc::default(),
            );
            let _ = done_tx.send(result);
        });
        let server = Connection::local(server);
        (
            TempDirGuard(root),
            server,
            events,
            shared_writer,
            done_rx,
            thread,
        )
    }

    fn run_fresh_cases(name: &str, flag: &str, cases: &[&str], body: impl FnOnce(&str)) {
        if let Ok(case) = std::env::var(flag) {
            return body(&case);
        }
        for &case in cases {
            crate::fresh_process::run_in_fresh_process(name, flag, case);
        }
    }

    #[test]
    #[ignore = "daily"]
    fn bundle_holds_packets_until_closing_delimiter_and_emits_one_event_26_3() {
        let witness = network_witness("bundle.json");
        let (_dir, mut server, events, shared_writer, done_rx, thread) =
            spawn_play_harness("bundle-wire", Language::new());

        let steps = witness["ping_rotation_wire_steps"]["steps"]
            .as_array()
            .unwrap();
        let packets = [
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::Ping(cfg::Ping { id: -26 }),
            play::Clientbound::PlayerRotation(play::PlayerRotation {
                yaw: 0.0,
                relative_yaw: false,
                pitch: 0.0,
                relative_pitch: false,
            }),
            play::Clientbound::BundleDelimiter(Empty),
        ];
        assert_eq!(steps.len(), packets.len());
        for (step, packet) in steps.iter().zip(packets) {
            server.send(&packet.encode()).unwrap();
            let expected_count = step["emitted_count"].as_u64().unwrap() as usize;
            let mut got = 0usize;
            if expected_count > 0 {
                assert!(
                    events.recv_timeout_owned(Duration::from_secs(2)).is_ok(),
                    "step {}: expected {expected_count} emitted event(s)",
                    step["sent"]
                );
                got += 1;
            }
            while events.recv_timeout_owned(Duration::from_millis(80)).is_ok() {
                got += 1;
            }
            assert_eq!(got, expected_count, "step {}", step["sent"]);
        }

        // Empty bundle ([delimiter, delimiter]) emits one bundle event with 0 subpackets.
        assert!(
            witness["empty_bundle"]["emitted_is_bundle"]
                .as_bool()
                .unwrap()
        );
        server
            .send(&play::Clientbound::BundleDelimiter(Empty).encode())
            .unwrap();
        assert!(
            events
                .recv_timeout_owned(Duration::from_millis(60))
                .is_err(),
            "opening delimiter of empty bundle emitted an event"
        );
        server
            .send(&play::Clientbound::BundleDelimiter(Empty).encode())
            .unwrap();
        assert!(
            events.recv_timeout_owned(Duration::from_secs(2)).is_ok(),
            "closing delimiter of empty bundle did not emit an event"
        );
        assert!(
            events
                .recv_timeout_owned(Duration::from_millis(60))
                .is_err(),
            "empty bundle emitted more than one event"
        );

        // Back-to-back bundles followed by a single packet emit 3 events total.
        let expected_seq = witness["back_to_back_and_single"].as_array().unwrap().len();
        for packet in [
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::Ping(cfg::Ping { id: 1 }),
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::Ping(cfg::Ping { id: 2 }),
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::Ping(cfg::Ping { id: 3 }),
            play::Clientbound::Disconnect(cfg::Disconnect {
                reason: Tag::String("done".into()),
            }),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        let ended = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(ended.is_ok(), "unexpected error ending receive_play");
        let mut emitted = 0usize;
        while events.try_recv_owned().is_ok() {
            emitted += 1;
        }
        assert_eq!(emitted, expected_seq);

        drop(server);
        drop(events);
        drop(shared_writer);
        thread.join().unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn bundled_keepalive_waits_for_game_thread_in_bundle_order_26_3() {
        let witness = network_witness("bundle.json");
        let (_dir, mut server, events, shared_writer, _done_rx, thread) =
            spawn_play_harness("bundle-keepalive", Language::new());
        server
            .reader
            .set_read_timeout(Some(Duration::from_millis(120)))
            .unwrap();

        // Unbundled control: KeepAlive answers immediately on the network thread.
        assert_eq!(
            witness["keepalive_ordering"]["unbundled"]["sent_before_process"],
            serde_json::json!(["keep_alive:999"])
        );
        server
            .send(&play::Clientbound::Ping(cfg::Ping { id: 7 }).encode())
            .unwrap();
        server
            .send(&play::Clientbound::KeepAlive(cfg::KeepAlive { id: 999 }).encode())
            .unwrap();
        let (id, body) = server.recv().unwrap();
        assert_eq!(
            play::Serverbound::decode(id, &body).unwrap(),
            play::Serverbound::KeepAlive(cfg::KeepAlive { id: 999 })
        );
        assert!(events.recv_timeout_owned(Duration::from_secs(2)).is_ok());

        // Bundled: KeepAlive must not answer on the network thread before the game thread runs.
        assert_eq!(
            witness["keepalive_ordering"]["bundled"]["sent_before_process"],
            serde_json::json!([])
        );
        for packet in [
            play::Clientbound::BundleDelimiter(Empty),
            play::Clientbound::Ping(cfg::Ping { id: 7 }),
            play::Clientbound::KeepAlive(cfg::KeepAlive { id: 999 }),
            play::Clientbound::PlayerRotation(play::PlayerRotation {
                yaw: 90.0,
                relative_yaw: false,
                pitch: 12.5,
                relative_pitch: false,
            }),
            play::Clientbound::BundleDelimiter(Empty),
        ] {
            server.send(&packet.encode()).unwrap();
        }
        let early_reply = server.recv();
        assert!(
            early_reply.is_err(),
            "bundled KeepAlive replied on the network thread: {early_reply:?}"
        );
        let expected_tasks = witness["keepalive_ordering"]["bundled"]["queued_tasks_before_process"]
            .as_u64()
            .unwrap() as usize;
        let mut emitted = 0usize;
        assert!(events.recv_timeout_owned(Duration::from_secs(2)).is_ok());
        emitted += 1;
        while events.recv_timeout_owned(Duration::from_millis(80)).is_ok() {
            emitted += 1;
        }
        assert_eq!(emitted, expected_tasks);

        drop(server);
        drop(events);
        drop(shared_writer);
        thread.join().unwrap();
    }

    #[test]
    #[ignore = "daily"]
    fn bundle_rejects_terminal_packet_and_oversize_over_4096_26_3() {
        let witness = network_witness("bundle.json");
        let generic_lang = || -> Language {
            [("disconnect.genericReason".to_owned(), "%s".to_owned())]
                .into_iter()
                .collect()
        };

        // Terminal packet (`start_configuration`) inside a bundle disconnects.
        {
            let (_dir, mut server, events, shared_writer, done_rx, thread) =
                spawn_play_harness("bundle-terminal", generic_lang());
            server
                .send(&play::Clientbound::BundleDelimiter(Empty).encode())
                .unwrap();
            server
                .send(&play::Clientbound::StartConfiguration(Empty).encode())
                .unwrap();
            let ended = done_rx.recv_timeout(Duration::from_millis(400));
            drop(server);
            drop(events);
            drop(shared_writer);
            let _ = thread.join();
            let reason = match ended {
                Ok(Err(reason)) => reason,
                other => panic!(
                    "expected disconnect on terminal packet in bundle, got {:?}",
                    other.map(|r| r.map(|ok| ok.text).map_err(|e| e.text))
                ),
            };
            assert_eq!(
                reason.key.as_deref(),
                witness["terminal_packet"]["inside_disconnect_key"].as_str()
            );
            assert_eq!(
                Some(reason.text.as_str()),
                witness["terminal_packet"]["inside_disconnect_text"].as_str(),
                "terminal bundle disconnect text"
            );
        }

        // Terminal packet as the 4,097th member hits the terminal check before the size limit.
        {
            let limit = witness["bundle_size_limit"].as_u64().unwrap() as usize;
            let (_dir, mut server, events, shared_writer, done_rx, thread) =
                spawn_play_harness("bundle-terminal-4097", generic_lang());
            let _ = server.send(&play::Clientbound::BundleDelimiter(Empty).encode());
            for i in 0..limit {
                if server
                    .send(&play::Clientbound::Ping(cfg::Ping { id: i as i32 }).encode())
                    .is_err()
                {
                    break;
                }
            }
            let _ = server.send(&play::Clientbound::StartConfiguration(Empty).encode());
            let ended = done_rx.recv_timeout(Duration::from_secs(2));
            drop(server);
            drop(events);
            drop(shared_writer);
            let _ = thread.join();
            let reason = match ended {
                Ok(Err(reason)) => reason,
                other => panic!(
                    "expected terminal disconnect on 4097th member, got {:?}",
                    other.map(|r| r.map(|ok| ok.text).map_err(|e| e.text))
                ),
            };
            assert_eq!(
                reason.key.as_deref(),
                witness["terminal_at_4097"]["disconnect_key"].as_str()
            );
            assert_eq!(
                Some(reason.text.as_str()),
                witness["terminal_at_4097"]["disconnect_text"].as_str(),
                "terminal packet at 4097th member must fail as terminal before size limit"
            );
        }

        // Malformed frames fail decode before bundling in either bundle state.
        let malformed = &witness["malformed_before_bundling"];
        let mut delimiter_extra = play::Clientbound::BundleDelimiter(Empty).encode();
        delimiter_extra.push(0);
        let mut truncated_post_effects = ::sunrise_core::vanilla_bytes::Writer::new();
        truncated_post_effects
            .varint(play::Clientbound::PostEffects(play::PostEffects { effects: vec![] }).id());
        truncated_post_effects.varint(1);
        let mut unregistered_id = ::sunrise_core::vanilla_bytes::Writer::new();
        unregistered_id.varint(999);
        let mut move_pos_extra =
            play::Clientbound::MoveEntityPos(play::MoveEntityPos(play::MoveEntity {
                id: 1,
                delta: Some(play::VecDelta::Linear([0; 3])),
                rotation: None,
                on_ground: true,
            }))
            .encode();
        move_pos_extra.push(0x7F);
        let mut move_minecart_extra =
            play::Clientbound::MoveMinecartAlongTrack(play::MoveMinecartAlongTrack {
                id: 1,
                steps: Vec::new(),
            })
            .encode();
        move_minecart_extra.push(0x7F);
        let mut test_status_extra =
            play::Clientbound::TestInstanceBlockStatus(play::TestInstanceBlockStatus {
                status: Tag::String("ok".into()),
                size: None,
            })
            .encode();
        test_status_extra.push(0x7F);
        let mut waypoint_extra = play::Clientbound::TrackedWaypoint(play::TrackedWaypoint {
            operation: 1,
            id: play::WaypointId::Uuid(1),
            style: "minecraft:default".into(),
            color: None,
            target: play::WaypointTarget::Empty,
        })
        .encode();
        waypoint_extra.push(0x7F);
        for (case_name, frame, also_bundled) in [
            ("bundle_delimiter_extra_bytes", delimiter_extra, true),
            ("truncated_post_effects", truncated_post_effects.data, false),
            ("unregistered_packet_id", unregistered_id.data, true),
            ("move_entity_pos_extra_bytes", move_pos_extra, false),
            ("move_minecart_extra_bytes", move_minecart_extra, false),
            (
                "test_instance_block_status_extra_bytes",
                test_status_extra,
                false,
            ),
            ("waypoint_extra_bytes", waypoint_extra, false),
        ] {
            for bundled in if also_bundled {
                vec![false, true]
            } else {
                vec![false]
            } {
                let (_dir, mut server, events, shared_writer, done_rx, thread) =
                    spawn_play_harness(&format!("malformed-{case_name}-{bundled}"), generic_lang());
                if bundled {
                    let _ = server.send(&play::Clientbound::BundleDelimiter(Empty).encode());
                }
                let _ = server.send(&frame);
                let ended = done_rx.recv_timeout(Duration::from_secs(2));
                drop(server);
                drop(events);
                drop(shared_writer);
                let _ = thread.join();
                let reason = match ended {
                    Ok(Err(reason)) => reason,
                    other => panic!(
                        "{case_name} (bundled={bundled}): expected decode disconnect, got {:?}",
                        other.map(|r| r.map(|ok| ok.text).map_err(|e| e.text))
                    ),
                };
                assert_eq!(
                    reason.key.as_deref(),
                    malformed[case_name]["disconnect_key"].as_str(),
                    "{case_name} (bundled={bundled})"
                );
                assert_eq!(
                    Some(reason.text.as_str()),
                    malformed[case_name]["disconnect_text"].as_str(),
                    "{case_name} (bundled={bundled}): full disconnect text"
                );
            }
        }

        let limit = witness["bundle_size_limit"].as_u64().unwrap() as usize;
        assert_eq!(limit, 4096);

        // Exact limit (4096 subpackets) succeeds and emits one bundle event.
        {
            let (_dir, mut server, events, shared_writer, done_rx, thread) =
                spawn_play_harness("bundle-limit-4096", generic_lang());
            server
                .send(&play::Clientbound::BundleDelimiter(Empty).encode())
                .unwrap();
            for i in 0..limit {
                server
                    .send(&play::Clientbound::Ping(cfg::Ping { id: i as i32 }).encode())
                    .unwrap();
            }
            server
                .send(&play::Clientbound::BundleDelimiter(Empty).encode())
                .unwrap();
            let first = events.recv_timeout_owned(Duration::from_secs(5));
            let second = events.recv_timeout_owned(Duration::from_millis(80));
            let ended = done_rx.try_recv();
            drop(server);
            drop(events);
            drop(shared_writer);
            let _ = thread.join();
            assert!(first.is_ok(), "4096-packet bundle did not emit an event");
            assert!(
                second.is_err(),
                "4096-packet bundle emitted more than one event"
            );
            assert!(
                ended.is_err(),
                "4096-packet bundle unexpectedly disconnected"
            );
        }

        // One past limit (4097th subpacket) disconnects with Too many packets in a bundle.
        {
            let (_dir, mut server, events, shared_writer, done_rx, thread) =
                spawn_play_harness("bundle-limit-4097", generic_lang());
            let _ = server.send(&play::Clientbound::BundleDelimiter(Empty).encode());
            for i in 0..=limit {
                if server
                    .send(&play::Clientbound::Ping(cfg::Ping { id: i as i32 }).encode())
                    .is_err()
                {
                    break;
                }
            }
            let ended = done_rx.recv_timeout(Duration::from_secs(2));
            drop(server);
            drop(events);
            drop(shared_writer);
            let _ = thread.join();
            let reason = match ended {
                Ok(Err(reason)) => reason,
                other => panic!(
                    "expected disconnect on 4097th packet in bundle, got {:?}",
                    other.map(|r| r.map(|ok| ok.text).map_err(|e| e.text))
                ),
            };
            assert_eq!(
                reason.key.as_deref(),
                witness["size_limit"]["limit_4097_disconnect_key"].as_str()
            );
            assert_eq!(
                Some(reason.text.as_str()),
                witness["size_limit"]["limit_4097_disconnect_text"].as_str(),
                "oversize bundle disconnect text"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn configuration_post_effects_disconnects_with_packet_error_26_3() {
        run_fresh_cases(
            "net::tests::configuration_post_effects_disconnects_with_packet_error_26_3",
            "SUNRISE_NET_CONFIG_CASE",
            &["empty", "nonempty", "select_known_packs_extra"],
            |case_name| {
                if case_name == "select_known_packs_extra" {
                    let bundle = network_witness("bundle.json");
                    let known_packs_case = &bundle["malformed_before_bundling"]["config_select_known_packs_extra_bytes"];
                    let raw_frame = network_hex(known_packs_case["frame_hex"].as_str().unwrap());
                    let (root, data) = empty_item_data("cfg-select-known-packs-extra");
                    let _dir = TempDirGuard(root);
                    let (client, server) = sunrise_protocol::connection::Pipe::pair();
                    let (mut reader, writer) = Connection::local(client).split();
                    reader
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let shared_writer =
                        Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
                    let client_writer = shared_writer.clone();
                    let lang: Language = [("disconnect.genericReason".to_owned(), "%s".to_owned())]
                        .into_iter()
                        .collect();
                    let thread = std::thread::spawn(move || {
                        drive_configuration(
                            |send| {
                                configure(
                                    &mut reader,
                                    &client_writer,
                                    &Arc::new(AtomicBool::new(true)),
                                    &lang,
                                    send,
                                    &Cookies::default(),
                                    &data,
                                    &crate::raw_payloads::Transport::empty(),
                                )
                            },
                            |_| {},
                        )
                    });
                    let mut server = Connection::local(server);
                    let _ = server.send(&raw_frame);
                    drop(server);
                    let reason = thread
                        .join()
                        .unwrap()
                        .expect_err("select_known_packs extra bytes must disconnect");
                    drop(shared_writer);
                    assert_eq!(
                        reason.key.as_deref(),
                        known_packs_case["disconnect_key"].as_str()
                    );
                    assert_eq!(
                        Some(reason.text.as_str()),
                        known_packs_case["disconnect_text"].as_str(),
                        "config select_known_packs full disconnect text"
                    );
                    return;
                }
                let witness = network_witness("post_effects.json");
                let hex_key = match case_name {
                    "empty" => "config_empty_hex",
                    "nonempty" => "config_two_hex",
                    other => panic!("unknown configuration case {other}"),
                };
                let expected_key = witness["configuration_state"][case_name]["disconnect_key"]
                    .as_str()
                    .unwrap();
                let raw_frame = network_hex(witness["wire"][hex_key].as_str().unwrap());
                let (root, data) = empty_item_data(&format!("cfg-post-effects-{case_name}"));
                let _dir = TempDirGuard(root);
                let (client, server) = sunrise_protocol::connection::Pipe::pair();
                let (mut reader, writer) = Connection::local(client).split();
                reader
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let shared_writer = Arc::new(Mutex::new(Some(QueuedWriter::new(writer).unwrap())));
                let client_writer = shared_writer.clone();
                let lang: Language = [(
                    "disconnect.packetError".to_owned(),
                    "Network Protocol Error".to_owned(),
                )]
                .into_iter()
                .collect();
                let thread = std::thread::spawn(move || {
                    drive_configuration(
                        |send| {
                            configure(
                                &mut reader,
                                &client_writer,
                                &Arc::new(AtomicBool::new(true)),
                                &lang,
                                send,
                                &Cookies::default(),
                                &data,
                                &crate::raw_payloads::Transport::empty(),
                            )
                        },
                        |_| {},
                    )
                });
                let mut server = Connection::local(server);
                server
                    .reader
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let _ = server.send(&cfg::Clientbound::Ping(cfg::Ping { id: 19 }).encode());
                let _ = server.send(&raw_frame);
                let _ = server.send(&cfg::Clientbound::FinishConfiguration(Empty).encode());
                let mut replies = Vec::new();
                while let Ok((id, body)) = server.recv() {
                    replies.push(cfg::Serverbound::decode(id, &body).unwrap());
                }
                let ended = thread.join().unwrap();
                drop(shared_writer);
                let expected_replies: Vec<String> =
                    witness["configuration_state"][case_name]["sent_after_process"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect();
                let got_replies: Vec<String> = replies
                    .iter()
                    .map(|packet| match packet {
                        cfg::Serverbound::Pong(p) => format!("pong:{}", p.id),
                        cfg::Serverbound::FinishConfiguration(_) => {
                            "finish_configuration".to_owned()
                        }
                        other => format!("{other:?}"),
                    })
                    .collect();
                assert_eq!(
                    got_replies, expected_replies,
                    "config post_effects ({case_name}): earlier Ping must drain as Pong and FinishConfiguration must not send"
                );
                let got = match ended {
                    Ok(_) => "none".to_owned(),
                    Err(reason) => reason.key.unwrap_or(reason.text),
                };
                assert_eq!(got, expected_key, "config post_effects ({case_name})");
            },
        );
    }

    #[test]
    fn vanilla_clientbound_class_tables_match_registered_ids() {
        use sunrise_protocol::{State, ids};
        assert_eq!(
            STATUS_CLIENTBOUND_CLASSES.len(),
            ids::status::clientbound::NAMES.len()
        );
        assert_eq!(
            LOGIN_CLIENTBOUND_CLASSES.len(),
            ids::login::clientbound::NAMES.len()
        );
        assert_eq!(
            CONFIGURATION_CLIENTBOUND_CLASSES.len(),
            ids::configuration::clientbound::NAMES.len()
        );
        assert_eq!(
            PLAY_CLIENTBOUND_CLASSES.len(),
            ids::play::clientbound::NAMES.len()
        );
        assert_eq!(
            vanilla_clientbound_class_name(
                State::Configuration,
                ids::configuration::clientbound::SELECT_KNOWN_PACKS
            ),
            "ClientboundSelectKnownPacks"
        );
        assert_eq!(
            vanilla_clientbound_class_name(State::Play, ids::play::clientbound::MOVE_ENTITY_POS),
            "Pos"
        );
        assert_eq!(
            vanilla_clientbound_class_name(
                State::Play,
                ids::play::clientbound::MOVE_ENTITY_POS_ROT
            ),
            "PosRot"
        );
        assert_eq!(
            vanilla_clientbound_class_name(State::Play, ids::play::clientbound::MOVE_ENTITY_ROT),
            "Rot"
        );
        assert_eq!(
            vanilla_clientbound_class_name(
                State::Play,
                ids::play::clientbound::MOVE_MINECART_ALONG_TRACK
            ),
            "ClientboundMoveMinecartPacket"
        );
        assert_eq!(
            vanilla_clientbound_class_name(
                State::Play,
                ids::play::clientbound::TEST_INSTANCE_BLOCK_STATUS
            ),
            "ClientboundTestInstanceBlockStatus"
        );
        assert_eq!(
            vanilla_clientbound_class_name(State::Play, ids::play::clientbound::WAYPOINT),
            "ClientboundTrackedWaypointPacket"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn unknown_entity_data_serializer_disconnects_play_session_like_26_3() {
        let witness = network_witness("entity_data.json");
        let mut reg = Dynamic::default();
        let variants = witness["painting_variant_definitions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                let name = v["name"].as_str().unwrap().to_owned();
                let tag = Tag::compound([
                    ("width", Tag::Int(v["width"].as_i64().unwrap() as i32)),
                    ("height", Tag::Int(v["height"].as_i64().unwrap() as i32)),
                    (
                        "asset_id",
                        Tag::String(v["asset_id"].as_str().unwrap().into()),
                    ),
                ]);
                (name, Some(tag))
            })
            .collect();
        reg.registries
            .insert("minecraft:painting_variant".into(), variants);
        let registries = Arc::new(reg);
        let registered_item = witness["hover_item_component_validation"]["registered_item_id"]
            .as_str()
            .unwrap();
        let items_json = format!(r#"{{"{registered_item}":{{}}}}"#);
        let assert_disconnect =
            |name: &str, body: &[u8], expected_key: &str, expected_text: Option<&str>| {
                let lang: Language = [("disconnect.genericReason".to_owned(), "%s".to_owned())]
                    .into_iter()
                    .collect();
                let (_dir, mut server, events, shared_writer, done_rx, thread) =
                    spawn_play_harness_with_items(
                        &format!("entity-data-{name}"),
                        lang,
                        registries.clone(),
                        &items_json,
                    );
                let mut frame = ::sunrise_core::vanilla_bytes::Writer::new();
                frame.varint(sunrise_protocol::ids::play::clientbound::SET_ENTITY_DATA);
                frame.bytes(body);
                let _ = server.send(&frame.data);
                let ended = done_rx.recv_timeout(Duration::from_millis(500));
                drop(server);
                drop(events);
                drop(shared_writer);
                let _ = thread.join();
                let reason = match ended {
                    Ok(Err(reason)) => reason,
                    other => panic!(
                        "{name}: expected play disconnect on malformed entity_data, got {:?}",
                        other.map(|r| r.map(|ok| ok.text).map_err(|e| e.text))
                    ),
                };
                assert_eq!(reason.key.as_deref(), Some(expected_key), "{name}");
                assert_eq!(
                    Some(reason.text.as_str()),
                    expected_text,
                    "{name}: full disconnect text"
                );
            };
        for row in witness["unknown_serializers"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            let body = network_hex(row["packet_body_hex"].as_str().unwrap());
            assert_disconnect(
                name,
                &body,
                row["connection_disconnect_key"].as_str().unwrap(),
                row["connection_disconnect_text"].as_str(),
            );
        }
        let wrap_entity_data = |hex: &str| -> Vec<u8> {
            let mut body = ::sunrise_core::vanilla_bytes::Writer::new();
            body.varint(1);
            body.bytes(&network_hex(hex));
            body.data
        };
        for row in witness["invalid_painting_holders"].as_array().unwrap() {
            let name = row["name"].as_str().unwrap();
            let body = wrap_entity_data(row["entity_data_hex"].as_str().unwrap());
            assert_disconnect(
                &format!("painting-holder-{name}"),
                &body,
                row["connection_disconnect_key"].as_str().unwrap(),
                row["connection_disconnect_text"].as_str(),
            );
        }
        let bad_inline = &witness["malformed_inline_painting_component"];
        assert_disconnect(
            "malformed-inline-painting",
            &wrap_entity_data(bad_inline["entity_data_hex"].as_str().unwrap()),
            bad_inline["connection_disconnect_key"].as_str().unwrap(),
            bad_inline["connection_disconnect_text"].as_str(),
        );
        let hover = &witness["hover_item_component_validation"];
        assert_disconnect(
            "unregistered-hover-item",
            &wrap_entity_data(hover["unregistered_item_hex"].as_str().unwrap()),
            hover["unregistered_item_disconnect_key"].as_str().unwrap(),
            hover["unregistered_item_disconnect_text"].as_str(),
        );
        // Registered hover item decodes cleanly in play and delivers SetEntityData.
        {
            let (_dir, mut server, events, shared_writer, done_rx, thread) =
                spawn_play_harness_with_items(
                    "entity-data-registered-hover-item",
                    Language::new(),
                    registries.clone(),
                    &items_json,
                );
            let mut frame = ::sunrise_core::vanilla_bytes::Writer::new();
            frame.varint(sunrise_protocol::ids::play::clientbound::SET_ENTITY_DATA);
            frame.bytes(&wrap_entity_data(
                hover["registered_item_hex"].as_str().unwrap(),
            ));
            server.send(&frame.data).unwrap();
            let event = events
                .recv_timeout_owned(Duration::from_secs(2))
                .expect("registered hover item must deliver packet")
                .into_parts()
                .0;
            assert!(matches!(
                event,
                Event::Packet(pkt) if matches!(*pkt, play::Clientbound::SetEntityData(_))
            ));
            drop(server);
            let _ = done_rx.recv_timeout(Duration::from_millis(500));
            drop(shared_writer);
            let _ = thread.join();
        }
        let bad_particle = &witness["particle_metadata"];
        assert_disconnect(
            "unregistered-particle-128",
            &wrap_entity_data(bad_particle["unregistered_128_hex"].as_str().unwrap()),
            bad_particle["unregistered_128_disconnect_key"]
                .as_str()
                .unwrap(),
            bad_particle["unregistered_128_disconnect_text"].as_str(),
        );
    }
}
