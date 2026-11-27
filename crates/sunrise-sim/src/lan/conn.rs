//! One accepted connection: handshake, status ping, login (offline, or online
//! with encryption and the session service's verdict) and the
//! configuration phase (ServerStatusPacketListenerImpl,
//! ServerLoginPacketListenerImpl, ServerConfigurationPacketListenerImpl). A
//! player that completes configuration continues in the play state on the
//! network threads ([`io_pool`]): they forward decoded packets to the game
//! thread, frame, compress and encrypt what the game queues, and keep the
//! keep-alive schedule on their own so a long tick never times a client out.
mod configuration_tasks;
mod payloads;
#[path = "transaction_packets.rs"]
pub mod transaction_packets;
use super::config_payloads::ConfigIo;
use super::{
    Body, Joined, NetEvent, Out, Shared,
    data::{CORE_PACK, SyncData},
    io_pool::{self, Session as _},
};
use crate::raw_payloads::{Side, Transport};
use mod_api::{PayloadFrameStage, PayloadId, PayloadSendError, VirtualHost};
use std::{
    cell::Cell,
    net::TcpStream,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering},
        mpsc::{RecvTimeoutError, Sender, channel},
    },
    time::{Duration, Instant},
};
use sunrise_protocol::{
    PROTOCOL_VERSION,
    connection::{ConnWriter, Connection, Counters},
    crypto::{offline_uuid, server_hash, uuid_string},
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation, KnownPack},
        handshake, login, play, status,
    },
    payload_negotiation::Phase,
    text::translatable,
};

/// ServerCommonPacketListenerImpl.keepConnectionAlive: a keep-alive every 15 s,
/// and a client that has not answered the last one by the next is dropped.
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(15);
/// ReadTimeoutHandler(30) in ServerConnectionListener: a connection that sends
/// nothing for 30 s.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// How long the handshake, a status request and the login may wait for the client's next packet,
/// where vanilla waits 30 s: a client answers these at once, and each connection holds a thread
/// and one of the places the server keeps for connections, which a crowd of silent or slow
/// sockets would otherwise take for 30 s each. Configuration and play keep [`READ_TIMEOUT`] (a
/// resource pack downloads in configuration).
const LOGIN_TIMEOUT: Duration = Duration::from_secs(10);
/// Vanilla's server-side disconnect during login is a JSON component.
fn login_disconnect(c: &mut Connection, reason: &sunrise_core::nbt::Tag) {
    let json = component_json(reason);
    let _ =
        c.send(&login::Clientbound::Disconnect(login::Disconnect { reason_json: json }).encode());
}

/// A component as ComponentSerialization.CODEC writes it with JsonOps (and
/// Gson without HTML escaping): plain text collapses to a string, the
/// content fields come first, then `extra`, then the style.
pub fn component_json(tag: &sunrise_core::nbt::Tag) -> String {
    component_value(tag).compact()
}

/// A login disconnect's reason JSON for a component already in its JSON form, written as
/// [`component_json`] writes one, or None when `ClientboundLoginDisconnectPacket`'s codec
/// would refuse it.
pub fn login_disconnect_json(component: &serde_json::Value) -> Option<String> {
    let reason_json = json_component_value(component).compact();
    let packet = login::Clientbound::Disconnect(login::Disconnect { reason_json }).encode();
    // The packet id, 0, is one VarInt byte.
    match login::Clientbound::decode(i32::from(packet[0]), &packet[1..]) {
        Ok(login::Clientbound::Disconnect(login::Disconnect { reason_json })) => Some(reason_json),
        _ => None,
    }
}

/// [`component_value`] for a JSON component: its numbers keep their JSON form.
fn json_component_value(value: &serde_json::Value) -> crate::server::json::Json {
    use crate::server::json::Json;
    use serde_json::Value;
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => Json::Number(n.to_string()),
        Value::String(s) => Json::String(s.clone()),
        Value::Array(items) => Json::Array(items.iter().map(json_component_value).collect()),
        Value::Object(map) => {
            if map.len() == 1
                && let Some(Value::String(t)) = map.get("text")
            {
                return Json::String(t.clone());
            }
            let mut members: Vec<(&String, &Value)> = map.iter().collect();
            members.sort_by_key(|(k, _)| component_key_rank(k));
            Json::Object(
                members
                    .into_iter()
                    .map(|(k, v)| (k.clone(), json_component_value(v)))
                    .collect(),
            )
        }
    }
}

/// Where ComponentSerialization.CODEC writes the field `key`: unknown fields last.
fn component_key_rank(key: &str) -> usize {
    const ORDER: &[&str] = &[
        "type",
        "text",
        "translate",
        "fallback",
        "with",
        "score",
        "selector",
        "separator",
        "keybind",
        "nbt",
        "interpret",
        "block",
        "entity",
        "storage",
        "source",
        "object",
        "sprite",
        "atlas",
        "player",
        "hat",
        "extra",
        "color",
        "shadow_color",
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
        "click_event",
        "hover_event",
        "insertion",
        "font",
    ];
    ORDER.iter().position(|o| *o == key).unwrap_or(ORDER.len())
}

fn component_value(tag: &sunrise_core::nbt::Tag) -> crate::server::json::Json {
    use crate::server::json::Json;
    use sunrise_core::nbt::Tag;
    match tag {
        Tag::String(s) => Json::String(s.to_string_lossy()),
        Tag::List(items) => Json::Array(items.iter().map(component_value).collect()),
        Tag::Compound(map) => {
            if map.len() == 1
                && let Some(Tag::String(t)) = map.get(&sunrise_core::nbt::JavaString::from("text"))
            {
                return Json::String(t.to_string_lossy());
            }
            let mut keys: Vec<String> = map.keys().map(|k| k.to_string_lossy()).collect();
            keys.sort_by_key(|k| component_key_rank(k));
            Json::Object(
                keys.into_iter()
                    .map(|k| {
                        let v = &map[&sunrise_core::nbt::JavaString::from(k.as_str())];
                        let value = match v {
                            Tag::Byte(b) => Json::Bool(*b != 0),
                            Tag::Int(i) => Json::Number(i.to_string()),
                            other => component_value(other),
                        };
                        (k, value)
                    })
                    .collect(),
            )
        }
        Tag::Byte(b) => Json::Bool(*b != 0),
        Tag::Int(i) => Json::Number(i.to_string()),
        Tag::Long(i) => Json::Number(i.to_string()),
        _ => Json::Null,
    }
}

/// The inverse of [`component_value`]: a JSON component (a server.properties
/// prompt) as ComponentSerialization.CODEC writes it with NbtOps. An array is
/// its first component with the rest appended (`createFromList`); a list of
/// components mixing plain strings and objects has the strings as `{text}`.
pub fn component_tag(v: &crate::server::json::Json) -> sunrise_core::nbt::Tag {
    use crate::server::json::Json;
    use sunrise_core::nbt::{JavaString, Tag};
    fn list(items: &[Json]) -> Tag {
        same_kind(items.iter().map(component_tag).collect())
    }
    fn same_kind(mut tags: Vec<Tag>) -> Tag {
        if tags.iter().any(|t| matches!(t, Tag::Compound(_))) {
            for t in &mut tags {
                if let Tag::String(s) = t {
                    *t = Tag::compound([("text", Tag::String(s.clone()))]);
                }
            }
        }
        Tag::List(tags)
    }
    fn value(v: &Json) -> Tag {
        match v {
            Json::Bool(b) => Tag::Byte(i8::from(*b)),
            Json::Number(n) => n.parse().map_or_else(
                |_| Tag::Double(n.parse::<f64>().unwrap_or(0.0).to_bits()),
                Tag::Int,
            ),
            Json::String(s) => Tag::String(JavaString::from(s.as_str())),
            Json::Array(items) => list(items),
            Json::Object(members) => Tag::compound(
                members
                    .iter()
                    .map(|(k, v)| (k.as_str(), value(v)))
                    .collect::<Vec<_>>(),
            ),
            Json::Null => Tag::String(JavaString::from("")),
        }
    }
    match v {
        Json::Array(items) if !items.is_empty() => {
            let mut first = component_tag(&items[0]);
            if let Tag::String(s) = &first {
                first = Tag::compound([("text", Tag::String(s.clone()))]);
            }
            if let Tag::Compound(map) = &mut first {
                let mut extra = match map.remove(&JavaString::from("extra")) {
                    Some(Tag::List(e)) => e,
                    _ => Vec::new(),
                };
                extra.extend(items[1..].iter().map(component_tag));
                map.insert(JavaString::from("extra"), same_kind(extra));
            }
            first
        }
        Json::Object(members) => {
            if let [(key, Json::String(text))] = members.as_slice()
                && key == "text"
            {
                return Tag::String(JavaString::from(text.as_str()));
            }
            Tag::compound(
                members
                    .iter()
                    .map(|(k, v)| {
                        let tag = match k.as_str() {
                            "extra" | "with" => match v {
                                Json::Array(items) => list(items),
                                other => value(other),
                            },
                            _ => value(v),
                        };
                        (k.as_str(), tag)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        Json::String(s) | Json::Number(s) => Tag::String(JavaString::from(s.as_str())),
        Json::Bool(b) => Tag::String(JavaString::from(b.to_string().as_str())),
        _ => Tag::String(JavaString::from("")),
    }
}

pub use crate::offline_name::valid_name;

thread_local! {
    static HOLDING: Cell<bool> = const { Cell::new(false) };
}

/// While one is alive, the packets this thread sends to each player are held
/// and handed to the player's network thread together by
/// [`PlayerSender::flush`] or [`flush_all`]: one hand-over per player per tick
/// rather than one per packet. The bytes and their order don't change.
pub struct Holding(bool);
impl Holding {
    pub fn start() -> Self {
        Self(HOLDING.replace(true))
    }
}
impl Drop for Holding {
    fn drop(&mut self) {
        HOLDING.set(self.0);
    }
}

/// The packets (and their bytes) a connection's network thread has handed the hub and the hub hasn't
/// taken from its channel yet. The hub's inbox caps apply only as it drains that channel, at the start
/// of a tick: while a tick runs long (an autosave, chunk generation) a client could otherwise queue
/// tiny packets without bound, each a boxed `Serverbound` many times its size on the wire.
#[derive(Debug, Default)]
pub struct Undrained {
    packets: AtomicUsize,
    bytes: AtomicUsize,
}
impl Undrained {
    /// Twice the hub's inbox caps: a client within them never comes near.
    pub const MAX_PACKETS: usize = 2 * super::hub::INBOX_LIMIT;
    pub const MAX_BYTES: usize = 2 * super::hub::INBOX_BYTES;
    /// Count a packet of `bytes` handed to the hub, or `false` (counting nothing) when that would
    /// pass a cap: the network thread disconnects the client then.
    pub fn handed(&self, bytes: usize) -> bool {
        let packets = self.packets.fetch_add(1, Ordering::AcqRel);
        let total = self.bytes.fetch_add(bytes, Ordering::AcqRel);
        if packets >= Self::MAX_PACKETS || total.saturating_add(bytes) > Self::MAX_BYTES {
            self.taken(bytes);
            return false;
        }
        true
    }
    /// The hub took a packet of `bytes` from its channel.
    pub fn taken(&self, bytes: usize) {
        self.packets.fetch_sub(1, Ordering::AcqRel);
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
    }
}

/// The sending half given to the game thread.
#[derive(Clone)]
pub struct PlayerSender {
    payload_transport: Arc<Transport>,
    virtual_host: Option<Arc<VirtualHost>>,
    /// What the player's network thread has handed the hub and the hub hasn't taken yet.
    pub undrained: Arc<Undrained>,
    /// The bytes of the bodies sent and not framed yet.
    pub pending: Arc<AtomicUsize>,
    /// Bytes a network client may leave queued before it is closed ([`io_pool::MAX_PENDING`]).
    cap: usize,
    held: Arc<Mutex<Vec<Body>>>,
    framing: sunrise_protocol::connection::Framing,
    to: To,
    /// The connection's bytes on the wire (after compression and encryption), counted by its
    /// network thread: `sunrise-report`'s network use per player. None for a test's channel.
    wire: Option<Arc<Counters>>,
    /// Registry negotiation belongs to this live connection, never a process-wide UUID cache.
    pub datapack_offer: Option<Arc<[u8]>>,
    /// The plugin channels the client announced, in configuration and play.
    pub announced: Arc<super::announced_channels::AnnouncedChannels>,
    /// Its `sunrise:levels` answer: its revision and the levels besides its own it can view.
    pub viewed_levels: sunrise_protocol::viewed_levels::Answer,
    /// The `sunrise:menu_extra` revision the client answered in configuration,
    /// or 0 when the client did not answer (for example, a vanilla client).
    pub menu_extra_rev: u8,
}
#[derive(Clone)]
enum To {
    /// A connectionless actor: nothing is ever delivered.
    Disabled,
    /// A test's channel.
    Channel(Sender<Out>),
    /// A connection on the network threads, and its keep-alive clock.
    Pool(Arc<io_pool::Outbox>, Arc<KeepAliveWatch>),
}
impl PlayerSender {
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub(crate) fn payload_diagnostic_observer(&self) -> Box<dyn Fn() -> String + Send + Sync> {
        let pending = Arc::downgrade(&self.pending);
        let held = Arc::downgrade(&self.held);
        let (outbox, kind) = match &self.to {
            To::Pool(outbox, _) => (Some(Arc::downgrade(outbox)), "pool"),
            To::Disabled => (None, "disabled"),
            To::Channel(_) => (None, "test channel"),
        };
        Box::new(move || {
            let pending = pending.upgrade().map(|p| p.load(Ordering::Acquire));
            let window = pending.map(|p| io_pool::MAX_PENDING.saturating_sub(p));
            let held = match held.upgrade() {
                Some(held) => match held.try_lock() {
                    Ok(h) => format!(
                        "items={} bytes={}",
                        h.len(),
                        h.iter().map(|body| body.len()).sum::<usize>()
                    ),
                    Err(_) => "busy/poisoned".into(),
                },
                None => "owner dropped".into(),
            };
            let outbox = match &outbox {
                Some(outbox) => outbox
                    .upgrade()
                    .map(|o| o.diagnostics())
                    .unwrap_or_else(|| "outbox owner dropped".into()),
                None => kind.into(),
            };
            format!(
                "server window={window:?} capacity={} pending_charge={pending:?} (None=owner dropped) held={held} {outbox}",
                io_pool::MAX_PENDING
            )
        })
    }
    /// The immutable codec snapshot and live epochs for this connection.
    pub fn payload_transport(&self) -> &Arc<Transport> {
        &self.payload_transport
    }
    /// The client-supplied destination from this connection's login handshake.
    pub fn virtual_host(&self) -> Option<&VirtualHost> {
        self.virtual_host.as_deref()
    }
    /// Frozen physical framing, used to reject before copying a large raw body.
    pub(crate) fn payload_framing(&self) -> sunrise_protocol::connection::Framing {
        self.framing
    }
    pub(crate) fn payload_capacity(&self) -> usize {
        io_pool::MAX_PENDING
    }
    /// A sender whose packets go to `tx` (a test's).
    pub fn new(tx: Sender<Out>, pending: Arc<AtomicUsize>) -> Self {
        Self {
            undrained: Arc::default(),
            pending,
            cap: io_pool::MAX_PENDING,
            held: Arc::default(),
            framing: sunrise_protocol::connection::Framing::Network(None),
            to: To::Channel(tx),
            wire: None,
            payload_transport: Transport::empty(),
            virtual_host: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
        }
    }
    #[cfg(test)]
    pub(crate) fn new_with_payload_transport(
        tx: Sender<Out>,
        pending: Arc<AtomicUsize>,
        transport: Arc<Transport>,
    ) -> Self {
        let mut sender = Self::new(tx, pending);
        sender.payload_transport = transport;
        sender
    }
    /// Real bounded output queue without socket adoption, for executor tests (and the
    /// game's, through `test-payload-probe`).
    #[cfg(any(test, feature = "test-payload-probe"))]
    pub fn payload_executor_test_sender(
        transport: Arc<Transport>,
    ) -> (Self, impl Fn() -> Vec<Vec<u8>>) {
        let pending = Arc::default();
        let outbox = io_pool::Outbox::new(Arc::clone(&pending), false).unwrap();
        let inspect = outbox.clone();
        let sender = Self {
            payload_transport: transport,
            virtual_host: None,
            undrained: Arc::default(),
            pending,
            cap: io_pool::MAX_PENDING,
            held: Arc::default(),
            framing: sunrise_protocol::connection::Framing::Memory,
            to: To::Pool(
                outbox,
                Arc::new(KeepAliveWatch {
                    start: Instant::now(),
                    alive: Arc::new(AtomicI64::new(-1)),
                }),
            ),
            wire: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
        };
        (sender, move || inspect.queued_contents())
    }
    /// A pooled test sender whose client answered no keep-alive since joining, longer ago than
    /// the keep-alive timeout.
    #[cfg(test)]
    pub(crate) fn timed_out_test_sender() -> Self {
        let (mut sender, _) = Self::payload_executor_test_sender(Transport::empty());
        let To::Pool(outbox, _) = &sender.to else {
            unreachable!("the executor test sender is pooled");
        };
        let start = Instant::now()
            .checked_sub(2 * KEEP_ALIVE_EVERY + Duration::from_secs(1))
            .unwrap();
        sender.to = To::Pool(
            outbox.clone(),
            Arc::new(KeepAliveWatch {
                start,
                alive: Arc::new(AtomicI64::new(-1)),
            }),
        );
        sender
    }
    /// Override framing on a test sender (e.g. `Framing::Network(None)` for TCP frame limits).
    #[cfg(test)]
    pub fn with_framing(mut self, framing: sunrise_protocol::connection::Framing) -> Self {
        self.framing = framing;
        self
    }
    /// A connectionless actor has no writer, queue or packet delivery.
    pub fn disabled() -> Self {
        Self {
            undrained: Arc::default(),
            pending: Arc::default(),
            cap: io_pool::MAX_PENDING,
            held: Arc::default(),
            framing: sunrise_protocol::connection::Framing::Network(None),
            to: To::Disabled,
            wire: None,
            payload_transport: Transport::empty(),
            virtual_host: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
        }
    }
    /// The bytes received from and sent to the client on the wire so far.
    pub fn wire_bytes(&self) -> Option<(u64, u64)> {
        self.wire.as_ref().map(|c| {
            (
                c.bytes_in.load(Ordering::Relaxed).max(0) as u64,
                c.bytes_out.load(Ordering::Relaxed).max(0) as u64,
            )
        })
    }
    /// Whether the client has answered no keep-alive for too long.
    pub fn timed_out(&self) -> bool {
        matches!(&self.to, To::Pool(_, watch) if watch.timed_out())
    }
    /// Whether the connection still accepts new packets.
    pub(super) fn is_live(&self) -> bool {
        match &self.to {
            To::Pool(outbox, _) => outbox.alive(),
            To::Channel(_) => true,
            To::Disabled => false,
        }
    }
    /// Close the connection now, dropping what is still queued for it.
    pub fn close(&self) {
        self.payload_transport.close();
        if let To::Pool(outbox, _) = &self.to {
            outbox.send(Vec::new(), Some(io_pool::End::Close));
        }
    }
    /// Retire the writer while keeping the epoch live so reservations still answer their captured limits.
    #[cfg(test)]
    pub(crate) fn retire_test_writer(&self) {
        if let To::Pool(outbox, _) = &self.to {
            outbox.send(Vec::new(), Some(io_pool::End::Close));
        }
    }
    fn held(&self) -> std::sync::MutexGuard<'_, Vec<Body>> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
    fn reserve(&self, cost: usize) -> bool {
        self.pending
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |n| {
                n.checked_add(cost).filter(|n| *n <= self.cap)
            })
            .is_ok()
    }
    /// Count a packet's bytes as queued. A network connection is refused (false) past its
    /// cap, where the caller closes it. A memory connection counts the bytes but never
    /// refuses: its reader is this process, and a join sends its whole first view in one
    /// burst. `pending` may then exceed [`io_pool::MAX_PENDING`] until it drains, and
    /// `prepare_payload` can answer `Backpressure` meanwhile.
    fn reserve_queued(&self, cost: usize) -> bool {
        if matches!(self.framing, sunrise_protocol::connection::Framing::Memory) {
            self.pending.fetch_add(cost, Ordering::AcqRel);
            return true;
        }
        self.reserve(cost)
    }
    /// Prepare a whole encoded packet without publishing it. The channel is diagnostic only;
    /// its wire identifier must already be included in `body`.
    pub fn prepare_payload(
        &self,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<PreparedPayload, PayloadSendError> {
        let To::Pool(outbox, _) = &self.to else {
            return Err(PayloadSendError::Disconnected);
        };
        if !outbox.alive() {
            return Err(PayloadSendError::Disconnected);
        }
        let bytes = self
            .framing
            .contents_owned(body)
            .map_err(|e| match e.frame_limit() {
                Some(d) => PayloadSendError::FrameTooLarge {
                    id: id.clone(),
                    stage: match d.stage {
                        sunrise_protocol::frame::FrameSize::Encoded => PayloadFrameStage::Encoded,
                        sunrise_protocol::frame::FrameSize::Uncompressed => {
                            PayloadFrameStage::Uncompressed
                        }
                    },
                    size: d.size as u64,
                    limit: d.limit as u64,
                },
                None => PayloadSendError::EncodeFailed(e.to_string()),
            })?;
        // Bookkeeping for PreparedPayload and the transaction's deferred journal record,
        // including geometric journal growth (records are at most 256 bytes). This is
        // retained-memory accounting, not a packet count or application-body cap.
        let cost = io_pool::charge(bytes.capacity()).saturating_add(512);
        if !self.reserve(cost) {
            return Err(PayloadSendError::Backpressure);
        }
        Ok(PreparedPayload {
            sender: self.clone(),
            bytes: Some(bytes),
            cost,
        })
    }
    pub fn send(&self, body: Vec<u8>) {
        self.send_body(body.into());
    }
    pub(super) fn send_body(&self, body: Body) {
        self.queue_body(body, true);
    }
    /// Admit a typed play ping in normal send order, without closing on backpressure.
    /// Admission to a held queue or transaction is not a promise of delivery.
    pub fn try_send_play_ping(&self, id: i32) -> bool {
        self.is_live()
            && play::Clientbound::Ping(cfg::Ping { id })
                .try_encode()
                .is_ok_and(|body| self.queue_body(body.into(), false))
    }
    #[cfg(feature = "test-support")]
    pub fn test_published_packets(&self) -> usize {
        let To::Pool(outbox, _) = &self.to else {
            panic!("receive watermark requires a live connection");
        };
        outbox.test_published()
    }

    #[cfg(feature = "test-support")]
    pub fn test_framed_through(&self, published: usize) -> Option<i64> {
        let To::Pool(outbox, _) = &self.to else {
            panic!("receive watermark requires a live connection");
        };
        outbox.test_framed_through(published)
    }

    fn queue_body(&self, body: Body, close_on_full: bool) -> bool {
        if matches!(self.to, To::Disabled) {
            return false;
        }
        let cost = io_pool::charge(body.retained());
        let body = match transaction_packets::capture(self, body, cost) {
            Ok(admitted) => return admitted,
            Err(body) => body,
        };
        if !self.reserve_queued(cost) {
            if close_on_full {
                self.close();
            }
            return false;
        }
        self.publish_reserved(body)
    }
    fn publish_reserved(&self, body: Body) -> bool {
        if HOLDING.get() {
            self.held().push(body);
            true
        } else {
            self.flush();
            match &self.to {
                To::Disabled => false,
                To::Channel(tx) => {
                    let cost = io_pool::body_charge(&body);
                    if tx.send(Out::Packet(body.into_vec())).is_err() {
                        self.pending.fetch_sub(cost, Ordering::Relaxed);
                        return false;
                    }
                    true
                }
                To::Pool(outbox, _) => outbox.try_send(vec![body], None),
            }
        }
    }
    /// Packets queued for the network thread or held by the tick, and their bytes.
    pub fn queued(&self) -> (usize, usize) {
        if matches!(self.to, To::Disabled) {
            return (0, 0);
        }
        let waiting = match &self.to {
            To::Pool(outbox, _) => outbox.queued(),
            To::Channel(_) | To::Disabled => 0,
        };
        (
            waiting + self.held().len(),
            self.pending.load(Ordering::Relaxed),
        )
    }
    /// Several packets in order, as [`Self::send`] each: under one lock
    /// while the tick holds them. Shared bodies stay shared up to the wire.
    pub fn send_many<B: Into<Body>>(&self, bodies: impl IntoIterator<Item = B>) {
        if matches!(self.to, To::Disabled) {
            return;
        }
        if !HOLDING.get() || transaction_packets::active() {
            bodies
                .into_iter()
                .for_each(|body| self.send_body(body.into()));
            return;
        }
        let mut held = self.held();
        for body in bodies {
            let body = body.into();
            if !self.reserve_queued(io_pool::charge(body.retained())) {
                self.close();
                return;
            }
            held.push(body);
        }
    }
    /// Queue a bundle (including both delimiters) without interleaving output.
    pub(super) fn send_batch<B: Into<Body>>(&self, bodies: impl IntoIterator<Item = B>) {
        if matches!(self.to, To::Disabled) {
            return;
        }
        let bodies = bodies.into_iter().map(Into::into).collect::<Vec<_>>();
        if bodies.is_empty() {
            return;
        }
        let cost = bodies
            .iter()
            .map(|body| io_pool::charge(body.retained()))
            .sum();
        let Err(bodies) = transaction_packets::capture(self, bodies, cost) else {
            return;
        };
        if !self.reserve_queued(cost) {
            self.close();
            return;
        }
        self.publish_reserved_many(bodies);
    }

    fn publish_reserved_many(&self, bodies: Vec<Body>) {
        if bodies.is_empty() {
            return;
        }
        if HOLDING.get() {
            self.held().extend(bodies);
        } else {
            self.flush();
            match &self.to {
                To::Disabled => {}
                To::Channel(tx) => drop(tx.send(unshared(bodies))),
                To::Pool(outbox, _) => outbox.send(bodies, None),
            }
        }
    }

    /// Tracking output is cut only at complete bundles or individual non-bundled
    /// packets. In particular, a late pairing is a single publication even when
    /// the tick no longer holds output: reliable publishers cannot split it.
    pub(super) fn send_tracking_bodies(&self, bodies: &mut Vec<Body>) {
        if matches!(self.to, To::Disabled) {
            bodies.clear();
            return;
        }
        if bodies.is_empty() {
            return;
        }
        let prepared = bodies.iter().any(Body::is_prepared);
        let cost = bodies.iter().map(io_pool::body_charge).sum();
        if HOLDING.get() && !prepared {
            let mut held = self.held();
            if self.reserve_queued(cost) {
                held.append(bodies);
            } else {
                drop(held);
                bodies.clear();
                self.close();
            }
            return;
        }
        self.flush();
        let mut bodies = std::mem::take(bodies);
        let To::Pool(outbox, _) = &self.to else {
            if let To::Channel(tx) = &self.to {
                drop(tx.send(unshared(bodies)));
            }
            return;
        };
        if !self.reserve_queued(cost) {
            self.close();
            return;
        }
        // A pass can be deferred until tracking callbacks. Check the current epoch
        // at publication, not only when the attachment bytes were prepared.
        if prepared {
            if let Some(epoch) = self.payload_transport.epoch() {
                bodies.retain(|body| body.pairing_epoch().is_none_or(|e| Arc::ptr_eq(e, &epoch)));
                let published =
                    epoch.with_active(|| outbox.send(std::mem::take(&mut bodies), None));
                if published.is_some() {
                    return;
                }
            }
            // Expiring mod attachments never suppress vanilla pairing packets.
            bodies.retain(|body| !body.is_prepared());
        }
        outbox.send(bodies, None);
    }
    /// Hand the held packets to the network thread.
    pub fn flush(&self) {
        let held = std::mem::take(&mut *self.held());
        if !held.is_empty() {
            match &self.to {
                To::Disabled => {}
                To::Channel(tx) => drop(tx.send(unshared(held))),
                To::Pool(outbox, _) => outbox.send(held, None),
            }
        }
    }
    /// Send a play disconnect with this reason after what was sent before,
    /// then close.
    pub fn disconnect(&self, reason: sunrise_core::nbt::Tag) {
        self.payload_transport.close();
        self.flush();
        match &self.to {
            To::Disabled => {}
            To::Channel(tx) => drop(tx.send(Out::Disconnect(reason))),
            To::Pool(outbox, _) => {
                let body = play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode();
                let body = Body::from(body);
                if !self.reserve_queued(io_pool::charge(body.retained())) {
                    self.close();
                    return;
                }
                outbox.send(vec![body], Some(io_pool::End::Disconnect));
            }
        }
    }
}

/// Held bodies as a test's channel gives them: each its own bytes.
fn unshared(held: Vec<Body>) -> Out {
    Out::Packets(held.into_iter().map(Body::into_vec).collect())
}

/// A transaction-owned packet. Dropping it cancels its retained-byte reservation.
pub struct PreparedPayload {
    sender: PlayerSender,
    bytes: Option<Vec<u8>>,
    cost: usize,
}
impl PreparedPayload {
    /// Publish exactly the bytes prepared earlier, after the transaction commits.
    pub fn publish(mut self) -> Result<(), PayloadSendError> {
        self.sender.flush();
        let bytes = self.bytes.take().expect("prepared bytes");
        let To::Pool(outbox, _) = &self.sender.to else {
            unreachable!()
        };
        if outbox.publish(bytes, self.cost) {
            Ok(())
        } else {
            Err(PayloadSendError::Disconnected)
        }
    }
}
impl crate::raw_payloads::PreparedSend for PreparedPayload {
    fn publish(self: Box<Self>) -> Result<(), PayloadSendError> {
        (*self).publish()
    }
}
impl crate::raw_payloads::HostedPreparedSend for PreparedPayload {
    fn into_pairing(
        self: Box<Self>,
        epoch: Arc<crate::raw_payloads::connection::Epoch>,
    ) -> Arc<crate::raw_payloads::PairingPayload> {
        let mut this = *self;
        let bytes = this.bytes.take().expect("prepared bytes");
        Arc::new(crate::raw_payloads::PairingPayload {
            bytes,
            epoch,
            pending: this.sender.pending.clone(),
            cost: this.cost,
        })
    }
}
/// One captured player outbox: the reservation keeps its prepaid body credit.
struct SenderAdmission {
    sender: PlayerSender,
    framing: sunrise_protocol::connection::Framing,
    capacity: usize,
}
impl crate::raw_payloads::Admission for SenderAdmission {
    fn framing(&self) -> sunrise_protocol::connection::Framing {
        self.framing
    }
    fn capacity(&self) -> usize {
        self.capacity
    }
    fn accepts_packets(&self) -> bool {
        true
    }
    fn prepare(
        self: Box<Self>,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::PreparedSend>, PayloadSendError> {
        self.sender
            .prepare_payload(body, id)
            .map(|prepared| Box::new(prepared) as Box<dyn crate::raw_payloads::PreparedSend>)
    }
}
impl crate::raw_payloads::HostedAdmission for SenderAdmission {
    fn prepare_hosted(
        self: Box<Self>,
        body: Vec<u8>,
        id: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::HostedPreparedSend>, PayloadSendError> {
        self.sender
            .prepare_payload(body, id)
            .map(|prepared| Box::new(prepared) as Box<dyn crate::raw_payloads::HostedPreparedSend>)
    }
}
impl crate::raw_payloads::HostedOutput for PlayerSender {
    fn admit_hosted(
        &self,
        _: sunrise_protocol::payload_negotiation::Phase,
        _: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::HostedAdmission>, PayloadSendError> {
        Ok(Box::new(SenderAdmission {
            sender: self.clone(),
            framing: self.payload_framing(),
            capacity: self.payload_capacity(),
        }))
    }
}
impl crate::raw_payloads::Output for PlayerSender {
    fn transport(&self) -> &Arc<Transport> {
        self.payload_transport()
    }
    fn side(&self) -> Side {
        Side::Server
    }
    fn admit(
        &self,
        _: sunrise_protocol::payload_negotiation::Phase,
        _: &PayloadId,
    ) -> Result<Box<dyn crate::raw_payloads::Admission>, PayloadSendError> {
        Ok(Box::new(SenderAdmission {
            sender: self.clone(),
            framing: self.payload_framing(),
            capacity: self.payload_capacity(),
        }))
    }
    fn disconnect(&self, reason: &crate::text::Text) {
        PlayerSender::disconnect(self, super::data::component_nbt(&reason.component()));
    }
    #[cfg(any(test, feature = "test-payload-probe"))]
    fn diagnostics(&self) -> Box<dyn Fn() -> String + Send + Sync> {
        self.payload_diagnostic_observer()
    }
}
impl Drop for PreparedPayload {
    fn drop(&mut self) {
        if self.bytes.is_some() {
            self.sender.pending.fetch_sub(self.cost, Ordering::Relaxed);
        }
    }
}

/// Hand every player's held packets to their network threads, waking each
/// thread once: a wake-up costs a system call, and on a busy machine often
/// the rest of the core's time slice. Each player's packets keep their order,
/// also with what is sent to it next.
pub fn flush_all<'a>(senders: impl IntoIterator<Item = &'a PlayerSender>) {
    let mut wake: Vec<&Arc<io_pool::Worker>> = Vec::new();
    for s in senders {
        let held = std::mem::take(&mut *s.held());
        if held.is_empty() {
            continue;
        }
        match &s.to {
            To::Disabled => {}
            To::Channel(tx) => drop(tx.send(unshared(held))),
            To::Pool(outbox, _) => {
                if let Some(worker) = outbox.push(held, None)
                    && !wake.iter().any(|w| Arc::ptr_eq(w, worker))
                {
                    wake.push(worker);
                }
            }
        }
    }
    wake.into_iter().for_each(|w| w.wake());
}

/// LegacyQueryHandler: the pre-1.7 server list ping. What the first read
/// holds decides the reply (1.3 and older: no more bytes; 1.4 and 1.5: 0x01;
/// 1.6: 0x01 then the MC|PingHost payload); anything else gets no reply.
fn serve_legacy(mut stream: TcpStream, shared: &Shared) -> Result<(), String> {
    use std::io::{Read, Write};
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut data = Vec::new();
    let mut chunk = [0u8; 1024];
    // The bytes of the first read: what arrived together.
    if let Ok(n) = stream.read(&mut chunk) {
        data.extend_from_slice(&chunk[..n]);
    }
    let in_string = |d: &[u8], at: &mut usize| -> Option<String> {
        let len = usize::from(u16::from_be_bytes([*d.get(*at)?, *d.get(*at + 1)?]));
        let bytes = d.get(*at + 2..*at + 2 + len * 2)?;
        *at += 2 + len * 2;
        Some(String::from_utf16_lossy(
            &bytes
                .chunks(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect::<Vec<_>>(),
        ))
    };
    let motd = shared.motd();
    let (online, max) = (shared.online(), shared.max_players());
    let body = if data.len() == 1 {
        format!("{motd}\u{a7}{online}\u{a7}{max}")
    } else {
        if data.get(1) != Some(&1) {
            return Ok(());
        }
        if data.len() > 2 {
            // readCustomPayloadPacket.
            let mut at = 3;
            let valid = data[2] == 0xFA
                && in_string(&data, &mut at).as_deref() == Some("MC|PingHost")
                && data
                    .get(at..at + 2)
                    .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                    == Some(data.len().saturating_sub(at + 2))
                && {
                    at += 2;
                    data.get(at).is_some_and(|v| *v >= 73)
                }
                && {
                    at += 1;
                    in_string(&data, &mut at).is_some()
                }
                && data
                    .get(at..at + 4)
                    .is_some_and(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]) <= 65535);
            if !valid {
                return Ok(());
            }
        }
        format!(
            "\u{a7}1\0127\0{}\0{motd}\0{online}\0{max}",
            sunrise_core::VANILLA_VERSION
        )
    };
    let units: Vec<u16> = body.encode_utf16().collect();
    let mut out = vec![0xFF];
    out.extend_from_slice(&(units.len() as u16).to_be_bytes());
    for u in units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    stream.write_all(&out).map_err(|e| e.to_string())?;
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Serve an accepted connection, which holds `counted`'s place among the
/// server's until it closes.
pub(super) fn serve(
    stream: TcpStream,
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
    counted: super::Counted,
) -> Result<(), String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    // LegacyQueryHandler, ahead of the frame decoder while status is on.
    let mut first = [0u8; 1];
    if shared.enable_status() && stream.peek(&mut first).is_ok_and(|n| n == 1) && first[0] == 0xFE {
        return serve_legacy(stream, &shared);
    }
    // A peer that stops reading (a frozen game, a dropped network) would otherwise hold
    // its writer in a full send buffer for good, or until TCP gives up minutes later, and
    // its thread and its place among the server's connections with it.
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let c = Connection::from_stream(stream).map_err(e)?;
    serve_connection(c, address, shared, Some(counted))
}

/// How long one write may wait on a peer that reads nothing: as long as a read may wait.
const WRITE_TIMEOUT: Duration = READ_TIMEOUT;

/// The address a memory connection logs in from: no network peer has port 0.
pub const MEMORY_ADDRESS: std::net::SocketAddr = std::net::SocketAddr::V4(
    std::net::SocketAddrV4::new(std::net::Ipv4Addr::LOCALHOST, 0),
);

/// `ServerConnectionListener.startMemoryChannel`: an integrated server's
/// own player, over one end of an in-memory pipe.
pub fn serve_local(
    pipe: sunrise_protocol::connection::Pipe,
    shared: Arc<Shared>,
) -> Result<(), String> {
    serve_connection(Connection::local(pipe), MEMORY_ADDRESS, shared, None)
}

fn serve_connection(
    mut c: Connection,
    address: std::net::SocketAddr,
    shared: Arc<Shared>,
    counted: Option<super::Counted>,
) -> Result<(), String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    c.reader.set_read_timeout(Some(LOGIN_TIMEOUT)).map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let intention = match handshake::Serverbound::decode(id, &payload).map_err(e)? {
        handshake::Serverbound::Intention(i) => i,
        _ => return Err("expected a handshake".into()),
    };
    match intention.intent {
        // ServerHandshakePacketListenerImpl: a server that does not reply
        // to status requests closes the connection.
        handshake::STATUS if !shared.enable_status() => Ok(()),
        handshake::STATUS => serve_status(&mut c, &shared),
        handshake::TRANSFER if !shared.accepts_transfers() => {
            login_disconnect(
                &mut c,
                &translatable("multiplayer.disconnect.transfers_disabled", &[]),
            );
            Ok(())
        }
        handshake::LOGIN | handshake::TRANSFER => {
            if intention.protocol != PROTOCOL_VERSION {
                let key = if intention.protocol < 754 {
                    "multiplayer.disconnect.outdated_client"
                } else {
                    "multiplayer.disconnect.incompatible"
                };
                let json = format!(
                    "{{\"translate\":\"{key}\",\"with\":[\"{}\"]}}",
                    sunrise_protocol::GAME_VERSION
                );
                let _ = c.send(
                    &login::Clientbound::Disconnect(login::Disconnect { reason_json: json })
                        .encode(),
                );
                return Ok(());
            }
            let handed_off = shared.handoffs.is_some() && shared.handshake_listeners.any();
            let intention = if handed_off {
                let id = counted.as_ref().map_or_else(
                    || shared.next_connection.fetch_add(1, Ordering::Relaxed),
                    |counted| counted.2,
                );
                let peer_ip = (!c.is_memory()).then_some(address.ip());
                let Some(decision) = shared.hand_off(id, intention, peer_ip) else {
                    return Ok(());
                };
                // A server draining its players for a stop keeps listening: `try_admit` refuses
                // an accepted login there as it refuses one no listener heard.
                if shared.stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                match decision {
                    Ok(accepted) => accepted,
                    Err(reason_json) => {
                        let _ = c.send(
                            &login::Clientbound::Disconnect(login::Disconnect { reason_json })
                                .encode(),
                        );
                        return Ok(());
                    }
                }
            } else {
                intention
            };
            serve_login(c, intention, handed_off, address, &shared, counted)
        }
        other => Err(format!("unknown intent {other}")),
    }
}

/// Keep the destination of an accepted login or transfer, without proxy or loader data: the
/// host before its first NUL, empty or not, and the port, as the handshake sent them. A memory
/// connection's handshake names its address and port 0, as `Minecraft.doWorldLoad`'s does.
fn login_virtual_host(intention: handshake::Intention) -> VirtualHost {
    let handshake::Intention { host, port, .. } = intention;
    let host = match host.split_once('\0') {
        Some((prefix, _)) => prefix.to_owned(),
        None => host,
    };
    VirtualHost { host, port }
}

fn serve_status(c: &mut Connection, shared: &Shared) -> Result<(), String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    let mut requested = false;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            // ServerStatusPacketListenerImpl: a second request disconnects.
            status::Serverbound::StatusRequest(_) if requested => return Ok(()),
            status::Serverbound::StatusRequest(_) => {
                requested = true;
                let json = shared.status_json();
                c.send(
                    &status::Clientbound::StatusResponse(status::StatusResponse { json }).encode(),
                )
                .map_err(e)?;
            }
            status::Serverbound::PingRequest(p) => {
                c.send(&status::Clientbound::PongResponse(p).encode())
                    .map_err(e)?;
                return Ok(());
            }
            status::Serverbound::Unknown(..) => return Ok(()),
        }
    }
}

/// ServerLoginPacketListenerImpl.getUserName: the name and the address
/// (Connection.getLoggableAddress, `/ip:port` as InetSocketAddress prints it).
fn user_name(name: &str, address: std::net::SocketAddr, log: &super::NetLog) -> String {
    if log.log_ips {
        format!("{name} (/{address})")
    } else {
        format!("{name} (<ip address withheld>)")
    }
}

/// ServerLoginPacketListenerImpl.disconnect: log the reason's text and send
/// it. Returns the text, for the "lost connection" line that follows.
fn refuse(
    c: &mut Connection,
    shared: &Shared,
    thread: &str,
    user: &str,
    reason: &sunrise_core::nbt::Tag,
) -> String {
    let text = shared
        .config
        .log
        .as_ref()
        .map(|log| sunrise_protocol::text::plain(reason, &log.lang))
        .unwrap_or_default();
    if let Some(log) = &shared.config.log {
        (log.write)(thread, "INFO", &format!("Disconnecting {user}: {text}"));
    }
    login_disconnect(c, reason);
    text
}

/// ServerLoginPacketListenerImpl.onDisconnect, once the server thread sees
/// the connection closed: by a refusal, or by the client ("Disconnected").
fn lost(shared: &Shared, user: &str, text: &str) {
    if let Some(log) = &shared.config.log {
        (log.write)(
            "Server thread",
            "INFO",
            &format!("{user} lost connection: {text}"),
        );
    }
}

/// UNIQUE_THREAD_ID: the User Authenticator threads are numbered from 1.
static AUTHENTICATORS: AtomicU32 = AtomicU32::new(0);

/// ServerLoginPacketListenerImpl.handleKey and the User Authenticator
/// thread: check the client's answer to the encryption request, encrypt the
/// connection, and ask the session service whether the player joined with
/// this server's hash. The profile, or the text of the refusal.
fn authenticate(
    c: &mut Connection,
    shared: &Shared,
    online: &super::OnlineMode,
    name: &str,
    address: std::net::SocketAddr,
    user: &str,
) -> Result<Result<(u128, Vec<sunrise_core::profile::AccountProperty>), String>, String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    let mut challenge = [0u8; 4];
    sunrise_protocol::crypto::random_bytes(&mut challenge).map_err(e)?;
    c.send(
        &login::Clientbound::Hello(login::Hello {
            server_id: String::new(),
            public_key: online.public_der.clone(),
            challenge: challenge.to_vec(),
            should_authenticate: true,
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    let key = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Key(k) => k,
        _ => return Err("Unexpected packet during login".into()),
    };
    // ServerboundKeyPacket.isChallengeValid and getSecretKey.
    let protocol_error = || "Protocol error".to_owned();
    if online.key.decrypt(&key.challenge).ok().as_deref() != Some(&challenge[..]) {
        return Err(protocol_error());
    }
    let secret: [u8; 16] = online
        .key
        .decrypt(&key.key)
        .ok()
        .and_then(|s| s.try_into().ok())
        .ok_or_else(protocol_error)?;
    let digest = server_hash("", &secret, &online.public_der);
    c.enable_encryption(&secret);
    let thread = format!(
        "User Authenticator #{}",
        AUTHENTICATORS.fetch_add(1, Ordering::Relaxed) + 1
    );
    let log = |level: &str, line: &str| {
        if let Some(log) = &shared.config.log {
            (log.write)(&thread, level, line);
        }
    };
    let ip = online.prevent_proxy.then(|| address.ip());
    match (online.verify)(name, &digest, ip) {
        Ok(Some(profile)) => {
            log(
                "INFO",
                &format!("UUID of player {name} is {}", uuid_string(profile.uuid)),
            );
            Ok(Ok((profile.uuid, profile.properties)))
        }
        Ok(None) => {
            let text = refuse(
                c,
                shared,
                &thread,
                user,
                &translatable("multiplayer.disconnect.unverified_username", &[]),
            );
            log(
                "ERROR",
                &format!("Username '{name}' tried to join with an invalid session"),
            );
            Ok(Err(text))
        }
        Err(_) => {
            let text = refuse(
                c,
                shared,
                &thread,
                user,
                &translatable("multiplayer.disconnect.authservers_down", &[]),
            );
            log(
                "ERROR",
                "Couldn't verify username because servers are unavailable",
            );
            Ok(Err(text))
        }
    }
}

fn serve_login(
    mut c: Connection,
    // The accepted intention, after any `player_handshake` rewrite, is the player's virtual host.
    intention: handshake::Intention,
    handed_off: bool,
    address: std::net::SocketAddr,
    shared: &Arc<Shared>,
    counted: Option<super::Counted>,
) -> Result<(), String> {
    let virtual_host = Some(login_virtual_host(intention));
    let e = |e: sunrise_protocol::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello = match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::Hello(h) => h,
        _ => return Err("expected a login hello".into()),
    };
    // A peer that left while the server tick decided its handshake may have left a whole
    // Hello behind: its connection is gone before authentication or admission sees it.
    if handed_off && c.reader.peer_closed() {
        return Ok(());
    }
    if !valid_name(&hello.name) {
        // handleHello's validState throws; Connection.exceptionCaught sends the
        // exception as the login disconnect.
        login_disconnect(
            &mut c,
            &translatable(
                "disconnect.genericReason",
                &[
                    "Internal Exception: java.lang.IllegalStateException: Invalid characters in username",
                ],
            ),
        );
        return Ok(());
    }
    let mut name = hello.name.clone();
    let user = shared
        .config
        .log
        .as_ref()
        .map(|log| user_name(&name, address, log))
        .unwrap_or_default();
    // handleHello: online mode authenticates first; offline mode, and the
    // integrated server's own player through memory, derive the profile
    // from the name.
    let online = shared.config.online.as_ref().filter(|_| !c.is_memory());
    // `ServerLoginPacketListenerImpl.handleHello`: the integrated server's own player
    // (the name matches `getSingleplayerProfile`) logs in as that profile — the account's
    // UUID — never as an offline profile made from the name. Other memory logins stay offline.
    // TCP does not get this shortcut; the integrated server reserves the owner's identity.
    let singleplayer = shared
        .config
        .singleplayer_profile
        .as_ref()
        .filter(|(owner_name, _)| c.is_memory() && hello.name.eq_ignore_ascii_case(owner_name))
        .map(|(owner_name, uuid)| (owner_name.clone(), *uuid));
    let (uuid, properties) = match online {
        Some(online) => match authenticate(&mut c, shared, online, &hello.name, address, &user) {
            Ok(Ok(profile)) => profile,
            Ok(Err(refused)) => {
                lost(shared, &user, &refused);
                return Ok(());
            }
            Err(err) => {
                lost(shared, &user, "Disconnected");
                return Err(err);
            }
        },
        None => match singleplayer {
            Some((owner_name, owner_uuid)) => {
                name = owner_name;
                let properties = shared
                    .config
                    .owner_profile
                    .as_ref()
                    .map(|profile| profile.properties().to_vec())
                    .unwrap_or_default();
                (owner_uuid, properties)
            }
            None => (offline_uuid(&hello.name), Vec::new()),
        },
    };
    // verifyLoginAndFinishConnectionSetup: PlayerList.canPlayerLogin, then
    // duplicates and a full server.
    if let Some(reason) = shared.try_admit(address, uuid, &name) {
        let text = refuse(&mut c, shared, "Server thread", &user, &reason);
        lost(shared, &user, &text);
        return Ok(());
    }
    // A player a proxy said would arrive (`player-expect`) uses the expectation up
    // here, as the login starts: one that ran out while the player was still in
    // configuration (a resource pack prompt) would otherwise make it an ordinary
    // login from this server's own file.
    let expected = shared.config.arrivals.as_ref().and_then(|table| {
        table
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_until(uuid, std::time::Instant::now())
    });
    // A login that does not reach play (it failed in configuration) gives the
    // expectation back, so the proxy's next attempt is still expected.
    let put_back = || {
        if let (Some(deadline), Some(table)) = (expected, shared.config.arrivals.as_ref()) {
            table.lock().unwrap_or_else(|e| e.into_inner()).put_back(
                uuid,
                deadline,
                std::time::Instant::now(),
            );
        }
    };
    let result = login_and_configure(&mut c, shared, uuid, &name, &user, properties);
    match result {
        Ok(Some(parts)) => {
            play(
                c,
                shared,
                parts,
                address,
                counted,
                expected.is_some(),
                virtual_host,
            );
            Ok(())
        }
        Ok(None) => {
            put_back();
            shared.release(uuid);
            Ok(())
        }
        Err(err) => {
            put_back();
            shared.release(uuid);
            // ServerConfigurationPacketListenerImpl.onDisconnect.
            let profile = format!("{name} ({})", uuid_string(uuid));
            lost(shared, &profile, "Disconnected");
            Err(err)
        }
    }
}

struct Parts {
    sync: Arc<SyncData>,
    payload_transport: Arc<Transport>,
    uuid: u128,
    name: String,
    properties: Vec<sunrise_core::profile::AccountProperty>,
    info: ClientInformation,
    brand: String,
    datapack_offer: Option<Vec<u8>>,
    configured: Option<super::configuration_tasks::Configured>,
    channels: super::config_payloads::EngineChannels,
}

fn login_and_configure(
    c: &mut Connection,
    shared: &Arc<Shared>,
    uuid: u128,
    name: &str,
    user: &str,
    properties: Vec<sunrise_core::profile::AccountProperty>,
) -> Result<Option<Parts>, String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    // `player_login_profile`: the login's properties, as a mod edits them, go to both login
    // success and the player list.
    let properties =
        super::login_profile::edited(shared.login_profiles.as_ref(), uuid, name, properties);
    // ServerLoginPacketListenerImpl: memory connections are not compressed.
    let threshold = shared.config.compression_threshold;
    if threshold >= 0 && !c.is_memory() {
        c.send(&login::Clientbound::Compression(login::Compression { threshold }).encode())
            .map_err(e)?;
        c.set_compression(threshold);
    }
    c.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid,
                name: name.to_owned(),
                properties: properties.clone(),
            },
            session: uuid,
        })
        .encode(),
    )
    .map_err(e)?;
    let (id, payload) = c.recv().map_err(e)?;
    match login::Serverbound::decode(id, &payload).map_err(e)? {
        login::Serverbound::LoginAcknowledged(_) => {}
        // handleCustomQueryPacket and handleCookieResponse: this server sends no query and no
        // cookie request here, so an answer is unexpected (DISCONNECT_UNEXPECTED_QUERY).
        login::Serverbound::CustomQueryAnswer(_) | login::Serverbound::CookieResponse(_) => {
            let reason = translatable("multiplayer.disconnect.unexpected_query_response", &[]);
            let text = refuse(c, shared, "Server thread", user, &reason);
            lost(shared, user, &text);
            return Ok(None);
        }
        _ => return Err("unexpected packet during login".into()),
    }
    c.reader.set_read_timeout(Some(READ_TIMEOUT)).map_err(e)?;
    // Configuration.
    let payload_transport = Transport::new(
        shared.payload_local.clone(),
        Side::Server,
        crate::lan::PAYLOAD_RECORD_OVERHEAD,
    );
    let mut config_io =
        ConfigIo::new(c, payload_transport.clone(), shared.events.clone()).map_err(e)?;
    let c = &mut config_io;
    let sync_data = shared.sync();
    let sync: &SyncData = &sync_data;
    let send = |c: &mut ConfigIo<'_>, p: cfg::Clientbound| {
        if let cfg::Clientbound::Disconnect(packet) = p {
            c.disconnect(packet.reason);
            return Ok(());
        }
        p.try_encode().and_then(|body| c.send(&body)).map_err(e)
    };
    send(
        c,
        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: {
                let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
                // MinecraftServer.getServerModName: "vanilla" there; a modded
                // server names itself, and Sunrise's says "sunrise", as its client does.
                w.string("sunrise");
                w.data
            },
        }),
    )?;
    // ServerConfigurationPacketListenerImpl.startConfiguration: links, only
    // when there are any (MinecraftServer.serverLinks is empty elsewhere).
    if !shared.config.server_links.is_empty() {
        send(
            c,
            cfg::Clientbound::ServerLinks(cfg::ServerLinks {
                links: shared.config.server_links.clone(),
            }),
        )?;
    }
    send(
        c,
        cfg::Clientbound::UpdateEnabledFeatures(cfg::UpdateEnabledFeatures {
            features: sync.features.clone(),
        }),
    )?;
    let mut payload_negotiation = sunrise_protocol::payload_negotiation::Negotiation::new(
        payload_transport.local.manifest.clone(),
    );
    // Only a mod's declaration opens negotiation before the client is known: with the
    // engine's alone a vanilla client is sent nothing vanilla's server would not send it.
    // A Sunrise client is asked once it names itself, and answers before the barrier at its
    // known-packs reply.
    if payload_transport.local.has_mod_declarations()
        && let Some(control) = payload_negotiation.initial()
    {
        send_payload_control(c, control)?;
    }
    // Sunrise clients announce their list after recognizing our brand, before
    // answering known packs. That reply orders the announcement without a ping.
    let server_mods = sunrise_protocol::mod_list::local();
    let mut client_mods = Vec::new();
    let mut datapack_offer = None;
    let core = KnownPack {
        namespace: CORE_PACK.0.into(),
        id: CORE_PACK.1.into(),
        version: CORE_PACK.2.into(),
    };
    send(
        c,
        cfg::Clientbound::SelectKnownPacks(cfg::SelectKnownPacks {
            packs: vec![core.clone()],
        }),
    )?;
    let mut info = ClientInformation::new(10);
    let mut brand = String::from("vanilla");
    let known_pack = loop {
        let (id, payload) = c.recv().map_err(e)?;
        let Some(packet) = receive_configuration(
            c,
            id,
            &payload,
            &payload_transport,
            &mut payload_negotiation,
        )?
        else {
            continue;
        };
        match packet {
            cfg::Serverbound::ClientInformation(i) => info = i,
            cfg::Serverbound::CustomPayload(p) if p.channel == "minecraft:brand" => {
                brand = sunrise_protocol::buf::Reader::new(&p.data)
                    .string(32767)
                    .unwrap_or(brand);
                if brand == "sunrise" {
                    if let Some(control) = payload_negotiation.require_protocol() {
                        send_payload_control(c, control)?;
                    }
                    send(
                        c,
                        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                            channel: sunrise_protocol::mod_list::CHANNEL.into(),
                            data: sunrise_protocol::mod_list::encode(&server_mods),
                        }),
                    )?;
                }
            }
            cfg::Serverbound::CustomPayload(p)
                if p.channel == sunrise_protocol::mod_list::CHANNEL =>
            {
                match sunrise_protocol::mod_list::decode(&p.data) {
                    Ok(list) => client_mods = list,
                    Err(error) => {
                        let text = mod_list_unreadable(&error);
                        let reason = super::data::component_nbt(&text.component());
                        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
                        lost(
                            shared,
                            &format!("{name} ({})", uuid_string(uuid)),
                            &text.plain(),
                        );
                        return Ok(None);
                    }
                }
            }
            cfg::Serverbound::CustomPayload(p)
                if p.channel == crate::datapack_net::OFFER_CHANNEL =>
            {
                datapack_offer = Some(p.data);
            }
            cfg::Serverbound::SelectKnownPacks(p) => {
                // A Sunrise client's Offer answers this server's Query (sent on its
                // brand) or Offer (sent on its own Query), both after the known
                // packs, so its pack reply may come first: a ping orders the Offer
                // before the pong.
                if brand == "sunrise" && payload_negotiation.accepted().is_none() {
                    await_payload_answer(
                        c,
                        &payload_transport,
                        &mut payload_negotiation,
                        &mut info,
                    )?;
                }
                let admission = payload_negotiation
                    .finish_without_offer()
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        if payload_transport.epoch().is_some() {
                            return Ok(());
                        }
                        payload_transport.accept(payload_negotiation.accepted().unwrap_or_default())
                    });
                if let Err(error) = admission {
                    return Err(payload_disconnect(c, error));
                }
                break p.packs == vec![core.clone()];
            }
            _ => {}
        }
    };
    // A Sunrise client hears that this server runs its mods' quiet commands
    // (sunrise_protocol::quiet); a vanilla one is told nothing.
    if brand == "sunrise" {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: sunrise_protocol::quiet::CHANNEL.into(),
                data: Vec::new(),
            }),
        )?;
        c.ask_viewed_levels().map_err(e)?;
        c.ask_menu_extra_rev().map_err(e)?;
    }
    // Any brand can use the book channel. Clients that don't understand this advertisement
    // ignore it; the server sends no settings or placement data before the play exchange.
    send(
        c,
        cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: sunrise_protocol::custom_book::CHANNEL.into(),
            data: sunrise_protocol::custom_book::CAPABILITY.to_vec(),
        }),
    )?;
    // And how far it samples far terrain for it (crate::lod_net).
    if brand == "sunrise" && shared.config.lod_distance > 0 {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: crate::lod_net::CHANNEL.into(),
                data: crate::lod_net::Offer::new(shared.config.lod_distance).encode(),
            }),
        )?;
    }
    let difference = sunrise_protocol::mod_list::compare(&server_mods, &client_mods);
    if !difference.is_empty() {
        let text = mod_mismatch(&difference);
        let reason = super::data::component_nbt(&text.component());
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        lost(
            shared,
            &format!("{name} ({})", uuid_string(uuid)),
            &text.plain(),
        );
        return Ok(None);
    }
    // Custom component and statistic IDs need Sunrise's registry check.
    // Operators may require Sunrise clients without either extension.
    if brand != "sunrise"
        && (shared.config.sunrise_clients_only || crate::content_identity::requires_sunrise())
    {
        let text = if shared.config.sunrise_clients_only {
            super::SUNRISE_CLIENTS_ONLY_REASON
        } else {
            crate::content_identity::MISMATCH
        };
        let reason = sunrise_core::nbt::Tag::String(sunrise_core::nbt::JavaString::from(text));
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        if let Some(log) = &shared.config.log {
            let line =
                format!("Disconnecting {name}: its client is \"{brand}\", not Sunrise ({text})");
            (log.write)("Server thread", "INFO", &line);
        }
        lost(shared, &format!("{name} ({})", uuid_string(uuid)), text);
        return Ok(None);
    }
    if brand == "sunrise"
        && let Some(hash) = sync.content_identity
    {
        send(
            c,
            cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                channel: crate::content_identity::CHANNEL.into(),
                data: hash.to_vec(),
            }),
        )?;
        send(c, cfg::Clientbound::Ping(cfg::Ping { id: 0 }))?;
        let mut content_match = false;
        loop {
            let (id, payload) = c.recv().map_err(e)?;
            let Some(packet) = receive_configuration(
                c,
                id,
                &payload,
                &payload_transport,
                &mut payload_negotiation,
            )?
            else {
                continue;
            };
            match packet {
                cfg::Serverbound::CustomPayload(p)
                    if p.channel == crate::content_identity::CHANNEL =>
                {
                    content_match = crate::content_identity::decode_payload(&p.data)
                        .map_err(|error| error.to_string())?
                        == Some(hash);
                }
                cfg::Serverbound::Pong(p) if p.id == 0 => break,
                cfg::Serverbound::ClientInformation(i) => info = i,
                _ => {}
            }
        }
        if !content_match {
            let reason = sunrise_core::nbt::Tag::String(sunrise_core::nbt::JavaString::from(
                crate::content_identity::MISMATCH,
            ));
            let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
            lost(
                shared,
                &format!("{name} ({})", uuid_string(uuid)),
                crate::content_identity::MISMATCH,
            );
            return Ok(None);
        }
    }
    // The mods' datapack registries this client has, or the reason it cannot play (#811).
    match crate::datapack_net::server_step(brand == "sunrise", datapack_offer.as_deref()) {
        crate::datapack_net::Step::Nothing => {}
        crate::datapack_net::Step::Send(data) => {
            for data in crate::datapack_net::chunks(&data)? {
                send(
                    c,
                    cfg::Clientbound::CustomPayload(cfg::CustomPayload {
                        channel: crate::datapack_net::CHANNEL.into(),
                        data,
                    }),
                )?;
            }
        }
        crate::datapack_net::Step::Refuse(text) => {
            let reason =
                sunrise_core::nbt::Tag::String(sunrise_core::nbt::JavaString::from(text.as_str()));
            let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
            lost(shared, &format!("{name} ({})", uuid_string(uuid)), &text);
            return Ok(None);
        }
    }
    for packet in sync.registry_packets(known_pack, &shared.config.levels) {
        send(c, cfg::Clientbound::RegistryData(packet))?;
    }
    send(c, cfg::Clientbound::UpdateTags(sync.tags.clone()))?;
    // addOptionalTasks: the code of conduct task comes first.
    if let Some(text) =
        super::code_of_conduct::pick(&shared.config.code_of_conducts, &info.language)
    {
        accept_code_of_conduct(
            c,
            text,
            &mut info,
            &payload_transport,
            &mut payload_negotiation,
        )?;
    }
    if let Some(pack) = &shared.config.resource_pack
        && let Some(rejected) = offer_pack(
            c,
            pack,
            &mut info,
            &payload_transport,
            &mut payload_negotiation,
        )?
    {
        let (reason, text) = pack_rejected(shared, name, rejected);
        let _ = send(c, cfg::Clientbound::Disconnect(cfg::Disconnect { reason }));
        lost(shared, &format!("{name} ({})", uuid_string(uuid)), &text);
        return Ok(None);
    }
    let configured = configuration_tasks::configure(
        c,
        shared,
        uuid,
        name,
        &mut info,
        &payload_transport,
        &mut payload_negotiation,
    )?;
    if let Some(epoch) = payload_transport.epoch()
        && !epoch.wait_main()
    {
        return Err("configuration payload epoch ended".into());
    }
    send(c, cfg::Clientbound::FinishConfiguration(Empty))?;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        let Some(packet) = receive_configuration(
            c,
            id,
            &payload,
            &payload_transport,
            &mut payload_negotiation,
        )?
        else {
            continue;
        };
        match packet {
            cfg::Serverbound::FinishConfiguration(_) => break,
            cfg::Serverbound::ClientInformation(i) => info = i,
            _ => {}
        }
    }
    payload_transport.finish_configuration()?;
    let channels = std::mem::take(&mut c.channels);
    c.finish()?;
    Ok(Some(Parts {
        sync: sync_data,
        channels,
        payload_transport,
        uuid,
        name: name.to_owned(),
        properties,
        info,
        brand,
        datapack_offer,
        configured,
    }))
}

fn send_payload_control(
    c: &mut ConfigIo<'_>,
    control: sunrise_protocol::payload_negotiation::Control,
) -> Result<(), String> {
    c.send(
        &cfg::Clientbound::CustomPayload(cfg::CustomPayload {
            channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
            data: control.encode().map_err(|e| e.to_string())?,
        })
        .encode(),
    )
    .map_err(|e| e.to_string())
}

/// The configuration ping that orders a Sunrise client's payload Offer before
/// its pong (the content identity check pings with 0 later).
const PAYLOAD_ANSWER_PING: i32 = 1;

/// Read on until the client's Offer froze the negotiation, or its pong came: a client
/// reads in order, so an answer to what it was sent on `sunrise:payloads` comes before
/// the pong, and a pong without one is a client that does not negotiate. A pong after
/// the Offer is read later with the configuration's other packets.
fn await_payload_answer(
    c: &mut ConfigIo<'_>,
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
    info: &mut ClientInformation,
) -> Result<(), String> {
    c.send(
        &cfg::Clientbound::Ping(cfg::Ping {
            id: PAYLOAD_ANSWER_PING,
        })
        .encode(),
    )
    .map_err(|e| e.to_string())?;
    while negotiation.accepted().is_none() {
        let (id, payload) = c.recv().map_err(|e| e.to_string())?;
        match receive_configuration(c, id, &payload, transport, negotiation)? {
            Some(cfg::Serverbound::Pong(p)) if p.id == PAYLOAD_ANSWER_PING => break,
            Some(cfg::Serverbound::ClientInformation(i)) => *info = i,
            _ => {}
        }
    }
    Ok(())
}
fn payload_disconnect(c: &mut ConfigIo<'_>, error: String) -> String {
    let reason = sunrise_core::nbt::Tag::String(error.as_str().into());
    c.disconnect(reason);
    error
}

/// Consume transport metadata in every configuration receive loop. Application
/// payloads stay ordered for the configuration dispatcher (not implemented here).
fn receive_configuration(
    c: &mut ConfigIo<'_>,
    id: i32,
    bytes: &[u8],
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
) -> Result<Option<cfg::Serverbound>, String> {
    let packet = cfg::Serverbound::decode_with_payload_codecs(id, bytes, &transport.local.codecs)
        .map_err(|e| payload_disconnect(c, e.to_string()))?;
    if let cfg::Serverbound::CustomPayload(payload) = &packet
        && payload.channel == sunrise_protocol::payload_negotiation::CHANNEL
    {
        let result = sunrise_protocol::payload_negotiation::Control::decode(&payload.data)
            .and_then(|control| negotiation.receive(control));
        let reply = result.map_err(|e| payload_disconnect(c, e.to_string()))?;
        if let Some(reply) = reply {
            send_payload_control(c, reply)?;
        }
        if let Some(accepted) = negotiation.accepted() {
            transport
                .accept(accepted)
                .map_err(|e| payload_disconnect(c, e))?;
        }
        return Ok(None);
    }
    match packet {
        cfg::Serverbound::CustomPayload(payload) => c
            .application(payload)
            .map(|payload| payload.map(cfg::Serverbound::CustomPayload)),
        packet => Ok(Some(packet)),
    }
}

/// ServerboundResourcePackPacket.Action.DECLINED.
const PACK_DECLINED: i32 = 1;

fn mod_mismatch(difference: &sunrise_protocol::mod_list::Difference) -> crate::text::Text {
    use crate::{lang::Language, text::Text};
    // Vanilla has no Sunrise keys. Keep English fallbacks on the wire, while a
    // Sunrise client resolves each component in its own selected language.
    let english = Language::load(&[], crate::lang::DEFAULT);
    let translated = |key: &str, args: &[Text]| {
        Text::translate_with_fallback(key, english.get(key).unwrap(), args)
    };
    let mut text = translated("sunrise.mods.mismatch", &[]);
    for (key, names) in [
        ("sunrise.mods.missing", &difference.missing),
        ("sunrise.mods.extra", &difference.extra),
    ] {
        if !names.is_empty() {
            text = text
                .append("\n")
                .append(translated(key, &[Text::literal(names.join("\n"))]));
        }
    }
    for id in &difference.runtime {
        text = text
            .append("\n")
            .append(translated("sunrise.mods.runtime", &[Text::literal(id)]));
    }
    for changed in &difference.changed {
        text = text.append("\n").append(translated(
            "sunrise.mods.version",
            &[
                Text::literal(&changed.id),
                Text::literal(&changed.server),
                Text::literal(&changed.client),
            ],
        ));
    }
    text
}

/// A mod list that breaks a cap is refused whole, never compared in part or
/// read as empty: like a mismatch, the reason reaches the client as a
/// configuration disconnect naming the decode error, and the join never
/// reaches play.
fn mod_list_unreadable(error: &sunrise_protocol::Error) -> crate::text::Text {
    use crate::{lang::Language, text::Text};
    let english = Language::load(&[], crate::lang::DEFAULT);
    Text::translate_with_fallback(
        "sunrise.mods.unreadable",
        english.get("sunrise.mods.unreadable").unwrap(),
        &[Text::literal(error.to_string())],
    )
}

/// ServerCommonPacketListenerImpl.handleResourcePackResponse: a player who
/// declined the required pack is disconnected. The reason and its text.
fn pack_rejected(shared: &Shared, name: &str, pack: u128) -> (sunrise_core::nbt::Tag, String) {
    let reason = translatable("multiplayer.requiredTexturePrompt.disconnect", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line = format!(
        "Disconnecting {name} due to resource pack {} rejection",
        uuid_string(pack)
    );
    (log.write)("Server thread", "INFO", &line);
    let text = sunrise_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

/// RateKickingConnection: a player whose packets a second passed rate-limit
/// is disconnected ("disconnect.exceeded_packet_rate") with a WARN, as
/// vanilla's kick logs it.
/// The kick for a client whose packets wait past [`Undrained`]'s caps: the same message as the
/// hub's own inbox check, and a line in the log.
fn undrained_exceeded(shared: &Shared, name: &str) -> (sunrise_core::nbt::Tag, String) {
    let reason = translatable("disconnect.exceeded_packet_rate", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line =
        format!("{name} sent more packets than the server handles before its tick took them");
    (log.write)("Server thread", "WARN", &line);
    let text = sunrise_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

fn rate_exceeded(shared: &Shared, average: f32) -> (sunrise_core::nbt::Tag, String) {
    let reason = translatable("disconnect.exceeded_packet_rate", &[]);
    let Some(log) = &shared.config.log else {
        return (reason, String::new());
    };
    let line = format!(
        "Player exceeded rate-limit (sent {} packets per second)",
        sunrise_core::java_number::float_to_string(average)
    );
    (log.write)("Server thread", "WARN", &line);
    let text = sunrise_protocol::text::plain(&reason, &log.lang);
    (reason, text)
}

/// ServerResourcePackConfigurationTask: offer the server's pack, then wait
/// for the client's final answer (`Action.isTerminal`: anything but
/// ACCEPTED or DOWNLOADED). The player may take a while to answer the
/// prompt, so the connection is kept alive meanwhile, as
/// ServerConfigurationPacketListenerImpl.tick does. The id of a required
/// pack the client declined, which disconnects it.
fn offer_pack(
    c: &mut ConfigIo<'_>,
    pack: &crate::server::properties::ResourcePackInfo,
    info: &mut ClientInformation,
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
) -> Result<Option<u128>, String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    let push = cfg::ResourcePackPush {
        id: pack.id,
        url: pack.url.clone(),
        hash: pack.hash.clone(),
        required: pack.required,
        prompt: pack.prompt.as_ref().map(component_tag),
    };
    c.send(&cfg::Clientbound::ResourcePackPush(push).encode())
        .map_err(e)?;
    wait_alive(c, transport, negotiation, |packet| match packet {
        Ok(cfg::Serverbound::ResourcePack(r)) if !matches!(r.action, 3 | 4) => {
            let rejected = r.action == PACK_DECLINED && pack.required;
            Some(Ok(rejected.then_some(r.id)))
        }
        Ok(cfg::Serverbound::ClientInformation(i)) => {
            *info = i;
            None
        }
        Ok(_) => None,
        Err(err) => Some(Err(err)),
    })
}

/// ServerCodeOfConductConfigurationTask: send the text, then wait for
/// ServerboundAcceptCodeOfConductPacket (`handleAcceptCodeOfConduct`),
/// keeping the connection alive while the player reads. Finishing the
/// configuration first is `finishCurrentTask` of the wrong task type, an
/// error that drops the connection.
fn accept_code_of_conduct(
    c: &mut ConfigIo<'_>,
    text: &str,
    info: &mut ClientInformation,
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
) -> Result<(), String> {
    let e = |e: sunrise_protocol::Error| e.to_string();
    c.send(&cfg::Clientbound::CodeOfConduct(cfg::CodeOfConduct { text: text.into() }).encode())
        .map_err(e)?;
    wait_alive(c, transport, negotiation, |packet| match packet {
        Ok(cfg::Serverbound::AcceptCodeOfConduct(_)) => Some(Ok(())),
        Ok(cfg::Serverbound::FinishConfiguration(_)) => {
            Some(Err("Unexpected configuration task".to_owned()))
        }
        Ok(cfg::Serverbound::ClientInformation(i)) => {
            *info = i;
            None
        }
        Ok(_) => None,
        Err(err) => Some(Err(err)),
    })
}

/// Read configuration packets until `answer` has a result, sending a
/// keep-alive every `KEEP_ALIVE_EVERY` meanwhile
/// (ServerConfigurationPacketListenerImpl.tick).
fn wait_alive<T>(
    c: &mut ConfigIo<'_>,
    transport: &Transport,
    negotiation: &mut sunrise_protocol::payload_negotiation::Negotiation,
    mut answer: impl FnMut(Result<cfg::Serverbound, String>) -> Option<Result<T, String>>,
) -> Result<T, String> {
    if let Some(sender) = c.sender.clone() {
        return std::thread::scope(|scope| {
            let (stop, stopped) = channel::<()>();
            scope.spawn(move || {
                let start = Instant::now();
                while stopped.recv_timeout(KEEP_ALIVE_EVERY) == Err(RecvTimeoutError::Timeout) {
                    let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
                    if sender
                        .send_packet(&cfg::Clientbound::KeepAlive(cfg::KeepAlive { id }).encode())
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let result = loop {
                let (id, payload) = match c.recv() {
                    Ok(packet) => packet,
                    Err(e) => break Err(e.to_string()),
                };
                let packet = match receive_configuration(c, id, &payload, transport, negotiation) {
                    Ok(Some(packet)) => Ok(packet),
                    Ok(None) => continue,
                    Err(e) => Err(e),
                };
                if let Some(result) = answer(packet) {
                    break result;
                }
            };
            drop(stop);
            result
        });
    }
    // The engine's channels live on ConfigIo, which the wait below borrows as a plain connection.
    let mut channels = std::mem::take(&mut c.channels);
    let result = {
        let connection = &mut **c;
        let writer = Mutex::new(&mut connection.writer);
        std::thread::scope(|scope| {
            let (stop, stopped) = channel::<()>();
            let keep_writer = &writer;
            scope.spawn(move || {
                let start = Instant::now();
                while stopped.recv_timeout(KEEP_ALIVE_EVERY) == Err(RecvTimeoutError::Timeout) {
                    let id = i64::try_from(start.elapsed().as_millis()).unwrap_or(0);
                    let keep_alive = cfg::Clientbound::KeepAlive(cfg::KeepAlive { id });
                    if keep_writer
                        .lock()
                        .unwrap()
                        .send(&keep_alive.encode())
                        .is_err()
                    {
                        break;
                    }
                }
            });
            let result = loop {
                let packet = match connection.reader.recv() {
                    Ok((id, payload)) => cfg::Serverbound::decode_with_payload_codecs(
                        id,
                        &payload,
                        &transport.local.codecs,
                    )
                    .map_err(|e| e.to_string()),
                    Err(err) => break Err(err.to_string()),
                };
                if let Ok(cfg::Serverbound::CustomPayload(payload)) = &packet
                    && payload.channel == sunrise_protocol::payload_negotiation::CHANNEL
                {
                    let result = (|| {
                        let control =
                            sunrise_protocol::payload_negotiation::Control::decode(&payload.data)
                                .map_err(|e| e.to_string())?;
                        if let Some(reply) =
                            negotiation.receive(control).map_err(|e| e.to_string())?
                        {
                            let body = cfg::Clientbound::CustomPayload(
                                sunrise_protocol::packets::custom_payload::CustomPayload {
                                    channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
                                    data: reply.encode().map_err(|e| e.to_string())?,
                                },
                            )
                            .encode();
                            writer
                                .lock()
                                .unwrap()
                                .send(&body)
                                .map_err(|e| e.to_string())?;
                        }
                        if let Some(accepted) = negotiation.accepted() {
                            transport.accept(accepted)?;
                        }
                        Ok::<(), String>(())
                    })();
                    if let Err(error) = result {
                        break Err(error);
                    }
                    continue;
                }
                // As receive_configuration does: the engine's channels are read here too.
                if let Ok(cfg::Serverbound::CustomPayload(payload)) = &packet {
                    match channels.receive(payload) {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(error) => break Err(error),
                    }
                }
                if let Some(result) = answer(packet) {
                    break result;
                }
            };
            drop(stop);
            result
        })
    };
    c.channels = channels;
    result.map_err(|error| payload_disconnect(c, error))
}

fn play(
    c: Connection,
    shared: &Arc<Shared>,
    parts: Parts,
    address: std::net::SocketAddr,
    counted: Option<super::Counted>,
    expected: bool,
    virtual_host: Option<VirtualHost>,
) {
    let Some(conn) = shared.next_id() else {
        shared.release(parts.uuid);
        return;
    };
    shared.entered_play();
    let pending = Arc::new(AtomicUsize::new(0));
    let outbox = match io_pool::Outbox::new(pending.clone(), !c.is_memory()) {
        Ok(outbox) => outbox,
        Err(e) => {
            log::warn!("[lan] {e}");
            shared.release(parts.uuid);
            return;
        }
    };
    if let Some(queue) = parts
        .payload_transport
        .connection
        .as_ref()
        .and_then(|connection| connection.droppable())
    {
        outbox.attach_droppable(queue.clone());
    }
    let latency = Arc::new(AtomicI32::new(0));
    let watch = Arc::new(KeepAliveWatch {
        start: Instant::now(),
        alive: Arc::new(AtomicI64::new(-1)),
    });
    let undrained = Arc::<Undrained>::default();
    let sender = PlayerSender {
        undrained: undrained.clone(),
        pending,
        cap: io_pool::MAX_PENDING,
        held: Arc::default(),
        framing: c.writer.framing(),
        to: To::Pool(outbox.clone(), watch.clone()),
        wire: Some(c.reader.counters.clone()),
        payload_transport: parts.payload_transport.clone(),
        virtual_host: virtual_host.map(Arc::new),
        datapack_offer: parts.datapack_offer.map(Into::into),
        announced: parts.channels.announced.clone().unwrap_or_default(),
        viewed_levels: parts.channels.viewed_levels.answer(),
        menu_extra_rev: parts.channels.menu_extra_rev.revision(),
    };
    let required_pack = shared
        .config
        .resource_pack
        .as_ref()
        .is_some_and(|p| p.required);
    // One output for this physical connection: every delivery clones the `Arc`.
    let payload_endpoint = crate::raw_payloads::Endpoint::shared_server(Arc::new(sender.clone()));
    let session = PlayConn {
        sync: parts.sync,
        payload_sender: sender.clone(),
        payload_endpoint,
        payload_transport: parts.payload_transport,
        conn,
        shared: shared.clone(),
        uuid: parts.uuid,
        name: parts.name.clone(),
        required_pack,
        rate: PacketRate::new(shared.config.rate_limit, Instant::now()),
        latency: latency.clone(),
        sent_at: None,
        next_keep_alive: watch.start + KEEP_ALIVE_EVERY,
        watch,
        counted,
        undrained,
    };
    let joined = Joined {
        conn,
        uuid: parts.uuid,
        name: parts.name,
        properties: parts.properties,
        info: parts.info,
        brand: parts.brand,
        sender,
        latency,
        address,
        expected,
    };
    let mut session = Box::new(session);
    if shared
        .events
        .send(NetEvent::Joined(Box::new(joined), parts.configured))
        .is_err()
    {
        session.left("server closed".to_owned());
        session.closed();
        return;
    }
    let (socket, reader, writer) = c.detach();
    io_pool::adopt(socket, (reader, writer), outbox, session, READ_TIMEOUT);
}

/// A player's play state on its network thread.
struct PlayConn {
    sync: Arc<SyncData>,
    payload_sender: PlayerSender,
    /// The one output this physical connection shares with its deliveries.
    payload_endpoint: crate::raw_payloads::Endpoint,
    payload_transport: Arc<Transport>,
    conn: u32,
    shared: Arc<Shared>,
    uuid: u128,
    name: String,
    required_pack: bool,
    rate: Option<PacketRate>,
    watch: Arc<KeepAliveWatch>,
    /// Smoothed keep-alive round trip in milliseconds.
    latency: Arc<AtomicI32>,
    /// The keep-alive waiting for its answer, and when it was sent.
    sent_at: Option<(i64, Instant)>,
    next_keep_alive: Instant,
    /// The connection's place among the server's, given back once closed.
    counted: Option<super::Counted>,
    /// Shared with the player's [`PlayerSender`]: what the hub hasn't taken yet.
    undrained: Arc<Undrained>,
}
impl io_pool::Session for PlayConn {
    /// Forward the player's packet, or say why it ends the connection: a
    /// required resource pack declined, a rate-limit excess, a cookie response
    /// nobody asked for, a malformed packet.
    fn packet(&mut self, id: i32, payload: Vec<u8>) -> io_pool::Verdict {
        use io_pool::Verdict;
        let kick = |(reason, text): (sunrise_core::nbt::Tag, String)| {
            let body = play::Clientbound::Disconnect(cfg::Disconnect { reason }).encode();
            Verdict::Kick(text, body)
        };
        if self.payload_transport.closed() {
            return Verdict::Go;
        }
        let now = Instant::now();
        if let Some(average) = self.rate.as_mut().and_then(|r| r.received(now)) {
            return kick(rate_exceeded(&self.shared, average));
        }
        match play::Serverbound::decode_reader(
            id,
            sunrise_protocol::buf::Reader::with_payload_codecs(
                &payload,
                &self.payload_transport.local.codecs,
            )
            .with_names(&self.sync.dynamic),
        )
        .and_then(checked_play)
        {
            // ServerCommonPacketListenerImpl.handleKeepAlive: only the pending one counts.
            Ok(play::Serverbound::KeepAlive(k)) => {
                if let Some((sent, at)) = self.sent_at
                    && k.id == sent
                {
                    self.watch.alive.store(k.id, Ordering::Relaxed);
                    let ms = i32::try_from(at.elapsed().as_millis()).unwrap_or(i32::MAX);
                    let old = self.latency.load(Ordering::Relaxed);
                    self.latency.store((old * 3 + ms) / 4, Ordering::Relaxed);
                    self.sent_at = None;
                    Verdict::Go
                } else if self.shared.is_singleplayer_owner(&self.name) {
                    Verdict::Go
                } else {
                    kick((
                        translatable("disconnect.timeout", &[]),
                        "Timed out".to_owned(),
                    ))
                }
            }
            // handleUseItemOn and handleUseItem: `ackBlockChangesUpTo` throws on a negative sequence
            // before anything is done, and the server only logs it ("Failed to handle packet ...,
            // suppressing error"): such a packet does nothing, and no player is disconnected for it.
            Ok(
                play::Serverbound::UseItem(play::UseItem { sequence, .. })
                | play::Serverbound::UseItemOn(play::UseItemOn { sequence, .. }),
            ) if sequence < 0 => Verdict::Go,
            Ok(play::Serverbound::ResourcePack(r))
                if r.action == PACK_DECLINED && self.required_pack =>
            {
                kick(pack_rejected(&self.shared, &self.name, r.id))
            }
            // ServerCommonPacketListenerImpl.handleCookieResponse: the server asks for no cookie, so
            // a response is always unexpected, for the singleplayer owner too.
            Ok(play::Serverbound::CookieResponse(_)) => kick((
                translatable("multiplayer.disconnect.unexpected_query_response", &[]),
                "Unexpected custom data from client".to_owned(),
            )),
            Ok(p) => {
                let p = match p {
                    play::Serverbound::CustomPayload(payload) => {
                        match crate::raw_payloads::Incoming::classify(
                            &self.payload_transport,
                            Phase::Play,
                            payload,
                        ) {
                            Ok(crate::raw_payloads::Classified::Application(incoming)) => {
                                crate::raw_payloads::dispatch_server(
                                    incoming,
                                    self.payload_endpoint.clone(),
                                    crate::raw_payloads::ServerQueue::new(
                                        self.shared.events.clone(),
                                        self.payload_endpoint.clone(),
                                        Some(self.conn),
                                    ),
                                );
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Raw(deliveries)) => {
                                for incoming in deliveries {
                                    crate::raw_payloads::dispatch_server(
                                        incoming,
                                        self.payload_endpoint.clone(),
                                        crate::raw_payloads::ServerQueue::new(
                                            self.shared.events.clone(),
                                            self.payload_endpoint.clone(),
                                            Some(self.conn),
                                        ),
                                    );
                                }
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Skipped) => return Verdict::Go,
                            Ok(crate::raw_payloads::Classified::Other(payload))
                                if self
                                    .payload_sender
                                    .announced
                                    .apply(&payload.channel, &payload.data) =>
                            {
                                return Verdict::Go;
                            }
                            Ok(crate::raw_payloads::Classified::Other(payload)) => {
                                play::Serverbound::CustomPayload(payload)
                            }
                            Err(error) => return Verdict::Close(error.to_string()),
                        }
                    }
                    packet => packet,
                };
                // Bounded here, where packets are made, not only where the hub drains them.
                if !self.undrained.handed(payload.len()) {
                    return kick(undrained_exceeded(&self.shared, &self.name));
                }
                let event = NetEvent::Packet(self.conn, Box::new(p), payload.len());
                match self.shared.events.send(event) {
                    Ok(()) => Verdict::Go,
                    Err(_) => Verdict::Close("server closed".to_owned()),
                }
            }
            // A malformed packet disconnects like vanilla's decoder exception.
            Err(e) => Verdict::Close(format!("Invalid packet {id}: {e}")),
        }
    }
    /// ServerCommonPacketListenerImpl.keepConnectionAlive: a keep-alive 15 s
    /// after the last was answered. The game thread times a client out
    /// ([`PlayerSender::timed_out`]).
    fn tick(&mut self, now: Instant) -> Option<Vec<u8>> {
        self.payload_tick(now)
    }
    fn left(&mut self, reason: String) {
        // Publish logout before releasing the profile, even if gameplay filled its queue.
        let _ = self
            .shared
            .events
            .send_terminal(NetEvent::Left(self.conn, reason));
        self.shared.closing.fetch_add(1, Ordering::AcqRel);
    }
    fn closed(&mut self) {
        self.payload_transport.close();
        self.shared.closing.fetch_sub(1, Ordering::AcqRel);
        self.shared.release(self.uuid);
        self.counted = None;
    }
}

/// RateKickingConnection: the packets a second a connection averages, as
/// Connection.tickSecond keeps it, against server.properties' rate-limit.
struct PacketRate {
    limit: f32,
    received: u32,
    average: f32,
    next: Instant,
}
impl PacketRate {
    /// The tracker of a connection, or none when the limit is off (0 or less).
    fn new(limit: i32, now: Instant) -> Option<Self> {
        (limit > 0).then(|| Self {
            limit: limit as f32,
            received: 0,
            average: 0.0,
            next: now + Duration::from_secs(1),
        })
    }
    /// A packet arrived. Every whole second the average updates
    /// (Mth.lerp(0.75F, received, average)): over the limit, Some of it.
    fn received(&mut self, now: Instant) -> Option<f32> {
        let exceeded = if now >= self.next {
            // One tick with the count, then a packetless tick's 0.75 decay
            // for each further whole second already gone.
            let first = self.received as f32 + 0.75 * (self.average - self.received as f32);
            self.average = first;
            self.received = 0;
            let elapsed = (now - self.next).as_secs() + 1;
            if elapsed > 1 {
                self.average *= 0.75f32.powi((elapsed - 1) as i32);
            }
            self.next += Duration::from_secs(elapsed);
            (first > self.limit).then_some(first)
        } else {
            None
        };
        self.received = self.received.saturating_add(1);
        exceeded
    }
}

const COMMAND_SUGGESTION: i32 = sunrise_protocol::ids::play::serverbound::COMMAND_SUGGESTION;

/// A play packet as vanilla's decoders read it.
fn checked_play(packet: play::Serverbound) -> sunrise_protocol::Result<play::Serverbound> {
    match &packet {
        // ServerboundCommandSuggestionPacket: a VarInt id, then the command, at most
        // 32500 chars. The game reads it later; its codec's limits apply here.
        play::Serverbound::Unknown(COMMAND_SUGGESTION, body) => {
            let mut r = sunrise_protocol::buf::Reader::new(body);
            r.varint()?;
            r.string(32500)?;
            Ok(packet)
        }
        _ => Ok(packet),
    }
}

/// Packet bodies at least this large are compressed once for every
/// connection: chunk columns, mostly, which every player near them receives.
const SHARED_FRAME_MIN: usize = 8 << 10;
/// The compressed frames kept, in bytes; the oldest go first.
const SHARED_FRAME_BYTES: usize = 32 << 20;

/// Compressed frame contents by the body they were made from: its length,
/// one 128-bit hash of it keyed by a random secret, and the threshold.
struct SharedFrames {
    /// XXH3's secret, random for each run, so that no body can be made to
    /// share its hash with another.
    secret: Box<[u8]>,
    frames: Mutex<FrameCache>,
    /// [`SHARED_FRAME_BYTES`].
    limit: usize,
}
type FrameKey = (usize, u128, usize);
/// A frame's contents, made once: whoever asks while another thread
/// compresses the same body waits for it rather than compressing it again.
type Frame = Arc<std::sync::OnceLock<Result<Arc<[u8]>, String>>>;
#[derive(Default)]
struct FrameCache {
    frames: crate::fast_hash::FxHashMap<FrameKey, Frame>,
    order: std::collections::VecDeque<FrameKey>,
    /// The bytes of the frames made and still here.
    bytes: usize,
}
impl SharedFrames {
    fn get() -> &'static Self {
        static FRAMES: std::sync::OnceLock<SharedFrames> = std::sync::OnceLock::new();
        FRAMES.get_or_init(|| Self::new(SHARED_FRAME_BYTES))
    }
    fn new(limit: usize) -> Self {
        use std::hash::{BuildHasher, Hasher};
        // 192 bytes, as XXH3's default secret.
        let secret = (0..24)
            .flat_map(|i: usize| {
                let mut h = std::hash::RandomState::new().build_hasher();
                h.write_usize(i);
                h.finish().to_le_bytes()
            })
            .collect();
        Self {
            secret,
            frames: Mutex::default(),
            limit,
        }
    }
    fn cache(&self) -> std::sync::MutexGuard<'_, FrameCache> {
        self.frames.lock().unwrap_or_else(PoisonError::into_inner)
    }
    /// The frames kept and the bytes of their contents.
    fn census(&self) -> (usize, usize) {
        let cache = self.cache();
        (cache.frames.len(), cache.bytes)
    }
    /// `frame::contents(body, Some(threshold))`, compressed only when no
    /// connection has sent the same body lately or compresses it now.
    fn contents(&self, body: &[u8], threshold: usize) -> sunrise_protocol::Result<Arc<[u8]>> {
        let hash = xxhash_rust::xxh3::xxh3_128_with_secret(body, &self.secret);
        let key = (body.len(), hash, threshold);
        let (frame, made_here) = {
            let mut cache = self.cache();
            match cache.frames.get(&key) {
                Some(frame) => (frame.clone(), false),
                None => {
                    let frame = Frame::default();
                    cache.frames.insert(key, frame.clone());
                    cache.order.push_back(key);
                    (frame, true)
                }
            }
        };
        let contents = frame
            .get_or_init(|| {
                sunrise_protocol::frame::contents(body, Some(threshold))
                    .map(Arc::from)
                    .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(sunrise_protocol::Error::new)?;
        if made_here {
            let mut cache = self.cache();
            // Unless pushed out while it was being made.
            if cache
                .frames
                .get(&key)
                .is_some_and(|f| Arc::ptr_eq(f, &frame))
            {
                cache.bytes += contents.len();
            }
            while cache.bytes > self.limit
                && let Some(old) = cache.order.pop_front()
            {
                let made = cache.frames.remove(&old);
                cache.bytes -= made
                    .and_then(|f| f.get()?.as_ref().ok().map(|c| c.len()))
                    .unwrap_or(0);
            }
        }
        Ok(contents)
    }
}

/// The compressed frames shared by every connection: how many and their bytes.
pub fn shared_frame_census() -> (usize, usize) {
    SharedFrames::get().census()
}

/// Frame one packet body into `out`: a large one compressed through
/// [`SharedFrames`], so a chunk column sent to fifty players is compressed
/// once, not fifty times. The bytes on the wire are the same.
pub(super) fn frame_body(
    writer: &mut ConnWriter,
    body: &[u8],
    out: &mut Vec<u8>,
) -> sunrise_protocol::Result<()> {
    match writer.framing() {
        sunrise_protocol::connection::Framing::Network(Some(t))
            if body.len() >= t.max(SHARED_FRAME_MIN) =>
        {
            writer.frame_into(&SharedFrames::get().contents(body, t)?, out);
        }
        _ => writer.frame_into(&writer.contents(body)?, out),
    }
    Ok(())
}

/// The keep-alive clock of a play connection, shared by its network thread
/// (which sends the keep-alives) and the game thread (which watches them).
struct KeepAliveWatch {
    /// What keep-alive ids count from: an id is its sending time in ms.
    start: Instant,
    /// The id of the last keep-alive the client answered (-1: none yet).
    alive: Arc<AtomicI64>,
}
impl KeepAliveWatch {
    /// keepConnectionAlive sends a keep-alive 15 s after the last and drops
    /// a client that has not answered it 15 s later: nothing answered of
    /// what was sent in the last 30 s.
    fn timed_out(&self) -> bool {
        let answered = u64::try_from(self.alive.load(Ordering::Relaxed)).unwrap_or(0);
        self.start.elapsed() > Duration::from_millis(answered) + 2 * KEEP_ALIVE_EVERY
    }
}

#[cfg(test)]
mod tests {
    include!("profession_join_tests.rs");

    #[test]
    fn login_virtual_host_uses_the_packet_destination() {
        for (intent, host, port, expected) in [
            (
                handshake::LOGIN,
                "Play.Example.test.\0proxy\0marker",
                25572,
                ("Play.Example.test.", 25572),
            ),
            (
                handshake::TRANSFER,
                "next.example.test",
                65535,
                ("next.example.test", 65535),
            ),
            // ClientIntentionPacket accepts an empty host name.
            (handshake::LOGIN, "", 25565, ("", 25565)),
            (handshake::LOGIN, "\0proxy", 25565, ("", 25565)),
        ] {
            let packet = handshake::Serverbound::Intention(handshake::Intention {
                protocol: PROTOCOL_VERSION,
                host: host.to_owned(),
                port,
                intent,
            })
            .encode();
            let handshake::Serverbound::Intention(intention) =
                handshake::Serverbound::decode(i32::from(packet[0]), &packet[1..]).unwrap()
            else {
                panic!("expected an intention packet");
            };
            let destination = login_virtual_host(intention);
            assert_eq!(
                (destination.host.as_str(), destination.port),
                expected,
                "intent {intent}, host {host:?}, port {port}",
            );
        }
    }

    /// Bodies shared between players reach each of them as the same bytes, in the order
    /// they were sent among the player's own.
    #[test]
    fn shared_bodies_reach_every_player_in_order() {
        let shared: Vec<Body> = [vec![1, 2, 3], vec![4]]
            .into_iter()
            .map(Body::shared)
            .collect();
        let players: Vec<_> = (0..2)
            .map(|_| {
                let (tx, rx) = std::sync::mpsc::channel();
                (PlayerSender::new(tx, Arc::default()), rx)
            })
            .collect();
        {
            let _holding = Holding::start();
            for (i, (sender, _)) in players.iter().enumerate() {
                sender.send(vec![9, i as u8]);
                sender.send_many(shared.iter().cloned());
                sender.send_many([vec![8]]);
            }
            flush_all(players.iter().map(|(sender, _)| sender));
        }
        for (i, (_, rx)) in players.iter().enumerate() {
            let Ok(Out::Packets(bodies)) = rx.try_recv() else {
                panic!("expected the held packets");
            };
            assert_eq!(bodies, [vec![9, i as u8], vec![1, 2, 3], vec![4], vec![8]]);
            assert!(rx.try_recv().is_err());
        }
    }

    #[test]
    fn undrained_packets_stop_at_twice_the_inbox() {
        let u = Undrained::default();
        // Up to the cap, counted; one more is refused and counts nothing.
        for _ in 0..Undrained::MAX_PACKETS {
            assert!(u.handed(3));
        }
        assert!(!u.handed(3));
        // The hub taking one gives its place back.
        u.taken(3);
        assert!(u.handed(3));
        // Bytes too: a few large packets reach the byte cap long before the count.
        let big = Undrained::default();
        let size = Undrained::MAX_BYTES / 4;
        for _ in 0..4 {
            assert!(big.handed(size));
        }
        assert!(!big.handed(1));
        big.taken(size);
        assert!(big.handed(size));
    }
    use super::*;

    /// #3209: a client may announce its channels while the server waits on a configuration
    /// task (here the code of conduct). With no typed declaration the wait reads packets
    /// itself; the list must still be kept.
    #[test]
    fn channels_announced_during_a_configuration_wait_are_kept() {
        use sunrise_protocol::connection::Pipe;
        use sunrise_protocol::packets::custom_payload::CustomPayload;
        let (client, server) = Pipe::pair();
        let mut client = Connection::local(client);
        let mut server = Connection::local(server);
        for packet in [
            cfg::Serverbound::CustomPayload(CustomPayload {
                channel: "minecraft:register".into(),
                data: b"voicechat:secret\0bungeecord:main".to_vec(),
            }),
            cfg::Serverbound::CustomPayload(CustomPayload {
                channel: "minecraft:unregister".into(),
                data: b"bungeecord:main".to_vec(),
            }),
            cfg::Serverbound::AcceptCodeOfConduct(sunrise_protocol::packets::Empty),
        ] {
            client.send(&packet.encode()).unwrap();
        }
        let transport = Transport::empty();
        let (events, _receiver) = super::super::event_channel_with_capacity(1 << 20);
        let mut io = ConfigIo::new(&mut server, transport.clone(), events).unwrap();
        assert!(io.sender.is_none(), "the wait reads packets itself");
        let mut negotiation = sunrise_protocol::payload_negotiation::Negotiation::new(
            transport.local.manifest.clone(),
        );
        let mut info = ClientInformation::new(10);
        accept_code_of_conduct(&mut io, "Be kind.", &mut info, &transport, &mut negotiation)
            .unwrap();
        let announced = io.channels.announced.as_ref().map(|list| list.list());
        assert_eq!(announced.unwrap().as_ref(), ["voicechat:secret"]);
    }

    #[test]
    fn mod_refusal_translates_on_the_client_and_names_every_difference() {
        use sunrise_protocol::mod_list::{Difference, VersionDifference};
        let difference = Difference {
            missing: (0..9).map(|i| format!("missing_{i} 1.0")).collect(),
            extra: vec!["extra 2.0".into()],
            changed: vec![VersionDifference {
                id: "changed".into(),
                server: "3.0".into(),
                client: "2.9".into(),
            }],
            runtime: vec!["baked".into()],
        };
        let reason = crate::lan::data::component_nbt(&mod_mismatch(&difference).component());
        let french = crate::lang::Language::load(&[], "fr_fr");
        let text = sunrise_protocol::text::plain(&reason, french.map());
        assert!(text.starts_with("Vos mods ne correspondent"));
        assert!(text.contains("changed (serveur 3.0, votre version 2.9)"));
        assert!(text.contains("extra 2.0"));
        let vanilla = sunrise_protocol::text::plain(&reason, &Default::default());
        assert!(vanilla.starts_with("Your mods don't match"));
        assert!(vanilla.contains("baked") && vanilla.contains("runtime files"));
        for name in &difference.missing {
            assert!(text.contains(name));
            assert!(vanilla.contains(name));
        }
    }

    /// ServerHandshakePacketListenerImpl.beginLogin: a client of another protocol is told it is outdated
    /// (below 754) or incompatible, as vanilla tells it.
    #[test]
    fn clients_of_another_protocol_are_refused_as_vanilla_refuses_them() {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::loopback(0, "Other protocols".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        for (protocol, key) in [
            (753, "multiplayer.disconnect.outdated_client"),
            (764, "multiplayer.disconnect.incompatible"),
            (776, "multiplayer.disconnect.incompatible"),
        ] {
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.send(
                &handshake::Serverbound::Intention(handshake::Intention {
                    protocol,
                    host: "127.0.0.1".into(),
                    port: 0,
                    intent: handshake::LOGIN,
                })
                .encode(),
            )
            .unwrap();
            // If this regresses to accepting an unsupported version, its hello produces a login
            // response instead of leaving the test waiting for one the server has not sent yet.
            let _ = c.send(
                &login::Serverbound::Hello(login::ServerHello {
                    name: "Old".into(),
                    uuid: offline_uuid("Old"),
                })
                .encode(),
            );
            let (id, payload) = c.recv().unwrap();
            let login::Clientbound::Disconnect(reason) =
                login::Clientbound::decode(id, &payload).unwrap()
            else {
                panic!("protocol {protocol} was let in")
            };
            assert!(
                reason.reason_json.contains(key),
                "protocol {protocol}: {}",
                reason.reason_json
            );
        }
    }

    /// Logs in through `c` and answers the `LoginFinished` profile.
    pub(super) fn login_profile(c: &mut Connection, name: &str) -> login::GameProfile {
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: sunrise_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: name.into(),
                uuid: offline_uuid(name),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(p) => return p.profile,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
    }

    /// A login by transfer: `Ok` once the profile is told, the reason's JSON when it is turned away.
    fn transfer_login(mut c: Connection, name: &str) -> Result<(), String> {
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: sunrise_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::TRANSFER,
            })
            .encode(),
        )
        .unwrap();
        // A server that does not accept transfers turns one away at the handshake and closes
        // (ServerHandshakePacketListenerImpl.handleIntention), so the hello may find the connection
        // gone: its disconnect is still there to read.
        let _ = c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: name.into(),
                uuid: offline_uuid(name),
            })
            .encode(),
        );
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => return Ok(()),
                login::Clientbound::Disconnect(d) => return Err(d.reason_json),
                other => panic!("unexpected login packet {other:?}"),
            }
        }
    }

    /// `serversettings/accept_transfers/set` changes what the next transfer finds, with no restart:
    /// a server that does not accept transfers turns one away, and takes it once it does.
    #[test]
    fn accepting_transfers_can_be_turned_on_and_off_live() {
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            accepts_transfers: false,
            ..crate::lan::NetConfig::loopback(0, "Transfers".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let refused = transfer_login(Connection::local(lan.connect_local().unwrap()), "Alex")
            .expect_err("a transfer is turned away while transfers are off");
        assert!(
            refused.contains("multiplayer.disconnect.transfers_disabled"),
            "{refused}"
        );
        lan.shared.set_accepts_transfers(true);
        transfer_login(Connection::local(lan.connect_local().unwrap()), "Alex")
            .expect("a transfer is taken once transfers are on");
        lan.shared.set_accepts_transfers(false);
        // The player of the last login is still here: a second profile tries.
        let again = transfer_login(Connection::local(lan.connect_local().unwrap()), "Steve")
            .expect_err("and turned away again once they are off");
        assert!(
            again.contains("multiplayer.disconnect.transfers_disabled"),
            "{again}"
        );
    }

    #[test]
    fn an_empty_server_checks_a_sunrise_clients_extra_mods() {
        use sunrise_protocol::mod_list::{self, Entry, Side};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        login_profile(&mut c, "Owner");
        c.send(&login::Serverbound::LoginAcknowledged(sunrise_protocol::packets::Empty).encode())
            .unwrap();
        let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
        brand.string("sunrise");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: brand.data,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: mod_list::CHANNEL.into(),
                data: mod_list::encode(&[Entry {
                    id: "shared_extra".into(),
                    version: "2.0".into(),
                    side: Side::Both,
                    runtime: None,
                }]),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &payload).unwrap() {
                cfg::Clientbound::CustomPayload(p)
                    if p.channel == sunrise_protocol::payload_negotiation::CHANNEL =>
                {
                    use sunrise_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                    if Control::decode(&p.data).unwrap() == Control::Query {
                        c.send(
                            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                channel: CHANNEL.into(),
                                data: Control::Offer(Manifest::default()).encode().unwrap(),
                            })
                            .encode(),
                        )
                        .unwrap();
                    }
                }
                cfg::Clientbound::SelectKnownPacks(p) => c
                    .send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                    .unwrap(),
                cfg::Clientbound::Disconnect(d) => {
                    assert!(
                        sunrise_protocol::text::plain(&d.reason, &Default::default())
                            .contains("shared_extra 2.0")
                    );
                    break;
                }
                cfg::Clientbound::FinishConfiguration(_) => panic!("extra mod was accepted"),
                _ => {}
            }
        }
    }

    #[test]
    fn sunrise_client_raw_protocol_admission_at_known_packs() {
        // 0: never answers on `sunrise:payloads` (it answers pings); 1: a wrong revision;
        // 2: answers the server's Query after its known-packs reply, as the Sunrise client
        // may; 3: holds its known-packs reply until it answered the Query, as sunrise-bots
        // does (its Offer is in before the barrier: no ping).
        for mode in [0, 1, 2, 3] {
            let wrong_version = mode == 1;
            let compatible = mode >= 2;
            let holds_packs = mode == 3;
            let config = crate::lan::NetConfig {
                announce: false,
                listen: false,
                ..crate::lan::NetConfig::lan(0, "test".into(), 8)
            };
            let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            login_profile(&mut c, "Owner");
            c.send(
                &login::Serverbound::LoginAcknowledged(sunrise_protocol::packets::Empty).encode(),
            )
            .unwrap();
            let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
            brand.string("sunrise");
            c.send(
                &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: brand.data,
                })
                .encode(),
            )
            .unwrap();
            if wrong_version {
                c.send(
                    &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: sunrise_protocol::payload_negotiation::CHANNEL.into(),
                        data: vec![0, sunrise_protocol::payload_negotiation::RAW_PROTOCOL + 1],
                    })
                    .encode(),
                )
                .unwrap();
            }
            // The server asks once it reads the brand, after its known-packs request: mode 2
            // answers that request first, so its Offer comes after its pack reply, and only
            // the server's ping orders it before the barrier.
            let mut packs_asked = false;
            let mut held_packs = None;
            let mut pings = Vec::new();
            loop {
                let (id, payload) = c.recv().unwrap();
                match cfg::Clientbound::decode(id, &payload).unwrap() {
                    cfg::Clientbound::CustomPayload(p)
                        if compatible
                            && p.channel == sunrise_protocol::payload_negotiation::CHANNEL =>
                    {
                        use sunrise_protocol::payload_negotiation::{CHANNEL, Control, Manifest};
                        if Control::decode(&p.data).unwrap() == Control::Query {
                            assert!(packs_asked, "the Query came before known packs");
                            c.send(
                                &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                                    channel: CHANNEL.into(),
                                    data: Control::Offer(Manifest::default()).encode().unwrap(),
                                })
                                .encode(),
                            )
                            .unwrap();
                            if let Some(packs) = held_packs.take() {
                                c.send(&cfg::Serverbound::SelectKnownPacks(packs).encode())
                                    .unwrap();
                            }
                        }
                    }
                    cfg::Clientbound::SelectKnownPacks(p) => {
                        packs_asked = true;
                        if holds_packs {
                            held_packs = Some(p);
                        } else {
                            let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                        }
                    }
                    cfg::Clientbound::Ping(p) => {
                        pings.push(p.id);
                        let _ = c.send(&cfg::Serverbound::Pong(p).encode());
                    }
                    cfg::Clientbound::Disconnect(d) => {
                        assert!(!compatible, "compatible empty Sunrise peer refused");
                        assert_eq!(pings, if wrong_version { vec![] } else { vec![1] });
                        let text = sunrise_protocol::text::plain(&d.reason, &Default::default());
                        assert!(text.contains(if wrong_version {
                            "protocol version"
                        } else {
                            "missing response"
                        }));
                        break;
                    }
                    cfg::Clientbound::FinishConfiguration(_) => {
                        assert!(compatible, "old Sunrise client admitted");
                        assert!(held_packs.is_none(), "admitted before its pack reply");
                        assert_eq!(pings, if holds_packs { vec![] } else { vec![1] });
                        c.send(
                            &cfg::Serverbound::FinishConfiguration(
                                sunrise_protocol::packets::Empty,
                            )
                            .encode(),
                        )
                        .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn raw_payload_peer_admission_over_memory_configuration() {
        use sunrise_protocol::payload_negotiation::{
            CHANNEL, Control, Flow, Manifest, RegistrationPhase,
        };
        for optional in [true, false] {
            let config = crate::lan::NetConfig {
                announce: false,
                listen: false,
                ..crate::lan::NetConfig::lan(0, "test".into(), 8)
            };
            let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let mut c = Connection::local(lan.connect_local().unwrap());
            c.reader
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            login_profile(&mut c, "Owner");
            c.send(
                &login::Serverbound::LoginAcknowledged(sunrise_protocol::packets::Empty).encode(),
            )
            .unwrap();
            let mut builder = Manifest::builder();
            for n in 0..500 {
                builder
                    .register(
                        RegistrationPhase::Play,
                        &format!("example:payload_{n:04}_with_long_name"),
                        Flow::ToServer,
                        "version_long_enough_for_large_offer",
                        optional,
                    )
                    .unwrap();
            }
            let offer = Control::Offer(builder.finish().unwrap()).encode().unwrap();
            assert!(offer.len() > 32_767);
            // A query followed by an offer crosses the server's known-pack request.
            for data in [Control::Query.encode().unwrap(), offer] {
                c.send(
                    &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.into(),
                        data,
                    })
                    .encode(),
                )
                .unwrap();
            }
            let mut saw_offer = false;
            loop {
                let (id, payload) = c.recv().unwrap();
                match cfg::Clientbound::decode_with_payload_codecs(
                    id,
                    &payload,
                    &lan.shared.payload_local.codecs,
                )
                .unwrap()
                {
                    cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                        if matches!(Control::decode(&p.data).unwrap(), Control::Offer(_)) {
                            saw_offer = true;
                        }
                    }
                    cfg::Clientbound::SelectKnownPacks(p) => {
                        // A required unmatched Offer is refused as soon as it is read.
                        // The disconnect can close the peer before this crossed ACK;
                        // still read and assert that named refusal below.
                        let sent = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                        if optional {
                            sent.unwrap();
                        }
                    }
                    cfg::Clientbound::Disconnect(d) => {
                        assert!(!optional);
                        assert!(
                            sunrise_protocol::text::plain(&d.reason, &Default::default())
                                .contains("example:payload_")
                        );
                        break;
                    }
                    cfg::Clientbound::FinishConfiguration(_) => {
                        assert!(optional && saw_offer);
                        c.send(
                            &cfg::Serverbound::FinishConfiguration(
                                sunrise_protocol::packets::Empty,
                            )
                            .encode(),
                        )
                        .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn book_offer_crosses_configuration_for_either_brand_without_disclosing_mods() {
        use crate::recipe_book::{
            CustomBooks, settings_codec, settings_connection, settings_sync::Message,
        };
        use sunrise_protocol::custom_book::{CAPABILITY, CHANNEL};
        use sunrise_protocol::packets::play::Serverbound as S;

        for (brand, supports_books) in [("vanilla", false), ("other-client", true)] {
            let config = crate::lan::NetConfig {
                announce: false,
                listen: false,
                ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
            };
            let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
            let mut c = Connection::local(lan.connect_local().unwrap());
            let timeout = Duration::from_secs(10);
            c.reader.set_read_timeout(Some(timeout)).unwrap();
            login_profile(&mut c, "Owner");
            c.send(
                &login::Serverbound::LoginAcknowledged(sunrise_protocol::packets::Empty).encode(),
            )
            .unwrap();
            let mut data = ::sunrise_core::vanilla_bytes::Writer::new();
            data.string(brand);
            c.send(
                &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: data.data,
                })
                .encode(),
            )
            .unwrap();
            // The book announcement is not a mod list or a mandatory protocol query.
            let mut announced = false;
            loop {
                let (id, payload) = c.recv().unwrap();
                match cfg::Clientbound::decode(id, &payload).unwrap() {
                    cfg::Clientbound::CustomPayload(p) if p.channel == CHANNEL => {
                        assert!(!announced);
                        assert_eq!(p.data.as_slice(), CAPABILITY);
                        announced = true;
                    }
                    cfg::Clientbound::CustomPayload(p) => assert_eq!(p.channel, "minecraft:brand"),
                    cfg::Clientbound::Ping(_) => {
                        panic!("unexpected configuration ping for {brand}")
                    }
                    cfg::Clientbound::SelectKnownPacks(p) => {
                        c.send(&cfg::Serverbound::SelectKnownPacks(p).encode())
                            .unwrap();
                    }
                    cfg::Clientbound::Disconnect(d) => panic!("unexpected refusal: {:?}", d.reason),
                    cfg::Clientbound::FinishConfiguration(_) => {
                        c.send(
                            &cfg::Serverbound::FinishConfiguration(
                                sunrise_protocol::packets::Empty,
                            )
                            .encode(),
                        )
                        .unwrap();
                        break;
                    }
                    _ => {}
                }
            }
            assert!(announced);
            let (event, _credit) = lan.events.recv_timeout_owned(timeout).unwrap().into_parts();
            let NetEvent::Joined(joined, _) = event else {
                panic!("expected a play connection");
            };
            assert_eq!(joined.brand, brand);

            let mut book_connection = settings_connection::Connection::default();
            let mut books = CustomBooks::default();
            let mut turn = |packet: S| {
                c.send(&packet.encode()).unwrap();
                let (event, _credit) = lan.events.recv_timeout_owned(timeout).unwrap().into_parts();
                let NetEvent::Packet(conn, packet, _) = event else {
                    panic!("expected a play packet");
                };
                assert_eq!(conn, joined.conn);
                if let S::CustomPayload(payload) = *packet {
                    assert_eq!(payload.channel, CHANNEL);
                    book_connection
                        .receive(&payload.data, &mut books, Default::default, || 1)
                        .map(|reply| reply.message)
                } else {
                    assert!(matches!(*packet, S::ClientInformation(_)));
                    None
                }
            };
            if supports_books {
                let mut send_book = |message| {
                    turn(S::CustomPayload(cfg::CustomPayload {
                        channel: CHANNEL.to_owned(),
                        data: settings_codec::encode(&message).unwrap(),
                    }))
                };
                assert_eq!(send_book(Message::Hello), Some(Message::Ready));
                assert_eq!(
                    send_book(Message::Offer(vec![])),
                    Some(Message::Snapshot {
                        epoch: 1,
                        revision: 0,
                        entries: vec![]
                    })
                );
                assert!(book_connection.authority.is_some());
            } else {
                // An unsupported client ignores the announcement and enters ordinary play.
                assert_eq!(turn(S::ClientInformation(ClientInformation::new(10))), None);
                assert!(book_connection.authority.is_none());
            }
        }
    }

    #[test]
    fn a_malformed_mod_list_gets_a_disconnect_reason() {
        use sunrise_protocol::mod_list;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::lan(0, "Owner - World".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.reader
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        login_profile(&mut c, "Owner");
        c.send(&login::Serverbound::LoginAcknowledged(sunrise_protocol::packets::Empty).encode())
            .unwrap();
        let mut brand = ::sunrise_core::vanilla_bytes::Writer::new();
        brand.string("sunrise");
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "minecraft:brand".into(),
                data: brand.data,
            })
            .encode(),
        )
        .unwrap();
        let mut malformed = mod_list::encode(&[]);
        // The extra byte encodes an empty runtime-digest section, which is refused.
        malformed.push(0);
        c.send(
            &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: mod_list::CHANNEL.into(),
                data: malformed,
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match cfg::Clientbound::decode(id, &payload).unwrap() {
                // The server may refuse the list and close before these replies land, so a
                // failed reply send is not the test's concern: the Disconnect below is.
                cfg::Clientbound::SelectKnownPacks(p) => {
                    let _ = c.send(&cfg::Serverbound::SelectKnownPacks(p).encode());
                }
                cfg::Clientbound::Disconnect(d) => {
                    let text = sunrise_protocol::text::plain(&d.reason, &Default::default());
                    assert!(text.contains("couldn't be read"));
                    assert!(text.contains("runtime digests"));
                    break;
                }
                cfg::Clientbound::FinishConfiguration(_) => panic!("malformed list was accepted"),
                _ => {}
            }
        }
    }

    #[test]
    fn payloads_and_suggestions_past_vanillas_limits_are_malformed() {
        let payload = |len: usize| {
            play::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "a:b".into(),
                data: vec![0; len],
            })
        };
        let decode = |packet: play::Serverbound| {
            let encoded = packet.encode();
            play::Serverbound::decode(
                sunrise_protocol::ids::play::serverbound::CUSTOM_PAYLOAD,
                &encoded[1..],
            )
        };
        assert!(decode(payload(32_767)).is_ok());
        assert!(decode(payload(32_768)).is_err());
        let config = |len: usize| {
            cfg::Serverbound::CustomPayload(cfg::CustomPayload {
                channel: "a:b".into(),
                data: vec![0; len],
            })
            .encode()
        };
        // The body after the packet's one-byte VarInt id.
        let id = sunrise_protocol::ids::configuration::serverbound::CUSTOM_PAYLOAD;
        assert!(cfg::Serverbound::decode(id, &config(32_767)[1..]).is_ok());
        assert!(cfg::Serverbound::decode(id, &config(32_768)[1..]).is_err());
        let suggestion = |len: usize| {
            let mut w = ::sunrise_core::vanilla_bytes::Writer::new();
            w.varint(7);
            w.string(&"a".repeat(len));
            play::Serverbound::Unknown(COMMAND_SUGGESTION, w.data)
        };
        assert!(checked_play(suggestion(32500)).is_ok());
        assert!(checked_play(suggestion(32501)).is_err());
    }

    #[test]
    fn prompts_become_the_components_vanilla_sends() {
        let tag = |json: &str| {
            let tag = component_tag(&crate::server::json::parse(json, true).unwrap());
            component_json(&tag)
        };
        assert_eq!(tag(r#"{"text":"Hi"}"#), r#""Hi""#);
        assert_eq!(tag(r#""Hi""#), r#""Hi""#);
        assert_eq!(
            tag(r#"["Hi ",{"text":"there","bold":true}]"#),
            r#"{"text":"Hi ","extra":[{"text":"there","bold":true}]}"#
        );
        assert_eq!(
            tag(r#"{"translate":"a","with":["b",{"text":"c","color":"red"}]}"#),
            r#"{"translate":"a","with":["b",{"text":"c","color":"red"}]}"#
        );
    }

    /// The owner's login never waits on the profile fetch, even one that
    /// never answers, and carries no properties.
    #[test]
    fn the_owners_login_never_waits_on_the_profile_fetch() {
        let (_hold, never) = channel::<()>();
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            owner_profile: Some(Arc::new(crate::lan::OwnerProfile::new(move || {
                let _ = never.recv();
                Vec::new()
            }))),
            ..crate::lan::NetConfig::loopback(0, "Owner - World".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let start = Instant::now();
        let owner = login_profile(
            &mut Connection::local(lan.connect_local().unwrap()),
            "Owner",
        );
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(owner.uuid, offline_uuid("Owner"));
        assert!(owner.properties.is_empty());
    }

    /// `ServerLoginPacketListenerImpl.handleHello`: the integrated server's own player
    /// logs in as `getSingleplayerProfile`'s UUID, not the offline one its name makes;
    /// anyone else through memory keeps the offline profile.
    #[test]
    fn the_integrated_owners_login_is_the_singleplayer_profile() {
        let owner_uuid = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdefu128;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".into(), owner_uuid)),
            ..crate::lan::NetConfig::loopback(0, "Owner - World".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let owner = login_profile(
            &mut Connection::local(lan.connect_local().unwrap()),
            "Owner",
        );
        assert_eq!(owner.uuid, owner_uuid);
        let guest = login_profile(&mut Connection::local(lan.connect_local().unwrap()), "Alex");
        assert_eq!(guest.uuid, offline_uuid("Alex"));
    }

    /// Logs "Alex" in over memory up to `LoginFinished`, sends `stray` if there is one, then
    /// `LoginAcknowledged` (so a server that lets the stray packet pass goes on to configuration
    /// instead of waiting for the next packet), and returns the server's next packet with the lines
    /// its log has written so far (en_us's text for the one key the refusal uses).
    fn login_with_stray_packet(
        stray: Option<login::Serverbound>,
    ) -> ((i32, Vec<u8>), Arc<Mutex<Vec<String>>>) {
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let into = lines.clone();
        let mut lang = sunrise_protocol::text::Language::new();
        lang.insert(
            "multiplayer.disconnect.unexpected_query_response".into(),
            "Unexpected custom data from client".into(),
        );
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            log: Some(crate::lan::NetLog {
                write: Arc::new(move |_, _, line| into.lock().unwrap().push(line.to_owned())),
                lang: Arc::new(lang),
                log_ips: false,
            }),
            ..crate::lan::NetConfig::loopback(0, "Stray packets".into(), 8)
        };
        let lan =
            crate::lan::Lan::start_with(Arc::new(crate::lan::data::SyncData::empty()), config)
                .unwrap();
        let mut c = Connection::local(lan.connect_local().unwrap());
        c.send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: sunrise_protocol::PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port: 0,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
        c.send(
            &login::Serverbound::Hello(login::ServerHello {
                name: "Alex".into(),
                uuid: offline_uuid("Alex"),
            })
            .encode(),
        )
        .unwrap();
        loop {
            let (id, payload) = c.recv().unwrap();
            match login::Clientbound::decode(id, &payload).unwrap() {
                login::Clientbound::Compression(p) => c.set_compression(p.threshold),
                login::Clientbound::LoginFinished(_) => break,
                other => panic!("unexpected login packet {other:?}"),
            }
        }
        if let Some(stray) = stray {
            c.send(&stray.encode()).unwrap();
        }
        // A server that has already refused the login may have closed the connection.
        let _ = c.send(&login::Serverbound::LoginAcknowledged(Empty).encode());
        (c.recv().unwrap(), lines)
    }

    /// `ServerLoginPacketListenerImpl.handleCustomQueryPacket` and `handleCookieResponse`: this
    /// server sends no login query and no cookie request, so an answer to either is unexpected and
    /// ends the login with `ServerCommonPacketListenerImpl.DISCONNECT_UNEXPECTED_QUERY`, before
    /// configuration starts.
    fn assert_unexpected_query_refused(stray: login::Serverbound) {
        use sunrise_protocol::ids::login::clientbound::LOGIN_DISCONNECT;
        let ((id, payload), lines) = login_with_stray_packet(Some(stray));
        assert_eq!(
            id, LOGIN_DISCONNECT,
            "the login went on: the server sent packet {id}"
        );
        let login::Clientbound::Disconnect(reason) =
            login::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!("not a login disconnect")
        };
        assert_eq!(
            reason.reason_json,
            r#"{"translate":"multiplayer.disconnect.unexpected_query_response"}"#
        );
        // The refusal is logged as vanilla's `disconnect` does, then the loss of the connection.
        let user = "Alex (<ip address withheld>)";
        let expected = [
            format!("Disconnecting {user}: Unexpected custom data from client"),
            format!("{user} lost connection: Unexpected custom data from client"),
        ];
        let until = Instant::now() + Duration::from_secs(10);
        while lines.lock().unwrap().len() < expected.len() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(*lines.lock().unwrap(), expected);
    }

    #[test]
    fn a_login_query_answer_nobody_asked_for_disconnects_the_client() {
        assert_unexpected_query_refused(login::Serverbound::CustomQueryAnswer(
            login::CustomQueryAnswer {
                transaction: 1,
                payload: Some(vec![1, 2, 3]),
            },
        ));
    }

    #[test]
    fn a_login_cookie_response_nobody_asked_for_disconnects_the_client() {
        assert_unexpected_query_refused(login::Serverbound::CookieResponse(
            login::CookieResponse {
                key: "sunrise:test".into(),
                payload: None,
            },
        ));
    }

    /// The same login with nothing stray goes on to configuration: its first packet is the brand.
    #[test]
    fn a_login_with_nothing_stray_reaches_configuration() {
        use sunrise_protocol::ids::configuration::clientbound::CUSTOM_PAYLOAD;
        let ((id, _), _) = login_with_stray_packet(None);
        assert_eq!(id, CUSTOM_PAYLOAD);
    }

    fn unpack(out: Out) -> Vec<Vec<u8>> {
        match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => vec![b"disconnect".to_vec()],
        }
    }

    fn bodies(rx: &std::sync::mpsc::Receiver<Out>) -> Vec<Vec<Vec<u8>>> {
        rx.try_iter().map(unpack).collect()
    }

    #[test]
    fn packets_sent_after_a_posted_tick_follow_it() {
        let (tx, rx) = channel();
        let (tx2, rx2) = channel();
        let sender = PlayerSender::new(tx, Arc::default());
        let other = PlayerSender::new(tx2, Arc::default());
        for round in 0..50u8 {
            let holding = Holding::start();
            sender.send(vec![round, 1]);
            sender.send(vec![round, 2]);
            other.send(vec![round]);
            drop(holding);
            flush_all([&sender, &other]);
            sender.send(vec![round, 3]);
            let next = || unpack(rx.recv_timeout(Duration::from_secs(10)).unwrap());
            assert_eq!(next(), [vec![round, 1], vec![round, 2]]);
            assert_eq!(next(), [vec![round, 3]]);
            let got = unpack(rx2.recv_timeout(Duration::from_secs(10)).unwrap());
            assert_eq!(got, [vec![round]]);
        }
        let holding = Holding::start();
        sender.send(vec![9]);
        drop(holding);
        flush_all([&sender]);
        sender.disconnect(sunrise_core::nbt::Tag::Byte(0));
        let next = || unpack(rx.recv_timeout(Duration::from_secs(10)).unwrap());
        assert_eq!(next(), [vec![9]]);
        assert_eq!(next(), [b"disconnect".to_vec()]);
        // Nothing sent twice.
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn shared_frames_are_the_frames_compressed_here() {
        use sunrise_protocol::frame;
        let frames = SharedFrames::new(usize::MAX);
        let a: Vec<u8> = (0..40_000u32).map(|i| (i * 7 % 13) as u8).collect();
        let mut b = a.clone();
        b[20_000] ^= 1;
        for (body, threshold) in [(&a, 256), (&b, 256), (&a, 256), (&a, 64), (&b, 256)] {
            let shared = frames.contents(body, threshold).unwrap();
            assert_eq!(*shared, *frame::contents(body, Some(threshold)).unwrap());
        }
        // One frame per body and threshold.
        assert_eq!(frames.frames.lock().unwrap().frames.len(), 3);
    }

    /// Senders asking for the same body at once share one compression: they
    /// all get the frame the first made.
    #[test]
    fn a_body_compressed_now_is_not_compressed_again() {
        let frames = SharedFrames::new(usize::MAX);
        let body: Vec<u8> = (0..400_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let start = std::sync::Barrier::new(8);
        let made: Vec<Arc<[u8]>> = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        frames.contents(&body, 256).unwrap()
                    })
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).collect()
        });
        assert!(made.iter().all(|m| Arc::ptr_eq(m, &made[0])));
        let cache = frames.frames.lock().unwrap();
        assert_eq!(cache.frames.len(), 1);
        assert_eq!(cache.bytes, made[0].len());
    }

    #[test]
    fn shared_frames_keep_the_newest_within_their_limit() {
        let first = SharedFrames::new(0)
            .contents(&[1; 9000], 256)
            .unwrap()
            .len();
        let frames = SharedFrames::new(first * 2);
        for n in 0..5u8 {
            frames.contents(&[n; 9000], 256).unwrap();
        }
        let cache = frames.frames.lock().unwrap();
        assert!((1..=2).contains(&cache.frames.len()));
        assert_eq!(cache.order.len(), cache.frames.len());
        assert_eq!(
            cache.bytes,
            cache
                .frames
                .values()
                .map(|f| f.get().unwrap().as_ref().unwrap().len())
                .sum::<usize>()
        );
        assert!(cache.bytes <= first * 2);
    }

    #[test]
    fn held_packets_go_together_and_in_order() {
        let (tx, rx) = channel();
        let sender = PlayerSender::new(tx, Arc::default());
        sender.send(vec![1]);
        assert_eq!(bodies(&rx), [vec![vec![1]]]);
        let holding = Holding::start();
        sender.send(vec![2]);
        sender.send(vec![3]);
        assert!(bodies(&rx).is_empty());
        drop(holding);
        // A packet sent after the tick still follows the held ones.
        sender.send(vec![4]);
        assert_eq!(bodies(&rx), [vec![vec![2], vec![3]], vec![vec![4]]]);
        let holding = Holding::start();
        sender.send(vec![5]);
        sender.disconnect(sunrise_core::nbt::Tag::Byte(0));
        drop(holding);
        sender.flush();
        assert_eq!(bodies(&rx), [vec![vec![5]], vec![b"disconnect".to_vec()]]);
        assert_eq!(
            sender.pending.load(Ordering::Relaxed),
            5 + 10 * std::mem::size_of::<io_pool::Queued>()
        );
    }

    /// `handleUseItemOn` and `handleUseItem`: `ackBlockChangesUpTo(sequence)` throws on a negative
    /// sequence before the handler does anything, and the server only logs it, so such a packet
    /// does nothing and its sender stays. One with a sequence of 0 or more goes on to the hub.
    #[test]
    fn a_use_with_a_negative_sequence_is_dropped_before_the_hub() {
        use super::io_pool::Verdict;
        use sunrise_protocol::ids::play::serverbound::{USE_ITEM, USE_ITEM_ON};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            ..crate::lan::NetConfig::loopback(0, "Sequence".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let mut c = PlayConn {
            sync: Arc::new(SyncData::empty()),
            payload_sender: PlayerSender::disabled(),
            payload_endpoint: crate::raw_payloads::Endpoint::shared_server(Arc::new(
                PlayerSender::disabled(),
            )),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid: 2,
            name: "p".to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: None,
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        // The bodies after the packets' one-byte VarInt ids.
        let packets = |sequence: i32| {
            [
                (
                    USE_ITEM,
                    play::Serverbound::UseItem(play::UseItem {
                        hand: 0,
                        sequence,
                        yaw: 0.0,
                        pitch: 0.0,
                    })
                    .encode(),
                ),
                (
                    USE_ITEM_ON,
                    play::Serverbound::UseItemOn(play::UseItemOn {
                        hand: 0,
                        pos: [0, 64, 0],
                        face: 1,
                        cursor: [0.5, 1.0, 0.5],
                        inside: false,
                        world_border: false,
                        sequence,
                    })
                    .encode(),
                ),
            ]
        };
        let forwarded = |lan: &crate::lan::Lan| {
            let events = lan.poll();
            events
                .into_iter()
                .map(|receipt| receipt.into_parts().0)
                .filter(|e| matches!(e, crate::lan::NetEvent::Packet(..)))
                .count()
        };
        for sequence in [-1, i32::MIN] {
            for (id, packet) in packets(sequence) {
                assert!(matches!(c.packet(id, packet[1..].to_vec()), Verdict::Go));
            }
        }
        assert_eq!(forwarded(&lan), 0, "a negative sequence reached the hub");
        for sequence in [0, 7] {
            for (id, packet) in packets(sequence) {
                assert!(matches!(c.packet(id, packet[1..].to_vec()), Verdict::Go));
            }
        }
        assert_eq!(forwarded(&lan), 4);
    }

    /// A client that answers no keep-alive is timed out 30 s after the last
    /// one it answered was sent (or after joining).
    #[test]
    fn a_silent_client_times_out() {
        let ago = |s| Instant::now().checked_sub(Duration::from_secs(s)).unwrap();
        let watch = |start, answered| KeepAliveWatch {
            start,
            alive: Arc::new(AtomicI64::new(answered)),
        };
        assert!(!watch(ago(29), -1).timed_out());
        assert!(watch(ago(31), -1).timed_out());
        // Answered the keep-alive sent 15 s in: fine until 45 s.
        assert!(!watch(ago(44), 15_000).timed_out());
        assert!(watch(ago(46), 15_000).timed_out());
    }

    #[test]
    fn a_rate_over_the_limit_kicks() {
        let t0 = Instant::now();
        let second = Duration::from_secs(1);
        // Off at 0 (and below).
        assert!(PacketRate::new(0, t0).is_none());
        assert!(PacketRate::new(-1, t0).is_none());
        let mut rate = PacketRate::new(10, t0).unwrap();
        // 15 packets inside the first second average 3.75 at the tick.
        for _ in 0..15 {
            assert_eq!(rate.received(t0), None);
        }
        assert_eq!(rate.received(t0 + second), None);
        // 50 a second averages 12.5: over a limit of 10 at the first tick.
        let mut rate = PacketRate::new(10, t0).unwrap();
        for _ in 0..50 {
            rate.received(t0);
        }
        assert_eq!(rate.received(t0 + second), Some(12.5));
    }

    #[test]
    fn an_idle_gap_decays_without_a_loop() {
        let t0 = Instant::now();
        let mut rate = PacketRate {
            limit: 100.0,
            received: 0,
            average: 40.0,
            next: t0,
        };
        // Ten seconds later: one tick (0 packets: 40 * 0.75 = 30), then ten
        // more packetless decays — ~1.69 — with `next` past now.
        assert_eq!(rate.received(t0 + Duration::from_secs(10)), None);
        assert!(rate.average > 1.5 && rate.average < 2.0, "{}", rate.average);
        assert!(rate.next > t0 + Duration::from_secs(10));
    }

    /// `ServerCommonPacketListenerImpl.handleKeepAlive`: an answer to the
    /// pending keep-alive with its id counts (and times the round trip); any
    /// other answer, or one with nothing pending, disconnects the player with
    /// `disconnect.timeout`, unless they are the singleplayer owner. A refused
    /// answer must not count as "alive" either.
    #[test]
    fn a_keep_alive_that_is_not_the_pending_one_disconnects_all_but_the_owner() {
        use super::io_pool::Verdict;
        use sunrise_protocol::ids::play::serverbound::KEEP_ALIVE;
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".to_owned(), 1)),
            ..crate::lan::NetConfig::loopback(0, "KeepAlive".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let conn = |uuid: u128, pending: Option<i64>| PlayConn {
            sync: Arc::new(SyncData::empty()),
            payload_sender: PlayerSender::disabled(),
            payload_endpoint: crate::raw_payloads::Endpoint::shared_server(Arc::new(
                PlayerSender::disabled(),
            )),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid,
            name: if uuid == 1 { "Owner" } else { "p" }.to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: pending.map(|id| (id, Instant::now())),
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        let answer = |c: &mut PlayConn, id: i64| c.packet(KEEP_ALIVE, id.to_be_bytes().to_vec());
        // The pending one: accepted.
        let mut c = conn(2, Some(7));
        assert!(matches!(answer(&mut c, 7), Verdict::Go));
        assert!(c.sent_at.is_none());
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), 7);
        // Another id while one is pending: kicked, and not counted as alive.
        let mut c = conn(2, Some(7));
        assert!(matches!(answer(&mut c, 8), Verdict::Kick(..)));
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), -1);
        assert!(c.sent_at.is_some());
        // Nothing pending: kicked.
        let mut c = conn(2, None);
        assert!(matches!(answer(&mut c, 7), Verdict::Kick(..)));
        // The singleplayer owner is never disconnected for it, nor counted alive.
        let mut c = conn(1, Some(7));
        assert!(matches!(answer(&mut c, 8), Verdict::Go));
        assert_eq!(c.watch.alive.load(Ordering::Relaxed), -1);
    }

    /// `ServerCommonPacketListenerImpl.handleCookieResponse`: the server asks for no cookie, so a
    /// response is always unexpected and disconnects with `DISCONNECT_UNEXPECTED_QUERY`, for the
    /// singleplayer owner too (unlike a keep-alive).
    #[test]
    fn a_cookie_response_in_play_disconnects_the_player() {
        use super::io_pool::Verdict;
        use sunrise_protocol::ids::play::serverbound::{CLIENT_TICK_END, COOKIE_RESPONSE};
        let config = crate::lan::NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".to_owned(), 1)),
            ..crate::lan::NetConfig::loopback(0, "Cookie".into(), 8)
        };
        let lan = crate::lan::Lan::start_with(Arc::new(SyncData::empty()), config).unwrap();
        let conn = |uuid: u128| PlayConn {
            sync: Arc::new(SyncData::empty()),
            payload_sender: PlayerSender::disabled(),
            payload_endpoint: crate::raw_payloads::Endpoint::shared_server(Arc::new(
                PlayerSender::disabled(),
            )),
            payload_transport: Transport::empty(),
            conn: 1,
            shared: lan.shared.clone(),
            uuid,
            name: if uuid == 1 { "Owner" } else { "p" }.to_owned(),
            required_pack: false,
            rate: None,
            watch: Arc::new(KeepAliveWatch {
                start: Instant::now(),
                alive: Arc::new(AtomicI64::new(-1)),
            }),
            latency: Arc::new(AtomicI32::new(0)),
            sent_at: None,
            next_keep_alive: Instant::now(),
            counted: None,
            undrained: Arc::default(),
        };
        // The body after the packet's one-byte VarInt id.
        let response = play::Serverbound::CookieResponse(cfg::CookieResponse {
            key: "minecraft:test".into(),
            payload: None,
        })
        .encode();
        let expected = play::Clientbound::Disconnect(cfg::Disconnect {
            reason: translatable("multiplayer.disconnect.unexpected_query_response", &[]),
        })
        .encode();
        for uuid in [2, 1] {
            match conn(uuid).packet(COOKIE_RESPONSE, response[1..].to_vec()) {
                Verdict::Kick(text, body) => {
                    assert_eq!(text, "Unexpected custom data from client");
                    assert_eq!(body, expected);
                }
                _ => panic!("the cookie response of player {uuid} did not disconnect them"),
            }
        }
        // A packet the player may send still goes through.
        assert!(matches!(
            conn(2).packet(CLIENT_TICK_END, Vec::new()),
            Verdict::Go
        ));
    }
}

#[cfg(test)]
mod outgoing_budget_tests {
    use super::*;
    use sunrise_protocol::connection::Framing;
    fn sender(framing: Framing) -> PlayerSender {
        let pending = Arc::default();
        let outbox =
            io_pool::Outbox::new(Arc::clone(&pending), !matches!(framing, Framing::Memory))
                .unwrap();
        PlayerSender {
            payload_transport: Transport::empty(),
            virtual_host: None,
            undrained: Arc::default(),
            pending,
            cap: io_pool::MAX_PENDING,
            held: Arc::default(),
            framing,
            to: To::Pool(
                outbox,
                Arc::new(KeepAliveWatch {
                    start: Instant::now(),
                    alive: Arc::new(AtomicI64::new(-1)),
                }),
            ),
            wire: None,
            datapack_offer: None,
            announced: Arc::default(),
            viewed_levels: Default::default(),
            menu_extra_rev: 0,
        }
    }
    #[test]
    fn play_ping_reports_queue_admission() {
        let mut s = sender(Framing::Network(None));
        s.cap = 0;
        assert!(!s.try_send_play_ping(-1));
        assert!(s.is_live());
        let transaction = transaction_packets::Scope::begin();
        assert!(!s.try_send_play_ping(-1));
        assert!(transaction_packets::Scope::refused());
        drop(transaction);
        assert_eq!(s.queued(), (0, 0));
        assert!(s.is_live());

        s.cap = io_pool::MAX_PENDING;
        let transaction = transaction_packets::Scope::begin();
        assert!(s.try_send_play_ping(42));
        assert_eq!(s.queued().0, 0, "tentative packets cannot reach the outbox");
        drop(transaction);
        assert_eq!(s.queued(), (0, 0));
        assert!(
            s.try_send_play_ping(42),
            "admission does not require a worker wake"
        );
        assert_eq!(s.queued().0, 1);
        s.close();
        assert!(!s.try_send_play_ping(43));

        let (tx, rx) = std::sync::mpsc::channel();
        let channel = PlayerSender::new(tx, Arc::default());
        drop(rx);
        assert!(!channel.try_send_play_ping(44));
        assert_eq!(channel.queued(), (0, 0));
    }

    #[test]
    fn empty_prepared_journal_records_are_bounded_and_fully_refunded() {
        let s = sender(Framing::Memory);
        let id = PayloadId("test:raw".into());
        let cost = io_pool::charge(0) + 512;
        let mut permits = Vec::new();
        for _ in 0..io_pool::MAX_PENDING / cost {
            permits.push(s.prepare_payload(Vec::new(), &id).unwrap());
        }
        assert_eq!(s.pending.load(Ordering::Relaxed), permits.len() * cost);
        assert!(s.pending.load(Ordering::Relaxed) <= io_pool::MAX_PENDING);
        assert!(matches!(
            s.prepare_payload(Vec::new(), &id),
            Err(PayloadSendError::Backpressure)
        ));
        drop(permits.pop());
        assert_eq!(s.pending.load(Ordering::Relaxed), permits.len() * cost);
        let replacement = s.prepare_payload(Vec::new(), &id).unwrap();
        drop(replacement);
        drop(permits);
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn prepared_reservations_refund_and_share_capacity() {
        let s = sender(Framing::Memory);
        let id = PayloadId("test:raw".into());
        let body = Vec::with_capacity(io_pool::MAX_PENDING - 4096);
        let p = s.prepare_payload(body, &id).unwrap();
        assert!(s.pending.load(Ordering::Relaxed) > io_pool::MAX_PENDING - 4096);
        assert!(matches!(
            s.prepare_payload(vec![0; 8192], &id),
            Err(PayloadSendError::Backpressure)
        ));
        drop(p);
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
        let p = s.prepare_payload(vec![0; 9 << 20], &id).unwrap();
        s.close();
        assert!(matches!(p.publish(), Err(PayloadSendError::Disconnected)));
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
    }
    #[test]
    fn prepare_does_not_deliver_and_publish_reuses_compressed_contents() {
        let s = sender(Framing::Network(Some(32)));
        let id = PayloadId("test:raw".into());
        let body = vec![42; 10000];
        let expected = Framing::Network(Some(32)).contents(&body).unwrap();
        let p = s.prepare_payload(body, &id).unwrap();
        let To::Pool(outbox, _) = &s.to else {
            unreachable!()
        };
        assert!(outbox.queued_contents().is_empty());
        assert_eq!(p.bytes.as_ref().unwrap(), &expected);
        p.publish().unwrap();
        assert_eq!(outbox.queued_contents(), [expected]);
        let before = s.pending.load(Ordering::Relaxed);
        let hold = Holding::start();
        s.send(Vec::with_capacity(20000));
        assert_eq!(
            s.pending.load(Ordering::Relaxed),
            before + io_pool::charge(20000)
        );
        drop(hold);
        s.flush();
        let reserved = s.prepare_payload(vec![1; 400], &id).unwrap();
        assert!(s.pending.load(Ordering::Relaxed) > before + 20000);
        drop(reserved);
    }
    #[test]
    fn a_pairing_attachment_keeps_its_prepaid_credit_and_play_epoch() {
        for stale in [false, true] {
            let transport = Transport::capture(
                crate::raw_payloads::Side::Server,
                crate::lan::PAYLOAD_RECORD_OVERHEAD,
            )
            .unwrap();
            transport
                .accept(transport.local.manifest.entries())
                .unwrap();
            transport.finish_configuration().unwrap();
            let (sender, inspect) = PlayerSender::payload_executor_test_sender(transport.clone());
            let prepared = sender
                .prepare_payload(vec![2], &PayloadId("test:attachment".into()))
                .unwrap();
            let prepaid = sender.pending.load(Ordering::Relaxed);
            assert!(prepaid > 0);
            let pairing = crate::raw_payloads::HostedPreparedSend::into_pairing(
                Box::new(prepared),
                transport.epoch().unwrap(),
            );
            let mut out = vec![vec![0].into(), Body::Prepared(pairing), vec![0].into()];
            assert_eq!(sender.pending.load(Ordering::Relaxed), prepaid);
            if stale {
                transport.begin_configuration();
            }
            sender.send_tracking_bodies(&mut out);
            assert!(out.is_empty());
            let expected = if stale {
                vec![vec![0], vec![0]]
            } else {
                vec![vec![0], vec![2], vec![0]]
            };
            assert_eq!(inspect(), expected);
            let expected_charge = 2 * io_pool::charge(1) + if stale { 0 } else { prepaid };
            assert_eq!(sender.pending.load(Ordering::Relaxed), expected_charge);
        }
    }

    /// #3423: a join's first view can queue more than the limit in one burst; a memory
    /// connection (single player) outlasts it, a network client past it is closed.
    #[test]
    fn a_memory_connection_outlasts_a_burst_past_the_limit() {
        let queue = |framing| {
            let mut s = sender(framing);
            s.cap = 4096;
            for _ in 0..6 {
                s.send(vec![0; 1024]);
            }
            s
        };
        let To::Pool(outbox, _) = &queue(Framing::Memory).to else {
            unreachable!()
        };
        assert!(outbox.alive());
        let To::Pool(outbox, _) = &queue(Framing::Network(None)).to else {
            unreachable!()
        };
        assert!(!outbox.alive());
    }
    #[test]
    fn network_refuses_before_queueing_and_empty_messages_cost_space() {
        let s = sender(Framing::Network(None));
        assert!(matches!(
            s.prepare_payload(vec![0; 1 << 21], &PayloadId("test:raw".into())),
            Err(PayloadSendError::FrameTooLarge { .. })
        ));
        assert_eq!(s.pending.load(Ordering::Relaxed), 0);
        let _hold = Holding::start();
        for _ in 0..(io_pool::MAX_PENDING / io_pool::charge(0) + 10) {
            s.send(Vec::new());
        }
        assert!(s.pending.load(Ordering::Relaxed) <= io_pool::MAX_PENDING);
        assert!(s.held().len() < io_pool::MAX_PENDING / std::mem::size_of::<Vec<u8>>());
    }
}

#[cfg(test)]
#[path = "datapack_offer_tests.rs"]
mod datapack_offer_tests;
