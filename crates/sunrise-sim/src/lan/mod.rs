//! Hosting a world on the local network. The network side lives here: the
//! listener, the per-connection handshake/login/configuration threads
//! (`conn`), the network threads every player in the play state is served on
//! (`io_pool`), plus the data the configuration phase sends (`data`). The play
//! state is driven by the game thread (`hub`), which only ever exchanges
//! decoded packets with these threads, so a world that is not open to the
//! network pays nothing.
pub mod abilities;
mod advancements;
pub mod announced_channels;
pub mod config_payloads;
pub mod configuration_tasks;
pub mod server_effects;
pub use advancements::ServerAdvancements;
pub mod arrivals;
pub mod attributes;
mod biome_resend;
mod body;
pub mod border;
pub mod chunk_watch;
pub mod chunks;
pub mod code_of_conduct;
mod command_order;
pub mod command_suggestions;
pub mod commands;
pub mod conn;
pub mod containers;
pub mod data;
pub mod discovery;
mod effect_meta;
pub mod entity;
pub mod equipment_cache;
pub mod guest;
pub mod guest_list;
pub mod handoff;
pub mod hub;
pub mod import;
mod io_pool;
pub mod login_profile;
pub mod menu;
pub mod menu_extra_rev;
#[cfg(test)]
mod payload_bounds_tests;
pub mod player_shown;
pub mod playerdata;
pub mod recipes;
mod registry_names;
pub mod scoreboard;
mod sound_order;
pub mod tracking;
pub mod viewed_levels;
pub mod waypoints;
mod weather;

pub use body::Body;
pub use conn::{Holding, PlayerSender, flush_all};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{Receiver, Sender, channel},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use sunrise_protocol::packets::play;
use sunrise_protocol::{
    GAME_VERSION, PROTOCOL_VERSION,
    crypto::uuid_string,
    packets::{configuration::ClientInformation, handshake::Intention},
    text::translatable,
};

/// Staging margin for the hosted side's ordered event unions and delivery record.
pub(crate) const PAYLOAD_RECORD_OVERHEAD: usize =
    crate::raw_payloads::connection::record_overhead(&[
        std::mem::size_of::<NetEvent>(),
        std::mem::size_of::<hub::HostEvent>(),
    ]);

#[cfg(test)]
mod payload_epoch_tests;
#[cfg(test)]
mod payload_executor_tests;
#[cfg(test)]
mod payload_probe;
#[cfg(test)]
mod payload_raw_channel_tests;

/// IntegratedServer.MAX_PLAYERS.
pub const MAX_PLAYERS: usize = 8;

/// Messages from the connection threads to the game thread.
pub enum NetEvent {
    /// Frozen receiver identity and credit, retained through its main root transaction.
    RawPayload(Box<crate::raw_payloads::Delivery>),
    /// A client finished configuration and enters the play state.
    Joined(Box<Joined>, Option<configuration_tasks::Configured>),
    /// A packet and the bytes of its body on the wire.
    Packet(u32, Box<play::Serverbound>, usize),
    /// The connection ended (reason for the log).
    Left(u32, String),
    /// PlayerList.disconnectAllPlayersWithProfile: a new login of this
    /// profile replaces the player already in the world.
    Duplicate(u128),
}
impl crate::raw_payloads::HostedEvent for NetEvent {
    fn delivery(
        conn: Option<u32>,
        endpoint: crate::raw_payloads::Endpoint,
        incoming: crate::raw_payloads::Incoming,
    ) -> Self {
        Self::RawPayload(Box::new(crate::raw_payloads::Delivery {
            conn,
            endpoint,
            incoming,
        }))
    }
    fn into_incoming(self) -> crate::raw_payloads::Incoming {
        let Self::RawPayload(delivery) = self else {
            unreachable!("only a delivery is taken back")
        };
        delivery.incoming
    }
}
pub struct Joined {
    pub conn: u32,
    pub uuid: u128,
    pub name: String,
    /// The profile's properties (skin textures, in online mode).
    pub properties: Vec<sunrise_core::profile::AccountProperty>,
    pub info: ClientInformation,
    pub brand: String,
    pub sender: PlayerSender,
    /// Smoothed keep-alive round trip in milliseconds.
    pub latency: Arc<AtomicI32>,
    pub address: SocketAddr,
    /// The login used up an expectation of a proxy's (`player-expect`) when it
    /// started (`arrivals`).
    pub expected: bool,
}

pub(crate) fn event_channel() -> (
    crate::raw_payloads::ingress::Producer<NetEvent>,
    crate::raw_payloads::ingress::Consumer<NetEvent>,
) {
    crate::raw_payloads::ingress::channel(96 << 20, NetEvent::ingress_charge)
        .expect("positive incoming byte budget")
}
#[cfg(test)]
pub(crate) fn event_channel_with_capacity(
    capacity: usize,
) -> (
    crate::raw_payloads::ingress::Producer<NetEvent>,
    crate::raw_payloads::ingress::Consumer<NetEvent>,
) {
    crate::raw_payloads::ingress::channel(capacity, NetEvent::ingress_charge)
        .expect("positive test incoming byte budget")
}
impl NetEvent {
    fn ingress_charge(&self) -> usize {
        use std::mem::size_of;
        use sunrise_protocol::retained::OwnedHeap;
        let owned = match self {
            Self::RawPayload(delivery) => size_of::<crate::raw_payloads::Delivery>()
                .saturating_add(delivery.incoming.owned_heap_bytes()),
            Self::Joined(joined, _) => {
                let Joined {
                    conn: _,
                    uuid: _,
                    name,
                    properties,
                    info,
                    brand,
                    sender: _,
                    latency: _,
                    address: _,
                    expected: _,
                } = joined.as_ref();
                // sender/latency are shared physical provider state, not this record.
                size_of::<Joined>()
                    .saturating_add(name.capacity())
                    .saturating_add(properties.owned_heap_bytes())
                    .saturating_add(info.owned_heap_bytes())
                    .saturating_add(brand.capacity())
            }
            Self::Packet(_, packet, _) => {
                size_of::<play::Serverbound>().saturating_add(packet.as_ref().owned_heap_bytes())
            }
            Self::Left(_, reason) => reason.capacity(),
            Self::Duplicate(_) => 0,
        };
        // Source snapshot, guest inbox, derived HostEvent batch and credit marker,
        // with spare-capacity allowance. No wire-length proxy for owned buffers.
        owned.saturating_add(
            8 * size_of::<(
                Self,
                play::Serverbound,
                hub::HostEvent,
                crate::raw_payloads::ingress::ReceiveLease<Self>,
                usize,
                usize,
            )>(),
        )
    }
}
/// What a [`PlayerSender`] made for a test's channel gives it.
pub enum Out {
    /// A packet body: VarInt id and payload.
    Packet(Vec<u8>),
    /// Packet bodies held during a tick, in order.
    Packets(Vec<Vec<u8>>),
    /// Send a play disconnect with this component, then close.
    Disconnect(sunrise_core::nbt::Tag),
}

/// PlayerList.canPlayerLogin for a dedicated server: the disconnect reason
/// (bans, whitelist) for a profile connecting from an address, if refused.
pub type LoginGate =
    Arc<dyn Fn(SocketAddr, u128, &str) -> Option<sunrise_core::nbt::Tag> + Send + Sync>;

#[derive(Clone, Debug)]
pub struct VerifiedSession {
    pub uuid: u128,
    pub properties: Vec<sunrise_core::profile::AccountProperty>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionVerifyError {
    Unavailable,
}

/// `MinecraftSessionService.hasJoinedServer`:
/// `Ok(Some(session))` verified, `Ok(None)` invalid session, `Err(_)` auth servers unavailable.
pub type SessionVerifier = Arc<
    dyn Fn(
            &str,
            &str,
            Option<std::net::IpAddr>,
        ) -> Result<Option<VerifiedSession>, SessionVerifyError>
        + Send
        + Sync,
>;

/// online-mode=true: the key pair the server made at startup
/// (MinecraftServer.initializeKeyPair) and how players are verified.
pub struct OnlineMode {
    pub key: sunrise_protocol::rsa::RsaPrivateKey,
    /// The public key as X.509 (PublicKey.getEncoded), sent in the hello.
    pub public_der: Vec<u8>,
    /// prevent-proxy-connections: the session service also checks the address.
    pub prevent_proxy: bool,
    pub verify: SessionVerifier,
}

impl OnlineMode {
    /// MinecraftServer.initializeKeyPair.
    pub fn generate(prevent_proxy: bool, verify: SessionVerifier) -> Result<Self, String> {
        let key = sunrise_protocol::rsa::RsaPrivateKey::generate(1024)
            .map_err(|e| format!("Failed to generate key pair: {e}"))?;
        Ok(Self {
            public_der: key.public_der(),
            key,
            prevent_proxy,
            verify,
        })
    }
    /// IntegratedServer.initServer (`setUsesAuthentication(true)`): a LAN or
    /// friends world authenticates its guests when the host is signed in;
    /// an offline name hosts in offline mode.
    pub fn lan(signed_in: bool, verify: SessionVerifier) -> Option<Arc<Self>> {
        if !signed_in {
            return None;
        }
        Self::generate(false, verify)
            .map_err(|e| log::warn!("LAN stays in offline mode: {e}"))
            .ok()
            .map(Arc::new)
    }
}

/// The dedicated server's log for the login phase: (thread, level, line)
/// written as vanilla's Log4j would, and the language that turns a
/// disconnect reason into its text (Component.getString).
#[derive(Clone)]
pub struct NetLog {
    pub write: Arc<dyn Fn(&str, &str, &str) + Send + Sync>,
    pub lang: Arc<sunrise_protocol::text::Language>,
    /// log-ips: addresses in the log, or "<ip address withheld>".
    pub log_ips: bool,
}

/// The server.properties key that lets only the Sunrise client join (off unless "true").
pub const SUNRISE_CLIENTS_ONLY: &str = "sunrise-clients-only";
/// What another client is told.
pub const SUNRISE_CLIENTS_ONLY_REASON: &str =
    "This server is for the Sunrise client: playsunrise.net";

/// How the server side of the network behaves: an integrated server's LAN
/// uses the defaults, a dedicated server takes them from server.properties.
#[derive(Clone)]
pub struct NetConfig {
    /// server-ip (all interfaces when absent).
    pub bind: Option<std::net::IpAddr>,
    /// 0 picks a free port.
    pub port: u16,
    pub motd: String,
    pub max_players: usize,
    /// network-compression-threshold; negative disables compression.
    pub compression_threshold: i32,
    /// rate-limit: packets a second a player may average before being kicked
    /// (RateKickingConnection); 0 is off.
    pub rate_limit: i32,
    /// enable-status: answer status pings.
    pub enable_status: bool,
    pub hide_online_players: bool,
    /// server-icon.png (64x64 PNG bytes).
    pub favicon: Option<Vec<u8>>,
    /// Secure chat (a dedicated server; none on a LAN world).
    pub secure_chat: Option<Arc<SecureChat>>,
    pub accepts_transfers: bool,
    /// LanServerPinger: announce the world on the local network.
    pub announce: bool,
    /// Accept connections from the network. Without it only local ones
    /// (`Lan::connect_local`): an integrated server not open to the LAN.
    pub listen: bool,
    /// Ops bypass a full server (ServerOpList.canBypassPlayerLimit).
    pub bypasses_limit: Option<Arc<dyn Fn(u128) -> bool + Send + Sync>>,
    pub gate: Option<LoginGate>,
    /// Online mode: players are authenticated (a LAN world's when its host is signed in).
    pub online: Option<Arc<OnlineMode>>,
    /// `[transfer]` on: the players a proxy said would arrive
    /// (`arrivals::Transfers`), shared with the hub. A login uses its
    /// expectation up as it starts, once the bans and the whitelist let it in.
    pub arrivals: Option<Arc<std::sync::Mutex<arrivals::Expectations>>>,
    pub log: Option<NetLog>,
    /// server.properties `resource-pack`: offered to every player during
    /// configuration (a dedicated server's; MinecraftServer.getServerResourcePack).
    pub resource_pack: Option<crate::server::properties::ResourcePackInfo>,
    /// `enable-code-of-conduct`: the `codeofconduct/<language>.txt` texts
    /// (`MinecraftServer.getCodeOfConducts`), offered before the resource
    /// pack. Empty: no code of conduct.
    pub code_of_conducts: Vec<(String, String)>,
    /// MinecraftServer.serverLinks (a dedicated server's `bug-report-link`):
    /// sent in configuration when non-empty
    /// (ServerConfigurationPacketListenerImpl.startConfiguration).
    pub server_links: Vec<sunrise_protocol::packets::configuration::ServerLink>,
    /// An integrated server's owner profile: the properties the window's own
    /// (memory) connection logs in with.
    pub owner_profile: Option<Arc<OwnerProfile>>,
    /// `IntegratedServer.getSingleplayerProfile` (`Minecraft.getGameProfile`): the
    /// integrated server's own player's name and UUID. A memory login whose name matches
    /// is that profile (the account's UUID); every other login derives the offline one.
    pub singleplayer_profile: Option<(String, u128)>,
    /// `sunrise-clients-only` (Sunrise's own server.properties key, off): a
    /// client whose `minecraft:brand` isn't Sunrise's is turned away during
    /// configuration. A brand can be spoofed: a gate, not security.
    pub sunrise_clients_only: bool,
    /// `sunrise-lod-distance` (Sunrise's own server.properties key, 64): how
    /// far, in chunks, the server samples far terrain for Sunrise clients
    /// (`lod_net`); 0 serves none.
    pub lod_distance: u32,
    /// The shapes of the world's levels: their dimension types go to
    /// clients when the world's data packs changed them.
    pub levels: crate::chunk::LevelShapes,
}
impl NetConfig {
    /// A real TCP server confined to this machine, without multicast announcements.
    /// Use this for socket tests; `lan` retains the public LAN behaviour.
    pub fn loopback(port: u16, motd: String, max_players: usize) -> Self {
        Self {
            bind: Some(std::net::Ipv4Addr::LOCALHOST.into()),
            announce: false,
            ..Self::lan(port, motd, max_players)
        }
    }
    /// MinecraftServer.enforceSecureProfile.
    pub fn enforces_secure_chat(&self) -> bool {
        self.secure_chat.as_ref().is_some_and(|s| s.enforced())
    }
    /// IntegratedServer.publishServer.
    pub fn lan(port: u16, motd: String, max_players: usize) -> Self {
        Self {
            bind: None,
            port,
            motd,
            max_players,
            compression_threshold: 256,
            rate_limit: 0,
            enable_status: true,
            hide_online_players: false,
            favicon: None,
            secure_chat: None,
            accepts_transfers: false,
            announce: true,
            listen: true,
            bypasses_limit: None,
            gate: None,
            online: None,
            arrivals: None,
            log: None,
            resource_pack: None,
            code_of_conducts: Vec::new(),
            server_links: Vec::new(),
            sunrise_clients_only: false,
            lod_distance: crate::lod_net::DEFAULT_DISTANCE,
            levels: crate::chunk::LevelShapes::VANILLA,
            owner_profile: None,
            singleplayer_profile: None,
        }
    }
}

/// `IntegratedServer.getSingleplayerProfile` (`Minecraft.getGameProfile`):
/// the signed-in account's profile properties, its signed `textures` among
/// them, which the player list gives the owner so that LAN guests see the
/// owner's skin. The fetch starts in the background when the world opens.
/// Integrated-server creation waits for the result; callers of `properties()`
/// do not. A failed fetch yields no properties.
pub struct OwnerProfile {
    properties: std::sync::OnceLock<Vec<sunrise_core::profile::AccountProperty>>,
    fetch: Mutex<Receiver<Vec<sunrise_core::profile::AccountProperty>>>,
}

impl OwnerProfile {
    pub fn new(
        fetch: impl FnOnce() -> Vec<sunrise_core::profile::AccountProperty> + Send + 'static,
    ) -> Self {
        let (send, receive) = channel();
        let _ = std::thread::Builder::new()
            .name("Profile fetcher".into())
            .spawn(move || {
                let _ = send.send(fetch());
            });
        Self {
            properties: std::sync::OnceLock::new(),
            fetch: Mutex::new(receive),
        }
    }
    /// The owner's properties once fetched; empty while the fetch is out
    /// or when it failed. Never blocks.
    pub fn properties(&self) -> &[sunrise_core::profile::AccountProperty] {
        if let Some(properties) = self.properties.get() {
            return properties;
        }
        // Kept for the answer's whole store, so `wait` never sees the channel emptied first.
        let Ok(fetch) = self.fetch.try_lock() else {
            return &[];
        };
        match fetch.try_recv() {
            Ok(properties) => self.properties.get_or_init(|| properties),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.properties.get_or_init(Vec::new)
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => &[],
        }
    }
    /// The owner's properties, waiting for the fetch if it is still out
    /// (`Minecraft.getGameProfile`'s `profileFuture.join()`): empty when it
    /// failed. Every later read, [`Self::properties`] too, sees the same list.
    pub fn wait(&self) -> &[sunrise_core::profile::AccountProperty] {
        if let Some(properties) = self.properties.get() {
            return properties;
        }
        let fetch = self.fetch.lock();
        if let Some(properties) = self.properties.get() {
            return properties;
        }
        let received = fetch.as_ref().ok().and_then(|fetch| fetch.recv().ok());
        let properties = self.properties.get_or_init(|| received.unwrap_or_default());
        drop(fetch);
        properties
    }
}

/// A dedicated server's secure chat (DedicatedServer.enforceSecureProfile,
/// Services.profileKeySignatureValidator): the services' profile-key keys,
/// fetched in the background from startup and waited for by their first
/// use, as authlib's RetryableFetch does.
pub struct SecureChat {
    /// enforce-secure-profile, in online mode.
    pub enforce: bool,
    keys: std::sync::OnceLock<Vec<sunrise_protocol::rsa::RsaPublicKey>>,
    fetch: Mutex<Option<JoinHandle<Vec<sunrise_protocol::rsa::RsaPublicKey>>>>,
}

impl SecureChat {
    pub fn new(
        enforce: bool,
        fetch: impl FnOnce() -> Vec<sunrise_protocol::rsa::RsaPublicKey> + Send + 'static,
    ) -> Self {
        let handle = std::thread::Builder::new()
            .name("Services keys".into())
            .spawn(fetch)
            .ok();
        Self {
            enforce,
            keys: std::sync::OnceLock::new(),
            fetch: Mutex::new(handle),
        }
    }
    /// The keys players' chat keys must be signed by (none when the
    /// services couldn't be reached).
    pub fn keys(&self) -> &[sunrise_protocol::rsa::RsaPublicKey] {
        self.keys.get_or_init(|| {
            let handle = self.fetch.lock().ok().and_then(|mut h| h.take());
            handle.and_then(|h| h.join().ok()).unwrap_or_default()
        })
    }
    /// enforceSecureProfile: enforced, and able to check keys.
    pub fn enforced(&self) -> bool {
        self.enforce && !self.keys().is_empty()
    }
}

/// What the server tick decides for a [`Handoff`]: the intention to log in with, or the
/// login disconnect's reason as the packet's JSON.
type HandshakeDecision = Result<Intention, String>;

/// One eligible intention awaiting the server tick's pre-profile login decision. Its connection
/// waits on the other end of `reply`, so dropping the handoff (the connection was closed to make
/// room, or the server stopped) ends that wait.
pub struct Handoff {
    id: u64,
    pub intention: Intention,
    pub peer_ip: Option<IpAddr>,
    reply: Sender<HandshakeDecision>,
}

impl Handoff {
    /// Tells the waiting connection, if it still waits, to log in with this handoff's
    /// intention, or to disconnect with `refusal`, the login disconnect's reason JSON.
    pub fn answer(self, refusal: Option<String>) {
        let _ = self.reply.send(refusal.map_or(Ok(self.intention), Err));
    }
}

/// State shared with the connection threads.
pub struct Shared {
    payload_local: Arc<crate::raw_payloads::Local>,
    /// What the next login is sent; `PlayerList.reloadResources` swaps it.
    sync: Mutex<Arc<data::SyncData>>,
    pub events: crate::raw_payloads::ingress::Producer<NetEvent>,
    /// Handshakes waiting for the server tick, when a game drives this listener.
    handoffs: Option<Mutex<Vec<Handoff>>>,
    handshake_listeners: mod_api::__private::EnabledListeners,
    configuration_requests: Option<Sender<configuration_tasks::ConfigurationRequest>>,
    /// Admitted logins' properties for `player_login_profile`, on a server the game drives.
    login_profiles: Option<Sender<login_profile::LoginProfileRequest>>,
    prepare_spawn: bool,
    lan_incarnation: u64,
    next_configuration_connection: AtomicU64,
    pub config: NetConfig,
    motd: Mutex<String>,
    max_players: AtomicUsize,
    hide_online_players: AtomicBool,
    enable_status: AtomicBool,
    /// `DedicatedServer.setAcceptsTransfers`: whether a login by transfer from another server
    /// is accepted (`accepts-transfers`), live.
    accepts_transfers: AtomicBool,
    /// PlayerList.players in join order: profile id and name.
    online: Mutex<Vec<(u128, String)>>,
    next: AtomicU32,
    stop: AtomicBool,
    random: Mutex<u64>,
    /// Connections accepted from the network whose thread still runs.
    connections: AtomicUsize,
    /// Those connections by address.
    by_address: Mutex<HashMap<IpAddr, usize>>,
    /// Those not playing yet (handshake, status, login, configuration), oldest first.
    waiting: Mutex<Vec<Waiting>>,
    next_connection: AtomicU64,
    /// Those whose play ended, waiting for their writer to finish.
    closing: AtomicUsize,
    /// Connections turned away since the last report of them, and when that was.
    turned_away: Mutex<(usize, Option<Instant>)>,
    /// A stop waits for a proxy to move the players out (`arrivals::StopWait`): no login comes in.
    shutting_down: AtomicBool,
}
impl Shared {
    /// `IntegratedServer.isSingleplayerOwner`: the integrated server's own player, by name
    /// (ignoring case). Never anyone on a server without a singleplayer profile.
    pub fn is_singleplayer_owner(&self, name: &str) -> bool {
        self.config
            .singleplayer_profile
            .as_ref()
            .is_some_and(|(owner, _)| owner.eq_ignore_ascii_case(name))
    }
    /// The registries and tags a new connection is sent.
    pub fn sync(&self) -> Arc<data::SyncData> {
        self.sync.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// `/reload`: the data new connections are sent from now on.
    pub fn set_sync(&self, sync: Arc<data::SyncData>) {
        *self.sync.lock().unwrap_or_else(|e| e.into_inner()) = sync;
    }
    /// Allocate a per-server configuration connection identifier bound to this
    /// `Lan` incarnation. Exhaustion is permanent for this server; never wrap and
    /// reuse a token.
    pub(super) fn next_configuration_connection(&self) -> Option<mod_api::ConfigurationConnection> {
        self.next_configuration_connection
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .ok()
            .map(|id| mod_api::__private::configuration_connection(self.lan_incarnation, id))
    }
    /// Allocate from the shared network and connectionless actor namespace.
    /// Exhaustion is permanent for this server; never wrap and reuse a token.
    pub fn next_id(&self) -> Option<u32> {
        let mut id = self.next.load(Ordering::Relaxed);
        loop {
            let next = id.checked_add(1)?;
            match self
                .next
                .compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return Some(id),
                Err(current) => id = current,
            }
        }
    }
    /// PlayerList.canPlayerLogin then disconnectAllPlayersWithProfile: the
    /// refusal reason, or None once the profile holds a place. A profile
    /// already playing is kicked and its place awaited.
    pub fn try_admit(
        &self,
        address: SocketAddr,
        uuid: u128,
        name: &str,
    ) -> Option<sunrise_core::nbt::Tag> {
        if self.shutting_down.load(Ordering::Relaxed) {
            return Some(translatable("multiplayer.disconnect.server_shutdown", &[]));
        }
        // IntegratedPlayerList.canPlayerLogin: the owner's name (ignoring case,
        // `isSingleplayerOwner`) is refused while a player of that name is online.
        if self.is_singleplayer_owner(name)
            && self
                .online
                .lock()
                .unwrap()
                .iter()
                .any(|(_, online)| online.eq_ignore_ascii_case(name))
        {
            return Some(translatable("multiplayer.disconnect.name_taken", &[]));
        }
        if let Some(gate) = &self.config.gate
            && let Some(reason) = gate(address, uuid, name)
        {
            return Some(reason);
        }
        {
            let online = self.online.lock().unwrap();
            let bypass = self.config.bypasses_limit.as_ref().is_some_and(|b| b(uuid));
            let taken = online.len();
            if taken >= self.max_players() && !bypass {
                return Some(translatable("multiplayer.disconnect.server_full", &[]));
            }
        }
        if self.is_online(uuid) {
            let _ = self.events.send(NetEvent::Duplicate(uuid));
            let deadline = std::time::Instant::now() + Duration::from_secs(30);
            while self.is_online(uuid) {
                if std::time::Instant::now() > deadline {
                    return Some(translatable("multiplayer.disconnect.slow_login", &[]));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.online.lock().unwrap().push((uuid, name.to_owned()));
        None
    }
    /// A stop has begun: every login from now on is refused as the server's shutdown.
    pub fn refuse_logins(&self) {
        self.shutting_down.store(true, Ordering::Relaxed);
    }
    fn is_online(&self, uuid: u128) -> bool {
        self.online.lock().unwrap().iter().any(|(u, _)| *u == uuid)
    }
    pub fn motd(&self) -> String {
        self.motd.lock().unwrap().clone()
    }
    pub fn set_motd(&self, motd: String) {
        *self.motd.lock().unwrap() = motd;
    }
    pub fn max_players(&self) -> usize {
        self.max_players.load(Ordering::Relaxed)
    }
    /// `PlayerList.setMaxPlayers`: change the limit used by new logins and status responses.
    pub fn set_max_players(&self, max_players: usize) {
        self.max_players.store(max_players, Ordering::Relaxed);
    }
    /// `DedicatedServer.setHidesOnlinePlayers`: change whether status responses include player names.
    pub fn hide_online_players(&self) -> bool {
        self.hide_online_players.load(Ordering::Relaxed)
    }
    pub fn set_hide_online_players(&self, hide: bool) {
        self.hide_online_players.store(hide, Ordering::Relaxed);
    }
    /// `DedicatedServer.setRepliesToStatus`: change whether status pings are answered.
    pub fn enable_status(&self) -> bool {
        self.enable_status.load(Ordering::Relaxed)
    }
    pub fn set_enable_status(&self, enable: bool) {
        self.enable_status.store(enable, Ordering::Relaxed);
    }
    pub fn accepts_transfers(&self) -> bool {
        self.accepts_transfers.load(Ordering::Relaxed)
    }
    /// `DedicatedServer.setAcceptsTransfers`: the handshake of a transfer is accepted or turned
    /// away from now on.
    pub fn set_accepts_transfers(&self, accept: bool) {
        self.accepts_transfers.store(accept, Ordering::Relaxed);
    }
    pub fn release(&self, uuid: u128) {
        self.online.lock().unwrap().retain(|(u, _)| *u != uuid);
    }
    pub fn online(&self) -> usize {
        self.online.lock().unwrap().len()
    }
    fn next_random(&self, bound: usize) -> usize {
        let mut r = self.random.lock().unwrap();
        *r = r
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*r >> 33) as usize) % bound.max(1)
    }
    /// The status response (MinecraftServer.buildServerStatus encoded with
    /// ServerStatus.CODEC): description, players with up to 12 shuffled
    /// names, version, favicon and enforcesSecureChat when true.
    pub fn status_json(&self) -> String {
        use crate::server::json::Json;
        let motd = self.motd.lock().unwrap().clone();
        let online: Vec<(u128, String)> = self.online.lock().unwrap().clone();
        let count = self.online();
        let mut players = vec![
            (
                "max".to_owned(),
                Json::Number(self.max_players().to_string()),
            ),
            ("online".to_owned(), Json::Number(count.to_string())),
        ];
        if !self.hide_online_players() && !online.is_empty() {
            let size = online.len().min(12);
            let offset = self.next_random(online.len() - size + 1);
            let mut sample: Vec<(u128, String)> = online[offset..offset + size].to_vec();
            for i in (1..sample.len()).rev() {
                let j = self.next_random(i + 1);
                sample.swap(i, j);
            }
            players.push((
                "sample".to_owned(),
                Json::Array(
                    sample
                        .into_iter()
                        .map(|(u, n)| {
                            Json::Object(vec![
                                ("id".to_owned(), Json::String(uuid_string(u))),
                                ("name".to_owned(), Json::String(n)),
                            ])
                        })
                        .collect(),
                ),
            ));
        }
        let mut status = vec![
            ("description".to_owned(), Json::String(motd)),
            ("players".to_owned(), Json::Object(players)),
            (
                "version".to_owned(),
                Json::Object(vec![
                    ("name".to_owned(), Json::String(GAME_VERSION.to_owned())),
                    (
                        "protocol".to_owned(),
                        Json::Number(PROTOCOL_VERSION.to_string()),
                    ),
                ]),
            ),
        ];
        if let Some(icon) = &self.config.favicon {
            status.push((
                "favicon".to_owned(),
                Json::String(format!("data:image/png;base64,{}", base64(icon))),
            ));
        }
        if self.config.enforces_secure_chat() {
            status.push(("enforcesSecureChat".to_owned(), Json::Bool(true)));
        }
        Json::Object(status).compact()
    }
}

/// java.util.Base64.getEncoder (padded, no line breaks).
pub fn base64(data: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(char::from(A[(n >> 18) as usize & 63]));
        out.push(char::from(A[(n >> 12) as usize & 63]));
        out.push(if chunk.len() > 1 {
            char::from(A[(n >> 6) as usize & 63])
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            char::from(A[n as usize & 63])
        } else {
            '='
        });
    }
    out
}

/// A running LAN server.
pub struct Lan {
    pub shared: Arc<Shared>,
    pub configuration_tasks: configuration_tasks::Tasks,
    events: crate::raw_payloads::ingress::Consumer<NetEvent>,
    configuration_requests: Option<Receiver<configuration_tasks::ConfigurationRequest>>,
    login_profiles: Option<Receiver<login_profile::LoginProfileRequest>>,
    pub port: u16,
    listener: Option<JoinHandle<()>>,
    /// Stops the accept loop alone: local connections stay.
    listener_stop: Arc<AtomicBool>,
    /// The multicast announcement (LanServerPinger); absent when the socket failed.
    _pinger: Option<discovery::Pinger>,
}
impl Lan {
    /// Listen on `port` (0 picks a free one).
    pub fn start(
        sync: Arc<data::SyncData>,
        port: u16,
        motd: String,
        max_players: usize,
    ) -> io::Result<Self> {
        Self::start_with(sync, NetConfig::lan(port, motd, max_players))
    }
    /// ServerConnectionListener.startTcpServerListener with a configuration.
    pub fn start_with(sync: Arc<data::SyncData>, config: NetConfig) -> io::Result<Self> {
        Self::start_listener(sync, config, false, false)
    }
    /// Start a listener whose game thread runs configuration tasks and prepares each spawn.
    pub fn start_with_spawn_preparation(
        sync: Arc<data::SyncData>,
        config: NetConfig,
    ) -> io::Result<Self> {
        Self::start_listener(sync, config, true, true)
    }
    fn start_listener(
        sync: Arc<data::SyncData>,
        config: NetConfig,
        drive_game: bool,
        prepare_spawn: bool,
    ) -> io::Result<Self> {
        let (tx, events) = event_channel();
        let (request_tx, request_rx) = if drive_game {
            let (tx, rx) = channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let (profile_tx, profile_rx) = if drive_game {
            let (tx, rx) = channel();
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let (listen, port, announce) = (config.listen, config.port, config.announce);
        let hide_online_players = config.hide_online_players;
        let enable_status = config.enable_status;
        static NEXT_LAN_INCARNATION: AtomicU64 = AtomicU64::new(1);
        let lan_incarnation = NEXT_LAN_INCARNATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| io::Error::other("LAN incarnation identifiers exhausted"))?;
        let shared = Arc::new(Shared {
            payload_local: crate::raw_payloads::Local::capture(crate::raw_payloads::Side::Server)
                .map_err(io::Error::other)?,
            sync: Mutex::new(sync),
            events: tx,
            handoffs: drive_game.then(|| Mutex::new(Vec::new())),
            handshake_listeners: mod_api::__private::EnabledListeners::capture(
                mod_api::HookId::PlayerHandshake,
            ),
            configuration_requests: request_tx,
            login_profiles: profile_tx,
            prepare_spawn,
            lan_incarnation,
            next_configuration_connection: AtomicU64::new(1),
            motd: Mutex::new(config.motd.clone()),
            max_players: AtomicUsize::new(config.max_players),
            hide_online_players: AtomicBool::new(hide_online_players),
            enable_status: AtomicBool::new(enable_status),
            accepts_transfers: AtomicBool::new(config.accepts_transfers),
            online: Mutex::new(Vec::new()),
            next: AtomicU32::new(1),
            stop: AtomicBool::new(false),
            connections: AtomicUsize::new(0),
            by_address: Mutex::new(HashMap::new()),
            waiting: Mutex::new(Vec::new()),
            next_connection: AtomicU64::new(0),
            closing: AtomicUsize::new(0),
            turned_away: Mutex::new((0, None)),
            shutting_down: AtomicBool::new(false),
            random: Mutex::new(sunrise_java_properties::now_millis() as u64 | 1),
            config,
        });
        let mut lan = Self {
            shared,
            configuration_tasks: configuration_tasks::Tasks::new(lan_incarnation),
            events,
            configuration_requests: request_rx,
            login_profiles: profile_rx,
            port: 0,
            listener: None,
            listener_stop: Arc::new(AtomicBool::new(false)),
            _pinger: None,
        };
        if listen {
            lan.listen(port, announce)?;
        }
        Ok(lan)
    }
    /// Take the login handshakes waiting for this server tick.
    pub fn take_handoffs(&self) -> Vec<Handoff> {
        self.shared
            .handoffs
            .as_ref()
            .map_or_else(Vec::new, |handoffs| {
                std::mem::take(&mut *handoffs.lock().unwrap_or_else(|e| e.into_inner()))
            })
    }
    /// Take registration requests before this server tick admits any guests.
    pub fn configuration_requests(
        &self,
    ) -> impl Iterator<Item = configuration_tasks::ConfigurationRequest> + '_ {
        self.configuration_requests
            .iter()
            .flat_map(Receiver::try_iter)
    }
    /// Take the admitted logins whose properties wait for `player_login_profile`.
    pub fn login_profile_requests(
        &self,
    ) -> impl Iterator<Item = login_profile::LoginProfileRequest> + '_ {
        self.login_profiles.iter().flat_map(Receiver::try_iter)
    }
    /// Start accepting connections on `port` (0 picks a free one), and
    /// announce the world on the local network when `announce`
    /// (`IntegratedServer.publishServer`'s LanServerPinger). Returns the port.
    pub fn listen(&mut self, port: u16, announce: bool) -> io::Result<u16> {
        if self.listener.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "already listening",
            ));
        }
        let listener = Self::bind(self.shared.config.bind, port)?;
        self.listen_on(listener, announce)
    }
    /// The socket [`Lan::listen`] accepts on, bound ahead so that a failure
    /// can be reported before the hub is at hand.
    pub fn bind(address: Option<std::net::IpAddr>, port: u16) -> io::Result<TcpListener> {
        let bind = address.unwrap_or_else(|| {
            if test_loopback() {
                std::net::Ipv4Addr::LOCALHOST.into()
            } else {
                std::net::Ipv4Addr::UNSPECIFIED.into()
            }
        });
        let listener = TcpListener::bind((bind, port))?;
        listener.set_nonblocking(true)?;
        Ok(listener)
    }
    /// [`Lan::listen`] on a socket already bound.
    pub fn listen_on(&mut self, listener: TcpListener, announce: bool) -> io::Result<u16> {
        if self.listener.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "already listening",
            ));
        }
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let (accept_shared, accept_stop) = (self.shared.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("sunrise-lan-accept".into())
            .spawn(move || accept_loop(listener, accept_shared, &accept_stop))?;
        self._pinger = if announce && !test_loopback() {
            discovery::Pinger::start(self.shared.motd(), port).ok()
        } else {
            None
        };
        self.listener = Some(thread);
        self.listener_stop = stop;
        self.port = port;
        Ok(port)
    }
    /// Stop accepting connections and announcing; connections already in
    /// (the local one included) stay.
    pub fn stop_listening(&mut self) {
        self.listener_stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.listener.take() {
            let _ = t.join();
        }
        self._pinger = None;
        self.port = 0;
    }
    /// Whether connections from the network are accepted.
    pub fn listening(&self) -> bool {
        self.listener.is_some()
    }
    /// `Connection.connectToLocalServer`: a connection through memory, its
    /// server end served on its own thread like an accepted socket.
    pub fn connect_local(&self) -> io::Result<sunrise_protocol::connection::Pipe> {
        let (client, server) = sunrise_protocol::connection::Pipe::pair();
        let shared = self.shared.clone();
        // Mod configuration payload handlers run on this thread.
        std::thread::Builder::new()
            .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
            .name("sunrise-local-conn".into())
            .spawn(move || {
                if let Err(e) = conn::serve_local(server, shared)
                    && std::env::var_os("SUNRISE_NET_DEBUG").is_some()
                {
                    log::warn!("[lan] local: {e}");
                }
            })?;
        Ok(client)
    }
    /// Everything the connection threads reported since the last call.
    pub fn poll(&self) -> Vec<crate::raw_payloads::ingress::OwnedReceipt<NetEvent>> {
        self.events.take_turn()
    }
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(handoffs) = &self.shared.handoffs {
            handoffs.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        self.stop_listening();
    }
}
impl Drop for Lan {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Connections from the network accepted beyond the server's player limit.
const SPARE_CONNECTIONS: usize = 64;

/// Connections from one address at once (a household or a school behind one address plays well
/// under it): past it a new one is closed, so that one machine can't take the places
/// [`SPARE_CONNECTIONS`] leaves for everyone's logins and server-list pings. Not for a local or
/// private address: a proxy, a tunnel or a container's port forward hands every player over from
/// one of those.
const PER_ADDRESS: usize = 10;

/// Whether connections from `ip` may be many: it is this machine or the local network's.
fn shared_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| shared_address(IpAddr::V4(v4)))
        }
    }
}

/// How often the log says how many connections were turned away, and who held the places.
const TURNED_AWAY_EVERY: Duration = Duration::from_secs(60);

/// How long a connection may wait before a full server closes it for a new one.
const MAKE_ROOM_AFTER: Duration = Duration::from_secs(2);

/// A network connection not playing yet.
struct Waiting {
    id: u64,
    since: Instant,
    stream: TcpStream,
}

thread_local! {
    /// The [`Waiting`] connection this thread serves.
    static WAITING: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// An accepted connection, uncounted once its thread ends (or couldn't start).
struct Counted(Arc<Shared>, IpAddr, u64);
impl Drop for Counted {
    fn drop(&mut self) {
        self.0
            .waiting
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|w| w.id != self.2);
        self.0.drop_handoff(self.2);
        let mut by = self.0.by_address.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = by.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                by.remove(&self.1);
            }
        }
        self.0.connections.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Shared {
    /// Counts a connection from `ip`, or None when `ip` holds its own most, or the server its
    /// most and no connection that waits could be closed to make room ([`Shared::make_room`]).
    fn count(self: &Arc<Self>, ip: IpAddr) -> Option<Counted> {
        let most = self.max_players().saturating_add(SPARE_CONNECTIONS);
        let here = |by: &HashMap<IpAddr, usize>| by.get(&ip).copied().unwrap_or(0);
        let by = self.by_address.lock().unwrap_or_else(|e| e.into_inner());
        if here(&by) >= PER_ADDRESS && !shared_address(ip) {
            return None;
        }
        // make_room logs, which reads by_address: never call it holding that lock.
        drop(by);
        if self.connections.load(Ordering::Acquire) >= most && !self.make_room() {
            return None;
        }
        let mut by = self.by_address.lock().unwrap_or_else(|e| e.into_inner());
        let n = here(&by) + 1;
        by.insert(ip, n);
        self.connections.fetch_add(1, Ordering::AcqRel);
        let id = self.next_connection.fetch_add(1, Ordering::Relaxed);
        Some(Counted(self.clone(), ip, id))
    }

    /// Waits for the server tick's decision on connection `id`'s `intention`: None when the
    /// connection was closed to make room or the server stopped first.
    fn hand_off(
        &self,
        id: u64,
        intention: Intention,
        peer_ip: Option<IpAddr>,
    ) -> Option<HandshakeDecision> {
        let (reply, decision) = channel();
        {
            let mut handoffs = self
                .handoffs
                .as_ref()?
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            // `Lan::stop` clears the list after setting `stop`, under this lock.
            if self.stop.load(Ordering::Relaxed) {
                return None;
            }
            handoffs.push(Handoff {
                id,
                intention,
                peer_ip,
                reply,
            });
        }
        decision.recv().ok()
    }

    /// Ends connection `id`'s wait for a handshake decision, if it waits.
    fn drop_handoff(&self, id: u64) {
        if let Some(handoffs) = &self.handoffs {
            handoffs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|h| h.id != id);
        }
    }

    /// Notes `stream`, counted as `counted`, as waiting until it plays.
    fn wait(&self, counted: &Counted, stream: &TcpStream) {
        if let Ok(stream) = stream.try_clone() {
            let w = Waiting {
                id: counted.2,
                since: Instant::now(),
                stream,
            };
            self.waiting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(w);
        }
    }

    /// The connection this thread serves plays now: a full server no longer closes it.
    pub(crate) fn entered_play(&self) {
        if let Some(id) = WAITING.take() {
            self.waiting
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .retain(|w| w.id != id);
        }
    }

    /// The server is full: closes the connection that has waited longest, if for more than
    /// [`MAKE_ROOM_AFTER`] (a login or a server-list ping takes a fraction of that), so that a
    /// crowd of sockets that never finish can't keep players out. Its thread ends at once. A
    /// player in the game is never closed.
    fn make_room(&self) -> bool {
        let mut waiting = self.waiting.lock().unwrap_or_else(|e| e.into_inner());
        let Some(oldest) = waiting
            .first()
            .filter(|w| w.since.elapsed() > MAKE_ROOM_AFTER)
        else {
            return false;
        };
        let _ = oldest.stream.shutdown(std::net::Shutdown::Both);
        let closed = waiting.remove(0).id;
        drop(waiting);
        self.drop_handoff(closed);
        self.turned_away();
        true
    }

    /// A connection was turned away: once a minute the log says how many, how many places were
    /// taken and by which addresses most.
    fn turned_away(&self) {
        let mut away = self.turned_away.lock().unwrap_or_else(|e| e.into_inner());
        away.0 += 1;
        let now = Instant::now();
        if away.1.is_some_and(|last| now < last + TURNED_AWAY_EVERY) {
            return;
        }
        let count = std::mem::take(&mut away.0);
        away.1 = Some(now);
        drop(away);
        let Some(log) = &self.config.log else {
            return;
        };
        let most = self.max_players().saturating_add(SPARE_CONNECTIONS);
        let before_play = self.waiting.lock().unwrap_or_else(|e| e.into_inner()).len();
        let closing = self.closing.load(Ordering::Acquire);
        let mut held: Vec<(IpAddr, usize)> = self
            .by_address
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(&ip, &n)| (ip, n))
            .collect();
        held.sort_by_key(|a| std::cmp::Reverse(a.1));
        let addresses = held.len();
        let top = held
            .iter()
            .take(5)
            .map(|(ip, n)| {
                if log.log_ips {
                    format!("{ip} ({n})")
                } else {
                    format!("<ip address withheld> ({n})")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        (log.write)(
            "Server Connection Thread",
            "WARN",
            &format!(
                "Turned away {count} connection(s), or closed one that waited to make room: {} of \
                 {most} places taken ({before_play} before play, {closing} closing), by \
                 {addresses} address(es), most by {top} (at most {PER_ADDRESS} an address)",
                self.connections.load(Ordering::Acquire)
            ),
        );
    }
}

fn accept_loop(listener: TcpListener, shared: Arc<Shared>, stop: &AtomicBool) {
    while !shared.stop.load(Ordering::Relaxed) && !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, addr)) => {
                // A thread a connection, where vanilla's Netty has a few for all: past the players
                // the server takes and [`SPARE_CONNECTIONS`] more (logins, server-list pings), a
                // new one is closed at once, so that a flood of sockets can't use up the threads a
                // container allows and crash the server.
                let Some(counted) = shared.count(addr.ip()) else {
                    drop(stream);
                    shared.turned_away();
                    continue;
                };
                shared.wait(&counted, &stream);
                // Mod configuration payload handlers run on this thread.
                let _ = std::thread::Builder::new()
                    .stack_size(sunrise_threads::GAME_THREAD_STACK_BYTES)
                    .name("sunrise-lan-conn".into())
                    .spawn(move || {
                        WAITING.set(Some(counted.2));
                        handle(stream, addr, counted);
                    });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
fn handle(stream: TcpStream, addr: SocketAddr, counted: Counted) {
    let _ = stream.set_nonblocking(false);
    let shared = counted.0.clone();
    if let Err(e) = conn::serve(stream, addr, shared, counted)
        && std::env::var_os("SUNRISE_NET_DEBUG").is_some()
    {
        log::warn!("[lan] {addr}: {e}");
    }
}

/// HttpUtil.getAvailablePort: a free local port.
pub fn available_port() -> u16 {
    let bind = if cfg!(test) || test_loopback() {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    };
    TcpListener::bind((bind, 0))
        .and_then(|l| l.local_addr())
        .map_or(25565, |a| a.port())
}

/// Same-machine harnesses opt in; real LAN runs keep the normal network interfaces.
pub(super) fn test_loopback() -> bool {
    std::env::var_os("SUNRISE_TEST_LOOPBACK").is_some_and(|v| v == "1")
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn a_local_server_accepts_only_the_requested_loopback_address() {
        let lan = super::Lan::start_with(
            std::sync::Arc::new(super::data::SyncData::empty()),
            super::NetConfig::loopback(0, "Local test".into(), 1),
        )
        .unwrap();
        let local = std::net::TcpStream::connect(("127.0.0.1", lan.port)).unwrap();
        let other = "127.0.0.2".parse::<std::net::IpAddr>().unwrap();
        assert!(
            std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::new(other, lan.port),
                std::time::Duration::from_millis(500),
            )
            .is_err()
        );
        drop(local);
    }
    use super::conn::MEMORY_ADDRESS;
    use super::*;
    use std::{io::Read, time::Instant};

    #[test]
    fn a_handshake_wait_ends_when_its_connection_closes_or_the_server_stops() {
        let config = NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::lan(0, "handoffs".into(), 1)
        };
        let mut lan =
            Lan::start_listener(Arc::new(data::SyncData::empty()), config, true, false).unwrap();
        let intention = sunrise_protocol::packets::handshake::Intention {
            host: "entry.test".into(),
            port: 25565,
            protocol: sunrise_protocol::PROTOCOL_VERSION,
            intent: sunrise_protocol::packets::handshake::LOGIN,
        };
        let wait = |id| {
            let (shared, intention) = (lan.shared.clone(), intention.clone());
            std::thread::spawn(move || shared.hand_off(id, intention, None))
        };
        let (closed, stopped) = (wait(1), wait(2));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while lan.shared.handoffs.as_ref().unwrap().lock().unwrap().len() < 2 {
            assert!(
                Instant::now() < deadline,
                "the handshakes were not handed off"
            );
            std::thread::yield_now();
        }
        // The path a connection closed to make room takes.
        lan.shared.drop_handoff(1);
        assert!(closed.join().unwrap().is_none());
        lan.stop();
        assert!(stopped.join().unwrap().is_none());
        assert!(lan.shared.hand_off(3, intention, None).is_none());
    }

    #[test]
    fn stale_configuration_connection_cannot_address_replacement_lan_with_same_local_id() {
        let config = || NetConfig {
            listen: false,
            announce: false,
            ..NetConfig::lan(0, "config-incarnation".into(), 1)
        };
        let lan_a =
            Lan::start_listener(Arc::new(data::SyncData::empty()), config(), true, false).unwrap();
        let mut lan_b =
            Lan::start_listener(Arc::new(data::SyncData::empty()), config(), true, false).unwrap();
        let conn_a = lan_a.shared.next_configuration_connection().unwrap();
        let conn_b = lan_b.shared.next_configuration_connection().unwrap();
        let (inc_a, local_a) = mod_api::__private::configuration_connection_parts(conn_a);
        let (inc_b, local_b) = mod_api::__private::configuration_connection_parts(conn_b);
        assert_eq!(local_a, 1);
        assert_eq!(local_b, 1);
        assert_ne!(inc_a, inc_b);
        assert_ne!(conn_a, conn_b);

        let (sender, _peer) =
            config_payloads::ConfigSender::test_sender(crate::raw_payloads::Transport::empty());
        let session = configuration_tasks::Session::new(sender);
        lan_b
            .configuration_tasks
            .insert(&configuration_tasks::ConfigurationRequest {
                connection: conn_b,
                player: 1,
                name: "Peer".into(),
                session: session.clone(),
            });
        assert!(lan_b.configuration_tasks.get(conn_b).is_some());
        assert!(lan_b.configuration_tasks.get(conn_a).is_none());
        assert!(lan_b.configuration_tasks.take_task(conn_a, 0).is_none());
        assert!(lan_b.configuration_tasks.remove(conn_a).is_empty());
        assert!(lan_b.configuration_tasks.get(conn_b).is_some());
    }

    #[test]
    #[ignore = "daily"]
    fn actor_ids_never_wrap() {
        let config = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "ids".into(), 1)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        lan.shared.next.store(u32::MAX - 1, Ordering::Relaxed);
        assert_eq!(lan.shared.next_id(), Some(u32::MAX - 1));
        assert_eq!(lan.shared.next_id(), None);
        assert_eq!(lan.shared.next_id(), None);
    }

    /// Sockets that never say a word each hold a connection thread until their read times out: past
    /// the player limit and [`SPARE_CONNECTIONS`], a new one is closed at once instead of taking one
    /// more thread, and the room comes back as they go.
    #[test]
    #[ignore = "daily"]
    fn a_flood_of_silent_sockets_is_turned_away() {
        let config = NetConfig {
            announce: false,
            ..NetConfig::loopback(0, "Flood".into(), 1)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        let address = (std::net::Ipv4Addr::LOCALHOST, lan.port);
        let most = 1 + SPARE_CONNECTIONS;
        let held: Vec<TcpStream> = (0..most)
            .map(|_| TcpStream::connect(address).unwrap())
            .collect();
        let deadline = Instant::now() + Duration::from_secs(10);
        while lan.shared.connections.load(Ordering::Acquire) < most {
            assert!(
                Instant::now() < deadline,
                "the held sockets were not all accepted"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut late = TcpStream::connect(address).unwrap();
        late.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut byte = [0u8; 1];
        assert!(
            match late.read(&mut byte) {
                Ok(n) => n == 0,
                Err(e) => !matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ),
            },
            "the socket past the limit should be closed"
        );
        assert_eq!(lan.shared.connections.load(Ordering::Acquire), most);
        drop(held);
        while lan.shared.connections.load(Ordering::Acquire) > 0 {
            assert!(
                Instant::now() < deadline,
                "the closed sockets' threads did not end"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Once a stop waits for a proxy, no login takes a place, an op's neither.
    #[test]
    fn a_server_that_refuses_logins_takes_no_one_in() {
        let config = NetConfig {
            announce: false,
            listen: false,
            bypasses_limit: Some(Arc::new(|_| true)),
            ..NetConfig::loopback(0, "Stopping".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 1, "Player").is_none());
        lan.shared.refuse_logins();
        assert_eq!(
            lan.shared.try_admit(MEMORY_ADDRESS, 2, "Late"),
            Some(translatable("multiplayer.disconnect.server_shutdown", &[]))
        );
        assert_eq!(lan.shared.online(), 1);
    }

    /// `IntegratedPlayerList.canPlayerLogin`: a login whose name is the
    /// singleplayer owner's (`IntegratedServer.isSingleplayerOwner`, ignoring
    /// case) is refused with `multiplayer.disconnect.name_taken` while a player
    /// of that name is online. The owner's own first login, and any other name,
    /// get in; a server with no owner profile (a dedicated one) has no such rule.
    #[test]
    fn a_login_with_the_owners_name_is_refused_while_the_owner_is_online() {
        let owned = NetConfig {
            announce: false,
            listen: false,
            singleplayer_profile: Some(("Owner".to_owned(), 1)),
            ..NetConfig::loopback(0, "Owned".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), owned).unwrap();
        let taken = Some(translatable("multiplayer.disconnect.name_taken", &[]));
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 1, "Owner").is_none());
        assert_eq!(lan.shared.try_admit(MEMORY_ADDRESS, 2, "oWNER"), taken);
        assert_eq!(lan.shared.try_admit(MEMORY_ADDRESS, 1, "Owner"), taken);
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 3, "Guest").is_none());
        assert_eq!(lan.shared.online(), 2);

        let dedicated = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::loopback(0, "Dedicated".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), dedicated).unwrap();
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 1, "Owner").is_none());
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 2, "oWNER").is_none());
    }

    #[test]
    fn hide_online_players_updates_status_response() {
        let config = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::loopback(0, "Status".into(), 8)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        assert!(lan.shared.try_admit(MEMORY_ADDRESS, 1, "Player").is_none());
        assert!(lan.shared.status_json().contains("\"sample\""));
        lan.shared.set_hide_online_players(true);
        let hidden = lan.shared.status_json();
        assert!(!hidden.contains("\"sample\""));
        assert!(!hidden.contains("Player"));
    }

    /// One public address takes at most [`PER_ADDRESS`] places, its own coming back as they end;
    /// another address, or a local one (a proxy hands everyone over from it), still gets one.
    #[test]
    fn one_address_takes_at_most_its_share() {
        let config = NetConfig {
            announce: false,
            listen: false,
            ..NetConfig::loopback(0, "Share".into(), 20)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let mut one: Vec<Counted> = (0..PER_ADDRESS)
            .map(|_| lan.shared.count(ip("203.0.113.7")).unwrap())
            .collect();
        assert!(lan.shared.count(ip("203.0.113.7")).is_none());
        assert!(lan.shared.count(ip("198.51.100.2")).is_some());
        let local: Vec<Counted> = (0..PER_ADDRESS + 5)
            .map(|_| lan.shared.count(ip("127.0.0.1")).unwrap())
            .collect();
        assert!(lan.shared.count(ip("172.17.0.1")).is_some());
        one.pop();
        assert!(lan.shared.count(ip("203.0.113.7")).is_some());
        drop((one, local));
        assert!(lan.shared.by_address.lock().unwrap().is_empty());
        assert_eq!(lan.shared.connections.load(Ordering::Acquire), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn a_full_server_closes_the_connection_that_waited_longest() {
        use std::io::Read;
        // With a log writer, as on a dedicated server: making room logs, and must not deadlock.
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let into = lines.clone();
        let config = NetConfig {
            announce: false,
            listen: false,
            log: Some(NetLog {
                write: Arc::new(move |_, _, line| into.lock().unwrap().push(line.to_owned())),
                lang: Arc::new(Default::default()),
                log_ips: true,
            }),
            ..NetConfig::loopback(0, "Room".into(), 1)
        };
        let lan = Lan::start_with(Arc::new(data::SyncData::empty()), config).unwrap();
        let shared = &lan.shared;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server_side, _) = listener.accept().unwrap();
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let most = shared.max_players() + SPARE_CONNECTIONS;
        let mut held: Vec<Counted> = (0..most).map(|_| shared.count(ip).unwrap()).collect();
        // A player who reached the game, and a socket that never finished its login.
        let playing = shared.count(ip);
        assert!(playing.is_none(), "full");
        shared.wait(&held[1], &server_side);
        assert!(shared.count(ip).is_none(), "a fresh wait isn't closed");
        shared.waiting.lock().unwrap()[0].since -= MAKE_ROOM_AFTER * 2;
        let newcomer = shared.count(ip).expect("room made");
        let mut byte = [0u8; 1];
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        assert_eq!(
            client.read(&mut byte).unwrap(),
            0,
            "the waiting socket was closed"
        );
        assert!(shared.waiting.lock().unwrap().is_empty());
        assert!(shared.count(ip).is_none(), "nothing else to close");
        let logged = lines.lock().unwrap().clone();
        assert_eq!(logged.len(), 1, "one report a minute: {logged:?}");
        assert!(logged[0].contains("127.0.0.1"), "{logged:?}");
        // Its thread ends and uncounts it; a connection that plays is never closed.
        held.remove(1);
        shared.wait(&newcomer, &server_side);
        WAITING.set(Some(newcomer.2));
        shared.entered_play();
        assert!(shared.waiting.lock().unwrap().is_empty());
        drop((held, newcomer));
        assert_eq!(shared.connections.load(Ordering::Acquire), 0);
    }
}
