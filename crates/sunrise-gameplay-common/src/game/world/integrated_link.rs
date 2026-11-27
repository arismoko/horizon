use crate::game::launch_options::Options;
use crate::game::owner_link::OwnerRequest;
use crate::game::{Dim, PackResources, Resources};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, Sender},
    },
    time::{Duration, Instant},
};
use sunrise_game_data::Result;
use sunrise_level::{level_loading::ServerLoadView, sim_settings::SimSettings};
use sunrise_protocol::connection::Pipe;
use sunrise_sim::{chunk_io::Reports, lan::OwnerProfile, world_options::WorldOptions};

/// How long the window waits for the server thread to answer an `ask`.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(10);
/// The error of an `ask` without an answer: the server stopped, or it was
/// busy past `CALL_TIMEOUT` (the log says which).
pub const NO_ANSWER: &str =
    "the integrated server stopped or did not answer within 10 s (the game log says which)";

/// The server thread holds its `Game` by value and runs every tick on it. It uses the game
/// worker stack floor or a larger `RUST_MIN_STACK` requested by a harness. std only reads that
/// variable for threads spawned without an explicit size.
pub fn server_thread_stack() -> usize {
    static SIZE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *SIZE.get_or_init(|| stack_for(std::env::var("RUST_MIN_STACK").ok().as_deref()))
}

/// Interpret `min_stack` as a byte count, as std parses `RUST_MIN_STACK`.
pub fn stack_for(min_stack: Option<&str>) -> usize {
    min_stack
        .and_then(|bytes| bytes.parse::<usize>().ok())
        .unwrap_or_default()
        .max(sunrise_threads::GAME_THREAD_STACK_BYTES)
}

/// `LevelLoadProgressTracker.EXPECTED_PLAYER_CHUNKS`: the 7x7 chunks around the spawn.
pub const PLAYER_CHUNKS: usize = 49;

/// What the window and its server thread share.
#[derive(Default)]
pub struct Flags {
    #[cfg(any(test, feature = "test-support"))]
    pub fail_tick: AtomicBool,
    /// `Minecraft.pause`: a single-player pause screen is open.
    pub paused: AtomicBool,
    pub stop: AtomicBool,
    pub finished: AtomicBool,
    pub failure: Mutex<Option<String>>,
    /// The owning window's report inbox, shared with this server and its region workers.
    pub reports: Arc<Reports>,
    /// `IntegratedServer.getPort`: the port it is open to the LAN on, 0
    /// while it is not.
    pub published: AtomicU16,
    pub smoothed_tick_ms: AtomicU32,
    /// `options.renderDistance` and `options.simulationDistance`, which the
    /// server follows while it runs (IntegratedServer.tickServer).
    pub render_distance: AtomicI32,
    pub simulation_distance: AtomicI32,
    /// `options.entityDistanceScaling` (an `f64`'s bits), which
    /// `IntegratedServer.getScaledTrackingDistance` reads on every call.
    pub entity_distance: AtomicU64,
    /// `MinecraftServer.isCurrentlySaving`: the server is autosaving.
    pub saving: AtomicBool,
    /// The generators of the levels the server has run, by dimension: what
    /// F3's chunk generation entry asks `ServerChunkCache.getGenerator`.
    pub terrain: Mutex<
        std::collections::HashMap<&'static str, Arc<sunrise_sim::vanillagen::VanillaTerrain>>,
    >,
    /// What the server's `LevelLoadListener` reports, for the window's
    /// `LevelLoadingScreen` (`LevelLoadTracker.serverProgress/statusView`).
    pub load: Arc<ServerLoadView>,
    /// What the world's upgrade has done (`UpgradeProgress`): the window shows it on
    /// `FileFixerProgressScreen` while the file fixers run, and its Cancel stops them.
    pub upgrade: Arc<sunrise_world::datafix::progress::UpgradeProgress>,
    /// Where the window's debug entries want the server's lines for (its
    /// dimension and feet), and the lines of the last tick there.
    pub debug_at: Mutex<Option<(Dim, [i32; 3])>>,
    pub debug: Mutex<(u64, crate::game::debug_lines::ServerDebug)>,
}

impl Flags {
    pub fn emergency_save(&self, server: std::thread::ThreadId, timeout: Duration) -> bool {
        if std::thread::current().id() == server || self.finished.load(Ordering::Acquire) {
            return true;
        }
        log::warn!("Sunrise: saving the world before the crash");
        self.stop.store(true, Ordering::Relaxed);
        let started = Instant::now();
        while !self.finished.load(Ordering::Acquire) {
            let Some(left) = timeout.checked_sub(started.elapsed()) else {
                log::warn!(
                    "Sunrise: the world did not save within {} s",
                    timeout.as_secs()
                );
                return false;
            };
            std::thread::sleep(left.min(Duration::from_millis(10)));
        }
        true
    }
}

// Signal even if serve returns an error or unwinds; no game data is touched here.
pub struct Finished(Arc<Flags>);
impl Finished {
    pub fn new(flags: Arc<Flags>) -> Self {
        Self(flags)
    }
}

impl Drop for Finished {
    fn drop(&mut self) {
        self.0.finished.store(true, Ordering::Release);
    }
}

/// What the server thread hands the window once the world is open: its end
/// of the connection and the world's name, or why it failed.
pub type Ready = std::result::Result<(Pipe, String, Arc<sunrise_level::biome::Biomes>), String>;

/// The world to open and the player who owns it.
pub struct Launch {
    pub folder: String,
    pub rule_registry: Arc<sunrise_game_data::gamerules::RuleRegistry>,
    pub name: Option<String>,
    pub seed: Option<i64>,
    pub creative: bool,
    pub create: Option<WorldOptions>,
    /// The window's profile: the only one let in until the world is open to
    /// the LAN, and the world's player.
    pub owner: u128,
    /// Its name, for the LAN's MOTD.
    pub player: String,
    /// The signed-in account's profile, fetched while the world loads.
    pub owner_profile: Option<Arc<OwnerProfile>>,
    pub online_auth: bool,
    pub session_verifier: sunrise_sim::lan::SessionVerifier,
}

/// What the world options screen shows of an integrated server.
#[derive(Clone, Copy, Debug)]
pub struct ServerOptions {
    pub default_mode: i32,
    /// The owner's own game mode.
    pub personal_mode: i32,
    pub cheats: bool,
    pub difficulty: u8,
    pub hardcore: bool,
    pub difficulty_locked: bool,
    pub port: Option<u16>,
    pub guest_commands: bool,
    pub force: bool,
}

/// Component oracles supply their own synthetic levels and entity phases.
/// They still execute on a server Game, never the client's world.
pub enum ComponentHarness {
    Dispenser(String),
    Sculk(String),
    /// The hot-path benchmark (`hot_paths`), which builds its own levels the same way.
    HotPaths(String),
    /// Authoritative boat-attack cases for `SUNRISE_BOAT_AUTHORITY_FIXTURE`.
    BoatAuthorityAttacks(
        std::path::PathBuf,
        Arc<Mutex<Vec<(usize, serde_json::Value)>>>,
    ),
}

pub struct CreationPackRead<'a> {
    pub resources: &'a Arc<Resources>,
    pub pack: &'a PackResources,
    pub options: &'a Options,
    pub settings: SimSettings,
    pub pack_dir: Option<&'a std::path::Path>,
    pub selected: &'a [String],
    pub profile_resolver: ProfileResolverFactory,
}

pub type ReadCreationPacks = fn(CreationPackRead<'_>) -> Result<()>;

pub type ProfileResolverFactory = fn() -> Box<dyn sunrise_level::profile_lookup::ProfileResolver>;
pub type Serve = fn(
    &Arc<Resources>,
    PackResources,
    Options,
    SimSettings,
    Launch,
    Arc<Flags>,
    Receiver<OwnerRequest>,
    &Sender<Ready>,
    ProfileResolverFactory,
) -> Result<()>;
pub type RunComponentHarness = fn(
    Arc<Resources>,
    PackResources,
    Options,
    SimSettings,
    ComponentHarness,
    ProfileResolverFactory,
) -> Result<()>;
