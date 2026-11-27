//! The owner link: the closed set of requests the window sends its integrated server thread.
use super::{
    Dim,
    integrated_link::ServerOptions,
    lan_common::LanSettings,
    memory_common::{ServerMemoryRequest, ServerSide},
    mods::ModsReload,
    profile_capture_common::{Live, Target},
};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, SyncSender},
    },
};
use sunrise_core::geometry::Vec3;
use sunrise_perf_format::v1::StopReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub default_mode: i32,
    pub personal_mode: i32,
    pub cheats: bool,
    pub difficulty: u8,
    pub locked: bool,
    pub lan: bool,
    pub port: u16,
    pub guest_commands: bool,
    pub force: bool,
}

/// Where the server puts the answer to a request the window waits for.
pub type Reply<T> = SyncSender<T>;

/// Work the window hands its server thread between two ticks (`MinecraftServer.execute`).
pub enum OwnerRequest {
    /// `IntegratedServer.publishServer`.
    Publish(LanSettings, Reply<Result<u16, String>>),
    /// `IntegratedServer.unpublishServer`: whether it was published.
    Unpublish(Reply<bool>),
    /// `sunrise-ctl lan status`.
    LanStatus(Reply<serde_json::Value>),
    /// The world options screen's state.
    ServerOptions(Reply<ServerOptions>),
    /// The world options screen's changes, from `before` to `wanted`.
    ApplyWorldOptions {
        before: Snapshot,
        wanted: Snapshot,
        done: Reply<(bool, Option<u16>)>,
    },
    /// Far terrain for a client in `dim`.
    LodSource(
        Dim,
        Reply<Option<Arc<dyn sunrise_level::chunk_streamer::LodSource>>>,
    ),
    /// The pregen commands are on or off.
    SetPregen(bool),
    /// A live content mod switch: the data packs reload from the new root.
    ReloadMods(Box<ModsReload>),
    /// The server thread's side of a capture starts, for `target`; the reply, when one waits, gets its path.
    StartServerCapture(Target, Arc<Live>, Option<Reply<Result<PathBuf, String>>>),
    /// The server thread's capture stops; the saved path goes to the reply when one waits.
    StopServerCapture(StopReason, Option<Sender<String>>),
    /// `/profiler mark`: a named moment in the server thread's capture.
    MarkServerCapture(String),
    /// What agent control asks of the world.
    Control(ControlRequest),
    /// Tests reach into the server thread's `Game` (`Integrated::call`, `Integrated::post`).
    #[cfg(any())]
    InTest(Box<dyn FnOnce(&mut crate::game::ServerGame<'_>) + Send>),
    #[cfg(feature = "test-owner-requests")]
    TestTask(Box<dyn std::any::Any + Send>),
}

/// The agent-control asks, answered on the server thread in the owner's level.
pub enum ControlRequest {
    /// `simulation blocks`: the server's own blocks in a box.
    Blocks {
        lo: [i32; 3],
        hi: [i32; 3],
        reply: Reply<Value>,
    },
    /// `simulation`: the server's clock and projectiles.
    Simulation(Reply<Value>),
    /// `server-hold`: stop or resume the server's tick.
    HoldServer { held: bool, done: Reply<()> },
    /// A console command run as the owner: the chat lines it produced and its outcome.
    ConsoleCommand {
        command: String,
        reply: Reply<(Vec<String>, Option<(bool, i32)>)>,
    },
    /// `time`: the server's day time.
    SetDayTime { time: i64, done: Reply<()> },
    /// `summon`: the new mob's id.
    Summon {
        kind: String,
        at: [f64; 3],
        baby: bool,
        reply: Reply<Result<u32, String>>,
    },
    /// `mobs`: the mobs near `center`.
    MobsAt {
        radius: f64,
        center: Vec3,
        reply: Reply<Value>,
    },
    /// `hurtlog`.
    HurtLog(Reply<Value>),
    /// `save` and `save-all`.
    Save {
        all: bool,
        reply: Reply<Result<(), String>>,
    },
    /// `setblock`, `fill`, `getblock` and `container`, with their words.
    BlockCommand {
        words: Vec<String>,
        reply: Reply<Result<Option<Value>, String>>,
    },
    /// The explicit memory report: the server's own parts.
    ServerSide(Reply<ServerSide>),
    /// A profiling sample, delivered directly to the worker while it is wanted.
    MemorySample(ServerMemoryRequest),
    /// `lan op NAME LEVEL`: whether a guest of that name was found.
    SetOp {
        name: String,
        level: u8,
        reply: Reply<bool>,
    },
    /// The save folder of the server's world.
    WorldRoot(Reply<PathBuf>),
    /// The recording's copy: save, then hold the server thread until `copied` is dropped.
    SaveThenWait {
        saved: Sender<Result<(), String>>,
        copied: Receiver<()>,
    },
    /// Owner's server-side shield disable cooldown ticks.
    ShieldCooldown(Reply<Result<u32, String>>),
    /// `sculk` control command, executed in the owner guest's turn.
    Sculk {
        words: Vec<String>,
        reply: Reply<Result<Value, String>>,
    },
    /// `dragon-hurt` control test hook, crediting the owner guest.
    DragonHurt {
        amount: f32,
        reply: Reply<Result<(), String>>,
    },
    /// Server-side `fishing` and `fight` fields for `sunrise-ctl status`.
    StatusExtras(Reply<Result<(Option<Value>, Value), String>>),
    /// `xp` station control command; returns resulting experience level.
    GiveLevels {
        levels: i32,
        reply: Reply<Result<i32, String>>,
    },
    /// `effect` / `effect clear` station control command.
    StationEffect {
        effect: Option<sunrise_level::effects::EffectInstance>,
        reply: Reply<Result<(), String>>,
    },
}
