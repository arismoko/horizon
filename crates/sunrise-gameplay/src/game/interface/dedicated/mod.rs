//! The dedicated server (net.minecraft.server.Main and DedicatedServer):
//! `sunrise-client --dedicated [--nogui] [--port N] [--world NAME]
//! [--universe DIR] [--initSettings]`. It reads server.properties and
//! eula.txt from the working directory, opens `<universe>/<level-name>` and
//! runs the same game as single player, without a local player: the world,
//! its simulation and the play-state hub that LAN hosting uses, configured
//! from server.properties, with the operator, whitelist and ban lists and a
//! console on standard input.
use super::{Options, lan::LanSettings, world_loading::WorldRequest};
use mod_api::PermissionLevel;
use std::{
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use sunrise_core::nbt::{JavaString, Tag};
use sunrise_game_data::{Result, text::Text};
use sunrise_perf_format::v1::{StopReason, Trigger};
use sunrise_protocol::text::translatable;
use sunrise_server_files::{
    eula,
    properties::{GAME_MODES, ServerProperties, SystemEntropy},
    user_cache::UserCache,
    user_lists::{
        BanEntry, BanList, Change, IpBanList, NameAndId, OpEntry, OpList, WhiteEntry, WhiteList,
        format_date,
    },
};
use sunrise_sim::lan::{
    LoginGate, NetConfig, NetLog, OnlineMode, SessionVerifier,
    hub::{HubConfig, effective_view_distance},
    viewed_levels::areas as viewed,
};

pub(in crate::game) mod biome_regions;
mod missing_mods;
pub use missing_mods::is_missing_mods_refusal;
pub(in crate::game) mod events;
mod management;
use sunrise_gameplay::dedicated::predicate_graph;
pub mod report;
mod status;
mod transfer;

pub struct Services {
    pub find_name: fn(&str) -> Option<NameAndId>,
    pub find_id: fn(u128) -> Option<NameAndId>,
    pub profile_keys: fn() -> Vec<sunrise_protocol::rsa::RsaPublicKey>,
    pub verify_session: SessionVerifier,
}

/// What the tick loop tells `ServerWatchdog`: when the tick in progress was
/// due (`getNextTickTime`), how long a tick is, and whether the server runs.
struct Watchdog {
    /// The thread running the ticks, whose stack the report carries.
    server: sunrise_threads::stack_dump::Watched,
    save_failure_diagnostics: sunrise_sim::chunk_io::SaveFailureDiagnostics,
    base: Instant,
    next_nanos: AtomicU64,
    tick_nanos: AtomicU64,
    running: AtomicBool,
}

/// Stop monitoring on every exit from the tick loop, including an error or panic.
struct StopWatchdog(Option<Arc<Watchdog>>);

impl Drop for StopWatchdog {
    fn drop(&mut self) {
        if let Some(watchdog) = &self.0 {
            watchdog.running.store(false, Ordering::Relaxed);
        }
    }
}

impl Watchdog {
    /// `DedicatedServer.initServer`: a thread that stops a server whose tick
    /// runs `max_tick_time` ms past its due time; none when that is 0 or less.
    /// Started from the thread that runs the ticks.
    fn start(
        max_tick_time: i64,
        save_failure_diagnostics: sunrise_sim::chunk_io::SaveFailureDiagnostics,
    ) -> Option<Arc<Self>> {
        let max = Duration::from_millis(u64::try_from(max_tick_time).ok().filter(|m| *m > 0)?);
        let watchdog = Arc::new(Self {
            server: sunrise_threads::stack_dump::Watched::current(),
            save_failure_diagnostics,
            base: Instant::now(),
            next_nanos: AtomicU64::new(0),
            tick_nanos: AtomicU64::new(50_000_000),
            running: AtomicBool::new(true),
        });
        let shared = watchdog.clone();
        std::thread::Builder::new()
            .name("Server Watchdog".into())
            .spawn(move || shared.run(max))
            .ok()?;
        Some(watchdog)
    }
    /// The tick loop is about to run the tick due at `next`.
    fn tick_due(&self, next: Instant, tick: Duration) {
        let nanos = next.saturating_duration_since(self.base).as_nanos();
        self.next_nanos
            .store(u64::try_from(nanos).unwrap_or(u64::MAX), Ordering::Relaxed);
        let tick = u64::try_from(tick.as_nanos()).unwrap_or(u64::MAX);
        self.tick_nanos.store(tick, Ordering::Relaxed);
    }
    /// `ServerWatchdog.run`.
    fn run(&self, max: Duration) {
        let max = u64::try_from(max.as_nanos()).unwrap_or(u64::MAX);
        while self.running.load(Ordering::Relaxed) {
            let next = self.next_nanos.load(Ordering::Relaxed);
            let now = u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX);
            let behind = now.saturating_sub(next);
            if behind > max {
                // ServerWatchdog: the server thread's stack first, while it is still stuck.
                let stack = self.server.stack(Duration::from_secs(5));
                let took = format!("{:.2}", behind as f32 / 1e9);
                let should = format!(
                    "{:.2}",
                    self.tick_nanos.load(Ordering::Relaxed) as f32 / 1e9
                );
                log_from(
                    "Server Watchdog",
                    "ERROR",
                    &format!("A single server tick took {took} seconds (should be max {should})"),
                );
                log_from(
                    "Server Watchdog",
                    "ERROR",
                    "Considering it to be crashed, server will forcibly shutdown.",
                );
                let message = format!(
                    "ServerWatchdog detected that a single server tick took {took} seconds (should be max {should})"
                );
                match sunrise_level::crash::report_watchdog(
                    std::path::Path::new("."),
                    "server",
                    message,
                    stack,
                    || self.save_failure_diagnostics.suppressed_summary(),
                ) {
                    Some(path) => log_from(
                        "Server Watchdog",
                        "ERROR",
                        &format!(
                            "This crash report has been saved to: {}",
                            path.canonicalize().unwrap_or(path).display()
                        ),
                    ),
                    None => log_from(
                        "Server Watchdog",
                        "ERROR",
                        "We were unable to save this crash report to disk.",
                    ),
                }
                std::process::exit(1);
            }
            // Thread.sleep(nextTickTime + maxTickTime - now).
            let wait = (next + max).saturating_sub(now).max(1_000_000);
            std::thread::sleep(Duration::from_nanos(wait));
        }
    }
}

/// What the startup read before the game's resources were loaded.
pub struct Setup {
    pub props: ServerProperties,
    pub universe: PathBuf,
    pub level_name: String,
    pub port: Option<u16>,
    /// `--allowMissingMods`: open a world whose saved mods are missing (`missing_mods`).
    pub allow_missing_mods: bool,
}
static SETUP: OnceLock<Mutex<Option<Setup>>> = OnceLock::new();

/// What the start answers when the server folder still has a `forwarding.secret`.
pub const FORWARDING_REMOVED: &str = "Velocity forwarding was removed from Sunrise; delete forwarding.secret from the \
     server folder (with online-mode=false, anyone who can reach the port then joins under any name)";

/// A `forwarding.secret` in `dir` stops the start: Velocity forwarding is gone, and an owner who relied on it
/// to keep an offline server closed must not find it silently open.
pub fn refuse_forwarding_secret(dir: &std::path::Path) -> std::result::Result<(), String> {
    if dir.join("forwarding.secret").exists() {
        return Err(FORWARDING_REMOVED.to_owned());
    }
    Ok(())
}

/// `DedicatedServer.initServer`: elapsed level-load nanoseconds as ROOT `%.3f` seconds.
fn startup_done(elapsed: Duration) -> String {
    let seconds = sunrise_core::java_number::format_fixed(elapsed.as_nanos() as f64 / 1e9, 3);
    format!("Done ({seconds}s)! For help, type \"help\"")
}

/// `MinecraftServer.runServer`'s overload warning for a server `behind` its
/// schedule, in milliseconds and in ticks of `tick`.
pub fn overload_warning(behind: Duration, tick: Duration) -> String {
    format!(
        "Can't keep up! Is the server overloaded? Running {}ms or {} ticks behind",
        behind.as_millis(),
        behind.as_nanos() / tick.as_nanos().max(1)
    )
}

/// `TpsDebugDimensions`: the full tick (the time between tick ends, as measured), the tick
/// method, the scheduled tasks (the wait minus its idle time) and the idle time.
fn tick_time_sample(full: Duration, method: Duration, wait: Duration, idle: Duration) -> [i64; 4] {
    let nanos = |d: Duration| d.as_nanos() as i64;
    [
        nanos(full),
        nanos(method),
        nanos(wait.saturating_sub(idle)),
        nanos(idle),
    ]
}

/// `ServerGamePacketListenerImpl.tick`'s idle kick: a player whose last action is more than
/// `minutes` (`player-idle-timeout`, none when 0 or less) behind, counted here in ticks, is kicked,
/// unless the End poem is showing (`ServerPlayer.wonGame`): its client sends no action until it
/// asks to respawn.
fn idles_out(idle_ticks: u64, minutes: i32, won_game: bool) -> bool {
    let Ok(minutes @ 1..) = u64::try_from(minutes) else {
        return false;
    };
    !won_game && idle_ticks > minutes * 1200
}

/// Log4j's console pattern: `[HH:mm:ss] [thread/LEVEL]: message`.
pub fn log(level: &str, message: &str) {
    log_from("Server thread", level, message);
}

/// Public chat goes to the server log, never to the RCON or console command result.
pub(in crate::game) fn log_public_chat(message: &str) {
    let held = CAPTURE.take();
    log("INFO", message);
    CAPTURE.set(held);
}

/// A line from another of vanilla's threads (the login's User Authenticator).
pub fn log_from(thread: &str, level: &str, message: &str) {
    let file_level = match level {
        "WARN" => ::log::Level::Warn,
        "ERROR" => ::log::Level::Error,
        _ => ::log::Level::Info,
    };
    sunrise_level::logging::write_mirrored_as(thread, file_level, message);
    CAPTURE.with_borrow_mut(|lines| {
        if let Some(lines) = lines {
            lines.push(message.to_owned());
        }
    });
    // Log4j's %d{HH:mm:ss}: the local time, as in logs/latest.log.
    // The launcher can close its stdout pipe while the server keeps running.
    let _ = writeln!(
        std::io::stdout().lock(),
        "[{}] [{thread}/{level}]: {message}",
        chrono::Local::now().format("%H:%M:%S")
    );
}

thread_local! {
    /// The lines logged while a control request runs a console command.
    static CAPTURE: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// An INFO line in the log that an RCON command's reply (`rcon`: one is
/// running) does not take, as `dedicated_console_say`'s.
pub(in crate::game::server_side) fn log_outside_rcon_reply(rcon: bool, message: &str) {
    let held = rcon.then(|| CAPTURE.take());
    log("INFO", message);
    if let Some(held) = held {
        CAPTURE.set(held);
    }
}

/// Runs `f` and returns the lines it logged.
pub fn capture_log(f: impl FnOnce()) -> Vec<String> {
    CAPTURE.set(Some(Vec::new()));
    f();
    CAPTURE.take().unwrap_or_default()
}

/// Main.main up to the server start: arguments, server.properties (stored
/// back at once), eula.txt. False when the process should stop here.
pub fn preflight(options: &mut Options) -> Result<bool> {
    let args = options.dedicated.clone().unwrap_or_default();
    let mut universe = PathBuf::from(".");
    let mut world = None;
    let mut port = None;
    let mut init_settings = false;
    let mut allow_missing_mods = false;
    for argument in sunrise_level::runtime_env::dedicated_arguments::parse(&args) {
        let (name, value) = argument?;
        match name {
            "nogui"
            | "demo"
            | "bonusChest"
            | "forceUpgrade"
            | "eraseCache"
            | "recreateRegionFiles"
            | "safeMode"
            | "jfrProfile"
            | "game-tests" => {}
            "initSettings" => init_settings = true,
            name if name == sunrise_sim::save::level_mods::ALLOW_FLAG => allow_missing_mods = true,
            "help" => {
                let allow = missing_mods::in_english(|| {
                    sunrise_game_data::lang::get("sunrise.missing_mods.server.help")
                });
                eprintln!(
                    "Option                Description\n------                -----------\n--allowMissingMods    {allow}\n--help\n--game-tests          Enables test-only mod content (or SUNRISE_GAME_TESTS=1/true)\n--initSettings        Initializes 'server.properties' and 'eula.txt', then quits\n--nogui\n--port <Integer>      (default: -1)\n--universe <String>   (default: .)\n--world <String>"
                );
                return Ok(false);
            }
            "universe" => universe = PathBuf::from(value.expect("parsed operand")),
            "world" => world = Some(value.expect("parsed operand").to_owned()),
            "port" => {
                let v: i32 = value.expect("parsed operand").parse()?;
                port = u16::try_from(v).ok();
            }
            "serverId" | "pidFile" => {
                let value = value.expect("parsed operand");
                if name == "pidFile" {
                    std::fs::write(value, std::process::id().to_string())?;
                }
            }
            _ => return Err(format!("{name} is not a recognized option").into()),
        }
    }
    let settings = PathBuf::from("server.properties");
    let props = ServerProperties::from_file(&settings, &mut SystemEntropy::default())?;
    for w in &props.warnings {
        log("WARN", w);
    }
    // A dedicated server's tree: /op, /ban, /whitelist... (single player's
    // server thread is an integrated one and never gets here).
    sunrise_command::vanilla::set_dedicated_commands(true);
    // Sunrise's own commands are the owner's choice, off as on vanilla.
    if props
        .raw
        .get(sunrise_command::vanilla::PREGEN_PROPERTY)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
    {
        sunrise_command::vanilla::set_sunrise_commands(true);
    }
    props.store(&settings)?;
    // `RegionFileVersion.configure`: the compression every region file this
    // server writes uses, from `region-file-compression`. An unknown name is
    // logged with the ones there are, and the default (deflate) stays.
    if !sunrise_world::compression::Compression::configure(&props.region_file_compression) {
        log(
            "ERROR",
            &format!(
                "Invalid `region-file-compression` value `{}` in server.properties. Please use one of: {}",
                props.region_file_compression,
                sunrise_world::compression::REGION_FILE_NAMES.join(",")
            ),
        );
    }
    let eula_file = PathBuf::from("eula.txt");
    let agreed = match eula::agreed(&eula_file) {
        Ok(a) => a,
        Err(w) => {
            log("WARN", &w);
            false
        }
    };
    if init_settings {
        let abs = |p: &PathBuf| std::env::current_dir().map_or(p.clone(), |d| d.join(p));
        log(
            "INFO",
            &format!(
                "Initialized '{}' and '{}'",
                abs(&settings).display(),
                abs(&eula_file).display()
            ),
        );
        return Ok(false);
    }
    if !agreed {
        log(
            "INFO",
            "You need to agree to the EULA in order to run the server. Go to eula.txt for more info.",
        );
        return Ok(false);
    }
    let level_name = world.unwrap_or_else(|| props.level_name.clone());
    SETUP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .replace(Setup {
            props,
            universe,
            level_name,
            port,
            allow_missing_mods,
        });
    Ok(true)
}

/// The event a change of a list is (`operators_added`, `bans_removed`...), and the entry it is about.
fn listed<E>(list: &str, change: Change<E>) -> (String, E) {
    match change {
        Change::Added(e) => (format!("{list}_added"), e),
        Change::Removed(e) => (format!("{list}_removed"), e),
    }
}

/// What a ban says besides who it is for: the reason and the source, and when it ends (milliseconds, or none).
fn ban_fields<T>(ban: &BanEntry<T>) -> serde_json::Value {
    serde_json::json!({ "reason": ban.reason, "source": ban.source, "expires": ban.expires })
}

/// The operator, whitelist and ban lists (DedicatedPlayerList), shared with
/// the connection threads that check logins, and the server's own switches
/// that an operator's command changes.
pub struct Lists {
    pub ops: OpList,
    pub white: WhiteList,
    pub bans: BanList,
    pub ip_bans: IpBanList,
    pub use_whitelist: bool,
    pub enforce_whitelist: bool,
    /// `op-permission-level`: what `/op` grants (operatorUserPermissions).
    pub op_level: i32,
    /// `MinecraftServer.playerIdleTimeout`: the minutes of idleness after
    /// which a player is kicked, `player-idle-timeout` at the start and
    /// `/setidletimeout` after it.
    pub player_idle_timeout: i32,
    /// `/stop` (MinecraftServer.halt): the server loop ends after this tick.
    pub stop: bool,
    /// `MinecraftServer.isAutoSave`: all of Sunrise's server levels share this save flag.
    pub autosave: bool,
}

impl Lists {
    /// The console's `/ban` of one player (`BanPlayerCommands.banPlayers`): whether they are banned now.
    /// One banned already (`isBanned`) is left as they are, their first reason kept.
    fn ban(&mut self, user: &NameAndId, reason: Option<String>) -> bool {
        !self.bans.contains(user)
            && self.bans.add(BanEntry::new(
                user.clone(),
                Some("Server".into()),
                None,
                reason,
            ))
    }

    /// The console's `/ban-ip` (`BanIpCommands.banIp`): whether the address is banned now. One banned
    /// already (`isBanned`) is left as it is.
    fn ban_ip(&mut self, ip: &str, reason: Option<String>) -> bool {
        let ip = ip.to_owned();
        !self.ip_bans.contains(&ip)
            && self
                .ip_bans
                .add(BanEntry::new(ip, Some("Server".into()), None, reason))
    }

    fn load() -> Self {
        let mut l = Self {
            ops: OpList::new("ops.json"),
            white: WhiteList::new("whitelist.json"),
            bans: BanList::new("banned-players.json"),
            ip_bans: IpBanList::new("banned-ips.json"),
            use_whitelist: false,
            enforce_whitelist: false,
            op_level: 4,
            player_idle_timeout: 0,
            stop: false,
            autosave: true,
        };
        // Each list tells the event stream what changes in it, as vanilla's tell their `NotificationService`.
        l.ops.watch(|change| {
            let (kind, e) = listed("operators", change);
            events::record(
                &kind,
                &e.user.name,
                e.user.id,
                serde_json::json!({ "level": e.level, "bypassesPlayerLimit": e.bypasses_player_limit }),
            );
        });
        l.white.watch(|change| {
            let (kind, e) = listed("allowlist", change);
            events::record(&kind, &e.user.name, e.user.id, serde_json::json!({}));
        });
        l.bans.watch(|change| {
            let (kind, e) = listed("bans", change);
            events::record(&kind, &e.user.name, e.user.id, ban_fields(&e));
        });
        l.ip_bans.watch(|change| {
            let (kind, e) = listed("ip_bans", change);
            let mut fields = ban_fields(&e);
            fields["ip"] = e.user.clone().into();
            events::record_server(&kind, fields);
        });
        // DedicatedPlayerList's constructor order and its LOGGER.warn lines.
        let warn = |what: &str, r: std::result::Result<(), String>| {
            if let Err(e) = r {
                log("WARN", &format!("Failed to {what}: {e}"));
            }
        };
        let saved = |r: std::io::Result<()>| r.map_err(|e| e.to_string());
        warn("load user banlist", l.bans.load());
        warn("save user banlist", saved(l.bans.save()));
        warn("load ip banlist", l.ip_bans.load());
        warn("save ip banlist", saved(l.ip_bans.save()));
        warn("load operators list", l.ops.load());
        warn("load white-list", l.white.load());
        warn("save operators list", saved(l.ops.save()));
        if !l.white.file.exists() {
            warn("save white-list", saved(l.white.save()));
        }
        l
    }
    /// PlayerList.isWhiteListed.
    fn whitelisted(&self, user: &NameAndId) -> bool {
        !self.use_whitelist || self.ops.contains(user) || self.white.contains(user)
    }
    /// PlayerList.canPlayerLogin before the player count.
    fn can_login(&mut self, address: std::net::SocketAddr, user: &NameAndId) -> Option<Tag> {
        if self.bans.contains(user)
            && let Some(ban) = self.bans.get(user).cloned()
        {
            return Some(ban_message(&ban));
        }
        if !self.whitelisted(user) {
            return Some(translatable("multiplayer.disconnect.not_whitelisted", &[]));
        }
        let ip = address.ip().to_string();
        if self.ip_bans.contains(&ip)
            && let Some(ban) = self.ip_bans.get(&ip).cloned()
        {
            let mut reason = translatable_tag(
                "multiplayer.disconnect.banned_ip.reason",
                vec![ban_reason(&ban.reason)],
            );
            if let Some(expires) = ban.expires {
                append(
                    &mut reason,
                    translatable_tag(
                        "multiplayer.disconnect.banned_ip.expiration",
                        vec![Tag::String(JavaString::from(ban_date(expires).as_str()))],
                    ),
                );
            }
            return Some(reason);
        }
        None
    }
}

/// DedicatedServerProperties.LEGACY_PRESET_NAMES: the old `level-type`
/// names, before the presets had their own ids.
const LEGACY_PRESET_NAMES: [(&str, sunrise_sim::save::WorldPreset); 2] = [
    ("default", sunrise_sim::save::WorldPreset::Normal),
    ("largebiomes", sunrise_sim::save::WorldPreset::LargeBiomes),
];

/// A tick slower than this, in milliseconds, enters the slow ring (five ticks' worth, a stall players feel) unless `SUNRISE_SPIKES` names another.
pub const SLOW_TICK_MS: f32 = 250.0;
/// `MinecraftServer.OVERLOADED_WARNING_INTERVAL_NANOS` plus 100 ticks: the
/// "Can't keep up!" warning comes at most this often.
const OVERLOAD_WARNING_INTERVAL: Duration = Duration::from_secs(15);
/// server.properties' line for `server.toml`'s `admin_feedback_to_ops` (sunrise-server writes it).
const ADMIN_FEEDBACK_PROPERTY: &str = "sunrise-admin-feedback-to-ops";

/// Identifier.tryParse: `namespace:path`, or a path in `minecraft`.
fn parse_identifier(text: &str) -> Option<String> {
    let (namespace, path) = text.split_once(':').unwrap_or(("minecraft", text));
    let valid = |s: &str, extra: &[char]| {
        s.chars()
            .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-') || extra.contains(&c))
    };
    (valid(namespace, &[]) && valid(path, &['/'])).then(|| format!("{namespace}:{path}"))
}

/// The world's packs failed to load: the start ends before Ready, as vanilla's `Main` does.
#[derive(Debug)]
struct DatapacksRefused;
impl std::fmt::Display for DatapacksRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Failed to load datapacks, can't proceed with server load")
    }
}
impl std::error::Error for DatapacksRefused {}

impl crate::game::ServerGame<'_> {
    /// Add the adopted packs' English at the dedicated server-starting notice.
    /// World and registry-load listeners still see only the built-in language.
    pub fn start_loaded_dedicated_world(
        &mut self,
        packs: &sunrise_gameplay_common::game::function_pack_stack::PackStack,
    ) {
        use sunrise_game_data::lang::{self, Language, ResourceSource};

        let layers = self
            .common
            .pack
            .mod_layers
            .iter()
            .chain(
                packs
                    .enabled_packs()
                    .map(sunrise_level::datapack::DataPack::source),
            )
            .map(|source| source as &dyn ResourceSource);
        let language = Language::load_server(&self.common.ui.vanilla_root, layers, |value| {
            sunrise_sim::text_grammar::build(value, &self.common.ui.registries, false)
        });
        lang::set(language);
        self.native_server_starting(true);
    }

    /// `LevelStorageAccess.hasWorldData`: a backup alone still belongs to an
    /// existing world. Let world loading recover it instead of replacing the
    /// world's saved generator settings with server.properties. Returns whether
    /// the world is new.
    pub fn prepare_world_metadata(
        &mut self,
        dir: &std::path::Path,
        props: &ServerProperties,
        allow_missing_mods: bool,
    ) -> Result<bool> {
        let new_world = !dir.join("level.dat").exists() && !dir.join("level.dat_old").exists();
        if new_world {
            self.write_world_gen(dir, props)?;
        } else {
            // Before anything writes the world: a refusal leaves it as it was saved.
            self.check_missing_mods(dir, allow_missing_mods)?;
        }
        Ok(new_world)
    }

    /// DedicatedServerProperties.createDimensions for a world that doesn't
    /// exist yet: `level-type`'s preset, a flat world's `generator-settings`
    /// and `generate-structures`, written before the world opens so that
    /// streaming reads them.
    fn write_world_gen(&self, dir: &std::path::Path, props: &ServerProperties) -> Result<()> {
        let level_type = &props.level_type;
        let preset = parse_identifier(level_type)
            .as_deref()
            .and_then(sunrise_sim::save::WorldPreset::of_id)
            .or_else(|| {
                LEGACY_PRESET_NAMES
                    .iter()
                    .find(|(name, _)| *name == level_type)
                    .map(|(_, preset)| *preset)
            })
            .unwrap_or_else(|| {
                log_from(
                    "Worker-Main-1",
                    "WARN",
                    &format!(
                        "Failed to parse level-type {level_type}, defaulting to minecraft:normal"
                    ),
                );
                sunrise_sim::save::WorldPreset::Normal
            });
        let flat = preset == sunrise_sim::save::WorldPreset::Flat;
        let settings = if flat {
            let biomes = world_biomes!(self);
            let (settings, errors) = sunrise_sim::flatgen::settings_from_json(
                &props.generator_settings,
                |b| self.common.pack.assets.registry.default_state(b).is_some(),
                |b| biomes.get(biomes.id(b)).name == b,
            );
            for e in errors {
                log_from("Worker-Main-1", "ERROR", &e);
            }
            settings
        } else {
            None
        };
        let options = sunrise_sim::save::WorldGen {
            preset,
            structures: props.generate_structures,
            bonus_chest: false,
        };
        sunrise_sim::save::write_world_gen_settings_with(
            dir, props.seed, options, settings, None, None,
        )
    }
}

/// PlayerList.BAN_DATE_FORMAT ("yyyy-MM-dd 'at' HH:mm:ss z"), in UTC.
fn ban_date(millis: i64) -> String {
    let s = format_date(millis);
    format!("{} at {} UTC", &s[..10], &s[11..19])
}
/// What a banned player is told: `multiplayer.disconnect.banned.reason` and, for a ban that ends,
/// `multiplayer.disconnect.banned.expiration` (PlayerList.canPlayerLogin).
pub(in crate::game::server_side) fn ban_message(ban: &BanEntry<NameAndId>) -> Tag {
    let mut reason = translatable_tag(
        "multiplayer.disconnect.banned.reason",
        vec![ban_reason(&ban.reason)],
    );
    if let Some(expires) = ban.expires {
        append(
            &mut reason,
            translatable_tag(
                "multiplayer.disconnect.banned.expiration",
                vec![Tag::String(JavaString::from(ban_date(expires).as_str()))],
            ),
        );
    }
    reason
}
/// BanListEntry.getReasonMessage.
fn ban_reason(reason: &Option<String>) -> Tag {
    match reason {
        Some(r) => Tag::String(JavaString::from(r.as_str())),
        None => translatable("multiplayer.disconnect.banned.reason.default", &[]),
    }
}
fn translatable_tag(key: &str, args: Vec<Tag>) -> Tag {
    Tag::compound([
        ("translate", Tag::String(JavaString::from(key))),
        ("with", Tag::List(args)),
    ])
}
/// MutableComponent.append.
fn append(to: &mut Tag, sibling: Tag) {
    if let Tag::Compound(map) = to {
        let extra = map
            .entry(JavaString::from("extra"))
            .or_insert_with(|| Tag::List(Vec::new()));
        if let Tag::List(items) = extra {
            items.push(sibling);
        }
    }
}

/// What the server loop keeps between ticks.
struct ServerState {
    lists: Arc<Mutex<Lists>>,
    console: mpsc::Receiver<String>,
    empty_ticks: i32,
    /// The mutable `DedicatedServer.pauseWhenEmptySeconds` setting.
    pause_when_empty_seconds: i32,
    /// When the last passes of the loop ended, for the tick rate sunrise-server shows.
    ticks: std::collections::VecDeque<Instant>,
    paused: bool,
    /// `broadcast-console-to-ops`.
    broadcast_console: bool,
    /// `broadcast-rcon-to-ops` (`MinecraftServer.shouldRconBroadcast`).
    broadcast_rcon: bool,
    /// `op-permission-level`: what `/op` grants (operatorUserPermissions).
    op_level: i32,
    /// `sunrise-report`: the ticks being recorded.
    report: Option<report::Recording>,
    /// When the running capture takes its next memory snapshot, and how many it has taken.
    memory_due: Option<(f64, u32)>,
    /// The always-on health history and slow ring, and the `.hzperf` a report writes from them.
    perf: sunrise_sim::server_perf::ServerPerf,
    /// The perf clock's zero: the tick loop's start.
    perf_epoch: Instant,
    /// The Sunrise switches a capture records.
    sunrise: sunrise_perf_format::v1::SunriseOptions,
    /// View and simulation distance, and the player limit, for the report.
    view: (u32, u32),
    max_players: usize,
    /// `network-compression-threshold`, for the capture's settings.
    compression: i32,
    /// The mutable `gamemode` server property (`DedicatedServer.gameMode`).
    game_mode: i32,
    /// The mutable `force-gamemode` server property (`DedicatedServer.forceGameMode`).
    force_game_mode: bool,
}

/// `SaveAllCommand.saveAll`'s reply to its source: `commands.save.success` when
/// the save went through, `commands.save.failed` (the `ERROR_FAILED` it throws)
/// when it did not.
fn save_all_reply(saved: bool) -> Text {
    Text::translate(
        if saved {
            "commands.save.success"
        } else {
            "commands.save.failed"
        },
        &[],
    )
}

/// `BanIpCommands.banIp`'s `commands.banip.info`: the count of affected players and
/// `EntitySelector.joinNames` of them (a gray ", " between names). None when no one is affected,
/// as vanilla sends no line then.
fn ban_ip_info(names: &[&str]) -> Option<Text> {
    if names.is_empty() {
        return None;
    }
    Some(Text::translate(
        "commands.banip.info",
        &[
            Text::literal(names.len().to_string()),
            sunrise_sim::scoreboard_cmd::format_list(
                names
                    .iter()
                    .map(|n| Text::literal((*n).to_owned()))
                    .collect(),
            ),
        ],
    ))
}

/// `MinecraftServer.isAutoSave`: all of Sunrise's server levels share this save flag.
fn auto_save(state: &ServerState) -> bool {
    state.lists.lock().unwrap().autosave
}

/// `MinecraftServer.tickServer`'s pause-when-empty count. `pause_after` is its
/// `pauseWhenEmptySeconds() * 20`; while that is above 0, a tick with no player and no
/// `/tick sprint` running (`ServerTickRateManager.isSprinting`) adds one to `empty_ticks`, and any
/// other tick sets it back to 0. A `/pregen` task keeps the server busy as a player would: paused,
/// nothing would post-process its chunks. Returns whether the server is paused this tick.
fn count_empty_tick(
    empty_ticks: &mut i32,
    pause_after: i32,
    players: usize,
    pregen: bool,
    sprinting: bool,
) -> bool {
    if pause_after > 0 {
        if players == 0 && !pregen && !sprinting {
            *empty_ticks += 1;
        } else {
            *empty_ticks = 0;
        }
    }
    pause_after > 0 && *empty_ticks >= pause_after
}

/// `MinecraftServer.setAutoSave`: enable or disable periodic saves and report whether it changed.
pub(in crate::game) fn set_auto_save(lists: &Mutex<Lists>, enabled: bool) -> bool {
    let mut l = lists.lock().unwrap();
    let changed = l.autosave != enabled;
    l.autosave = enabled;
    changed
}

/// The dedicated-only commands' replies vanilla sends with `broadcast`
/// (`sendSuccess(..., true)`): the operators hear of them.
const BROADCAST: &[&str] = &[
    "commands.ban.success",
    "commands.banip.info",
    "commands.banip.success",
    "commands.deop.success",
    "commands.kick.success",
    "commands.op.success",
    "commands.pardon.success",
    "commands.pardonip.success",
    "commands.save.disabled",
    "commands.save.enabled",
    "commands.save.success",
    "commands.stop.stopping",
    "commands.whitelist.add.success",
    "commands.whitelist.disabled",
    "commands.whitelist.enabled",
    "commands.whitelist.reloaded",
    "commands.whitelist.remove.success",
];

impl crate::game::ServerGame<'_> {
    /// DedicatedServer.initServer and MinecraftServer.runServer.
    pub(in crate::game) fn run_dedicated(&mut self) -> Result<()> {
        // This thread ticks the server (MinecraftServer.spin's priority).
        sunrise_threads::favour_current();
        self.common.join_world_rest()?;
        let setup = SETUP
            .get()
            .and_then(|m| m.lock().unwrap().take())
            .ok_or("dedicated server not prepared")?;
        self.run_dedicated_with_setup(setup)
    }

    pub fn run_dedicated_with_setup(&mut self, setup: Setup) -> Result<()> {
        self.run_server_lifecycle(true, |game| game.run_dedicated_inner(setup))
    }

    fn run_dedicated_inner(
        &mut self,
        setup: Setup,
    ) -> Result<super::server_lifecycle::ServerRunOutcome> {
        let Setup {
            props,
            universe,
            level_name,
            port,
            allow_missing_mods,
        } = setup;
        let services = self
            .server
            .dedicated_services
            .as_ref()
            .ok_or("dedicated server services not configured")?;
        let profile_keys = services.profile_keys;
        let verify_session = props
            .online_mode
            .then(|| Arc::clone(&services.verify_session));
        let world_dir = std::env::current_dir()?.join(&universe).join(&level_name);
        // The console thread starts first, as in initServer.
        let (tx, console) = mpsc::channel();
        std::thread::Builder::new()
            .name("Server console handler".into())
            .spawn(move || {
                let stdin = std::io::stdin();
                let mut line = String::new();
                while stdin.read_line(&mut line).is_ok_and(|n| n > 0) {
                    if tx
                        .send(line.trim_end_matches(['\r', '\n']).to_owned())
                        .is_err()
                    {
                        break;
                    }
                    line.clear();
                }
            })?;
        // Main: Services.create loads the universe's usercache.json.
        let (mut cache, warnings) = UserCache::load(universe.join("usercache.json"));
        for warning in warnings {
            log_from("main", "WARN", &warning);
        }
        // DedicatedServer.initServer: names nobody has resolve offline only without authentication.
        cache.resolve_offline_users = !props.online_mode;
        self.server.user_cache = Some(cache);
        // Services.create: the services' public keys, fetched in the
        // background (they check players' chat keys).
        let secure_chat = Arc::new(sunrise_sim::lan::SecureChat::new(
            props.enforce_secure_profile && props.online_mode,
            profile_keys,
        ));
        log(
            "INFO",
            &format!(
                "Starting minecraft server version {}",
                sunrise_core::VANILLA_VERSION
            ),
        );
        log("INFO", "Loading properties");
        let game_mode = props.game_mode;
        log(
            "INFO",
            &format!(
                "Default game type: {}",
                GAME_MODES[game_mode as usize].to_uppercase()
            ),
        );
        let port = port.unwrap_or(props.server_port as u16);
        // MinecraftServer.initializeKeyPair (only online mode uses it).
        log("INFO", "Generating keypair");
        let online = if let Some(verify_session) = verify_session {
            Some(Arc::new(OnlineMode::generate(
                props.prevent_proxy_connections,
                verify_session,
            )?))
        } else {
            None
        };
        log(
            "INFO",
            &format!(
                "Starting Minecraft server on {}:{port}",
                if props.server_ip.is_empty() {
                    "*"
                } else {
                    &props.server_ip
                }
            ),
        );
        // DedicatedServer.initServer binds the TCP socket before ServerAboutToStartEvent
        // and before preparing the level; a bind failure returns false so runServer skips
        // ServerStartedEvent/ServerStoppingEvent and runs only stopServer/ServerStoppedEvent.
        let bind = (!props.server_ip.is_empty())
            .then(|| props.server_ip.parse().ok())
            .flatten();
        let listener = match sunrise_sim::lan::Lan::bind(bind, port) {
            Ok(listener) => listener,
            Err(e) => {
                log("WARN", "**** FAILED TO BIND TO PORT!");
                log("WARN", &format!("The exception was: {e}"));
                log("WARN", "Perhaps a server is already running on that port?");
                return Ok(super::server_lifecycle::ServerRunOutcome::FailedInit(e));
            }
        };
        if online.is_none() {
            log("WARN", "**** SERVER IS RUNNING IN OFFLINE/INSECURE MODE!");
            log(
                "WARN",
                "The server will make no attempt to authenticate usernames. Beware.",
            );
            log(
                "WARN",
                "While this makes the game possible to play without internet access, it also opens up the ability for hackers to connect with any username they choose.",
            );
            log(
                "WARN",
                "To change this, set \"online-mode\" to \"true\" in the server.properties file.",
            );
        }
        let mut lists = Lists::load();
        lists.use_whitelist = props.white_list;
        lists.enforce_whitelist = props.enforce_whitelist;
        lists.op_level = props.op_permission_level;
        lists.player_idle_timeout = props.player_idle_timeout;
        let lists = Arc::new(Mutex::new(lists));
        self.server.server_lists.0 = Some(lists.clone());
        log("INFO", &format!("Preparing level \"{level_name}\""));
        let level_load_started = Instant::now();
        let new_world = self.prepare_world_metadata(&world_dir, &props, allow_missing_mods)?;
        let folder = world_dir.to_string_lossy().into_owned();
        // The mods' common and server configs, read before the server starts.
        self.native_configs_server_start(&world_dir, true);
        let prepared = self.prepare_world_and_rules(WorldRequest::new(
            &folder,
            Some(&level_name),
            Some(props.seed),
            game_mode == 1,
            None,
            Arc::new(sunrise_gameplay_common::game::api_values::game_rule_registry()),
        )?)?;
        self.native_server_about_to_start(true);
        self.native_lifecycle_open_started();
        // The registries the server sends are read while the world prepares.
        let sync = super::lan::sync::preload_sync_data(
            &self.common.res.core.item_data,
            self.common.res.shared_items(),
            self.common.res.biomes(),
        );
        let opened_packs = self.finish_open_world(prepared)?;
        // WorldLoader: the world's packs must load before the server starts. The data store is read
        // now rather than at the first tick (reading parses functions; it runs none), and a broken
        // predicate graph stops the start before anything listens (a partial check: predicate_graph.rs).
        self.ensure_datapacks();
        let store = &self.server.fns.0.store;
        let tag = |id: &str| {
            let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
            store
                .files
                .contains_key(&format!("tags/{ns}:predicate/{path}"))
        };
        if let Err(error) = predicate_graph::check(&store.predicates, tag) {
            let level = match error {
                predicate_graph::GraphError::Unbound(_)
                | predicate_graph::GraphError::UnboundTags(_) => "ERROR",
                predicate_graph::GraphError::Recursive(_) => "WARN",
            };
            for line in error.lines() {
                log(level, &line);
            }
            log(
                "WARN",
                "Failed to load datapacks, can't proceed with server load. You can either fix your datapacks or reset to vanilla with --safeMode",
            );
            return Err(Box::new(DatapacksRefused));
        }
        if new_world {
            self.common.level.hardcore = props.hardcore;
        }
        // The single-player host's state is not used: players have their own files.
        self.common.level.position = None;
        // DedicatedServer.initServer: worldData.setGameType; forceDifficulty.
        // `load_world` restored difficulty from level.dat; server.properties wins
        // (vanilla's forceDifficulty), including health_rules used for despawn (#4694).
        self.set_default_game_mode(game_mode);
        // MinecraftServer.setDifficulty: a hardcore world is always hard.
        self.common.ui.difficulty = if self.common.level.hardcore {
            3
        } else {
            props.difficulty as u8
        };
        self.common.level.set_difficulty(self.common.ui.difficulty);
        self.common.sync_rules();
        // DedicatedServer.initServer: a legacy `announce-player-achievements`
        // sets the show_advancement_messages rule once, as the file is read.
        if let Some(announce) = props.announce_player_achievements {
            let value = sunrise_game_data::gamerules::Value::Bool(announce);
            if self.common.ui.rules.set("show_advancement_messages", value) {
                self.game_rule_changed("minecraft:show_advancement_messages", value, None);
            }
        }
        self.server.settings.render_distance = effective_view_distance(props.view_distance);
        self.server.settings.simulation_distance = props.simulation_distance.clamp(2, 32);
        // Nothing is kept around the spawn while nobody is on.
        self.server.streaming.radius = -1;
        self.prepare_levels()?;
        let level_load_time = level_load_started.elapsed();
        let gate_lists = lists.clone();
        let gate: LoginGate = Arc::new(move |address, uuid, name| {
            // `ServerActivityMonitor.reportLoginActivity`: a login attempt, however it ends. The management
            // protocol's `server/activity` notification is rate limited from these.
            events::record_server("login_activity", serde_json::json!({}));
            let user = NameAndId {
                id: uuid,
                name: name.to_owned(),
            };
            gate_lists.lock().unwrap().can_login(address, &user)
        });
        let bypass_lists = lists.clone();
        // DedicatedServer's constructor: `enable-code-of-conduct` reads the
        // `codeofconduct` folder, and a folder it can't read stops the start.
        let code_of_conducts = if props.code_of_conduct {
            sunrise_sim::lan::code_of_conduct::read(std::path::Path::new("codeofconduct"))?
        } else {
            Vec::new()
        };
        let favicon = std::fs::read("server-icon.png")
            .ok()
            .filter(|b| png_size(b) == Some((64, 64)));
        // Sunrise only: a leftover `forwarding.secret` says that Velocity forwarding is gone.
        refuse_forwarding_secret(std::path::Path::new("."))?;
        // `[transfer]` on: a player's only way in must be the proxy, or one who joins this server
        // directly finds the older copy of itself that a hand-over left here, and items double.
        // Velocity forwarding made it so; without it nothing does.
        let transfers = self
            .common
            .options
            .transfers
            .clone()
            .map(sunrise_sim::lan::arrivals::Transfers::new);
        if transfers.is_some() {
            return Err("server.toml's [transfer] is on, but players can join this server without the proxy \
                 (Sunrise has no Velocity forwarding any more): turn [transfer] off"
                .into());
        }
        let net = NetConfig {
            bind,
            port,
            motd: props.motd.clone(),
            max_players: props.max_players.max(0) as usize,
            compression_threshold: props.network_compression_threshold,
            rate_limit: props.rate_limit,
            enable_status: props.enable_status,
            hide_online_players: props.hide_online_players,
            favicon,
            secure_chat: Some(secure_chat),
            accepts_transfers: props.accepts_transfers,
            announce: false,
            listen: false,
            bypasses_limit: Some(Arc::new(move |uuid| {
                bypass_lists
                    .lock()
                    .unwrap()
                    .ops
                    .entries()
                    .iter()
                    .any(|e| e.user.id == uuid && e.bypasses_player_limit)
            })),
            gate: Some(gate),
            online,
            arrivals: transfers.as_ref().map(|t| t.expected.clone()),
            log: Some(NetLog {
                write: Arc::new(log_from),
                lang: Arc::new(sunrise_sim::lang_load::load_language(
                    &self.common.options.resources,
                )),
                log_ips: props.log_ips,
            }),
            resource_pack: props.resource_pack.clone(),
            code_of_conducts,
            // DedicatedServer.serverLinks: `bug-report-link` as the only link
            // (ServerLinks.KnownLinkType.BUG_REPORT is id 0).
            levels: self.server.level_shapes.clone(),
            sunrise_clients_only: props
                .raw
                .get(sunrise_sim::lan::SUNRISE_CLIENTS_ONLY)
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("true")),
            lod_distance: sunrise_sim::lod_net::distance_property(
                props.raw.get(sunrise_sim::lod_net::PROPERTY).as_deref(),
            ),
            server_links: if props.bug_report_link.is_empty() {
                Vec::new()
            } else if sunrise_gameplay_common::game::untrusted_uri::valid_untrusted_uri(
                &props.bug_report_link,
            ) {
                vec![sunrise_protocol::packets::configuration::ServerLink {
                    kind: sunrise_protocol::packets::configuration::ServerLinkKind::Known(0),
                    link: props.bug_report_link.clone(),
                }]
            } else {
                log(
                    "WARN",
                    &format!("Failed to parse bug link {}", props.bug_report_link),
                );
                Vec::new()
            },
            owner_profile: None,
            singleplayer_profile: None,
        };
        let config = HubConfig {
            allow_flight: props.allow_flight,
            default_game_mode: game_mode,
            guest_commands: true,
            force_game_mode: props.force_game_mode,
            hardcore: self.common.level.hardcore,
            view_distance: props.view_distance,
            simulation_distance: props.simulation_distance.clamp(2, 32),
            dimension: self.common.dim.id().to_owned(),
            levels: self.server.level_shapes.clone(),
            level_seeds: self.server.levels.0.runtime.seeds(),
            seed: self.common.level.seed,
            max_players: props.max_players.max(0) as usize,
            integrated: false,
        };
        let settings = LanSettings {
            port,
            guest_commands: true,
            force_game_mode: props.force_game_mode,
        };
        if let Some(sync) = sync {
            sync.adopt(&self.pack_files());
        }
        self.lan_host(settings, net, config)?;
        if let Some(host) = self.server.lan.as_mut() {
            host.settings.port = host.hub.lan.listen_on(listener, false)?;
        }
        if let Some(transfers) = transfers
            && let Some(host) = self.server.lan.as_mut()
        {
            let s = &transfers.settings;
            log(
                "INFO",
                &format!(
                    "Transfers on (server.toml [transfer]): players expected for {} s ({} s at most, {} at once), frozen for {} ticks and held for {} ticks at most",
                    s.expect.as_secs(),
                    s.max_expect.as_secs(),
                    s.max_expected,
                    s.max_freeze_ticks,
                    s.max_hold_ticks
                ),
            );
            host.hub.transfers = Some(transfers);
        }
        self.sync_ops(&lists);
        if let Some(host) = self.server.lan.as_mut() {
            // The viewed areas' chunk caps (#3716): server.toml's `max_viewed_*` settings.
            let (caps, clamped) = viewed::ViewedCaps::from_properties(|key| props.raw.get(key));
            if let Some(asked) = clamped {
                let max = viewed::MAX_LOADED_PER_PLAYER;
                log(
                    "WARN",
                    &format!(
                        "max_viewed_loaded_chunks is {asked}, but a client keeps at most {max} \
                         columns of a viewed level: using {max}"
                    ),
                );
            }
            host.hub.viewed.caps = caps;
            host.hub.spawn_protection = props.spawn_protection;
            host.hub.chat_spam_threshold = props.chat_spam_threshold_seconds.wrapping_mul(20);
            host.hub.command_spam_threshold = props.command_spam_threshold_seconds.wrapping_mul(20);
            // Off: an operator's commands reach no other player, only the log.
            host.hub.admin_feedback_to_ops = props
                .raw
                .get(ADMIN_FEEDBACK_PROPERTY)
                .is_none_or(|v| v.trim() != "false");
        }
        log("INFO", &startup_done(level_load_time));
        // DedicatedServer.initServer ends with saveEverything(false, true, true)
        // followed by handleServerStarting(this), then runServer fires ServerStartedEvent.
        if self.save_dedicated_checkpoint("server startup")? == 0 {
            self.log_saved_levels();
        }
        if let Some(host) = self.server.lan.as_mut() {
            host.hub.entity_range_percent = props.entity_broadcast_range_percentage;
        }
        self.start_loaded_dedicated_world(&opened_packs);
        // The dedicated server's open is complete before its first tick.
        self.native_lifecycle_world_ready();
        let mut state = ServerState {
            lists,
            console,
            empty_ticks: 0,
            pause_when_empty_seconds: props.pause_when_empty_seconds,
            ticks: Default::default(),
            paused: false,
            broadcast_console: props.broadcast_console_to_ops,
            broadcast_rcon: props.broadcast_rcon_to_ops,
            op_level: props.op_permission_level,
            game_mode: props.game_mode,
            force_game_mode: props.force_game_mode,
            report: None,
            memory_due: None,
            perf: sunrise_sim::server_perf::ServerPerf::new(
                0.0,
                self.common.options.spikes.unwrap_or(SLOW_TICK_MS),
            ),
            perf_epoch: Instant::now(),
            sunrise: sunrise_sim::server_perf::sunrise_options(
                sunrise_sim::lod_net::distance_property(
                    props.raw.get(sunrise_sim::lod_net::PROPERTY).as_deref(),
                ),
            ),
            compression: props.network_compression_threshold,
            view: (
                props.view_distance.max(0) as u32,
                props.simulation_distance.clamp(2, 32) as u32,
            ),
            max_players: props.max_players.max(0) as usize,
        };
        sunrise_sim::server_perf::warm_hardware();
        if let Some(file) = std::env::var_os("SUNRISE_TRACE").filter(|f| !f.is_empty()) {
            self.record_report(
                &mut state,
                &report::Plan {
                    seconds: report::MAX_SECONDS,
                    samples: false,
                    timeline: true,
                    file: Some(file.into()),
                    trigger: Trigger::Env,
                    sampler: Default::default(),
                },
            );
        }
        let mut next = Instant::now();
        // The panic that ended the tick loop, when one did.
        let mut crashed: Option<String> = None;
        self.server.fns.0.permission = Some(props.function_permission_level.clamp(0, 4) as u8);
        let watchdog = Watchdog::start(
            props.max_tick_time,
            self.common.options.save_failure_diagnostics.clone(),
        );
        let _stop_watchdog = StopWatchdog(watchdog.clone());
        let mut overload_warned: Option<Instant> = None;
        // `isTickTimeLoggingEnabled`: read at the end of a tick, so the first sample goes out the
        // tick after a subscriber appears. `lastTickNanos` is when the last wait ended.
        let mut tick_sampling = false;
        let mut last_tick_end = Instant::now();
        // `/stop`, or a stop signal (the shutdown hook's `halt`): either way the players and levels are
        // saved below, after the wait `[transfer]` may make (transfer.rs).
        while !self.stop_now(&state) {
            // `BlockableEventLoop.relayDelayCrash`: a worker's panic is reported on this thread.
            sunrise_level::crash::relay_worker_panic();
            self.server.server_tick_deadline = Some(next + self.common.clock.tick_duration());
            let tick_start = Instant::now();
            if let Some(watchdog) = &watchdog {
                watchdog.tick_due(next, self.common.clock.tick_duration());
            }
            // The console's lines and the control socket run in the tick too
            // (`handleConsoleInputs`): a panic there is the same crash.
            let prelude = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while let Ok(line) = state.console.try_recv() {
                    self.console_line(&mut state, &line);
                }
                self.poll_server_control(&mut state);
            }));
            if let Err(payload) = prelude {
                crashed = Some(panic_message(payload.as_ref()));
                break;
            }
            let players = self.server.lan.as_ref().map_or(0, |h| h.hub.online_count());
            // Vanilla reads pauseWhenEmptySeconds on every tick, so management changes take effect
            // without restarting the server.
            let pause_after = state.pause_when_empty_seconds.wrapping_mul(20);
            let paused = count_empty_tick(
                &mut state.empty_ticks,
                pause_after,
                players,
                self.pregen_level().is_some(),
                self.common.clock.sprinting(),
            );
            state.paused = paused;
            if paused && state.empty_ticks == pause_after {
                log(
                    "INFO",
                    &format!(
                        "Server empty for {} seconds, pausing",
                        state.pause_when_empty_seconds
                    ),
                );
                let _ = self.save_dedicated_checkpoint("pause on empty")?;
            }
            if !paused && !auto_save(&state) {
                // `MinecraftServer.autoSave` keeps its schedule while every level's
                // `noSave` flag is set, but writes nothing. The tick below counts down
                // once, so reset one tick early when this save would have been due.
                if self.server.ticks_until_autosave <= 1 {
                    self.server.ticks_until_autosave = self.next_autosave_interval() + 1;
                }
            }
            // MinecraftServer.tickServer: while paused only the connections run.
            // `MinecraftServer.runServer` catches what the tick throws, writes the crash
            // report and still stops the server (saves). A panic here does the same.
            let slow_ms = state.perf.slow_threshold_ms();
            let capturing = state.perf.capturing();
            let mut tick_detail = None;
            let ticked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut detail = |timing: super::levels::TickTiming| {
                    if capturing || timing.wall_ms > slow_ms {
                        tick_detail = Some(sunrise_sim::server_perf::TickDetail {
                            cpu_ms: timing.cpu_ms,
                            spans: timing.raw,
                        });
                    }
                };
                match &mut state.report {
                    Some(recording) => {
                        let sampled = recording.sample_next();
                        super::tick_profile::set_sampling(sampled);
                        let ticked = self.tick_server_spans(paused, |timing| {
                            recording.add(sampled);
                            detail(timing)
                        });
                        super::tick_profile::set_sampling(false);
                        ticked
                    }
                    None => self.tick_server_spans(paused, detail),
                }
            }));
            let tick_method = match ticked {
                Ok(result) => result?,
                Err(payload) => {
                    crashed = Some(panic_message(payload.as_ref()));
                    break;
                }
            };
            if state.report.as_ref().is_some_and(report::Recording::done) {
                self.finish_perf_capture(&mut state, StopReason::Normal);
            }
            self.serve_perf_requests(&mut state);
            let done = report::log_saved(&mut state.perf, false);
            self.perf_captures_done(done);
            // DedicatedServer.sendLowDiskSpaceWarning: the operators hear of it
            // (no window drains the toasts here).
            if sunrise_sim::chunk_io::PROCESS_REPORTS.take().low_disk_space
                && let Some(host) = self.server.lan.as_ref()
            {
                host.hub.send_low_disk_space_warning();
            }
            // `PlayerList.tick` reads `MinecraftServer.playerIdleTimeout`
            // every tick, so `/setidletimeout` changes it while the server runs.
            let idle_timeout = state.lists.lock().unwrap().player_idle_timeout;
            let kick_start = Instant::now();
            if idle_timeout > 0 {
                self.kick_idle(idle_timeout);
            }
            // `tickServer` ends with the player ticking: the tick method is the level tick and
            // this, not the console or the capture work around them.
            let method = tick_method + kick_start.elapsed();
            // `ServerDebugSubscribers`, once per tick after the players ticked and before the
            // wait: this tick's recipients and whether the next tick measures.
            let recipients = self
                .server
                .lan
                .as_ref()
                .map_or_else(Vec::new, |h| h.hub.tick_time_subscribers());
            // MinecraftServer.tickServer records each tick's time, for /tick query (and the status).
            self.common.clock.record(tick_start.elapsed());
            state.perf.tick(
                state.perf_epoch.elapsed().as_secs_f64(),
                tick_start.elapsed().as_secs_f32() * 1000.0,
                tick_detail.take(),
                || self.perf_world_counts(),
                || self.wire_bytes(),
            );
            self.poll_server_memory_snapshot(&mut state);
            state.ticked(Instant::now());
            // MinecraftServer.runServer: a server more than a second's ticks
            // behind skips them and says so, at most every 15 seconds.
            let wait_start = Instant::now();
            let (behind, idle) = self.wait_for_next_server_tick(&mut next, paused);
            // `MinecraftServer.finishMeasuringTaskExecutionTime` ends the wait phase
            // before any reporting; `logFullTickTime` keeps the entire tick-to-tick span.
            let tick_end = Instant::now();
            if let Some(behind) = behind
                && overload_warned.is_none_or(|at| at.elapsed() >= OVERLOAD_WARNING_INTERVAL)
            {
                overload_warned = Some(Instant::now());
                log(
                    "WARN",
                    &overload_warning(behind, self.common.clock.tick_duration()),
                );
            }
            if tick_sampling && let Some(host) = self.server.lan.as_ref() {
                host.hub.send_to_each(
                    &recipients,
                    &sunrise_protocol::packets::play::Clientbound::DebugSample(
                        sunrise_protocol::packets::play::DebugSample {
                            sample: tick_time_sample(
                                tick_end - last_tick_end,
                                method,
                                tick_end - wait_start,
                                idle,
                            )
                            .to_vec(),
                            kind: sunrise_protocol::packets::play::DEBUG_SAMPLE_TICK_TIME,
                        },
                    ),
                );
            }
            last_tick_end = tick_end;
            tick_sampling = !recipients.is_empty();
        }
        if let Some(watchdog) = &watchdog {
            watchdog.running.store(false, Ordering::Relaxed);
        }
        self.finish_perf_capture(&mut state, StopReason::Shutdown);
        let done = report::log_saved(&mut state.perf, true);
        self.perf_captures_done(done);
        match crashed {
            Some(message) => std::panic::resume_unwind(Box::new(message)),
            None => Ok(super::server_lifecycle::ServerRunOutcome::Orderly),
        }
    }

    /// Prepare the integrated owner's player separately from setInitialSpawn.
    pub(in crate::game::server_side) fn settle_player_spawn(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(600);
        while self.common.player.is_none() {
            self.server_update_world();
            self.spawn_player()?;
            if Instant::now() > deadline {
                return Err("the spawn area did not load".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        self.common.player = None;
        Ok(())
    }

    /// The chunks to keep loaded: the players' (lan_update_streaming). While
    /// nobody is online a dedicated server keeps none (vanilla has no spawn
    /// ticket), an integrated one the spawn chunk's.
    pub(in crate::game::server_side) fn update_dedicated_streaming(&mut self) {
        let rule = self
            .common
            .ui
            .rules
            .bool_at(sunrise_game_data::gamerules::vanilla::SPECTATORS_GENERATE_CHUNKS);
        let guest = self.server.lan.as_ref().and_then(|h| {
            // A held player (arriving from another server) is sent the level too, so it
            // holds the world loaded around it as a placed player does.
            h.hub
                .loading_chunks(|g| super::game_mode::loads_chunks(rule, g.spectator()))
                .next()
        });
        // The live setter and startup put the effective ChunkMap radius in settings;
        // the PlayerList keeps the raw value separately for its packets.
        if self.common.is_dedicated_server()
            || self.common.dim != super::Dim::Overworld
            || guest.is_some()
        {
            self.server.streaming.radius = if guest.is_some() {
                self.server.settings.render_distance
            } else {
                -1
            };
        } else if self.server.serving.is_some() {
            self.server.streaming.radius = self.server.settings.render_distance;
        }
        let spawn = self
            .common
            .spawn
            .map(|s| ((s.x.floor() as i32) >> 4, (s.z.floor() as i32) >> 4));
        if let Some(c) = guest.or(spawn) {
            self.server.streaming.center = c;
        }
    }

    pub fn ban_ip_target(&self, target: &str) -> Option<String> {
        if target.parse::<std::net::IpAddr>().is_ok() {
            Some(target.to_owned())
        } else {
            self.server
                .lan
                .as_ref()?
                .hub
                .guests
                .iter()
                .find(|g| g.participates_online() && g.name.eq_ignore_ascii_case(target))
                .map(|g| g.ip_address().to_owned())
        }
    }

    /// Network players `BanIpCommands` and the management service disconnect for an address.
    pub fn ip_ban_players(&self, ip: &str) -> Vec<u128> {
        self.server.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.participates_online() && g.ip_address() == ip)
                .map(|g| g.uuid)
                .collect()
        })
    }

    /// ServerGamePacketListenerImpl.tick: players idle longer than the
    /// timeout are kicked.
    pub fn kick_idle(&mut self, minutes: i32) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        let idle: Vec<u32> = host
            .hub
            .guests
            .iter()
            .filter(|g| {
                let idle_ticks = host.hub.ticks.saturating_sub(g.last_action_tick);
                g.participates_online() && idles_out(idle_ticks, minutes, g.won_game)
            })
            .map(|g| g.conn)
            .collect();
        for conn in idle {
            host.hub
                .kick(conn, translatable("multiplayer.disconnect.idling", &[]));
        }
    }

    /// Ops' permission levels for the hub (PlayerList.getProfilePermissions).
    pub(in crate::game) fn sync_ops(&mut self, lists: &Arc<Mutex<Lists>>) {
        let Some(host) = self.server.lan.as_mut() else {
            return;
        };
        host.hub.ops = lists
            .lock()
            .unwrap()
            .ops
            .entries()
            .iter()
            .map(|e| (e.user.id, e.level as u8))
            .collect();
        self.resend_commands_filtered();
    }

    /// The mods' say on `user` getting the permission level `new` (0 for `deop`) in the
    /// operator list: true when one canceled it. The old level is the entry's, else 0.
    pub(in crate::game) fn permission_change(
        &mut self,
        lists: &Arc<Mutex<Lists>>,
        user: &NameAndId,
        new: i32,
    ) -> bool {
        let old = lists.lock().unwrap().ops.get(user).map_or(0, |e| e.level);
        let entity = self
            .server
            .lan
            .as_ref()
            .and_then(|h| {
                h.hub
                    .guests
                    .iter()
                    .find(|g| g.participates_online() && g.uuid == user.id)
            })
            .map(|g| g.entity_id);
        let mut cancel = false;
        self.native_permissions_changed(
            user.id,
            entity,
            &user.name,
            PermissionLevel::from_level(old),
            PermissionLevel::from_level(new),
            &mut cancel,
        );
        cancel
    }

    /// A player on the server, by name.
    pub fn online_player(&self, name: &str) -> Option<NameAndId> {
        let hub = &self.server.lan.as_ref()?.hub;
        let g = hub
            .guests
            .iter()
            .find(|g| g.participates_online() && g.name.eq_ignore_ascii_case(name))?;
        Some(NameAndId {
            id: g.uuid,
            name: g.name.clone(),
        })
    }

    /// GameProfileArgument for a name: the user cache, else the profile
    /// service, else (offline) the offline player. None, told to the console:
    /// no such player.
    fn resolve(&mut self, name: &str) -> Option<NameAndId> {
        let found = self.resolve_name(name);
        if found.is_none() {
            let unknown = Text::translate("argument.player.unknown", &[]);
            log("INFO", &format!("System chat: {}", unknown.plain()));
        }
        found
    }

    /// `Services.nameToIdCache().get(name)`: the user cache, else the profile
    /// service, else (offline) the offline player.
    pub(in crate::game) fn resolve_name(&mut self, name: &str) -> Option<NameAndId> {
        let find_name = self
            .server
            .dedicated_services
            .as_ref()
            .map(|services| services.find_name);
        self.server.user_cache.as_mut()?.get_by_name(name, |lower| {
            let found = find_name.and_then(|f| f(lower));
            if found.is_none() {
                log("WARN", &format!("Couldn't find profile with name: {lower}"));
            }
            found
        })
    }

    /// MinecraftServer.sendSystemMessage: what the console is told; while
    /// the console runs a command, the replies vanilla broadcasts go to the
    /// operators too.
    fn console_reply(&mut self, text: Text) {
        log("INFO", &format!("System chat: {}", text.plain()));
        if self.server.console_broadcast.is_some()
            && text
                .translation_key()
                .is_some_and(|k| BROADCAST.contains(&k))
        {
            self.broadcast_to_admins(text);
        }
    }

    /// One console line, then the operators' share of its feedback.
    fn console_line(&mut self, state: &mut ServerState, line: &str) {
        self.console_line_telling(state, line, state.broadcast_console);
    }

    /// One console line run as `RconConsoleSource` runs an RCON command: the same console at level 4,
    /// its source named `Rcon`, and the operators told when `broadcast-rcon-to-ops` is on.
    fn rcon_line(&mut self, state: &mut ServerState, line: &str) {
        self.server.rcon_source = true;
        self.console_line_telling(state, line, state.broadcast_rcon);
        self.server.rcon_source = false;
    }

    /// `console_line` with `inform` as `CommandSource.shouldInformAdmins`.
    fn console_line_telling(&mut self, state: &mut ServerState, line: &str, inform: bool) {
        self.server.console_broadcast = Some(inform);
        self.console_command(state, line);
        self.server.console_broadcast = None;
        // The fixture panics with the hub out of the game, as a panic in the network tick does.
        crash_fixture("hub", || {
            let placed = self
                .server
                .lan
                .as_ref()
                .is_some_and(|host| host.hub.guests.iter().any(|g| g.placed));
            if placed {
                self.server.lan = None;
            }
            placed
        });
        if self.server.lan.is_some() {
            self.run_level_teleports();
            self.send_admin_feedback();
        }
        self.server.admin_feedback.clear();
    }

    /// `MinecraftServer.createCommandSourceStack`: the console's source is
    /// bound to the world spawn's level (`findRespawnDimension`), whichever
    /// level the last server tick left in place (the last one ticked, often
    /// the nether). Its commands run there and the level in place is put back
    /// afterwards.
    pub fn as_console_in_spawn_level<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let back = console_level(self.common.dim, self.world_spawn_level())
            .and_then(|dim| self.enter_command_level(dim));
        let source = super::commands::execute::Source {
            silent: false,
            ..self.server_source()
        };
        let outer = self.server.command_source.replace(source);
        let out = self.as_console(f);
        self.server.command_source = outer;
        if let Some(back) = back {
            self.enter_command_level(back);
        }
        out
    }

    /// DedicatedServer.handleConsoleInputs: one console line, run as the
    /// server (permission level 4). The dedicated-only commands are here;
    /// everything else goes to the game's dispatcher.
    fn console_command(&mut self, state: &mut ServerState, line: &str) {
        let line = line.strip_prefix('/').unwrap_or(line);
        let mut words = line.split_whitespace();
        let Some(command) = words.next() else {
            return;
        };
        let rest: Vec<&str> = words.collect();
        let lit = |s: &str| Text::literal(s.to_owned());
        #[cfg(feature = "state-digest")]
        if command == "digest-sync" {
            match self.as_console_in_spawn_level(|g| g.state_digest_sync(&rest)) {
                Ok(()) => log("INFO", "digest-sync done"),
                Err(e) => log("WARN", &format!("digest-sync: {e}")),
            }
            return;
        }
        match command {
            "stop" => {
                self.console_reply(Text::translate("commands.stop.stopping", &[]));
                state.lists.lock().unwrap().stop = true;
            }
            // Sunrise's own, on the console only: a performance report (report.rs).
            "sunrise-report" | "sunrise-trace" => {
                let trace = command == "sunrise-trace";
                let parsed = if trace {
                    report::parse_trace(&rest)
                } else {
                    report::parse(&rest)
                };
                match parsed {
                    Err(()) if trace => log(
                        "INFO",
                        &format!(
                            "sunrise-trace [SECONDS | stop]: 1 to {} seconds, 30 by default",
                            report::MAX_SECONDS
                        ),
                    ),
                    Err(()) => log(
                        "INFO",
                        &format!(
                            "sunrise-report [SECONDS | stop | status] [--no-samples] [--timeline]: 1 to {} seconds, 60 by default",
                            report::MAX_SECONDS
                        ),
                    ),
                    Ok(report::Command::Stop) if state.report.is_some() => {
                        log("INFO", "Stopping the performance report: writing it now");
                        self.finish_perf_capture(state, StopReason::Normal);
                    }
                    Ok(report::Command::Stop) => {
                        log("INFO", "No performance report is being recorded")
                    }
                    Ok(report::Command::Status) => match &state.report {
                        Some(recording) => log("INFO", &recording.status()),
                        None => log("INFO", "No performance report is being recorded"),
                    },
                    Ok(report::Command::Record {
                        seconds,
                        samples,
                        timeline,
                    }) if state.report.is_none() => {
                        self.record_report(
                            state,
                            &report::Plan {
                                seconds,
                                samples,
                                timeline,
                                file: None,
                                trigger: Trigger::Console,
                                sampler: Default::default(),
                            },
                        );
                        log(
                            "INFO",
                            &format!("Recording a performance report for {seconds} s (reports/)"),
                        );
                    }
                    Ok(report::Command::Record { .. }) => {
                        log("INFO", "A performance report is already being recorded")
                    }
                }
            }
            "save-all" => {
                self.console_reply(Text::translate("commands.save.saving", &[]));
                let saved = self.save_game();
                if let Err(e) = &saved {
                    // The reason the save did not go through is Sunrise's own
                    // detail, so it stays in the log; the source hears what
                    // SaveAllCommand tells it (ERROR_FAILED).
                    log("ERROR", &format!("{e}"));
                }
                self.console_reply(save_all_reply(saved.is_ok()));
            }
            "save-off" => {
                if set_auto_save(&state.lists, false) {
                    self.console_reply(Text::translate("commands.save.disabled", &[]));
                } else {
                    self.console_reply(Text::translate("commands.save.alreadyOff", &[]));
                }
            }
            "save-on" => {
                if set_auto_save(&state.lists, true) {
                    self.console_reply(Text::translate("commands.save.enabled", &[]));
                } else {
                    self.console_reply(Text::translate("commands.save.alreadyOn", &[]));
                }
            }
            "list" => {
                let guests: Vec<(String, u128)> = self
                    .server
                    .lan
                    .as_ref()
                    .map(|h| {
                        h.hub
                            .online_players()
                            .map(|g| (g.name.clone(), g.uuid))
                            .collect()
                    })
                    .unwrap_or_default();
                let max = self
                    .server
                    .lan
                    .as_ref()
                    .map_or(0, |h| h.hub.lan.shared.max_players());
                let names: Vec<String> = if rest.first() == Some(&"uuids") {
                    guests
                        .iter()
                        .map(|(n, u)| {
                            format!("{n} ({})", sunrise_protocol::crypto::uuid_string(*u))
                        })
                        .collect()
                } else {
                    guests.iter().map(|(n, _)| n.clone()).collect()
                };
                self.console_reply(Text::translate(
                    "commands.list.players",
                    &[
                        lit(&guests.len().to_string()),
                        lit(&max.to_string()),
                        lit(&names.join(", ")),
                    ],
                ));
            }
            "op" | "deop" if !rest.is_empty() => {
                let Some(user) = self.resolve(rest[0]) else {
                    return;
                };
                let level = state.op_level;
                let known = state.lists.lock().unwrap().ops.contains(&user);
                let canceled = if command == "op" {
                    !known && self.permission_change(&state.lists, &user, level)
                } else {
                    known && self.permission_change(&state.lists, &user, 0)
                };
                let changed = !canceled && {
                    let mut l = state.lists.lock().unwrap();
                    if command == "op" {
                        if l.ops.contains(&user) {
                            false
                        } else {
                            l.ops.add(OpEntry {
                                user: user.clone(),
                                level,
                                bypasses_player_limit: false,
                            })
                        }
                    } else {
                        l.ops.remove(&user)
                    }
                };
                let key = match (command, changed) {
                    ("op", true) => "commands.op.success",
                    ("op", false) => "commands.op.failed",
                    (_, true) => "commands.deop.success",
                    _ => "commands.deop.failed",
                };
                self.console_reply(Text::translate(key, &[lit(&user.name)]));
                let lists = state.lists.clone();
                self.sync_ops(&lists);
                // DeOpCommands.deopPlayers.
                if command == "deop" && changed {
                    self.kick_unlisted_players(&lists);
                }
            }
            "whitelist" => self.console_whitelist(state, &rest),
            "ban" if !rest.is_empty() => {
                let Some(user) = self.resolve(rest[0]) else {
                    return;
                };
                let reason = (rest.len() > 1).then(|| rest[1..].join(" "));
                let added = state.lists.lock().unwrap().ban(&user, reason.clone());
                if added {
                    self.console_reply(Text::translate(
                        "commands.ban.success",
                        &[
                            lit(&user.name),
                            reason.as_deref().map_or_else(
                                || {
                                    Text::translate(
                                        "multiplayer.disconnect.banned.reason.default",
                                        &[],
                                    )
                                },
                                lit,
                            ),
                        ],
                    ));
                    self.kick_uuid(user.id, translatable("multiplayer.disconnect.banned", &[]));
                } else {
                    self.console_reply(Text::translate("commands.ban.failed", &[]));
                }
            }
            "ban-ip" if !rest.is_empty() => {
                let target = rest[0];
                let ip = self.ban_ip_target(target);
                let Some(ip) = ip else {
                    self.console_reply(Text::translate("commands.banip.invalid", &[]));
                    return;
                };
                let reason = (rest.len() > 1).then(|| rest[1..].join(" "));
                let added = state.lists.lock().unwrap().ban_ip(&ip, reason.clone());
                if !added {
                    self.console_reply(Text::translate("commands.banip.failed", &[]));
                    return;
                }
                self.console_reply(Text::translate(
                    "commands.banip.success",
                    &[
                        lit(&ip),
                        reason.as_deref().map_or_else(
                            || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
                            lit,
                        ),
                    ],
                ));
                let banned: Vec<(u128, String)> = self
                    .server
                    .lan
                    .as_ref()
                    .map(|h| {
                        h.hub
                            .guests
                            .iter()
                            .filter(|g| g.ip_address() == ip)
                            .map(|g| (g.uuid, g.name.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                let names: Vec<&str> = banned.iter().map(|(_, n)| n.as_str()).collect();
                if let Some(info) = ban_ip_info(&names) {
                    self.console_reply(info);
                }
                for (uuid, _) in banned {
                    self.kick_uuid(uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
                }
            }
            "pardon" if !rest.is_empty() => {
                let Some(user) = self.resolve(rest[0]) else {
                    return;
                };
                let removed = state.lists.lock().unwrap().bans.remove(&user);
                let key = if removed {
                    "commands.pardon.success"
                } else {
                    "commands.pardon.failed"
                };
                self.console_reply(Text::translate(key, &[lit(&user.name)]));
            }
            "pardon-ip" if !rest.is_empty() => {
                let ip = rest[0].to_owned();
                if ip.parse::<std::net::IpAddr>().is_err() {
                    self.console_reply(Text::translate("commands.pardonip.invalid", &[]));
                    return;
                }
                let removed = state.lists.lock().unwrap().ip_bans.remove(&ip);
                let key = if removed {
                    "commands.pardonip.success"
                } else {
                    "commands.pardonip.failed"
                };
                self.console_reply(Text::translate(key, &[lit(&ip)]));
            }
            "banlist" => {
                let l = state.lists.lock().unwrap();
                let mut lines = Vec::new();
                let which = rest.first().copied().unwrap_or("");
                if which != "ips" {
                    for b in l.bans.entries() {
                        lines.push((b.user.name.clone(), b.source.clone(), b.reason.clone()));
                    }
                }
                if which != "players" {
                    for b in l.ip_bans.entries() {
                        lines.push((b.user.clone(), b.source.clone(), b.reason.clone()));
                    }
                }
                drop(l);
                if lines.is_empty() {
                    self.console_reply(Text::translate("commands.banlist.none", &[]));
                } else {
                    self.console_reply(Text::translate(
                        "commands.banlist.list",
                        &[lit(&lines.len().to_string())],
                    ));
                    for (name, source, reason) in lines {
                        self.console_reply(Text::translate(
                            "commands.banlist.entry",
                            &[
                                lit(&name),
                                lit(&source),
                                reason.as_deref().map_or_else(
                                    || {
                                        Text::translate(
                                            "multiplayer.disconnect.banned.reason.default",
                                            &[],
                                        )
                                    },
                                    lit,
                                ),
                            ],
                        ));
                    }
                }
            }
            "kick" if !rest.is_empty() => {
                let Some(user) = self.online_player(rest[0]) else {
                    self.console_reply(Text::translate("argument.entity.notfound.player", &[]));
                    return;
                };
                let reason = if rest.len() > 1 {
                    Tag::String(JavaString::from(rest[1..].join(" ").as_str()))
                } else {
                    translatable("multiplayer.disconnect.kicked", &[])
                };
                self.kick_uuid(user.id, reason);
                self.console_reply(Text::translate(
                    "commands.kick.success",
                    &[
                        lit(&user.name),
                        if rest.len() > 1 {
                            lit(&rest[1..].join(" "))
                        } else {
                            Text::translate("multiplayer.disconnect.kicked", &[])
                        },
                    ],
                ));
            }
            _ => self.console_dispatch(line),
        }
    }

    /// A console line none of the dedicated-only commands took: a native mod's
    /// command, else the game's dispatcher. Its output is the console's
    /// (`MinecraftServer.sendSystemMessage`), logged once; `send_success` has
    /// already told the operators of what it broadcasts, so it is not filtered
    /// through `console_reply` again.
    pub fn console_dispatch(&mut self, line: &str) {
        // The game's dispatcher, captured for the console.
        self.server.chat_capture = Some(Vec::new());
        // A command a native mod registered goes to the mods first,
        // looked up by the same first word the match above read.
        if !self.perform_native_console_command(line.trim()) {
            self.as_console_in_spawn_level(|g| g.run_command(line));
        }
        for text in self.server.chat_capture.take().unwrap_or_default() {
            log("INFO", &format!("System chat: {}", text.plain()));
        }
        // ServerPlayer.onEffectAdded and onEffectUpdated send an effect
        // the command gave as it was given: vanilla's console runs after
        // the players' tick (`DedicatedServer.tickConnection`), so the
        // next tick is the first to count it down. A player's own
        // commands sync theirs the same way (`run_guest_command`).
        if let Some(h) = self.server.lan.as_mut() {
            h.hub.sync_all_effects();
        }
    }

    fn console_whitelist(&mut self, state: &mut ServerState, rest: &[&str]) {
        let lit = |s: &str| Text::literal(s.to_owned());
        match rest {
            ["on"] | ["off"] => {
                let on = rest[0] == "on";
                let l = state.lists.lock().unwrap();
                let turned_on = on && !l.use_whitelist;
                let key = match (on, l.use_whitelist) {
                    (true, true) => "commands.whitelist.alreadyOn",
                    (true, false) => "commands.whitelist.enabled",
                    (false, false) => "commands.whitelist.alreadyOff",
                    (false, true) => "commands.whitelist.disabled",
                };
                drop(l);
                self.set_using_whitelist(&state.lists, on);
                self.console_reply(Text::translate(key, &[]));
                if turned_on {
                    let lists = state.lists.clone();
                    self.kick_unlisted_players(&lists);
                }
            }
            ["list"] => {
                let names: Vec<String> = state
                    .lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.name.clone())
                    .collect();
                if names.is_empty() {
                    self.console_reply(Text::translate("commands.whitelist.none", &[]));
                } else {
                    self.console_reply(Text::translate(
                        "commands.whitelist.list",
                        &[lit(&names.len().to_string()), lit(&names.join(", "))],
                    ));
                }
            }
            ["reload"] => {
                let _ = state.lists.lock().unwrap().white.load();
                self.console_reply(Text::translate("commands.whitelist.reloaded", &[]));
                let lists = state.lists.clone();
                self.kick_unlisted_players(&lists);
            }
            ["add", name] | ["remove", name] => {
                let Some(user) = self.resolve(name) else {
                    return;
                };
                let mut l = state.lists.lock().unwrap();
                let (ok, key) = if rest[0] == "add" {
                    if l.white.contains(&user) {
                        (false, "commands.whitelist.add.failed")
                    } else {
                        l.white.add(WhiteEntry { user: user.clone() });
                        (true, "commands.whitelist.add.success")
                    }
                } else if l.white.remove(&user) {
                    (true, "commands.whitelist.remove.success")
                } else {
                    (false, "commands.whitelist.remove.failed")
                };
                drop(l);
                let args = if ok {
                    vec![lit(&user.name)]
                } else {
                    Vec::new()
                };
                self.console_reply(Text::translate(key, &args));
                if ok && rest[0] == "remove" {
                    let lists = state.lists.clone();
                    self.kick_unlisted_players(&lists);
                }
            }
            _ => self.console_reply(Text::translate("command.unknown.argument", &[])),
        }
    }

    /// `DedicatedServer.setUsingWhitelist`: the flag, and `white-list` in
    /// server.properties.
    pub(in crate::game) fn set_using_whitelist(&mut self, lists: &Arc<Mutex<Lists>>, on: bool) {
        let changed = {
            let mut lists = lists.lock().unwrap();
            let changed = lists.use_whitelist != on;
            lists.use_whitelist = on;
            changed
        };
        self.store_property("white-list", on.to_string());
        if changed {
            self.native_whitelist_state_update(on);
        }
    }

    /// `DedicatedServer.setPlayerIdleTimeout`: update the live idle sweep and persist its minutes.
    pub(in crate::game::server_side) fn set_player_idle_timeout(
        &mut self,
        lists: &Arc<Mutex<Lists>>,
        minutes: i32,
    ) {
        lists.lock().unwrap().player_idle_timeout = minutes;
        self.store_property("player-idle-timeout", minutes.to_string());
    }

    /// `DedicatedServer.setMaxPlayers`: the live login limit, and `max-players` in
    /// server.properties.
    fn set_max_players(&mut self, state: &mut ServerState, max_players: usize) {
        state.max_players = max_players;
        if let Some(lan) = &self.server.lan {
            lan.hub.lan.shared.set_max_players(max_players);
        }
        self.store_property("max-players", max_players.to_string());
    }

    /// `DedicatedServer.setAcceptsTransfers`: whether a transfer's handshake is accepted from
    /// now on, and `accepts-transfers` in server.properties.
    fn set_accepts_transfers(&mut self, accept: bool) {
        if let Some(lan) = &self.server.lan {
            lan.hub.lan.shared.set_accepts_transfers(accept);
        }
        self.store_property("accepts-transfers", accept.to_string());
    }

    /// `DedicatedServer.setEnforceWhitelist`: the flag, and `enforce-whitelist` in
    /// server.properties.
    pub(in crate::game::server_side) fn set_enforce_whitelist(
        &mut self,
        lists: &Arc<Mutex<Lists>>,
        on: bool,
    ) {
        lists.lock().unwrap().enforce_whitelist = on;
        self.store_property("enforce-whitelist", on.to_string());
    }

    /// `DedicatedServer.setPauseWhenEmptySeconds`: the live pause timer, and
    /// `pause-when-empty-seconds` in server.properties.
    fn set_pause_when_empty_seconds(&mut self, state: &mut ServerState, seconds: i32) {
        state.pause_when_empty_seconds = seconds;
        self.store_property("pause-when-empty-seconds", seconds.to_string());
    }

    /// DedicatedServerSettings.update: change one key and store the file.
    pub(in crate::game::server_side) fn store_property(&mut self, key: &str, value: String) {
        let path = PathBuf::from("server.properties");
        if let Ok(p) = ServerProperties::from_file(&path, &mut SystemEntropy::default())
            && let Ok(p) = p.update(key, value)
        {
            let _ = p.store(&path);
        }
    }

    /// MinecraftServer.kickUnlistedPlayers: with `enforce-whitelist` and the
    /// whitelist on, whoever is online and not on the whitelist file is
    /// disconnected (operators are not spared here).
    pub(in crate::game) fn kick_unlisted_players(&mut self, lists: &Arc<Mutex<Lists>>) {
        let unlisted: Vec<u128> = {
            let l = lists.lock().unwrap();
            let Some(host) = self.server.lan.as_ref() else {
                return;
            };
            if !(l.enforce_whitelist && l.use_whitelist) {
                return;
            }
            host.hub
                .guests
                .iter()
                .filter(|g| {
                    g.participates_online()
                        && !l.white.contains(&NameAndId {
                            id: g.uuid,
                            name: g.name.clone(),
                        })
                })
                .map(|g| g.uuid)
                .collect()
        };
        for uuid in unlisted {
            self.kick_uuid(
                uuid,
                translatable("multiplayer.disconnect.not_whitelisted", &[]),
            );
        }
    }

    pub(in crate::game) fn kick_uuid(&mut self, uuid: u128, reason: Tag) {
        let conns: Vec<u32> = self.server.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| g.participates_online() && g.uuid == uuid)
                .map(|g| g.conn)
                .collect()
        });
        for conn in conns {
            self.kick_conn(conn, reason.clone());
        }
    }

    /// `Hub::kick`, with the server's `kick` event recorded. Whether it started the guest's
    /// disconnect: false for a missing guest and one whose disconnect is already pending.
    pub(in crate::game) fn kick_conn(&mut self, conn: u32, reason: Tag) -> bool {
        let Some(h) = self.server.lan.as_mut() else {
            return false;
        };
        if h.hub.guest(conn).is_none_or(|g| g.disconnect_pending) {
            return false;
        }
        h.hub.kick(conn, reason);
        let Some(g) = h.hub.guest(conn) else {
            return false;
        };
        events::record(
            "kick",
            &g.name,
            g.uuid,
            serde_json::json!({ "reason": g.left_reason }),
        );
        true
    }

    /// MinecraftServer.stopServer: players saved and sent away, worlds saved.
    /// MinecraftServer.saveAllChunks, not silent: one line per level, in
    /// the server's level order.
    pub(in crate::game) fn log_saved_levels(&self) {
        for dim in self.loaded_level_order() {
            log(
                "INFO",
                &format!(
                    "Saving chunks for level 'ServerLevel[{}]'/{}",
                    self.level.name,
                    dim.id()
                ),
            );
        }
    }

    pub(in crate::game) fn stop_dedicated(&mut self, crashed: bool) -> Result<()> {
        self.server.streaming.abort_generation();
        self.server.levels.0.abort_parked_generation();
        // The close request starts before player and world saves.
        self.common.native_lifecycle.begin_close();
        log("INFO", "Stopping server");
        self.finish_server_save(true, crashed)
    }
}

/// `tools/test_dedicated_crash_save.py`'s fixture: with `SUNRISE_CRASH_FIXTURE` naming `point`
/// (`guest`: during a placed guest's turn, `hub`: after a console line while a player is in),
/// the tick panics there, to see what crash teardown keeps.
/// `armed` is asked only when the variable names `point`, so the tick pays one
/// load of a `OnceLock` without it.
pub(in crate::game::server_side) fn crash_fixture(
    point: &'static str,
    armed: impl FnOnce() -> bool,
) {
    use sunrise_world::crash_fixture::{CrashFixture, selected};
    if selected() == Some(CrashFixture::Panic(point)) && armed() {
        panic!("crash fixture at {point}");
    }
}

/// The text of a panic payload (`panic!("..")` is a `&str`, a formatted one a `String`).
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_owned())
}

/// The width and height of a PNG (its IHDR chunk).
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// The level a console line runs in, when it is not `active`, the one in place: the world
/// spawn's (`MinecraftServer.createCommandSourceStack` uses `findRespawnDimension`).
fn console_level(active: super::Dim, spawn: super::Dim) -> Option<super::Dim> {
    (active != spawn).then_some(spawn)
}

impl crate::game::ServerGame<'_> {
    /// The count is diagnostic; only hard failures stop the lifecycle.
    pub fn save_dedicated_checkpoint(&mut self, phase: &str) -> Result<u64> {
        let before = self.server.save.0.record_save_failures();
        self.save_game()?;
        let failed = self.server.save.0.record_save_failures() - before;
        if failed != 0 {
            log::warn!("Partial save during {phase}: {failed} chunk records could not be saved");
        }
        Ok(failed)
    }
}

use sunrise_gameplay_common::game::tables::WorldTablesExt as _;

use sunrise_gameplay_common::game::native_configs::NativeConfigsGameExt1 as _;

use sunrise_gameplay_common::game::native_lifecycle_common::NativeLifecycleCommonGameExt1 as _;

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::server_side::dedicated::tests::CHILD_HOME;

    use sunrise_gameplay_common::game::server_side::dedicated::tests::LOGIN;

    use sunrise_gameplay_common::game::server_side::dedicated::tests::READY;

    use sunrise_gameplay_common::game::server_side::dedicated::tests::WARNING;

    use sunrise_gameplay_common::game::server_side::dedicated::tests::assert_log_lines;

    use super::{
        FORWARDING_REMOVED, capture_log, log, log_from, overload_warning, panic_message,
        refuse_forwarding_secret,
    };
    use std::{
        fs,
        io::{self, BufRead, BufReader, Read, Write},
        process::{Command, Stdio},
        time::{SystemTime, UNIX_EPOCH},
    };

    /// `TpsDebugDimensions` order from controlled inputs: the full tick is passed through
    /// (never clamped to the method), the scheduled tasks are the wait minus its idle time.
    #[test]
    fn a_tick_time_sample_holds_full_method_scheduled_and_idle() {
        use std::time::Duration as D;
        let ms = |n| D::from_millis(n);
        assert_eq!(
            super::tick_time_sample(ms(50), ms(12), ms(35), ms(30)),
            [50_000_000, 12_000_000, 5_000_000, 30_000_000]
        );
        assert_eq!(
            super::tick_time_sample(ms(40), ms(45), ms(0), ms(0)),
            [40_000_000, 45_000_000, 0, 0]
        );
    }

    /// A console line runs in the world spawn's level whichever level a server tick left in place.
    #[test]
    fn console_commands_run_in_the_world_spawn_level() {
        use super::super::Dim;
        let overworld = Dim::Overworld;
        assert_eq!(
            super::console_level(Dim::Nether, overworld),
            Some(overworld)
        );
        assert_eq!(super::console_level(Dim::End, overworld), Some(overworld));
        assert_eq!(super::console_level(overworld, overworld), None);
        assert_eq!(super::console_level(overworld, Dim::End), Some(Dim::End));
        assert_eq!(super::console_level(Dim::End, Dim::End), None);
    }

    /// `BanIpCommands.banIp`: the info line names the affected players, one or several, and is
    /// absent when no one is affected.
    #[test]
    fn ban_ip_info_names_the_affected_players() {
        let args = |t: &super::Text| {
            let c = t.component();
            c["with"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a.to_string())
                .collect::<Vec<_>>()
        };
        assert!(super::ban_ip_info(&[]).is_none());
        let one = super::ban_ip_info(&["Alex"]).unwrap();
        assert_eq!(one.translation_key(), Some("commands.banip.info"));
        let a = args(&one);
        assert!(a[0].contains('1') && a[1].contains("Alex"), "{a:?}");
        let two = super::ban_ip_info(&["Alex", "Steve"]).unwrap();
        let a = args(&two);
        assert!(a[0].contains('2'), "{a:?}");
        assert!(
            a[1].contains("Alex") && a[1].contains(", ") && a[1].contains("Steve"),
            "{a:?}"
        );
    }

    /// A server folder that still has the forwarding secret is told what to do.
    #[test]
    fn a_leftover_forwarding_secret_says_what_to_do() {
        let dir = std::env::temp_dir().join(format!("sunrise-forwarding-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(refuse_forwarding_secret(&dir), Ok(()));
        fs::write(dir.join("forwarding.secret"), "secret\n").unwrap();
        assert_eq!(
            refuse_forwarding_secret(&dir),
            Err(FORWARDING_REMOVED.to_owned())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// ServerGamePacketListenerImpl.tick: a player idle past `player-idle-timeout` minutes is kicked,
    /// unless the End poem is showing (`ServerPlayer.wonGame`), which sends no action.
    #[test]
    fn a_player_watching_the_end_credits_is_not_kicked_for_idling() {
        use super::idles_out;
        // 5 minutes: 6000 ticks. One more is over the limit.
        assert!(!idles_out(6000, 5, false));
        assert!(idles_out(6001, 5, false));
        assert!(!idles_out(6001, 5, true));
        assert!(!idles_out(1_000_000, 5, true));
        // The credits are over (won_game false again): the same idle time kicks.
        assert!(idles_out(1_000_000, 5, false));
        // 0 or less: no timeout.
        assert!(!idles_out(1_000_000, 0, false));
        assert!(!idles_out(1_000_000, -3, false));
    }

    /// `SaveAllCommand.saveAll`: a save that throws is the `ERROR_FAILED` the
    /// source is told about, not a log line the caller never sees.
    #[test]
    fn a_failed_save_tells_the_source_that_it_failed() {
        assert_eq!(
            super::save_all_reply(true).translation_key(),
            Some("commands.save.success")
        );
        assert_eq!(
            super::save_all_reply(false).translation_key(),
            Some("commands.save.failed")
        );
    }

    /// Vanilla's words, with how far behind the server is in milliseconds and in whole ticks.
    #[test]
    fn done_uses_java_half_up_world_load_seconds() {
        assert_eq!(
            super::startup_done(std::time::Duration::from_nanos(2_345_500_000)),
            "Done (2.346s)! For help, type \"help\""
        );
    }

    #[test]
    fn the_overload_warning_reads_as_vanilla_s() {
        let tick = std::time::Duration::from_millis(50);
        assert_eq!(
            overload_warning(std::time::Duration::from_millis(13_027), tick),
            "Can't keep up! Is the server overloaded? Running 13027ms or 260 ticks behind"
        );
    }

    /// MinecraftServer.tickServer: a tick only counts toward pause-when-empty with no player and
    /// no `/tick sprint` running (`ServerTickRateManager.isSprinting`); a sprint on an empty server
    /// keeps the count at 0, so it is never paused, and the count starts again after it.
    #[test]
    fn a_sprint_on_an_empty_server_does_not_count_toward_the_pause() {
        let pause_after = 60 * 20;
        let mut empty_ticks = 0;
        // One server tick: (players, a `/pregen` task, a `/tick sprint`) -> paused.
        let tick = |empty_ticks: &mut i32, players, pregen, sprinting| {
            super::count_empty_tick(empty_ticks, pause_after, players, pregen, sprinting)
        };
        for _ in 0..2000 {
            assert!(!tick(&mut empty_ticks, 0, false, true));
        }
        assert_eq!(empty_ticks, 0);
        // The sprint ended: 1200 empty ticks later the server pauses, not before.
        for n in 1..=pause_after {
            assert_eq!(
                tick(&mut empty_ticks, 0, false, false),
                n == pause_after,
                "tick {n}"
            );
        }
        // A sprint that starts while paused resumes the server at once.
        assert!(!tick(&mut empty_ticks, 0, false, true));
        assert_eq!(empty_ticks, 0);
        // A player, or a `/pregen` task, resets the count too.
        empty_ticks = 5;
        assert!(!tick(&mut empty_ticks, 1, false, false));
        assert_eq!(empty_ticks, 0);
        empty_ticks = 5;
        assert!(!tick(&mut empty_ticks, 0, true, false));
        assert_eq!(empty_ticks, 0);
        // pause-when-empty-seconds=0 never pauses and leaves the count alone.
        empty_ticks = 5;
        assert!(!super::count_empty_tick(
            &mut empty_ticks,
            0,
            0,
            false,
            false
        ));
        assert_eq!(empty_ticks, 5);
    }

    /// A `panic!("literal")` carries a `&str`, a formatted one a `String`; anything else is
    /// named as unknown, so the crash line is never empty.
    #[test]
    fn a_panic_payload_is_read_as_text() {
        let literal = std::panic::catch_unwind(|| panic!("a literal")).unwrap_err();
        assert_eq!(panic_message(literal.as_ref()), "a literal");
        let n = 3;
        let formatted = std::panic::catch_unwind(|| panic!("tick {n}")).unwrap_err();
        assert_eq!(panic_message(formatted.as_ref()), "tick 3");
        let other = std::panic::catch_unwind(|| std::panic::panic_any(7_u8)).unwrap_err();
        assert_eq!(panic_message(other.as_ref()), "unknown panic");
    }

    #[test]
    #[ignore = "daily"]
    fn console_logging_survives_closed_stdout() {
        if let Some(home) = std::env::var_os(CHILD_HOME) {
            sunrise_level::logging::init(std::path::Path::new(&home), "Server thread");
            // Let the parent close the real stdout pipe only after libtest's startup output.
            io::stdout().write_all(READY.as_bytes()).unwrap();
            io::stdout().flush().unwrap();
            io::stdin().read_exact(&mut [0]).unwrap();
            std::thread::Builder::new()
                .name("Server thread".into())
                .spawn(|| {
                    let captured = capture_log(|| {
                        log("INFO", LOGIN);
                        log_from("User Authenticator #1", "WARN", WARNING);
                    });
                    assert_eq!(captured, [LOGIN, WARNING]);
                    sunrise_level::logging::flush();
                })
                .unwrap()
                .join()
                .unwrap();
            // Libtest's final status also writes to stdout; only the game logger is under test.
            std::process::exit(0);
        }

        for closed_stdout in [false, true] {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let home = std::env::temp_dir().join(format!(
                "sunrise-console-log-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&home).unwrap();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--include-ignored",
                    "--exact",
                    "game::server_side::dedicated::tests::console_logging_survives_closed_stdout",
                    "--nocapture",
                ])
                .env(CHILD_HOME, &home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            loop {
                let mut line = String::new();
                assert!(
                    stdout.read_line(&mut line).unwrap() > 0,
                    "child was not ready"
                );
                if line.ends_with(READY) {
                    break;
                }
            }
            if closed_stdout {
                drop(stdout); // No reader remains: the next console write gets EPIPE.
            } else {
                child.stdout = Some(stdout.into_inner());
            }
            child.stdin.take().unwrap().write_all(&[1]).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "closed_stdout={closed_stdout}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if !closed_stdout {
                assert_log_lines(std::str::from_utf8(&output.stdout).unwrap());
            }
            assert_log_lines(&fs::read_to_string(home.join("logs/latest.log")).unwrap());
            fs::remove_dir_all(home).unwrap();
        }
    }

    /// `BanPlayerCommands.banPlayers` skips a player who `isBanned`, and `BanIpCommands.banIp` refuses an
    /// address that is: the console's `/ban` and `/ban-ip` of one banned already change nothing (the first
    /// reason stays, and the console hears "Nothing changed"), whatever reason they give.
    #[test]
    fn the_console_does_not_ban_again_who_is_banned_already() {
        use super::{BanList, Lists, NameAndId};
        let dir = std::env::temp_dir().join(format!("sunrise-console-ban-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut lists = Lists {
            ops: super::OpList::new(dir.join("ops.json")),
            white: super::WhiteList::new(dir.join("whitelist.json")),
            bans: BanList::new(dir.join("banned-players.json")),
            ip_bans: super::IpBanList::new(dir.join("banned-ips.json")),
            use_whitelist: false,
            enforce_whitelist: false,
            op_level: 4,
            player_idle_timeout: 0,
            stop: false,
            autosave: true,
        };
        let alpha = NameAndId {
            id: 0x5ca1ab1e,
            name: "Alpha".into(),
        };
        assert!(lists.ban(&alpha, Some("cheating".into())));
        assert!(!lists.ban(&alpha, Some("again".into())));
        assert!(!lists.ban(&alpha, None));
        let reason = lists.bans.get(&alpha).and_then(|b| b.reason.clone());
        assert_eq!(reason.as_deref(), Some("cheating"));
        assert!(lists.ban_ip("1.2.3.4", Some("spam".into())));
        assert!(!lists.ban_ip("1.2.3.4", Some("more spam".into())));
        let reason = lists
            .ip_bans
            .get(&"1.2.3.4".to_owned())
            .and_then(|b| b.reason.clone());
        assert_eq!(reason.as_deref(), Some("spam"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A temporary ban keeps the player out and says when it ends;
    /// once it has ended, or after `/pardon`, they come in.
    #[test]
    fn a_temporarily_banned_player_is_refused_with_the_end_until_it_has_passed() {
        use super::{BanEntry, BanList, Lists, NameAndId};
        use sunrise_server_files::duration::now_millis;
        let dir = std::env::temp_dir().join(format!("sunrise-can-login-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut lists = Lists {
            ops: super::OpList::new(dir.join("ops.json")),
            white: super::WhiteList::new(dir.join("whitelist.json")),
            bans: BanList::new(dir.join("banned-players.json")),
            ip_bans: super::IpBanList::new(dir.join("banned-ips.json")),
            use_whitelist: false,
            enforce_whitelist: false,
            op_level: 4,
            player_idle_timeout: 0,
            stop: false,
            autosave: true,
        };
        // Every translation key and string in a message, in the order they come.
        fn words(tag: &super::Tag, out: &mut Vec<String>) {
            match tag {
                super::Tag::String(s) => out.push(s.to_string_lossy()),
                super::Tag::List(items) => items.iter().for_each(|t| words(t, out)),
                super::Tag::Compound(_) => {
                    for key in ["translate", "with", "extra"] {
                        if let Some(t) = tag.get(key) {
                            words(t, out);
                        }
                    }
                }
                _ => {}
            }
        }
        let message = |tag: Option<super::Tag>| {
            let mut out = Vec::new();
            words(&tag.expect("a refusal"), &mut out);
            out.join(" | ")
        };
        let address: std::net::SocketAddr = "127.0.0.1:25565".parse().unwrap();
        let user = |id: u128, name: &str| NameAndId {
            id,
            name: name.into(),
        };
        let ban = |user: &NameAndId, expires: Option<i64>| {
            BanEntry::new(
                user.clone(),
                Some("Op".into()),
                expires,
                Some("griefing".into()),
            )
        };
        let (steve, alex, dee) = (user(1, "Steve"), user(2, "Alex"), user(3, "Dee"));
        // Free until banned.
        assert!(lists.can_login(address, &steve).is_none());
        // A ban for an hour: refused, with the reason and when it ends.
        lists.bans.add(ban(&steve, Some(now_millis() + 3_600_000)));
        let refused = message(lists.can_login(address, &steve));
        assert!(
            refused.contains("multiplayer.disconnect.banned.reason"),
            "{refused}"
        );
        assert!(
            refused.contains("multiplayer.disconnect.banned.expiration"),
            "{refused}"
        );
        assert!(refused.contains("griefing"), "{refused}");
        // A permanent ban is refused with no end.
        lists.bans.add(ban(&alex, None));
        let refused = message(lists.can_login(address, &alex));
        assert!(!refused.contains("expiration"), "{refused}");
        // A ban that has ended lets the player in, and the list drops it.
        lists.bans.add(ban(&dee, Some(now_millis() - 1_000)));
        assert!(lists.can_login(address, &dee).is_none());
        assert!(!lists.bans.contains(&dee));
        // `/pardon` lifts the hour's ban.
        assert!(lists.bans.remove(&steve));
        assert!(lists.can_login(address, &steve).is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
