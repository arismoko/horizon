//! Captures (docs/perf-platform.md §4.1): `sunrise-ctl profile`, `/profiler` and `SUNRISE_PROFILE`. The window writes its own side; in single player the server thread writes its side into the same file.
use crate::game::ClientGame;
use crate::game::client_side::memory_client;
use crate::game::client_side::profile_lines::{self as lines, Notice};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    mpsc::{Receiver, Sender, TryRecvError},
};
use std::task::Poll;
use std::time::{Duration, Instant};
use sunrise_client::client_perf::{
    self, ClientPerf,
    request::{
        self, Failed, GpuSwitch, Marked, Opened, ProfileSide, Request, Shared, StartOptions,
        Started, Status, Stopped, Traced,
    },
};
use sunrise_command::{Arg, CommandError, Dispatcher, PROFILER_GPU, Registries};
use sunrise_game_data::Result;
use sunrise_gameplay_common::game::integrated_link::NO_ANSWER;
use sunrise_gameplay_common::game::memory_common::{MEMORY_EVERY_S, PendingMemory};
use sunrise_gameplay_common::game::owner_link::OwnerRequest;
use sunrise_gameplay_common::game::profile_capture_common::{
    ENV_DIR, Live, Target, reply_text, reports_dir, shown,
};
use sunrise_level::spans::{self, Span};
use sunrise_perf_format::{
    files,
    v1::{BenchResult, ClientSettings, StopReason, Trigger},
};
use sunrise_sim::perf_net::Link;
use sunrise_sim::server_perf::{self, TickDetail, WorldCounts};

/// How often a progress notice is set again so the action bar keeps it until the work is done.
pub(in crate::game) const PROGRESS_EVERY: Duration = Duration::from_secs(2);

/// The file `SUNRISE_TRACE` names: a timeline-only capture written there.
pub(in crate::game) const ENV_TRACE: &str = "SUNRISE_TRACE";

pub(in crate::game) const TRACE_USAGE: &str = "trace [FILE [SECONDS] | stop]";

/// What the `trace` words ask for.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum Trace<'a> {
    Status,
    Stop,
    Start { file: &'a str, seconds: Option<u32> },
}

/// The words after `trace`; SECONDS is a whole number in 1..=`MAX_WINDOW_S`, as for `profile --seconds`.
pub(in crate::game) fn parse_trace<'a>(
    words: &[&'a str],
) -> std::result::Result<Trace<'a>, &'static str> {
    match words {
        [] => Ok(Trace::Status),
        ["stop"] => Ok(Trace::Stop),
        [file] => Ok(Trace::Start {
            file,
            seconds: None,
        }),
        [file, seconds] => match seconds.parse::<u32>() {
            Ok(s) if s >= 1 && f64::from(s) <= server_perf::MAX_WINDOW_S => Ok(Trace::Start {
                file,
                seconds: Some(s),
            }),
            _ => Err(TRACE_USAGE),
        },
        _ => Err(TRACE_USAGE),
    }
}

pub(in crate::game) const NO_CAPTURE: &str = "no capture is running; start one with profile start";

/// Whether the single-player server thread writes into the window's capture.
pub(in crate::game) enum ServerLink {
    /// The capture is of the window alone.
    Off,
    /// The capture wants the server thread's side, which is asked to join once there is a server thread.
    Pending,
}

/// Which capture a `profile stop` ends.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum StopRoute {
    Window,
    ServerOnly,
    /// A capture of the connected server alone.
    Remote,
}

/// A capture of the server thread alone, which writes its own file.
pub(in crate::game) struct ServerOnly {
    pub(in crate::game) live: Arc<Live>,
    pub(in crate::game) path: PathBuf,
    pub(in crate::game) since: Instant,
    /// The window's game clock when the capture began.
    pub(in crate::game) began: f64,
}

/// An action bar notice that is set again every [`PROGRESS_EVERY`] until its work is done.
pub(in crate::game) struct Progress {
    pub(in crate::game) working: Working,
    pub(in crate::game) due: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum Working {
    Saving,
    Uploading,
}

/// What a player asked of a saved capture, through the captures hook.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum Job {
    Open,
    Share,
}

#[derive(Default)]
pub(in crate::game) struct MemorySchedule {
    /// When the next periodic sample is due and its elapsed period number.
    pub(in crate::game) due: Option<(f64, u32)>,
    /// The labeled sample, including canceled work that has not yet released its snapshots.
    pub(in crate::game) pending: Option<(String, f64, PendingMemory)>,
}

impl MemorySchedule {
    pub(in crate::game) fn cancel(&mut self) {
        self.due = None;
        if let Some((_, _, pending)) = &mut self.pending {
            pending.cancel();
        }
    }
}

/// What an iteration of the game loop did, waiting for its wall time: the interval from its
/// start to the next iteration's, which only that iteration knows. It books the frame with it,
/// so a slow frame is recorded with its own work, never with the frame after it.
pub(in crate::game) struct UnbookedFrame {
    pub(in crate::game) cpu_ms: f32,
    pub(in crate::game) gpu_ms: Option<f32>,
    pub(in crate::game) spans: Vec<Span>,
}

/// The game's capture state: the health history, the running capture and who is waiting on it.
pub(in crate::game) struct Profile {
    pub(in crate::game) link: ServerLink,
    /// The server thread's side of the running or last window capture, once it was asked.
    pub(in crate::game) joined: Option<Arc<Live>>,
    pub(in crate::game) server_only: Option<ServerOnly>,
    pub(in crate::game) perf: ClientPerf,
    pub(in crate::game) started_by: Trigger,
    pub(in crate::game) memory: MemorySchedule,
    /// Where the answer to a `/profiler` request that finishes later arrives.
    pub(in crate::game) chat_answer: Option<Receiver<String>>,
    pub(in crate::game) jobs: VecDeque<(PathBuf, Job)>,
    pub(in crate::game) job: Option<(PathBuf, Job, Receiver<std::result::Result<String, String>>)>,
    pub(in crate::game) last_capture: Option<PathBuf>,
    /// How long the capture had run when its stop was asked, for the saved line.
    pub(in crate::game) stopped_after: Option<f64>,
    /// The action bar notice that stays up until the work it names is done.
    pub(in crate::game) progress: Option<Progress>,
    /// `profile stop` waits for the file to be whole.
    pub(in crate::game) reply: Option<Sender<String>>,
    pub(in crate::game) spans_were_on: bool,
    /// The file the running capture was asked to write by `trace` or `SUNRISE_TRACE`.
    pub(in crate::game) trace_file: Option<String>,
    pub(in crate::game) hardware_warmed: bool,
    pub(in crate::game) env_tried: bool,
    /// What the running capture asks the renderer to time.
    pub(in crate::game) gpu: GpuSwitch,
    /// What the last iteration did, when it has a renderer to time it.
    pub(in crate::game) unbooked: Option<UnbookedFrame>,
    /// This client's capture on the server it is connected to.
    pub(in crate::game) server_link: Link,
    /// `profile` waits for the server's answer to its start, or for its file after a stop.
    pub(in crate::game) server_reply: Option<Sender<String>>,
    /// Whether the server's capture belongs to a window capture, whose stop ends it.
    pub(in crate::game) server_joined: bool,
    /// The window capture's file, held open for the server's part until it has all arrived.
    pub(in crate::game) server_sink: Option<super::perf_link::Sink>,
    /// The server's file while it arrives.
    pub(in crate::game) incoming: Option<super::perf_link::Incoming>,
}

impl Profile {
    pub(in crate::game) fn new(slow_ms: f32) -> Self {
        Self {
            link: ServerLink::Off,
            joined: None,
            server_only: None,
            perf: ClientPerf::new(slow_ms),
            started_by: Trigger::Unspecified,
            memory: MemorySchedule::default(),
            chat_answer: None,
            jobs: VecDeque::new(),
            job: None,
            last_capture: None,
            stopped_after: None,
            progress: None,
            reply: None,
            spans_were_on: false,
            trace_file: None,
            hardware_warmed: false,
            env_tried: false,
            gpu: GpuSwitch::Off,
            unbooked: None,
            server_link: Link::default(),
            server_reply: None,
            server_joined: false,
            server_sink: None,
            incoming: None,
        }
    }

    /// The GPU timing the running capture needs; off when none runs.
    pub(in crate::game::client_side) fn gpu(&self) -> GpuSwitch {
        self.gpu
    }

    pub(in crate::game::client_side) fn server_live(&self) -> bool {
        self.perf.capturing() && self.joined.as_ref().is_some_and(|l| l.active())
    }

    pub(in crate::game::client_side) fn server_only_live(&self) -> bool {
        self.server_only.as_ref().is_some_and(|s| s.live.active())
    }

    /// Whether the last window capture holds the server thread's side.
    pub(in crate::game) fn served(&self) -> bool {
        self.joined.as_ref().is_some_and(|l| l.wrote())
    }

    /// Where a `profile stop` for `side` (or for any side) goes.
    pub(in crate::game) fn stop_route(
        &self,
        side: Option<ProfileSide>,
    ) -> std::result::Result<StopRoute, &'static str> {
        match side {
            Some(ProfileSide::Client) if !self.perf.capturing() => {
                Err("no client capture is running")
            }
            Some(ProfileSide::Server) if !self.sides().contains(&ProfileSide::Server) => {
                Err("no server capture is running")
            }
            _ if self.perf.capturing() => Ok(StopRoute::Window),
            _ if self.server_only_live() => Ok(StopRoute::ServerOnly),
            _ if self.server_link.running() => Ok(StopRoute::Remote),
            _ => Err(NO_CAPTURE),
        }
    }

    /// The reply to `profile status`; a finished server-only capture is not reported.
    pub(in crate::game) fn status(&self) -> Status {
        let client = self.perf.status(self.perf.now());
        let alone = self
            .server_only
            .as_ref()
            .filter(|s| client.is_none() && s.live.active());
        let remote = self
            .server_link
            .status(Instant::now())
            .filter(|_| client.is_none() && alone.is_none());
        Status {
            ok: true,
            capturing: client.is_some() || alone.is_some() || remote.is_some(),
            sides: self.sides(),
            path: client
                .as_ref()
                .map(|s| shown(&s.path))
                .or_else(|| alone.map(|s| shown(&s.path)))
                .or_else(|| remote.map(|(path, _)| path.to_owned())),
            elapsed_s: client
                .as_ref()
                .map(|s| s.elapsed_s)
                .or_else(|| alone.map(|s| s.since.elapsed().as_secs_f64()))
                .or_else(|| remote.map(|(_, elapsed)| elapsed)),
            bytes: client
                .as_ref()
                .map(|s| s.bytes)
                .or_else(|| alone.and_then(|s| Some(std::fs::metadata(&s.path).ok()?.len()))),
        }
    }

    /// Marks the window's capture if `side` wants it; also the `t` of a server mark that is due, for the caller to post. Every `t` is on the game clock.
    pub(in crate::game) fn mark(
        &mut self,
        side: Option<ProfileSide>,
        name: &str,
    ) -> (Option<f64>, Option<f64>) {
        let want = |s| side.is_none_or(|only| only == s);
        let t = self.perf.now();
        let client = (want(ProfileSide::Client) && self.perf.mark(t, name)).then_some(t);
        let server = if !want(ProfileSide::Server) {
            None
        } else if self.server_live() {
            Some(t)
        } else {
            self.server_only
                .as_ref()
                .filter(|s| s.live.active())
                .map(|s| s.began + s.since.elapsed().as_secs_f64())
        };
        (client, server)
    }

    /// The sides a capture is writing, server first.
    pub(in crate::game::client_side) fn sides(&self) -> Vec<ProfileSide> {
        let server = self.server_live() || self.server_only_live() || self.server_link.running();
        [
            server.then_some(ProfileSide::Server),
            self.perf.capturing().then_some(ProfileSide::Client),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

pub(in crate::game) fn gpu_readout(
    wanted: GpuSwitch,
    renderer: Option<&sunrise_client::gfx::Renderer>,
) -> Option<sunrise_client::gfx::pack_timing::GpuReadout> {
    renderer.filter(|_| wanted != GpuSwitch::Off)?.gpu_readout()
}

pub(in crate::game) fn read_bench(path: &str) -> Result<BenchResult> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
    Ok(client_perf::bench_result(text).map_err(|e| format!("{path}: {e}"))?)
}

pub(in crate::game) fn json_of<T: serde::Serialize>(
    reply: &T,
) -> Result<Option<serde_json::Value>> {
    Ok(Some(serde_json::to_value(reply)?))
}

/// The fields of a deferred `profile` reply that chat shows.
#[derive(serde::Deserialize)]
pub(in crate::game) struct ChatAnswer {
    pub(in crate::game) error: Option<String>,
    pub(in crate::game) path: Option<String>,
    pub(in crate::game) removed: Option<Vec<String>>,
}

/// The words after `/profiler` when `command`, without its slash, is that command.
pub(in crate::game::client_side) fn profiler_args(command: &str) -> Option<&str> {
    command
        .strip_prefix("profiler")
        .filter(|rest| rest.is_empty() || rest.starts_with(' '))
}

pub(in crate::game) fn unavailable() -> String {
    sunrise_game_data::lang::get("sunrise.perf.unavailable")
}

/// A path a request names, relative to the game's home.
pub(in crate::game) fn game_path(file: &Path) -> PathBuf {
    sunrise_level::logging::game_home().join(file)
}

/// Why a `/profiler open` or `share` found no capture.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::game) enum Missing {
    Last,
    Named(String),
}

/// The capture `id` names, a capture file in `reports` (this session's `last` one when no id).
pub(in crate::game) fn resolve_capture(
    last: Option<&Path>,
    reports: &Path,
    id: Option<&str>,
) -> std::result::Result<PathBuf, Missing> {
    let Some(id) = id else {
        return last.map(Path::to_path_buf).ok_or(Missing::Last);
    };
    if let Some(last) = last.filter(|p| p.file_stem().is_some_and(|s| s == id)) {
        return Ok(last.to_path_buf());
    }
    let name = format!("{id}.hzperf");
    let path = reports.join(&name);
    if files::is_capture_name(&name) && path.is_file() {
        Ok(path)
    } else {
        Err(Missing::Named(id.to_owned()))
    }
}

/// The captures hook's answer: the share link for [`Job::Share`], nothing for [`Job::Open`].
pub(in crate::game) fn run_job(job: Job, file: &Path) -> std::result::Result<String, String> {
    let captures = sunrise_hooks::engine::captures()
        .ok_or_else(|| sunrise_game_data::lang::get("sunrise.perf.unavailable"))?;
    match job {
        Job::Open => captures.open(file).map(|()| String::new()),
        Job::Share => captures.share(file),
    }
}

/// A `/profiler` line the tree parsed, as the words `request::parse` reads: `[SIDE] ACTION [--gpu=MODE] [NAME]`.
pub(in crate::game) fn profiler_words(literals: &[String], name: Option<&str>) -> Vec<String> {
    let mut words: Vec<String> = literals
        .iter()
        .skip(1)
        .filter(|w| matches!(w.as_str(), "client" | "server"))
        .cloned()
        .collect();
    words.extend(literals.get(1).cloned());
    let [passes, detail] = PROFILER_GPU;
    words.extend(literals.iter().find_map(|w| {
        (w == passes)
            .then(|| "--gpu=passes".to_owned())
            .or_else(|| (w == detail).then(|| "--gpu=detail".to_owned()))
    }));
    words.extend(name.map(str::to_owned));
    words
}

#[cfg(feature = "client")]
impl ClientGame<'_> {
    /// `profile [client|server] start|stop|status|mark`; None while a `stop` waits for its file.
    pub(in crate::game::client_side) fn profile_command(
        &mut self,
        words: &[&str],
        reply: &Sender<String>,
    ) -> Result<Option<serde_json::Value>> {
        self.run_profile(words, reply, Trigger::Ctl)
    }

    pub(in crate::game::client_side) fn run_profile(
        &mut self,
        words: &[&str],
        reply: &Sender<String>,
        trigger: Trigger,
    ) -> Result<Option<serde_json::Value>> {
        let parsed = request::parse(words)?;
        match parsed.request {
            Request::Start(options) => {
                if parsed.side == Some(ProfileSide::Server) && self.client.integrated.is_none() {
                    return self.start_remote_only(&options, reply);
                }
                let started = self.start_profile(parsed.side, &options, trigger)?;
                json_of(&started)
            }
            Request::Stop { bench } => {
                match self.client.profile.stop_route(parsed.side)? {
                    StopRoute::Window => {
                        let bench = bench.map(|path| read_bench(&path)).transpose()?;
                        self.stop_profile_with(StopReason::Normal, bench.into_iter().collect())?;
                        self.client.profile.reply = Some(reply.clone());
                    }
                    StopRoute::Remote => self.stop_remote_only(reply)?,
                    StopRoute::ServerOnly => {
                        let integrated = self.client.integrated.as_ref().ok_or(NO_CAPTURE)?;
                        let reply = reply.clone();
                        if !integrated.tell(OwnerRequest::StopServerCapture(
                            StopReason::Normal,
                            Some(reply),
                        )) {
                            return Err(NO_ANSWER.into());
                        }
                    }
                }
                Ok(None)
            }
            Request::Status => json_of(&self.client.profile.status()),
            Request::Open(file) => {
                let captures = sunrise_hooks::engine::captures().ok_or_else(unavailable)?;
                captures.open(&game_path(&file))?;
                json_of(&Opened {
                    ok: true,
                    viewer: captures.viewer().to_owned(),
                })
            }
            Request::Share(file) => {
                let file = game_path(&file);
                let reply = reply.clone();
                sunrise_hooks::engine::captures().ok_or_else(unavailable)?;
                std::thread::Builder::new()
                    .name("perf-share".into())
                    .spawn(move || {
                        let _ = reply.send(match run_job(Job::Share, &file) {
                            Ok(url) => reply_text(&Shared { ok: true, url }),
                            Err(error) => reply_text(&Failed { error }),
                        });
                    })?;
                Ok(None)
            }
            Request::Mark(name) => {
                let (client, server) = self.client.profile.mark(parsed.side, &name);
                let posted = server.filter(|_| {
                    self.client.integrated.as_ref().is_some_and(|integrated| {
                        let label = name.clone();
                        integrated.tell(OwnerRequest::MarkServerCapture(label))
                    })
                });
                let t = client.or(posted).ok_or(NO_CAPTURE)?;
                json_of(&Marked { ok: true, name, t })
            }
        }
    }

    /// `trace [FILE [SECONDS] | stop]`: a timeline-only capture written to FILE; None while a `stop` waits for its file.
    pub(in crate::game::client_side) fn trace_command(
        &mut self,
        words: &[&str],
        reply: &Sender<String>,
    ) -> Result<Option<serde_json::Value>> {
        match parse_trace(words)? {
            Trace::Status => json_of(&Traced {
                ok: true,
                tracing: self.client.profile.trace_file.is_some(),
                file: self.client.profile.trace_file.clone(),
            }),
            Trace::Stop => {
                if self.client.profile.trace_file.is_none() {
                    return Err("no trace is being written".into());
                }
                self.stop_profile(StopReason::Normal)?;
                self.client.profile.reply = Some(reply.clone());
                Ok(None)
            }
            Trace::Start { file, seconds } => {
                let options = StartOptions {
                    seconds,
                    samples: false,
                    timeline: true,
                    file: Some(PathBuf::from(file)),
                    ..StartOptions::default()
                };
                self.start_profile(None, &options, Trigger::Ctl)
                    .map_err(|e| format!("trace {file}: {e}"))?;
                json_of(&Traced {
                    ok: true,
                    tracing: true,
                    file: Some(file.to_owned()),
                })
            }
        }
    }

    pub(in crate::game::client_side) fn profile_busy(&self) -> bool {
        self.client.profile.perf.capturing()
            || self.client.profile.perf.saving()
            || self.client.profile.server_only_live()
            || !self.client.profile.server_link.is_idle()
    }

    /// The client options a capture records: what sets how much world it keeps and draws.
    pub(in crate::game::client_side) fn perf_client_settings(&self) -> ClientSettings {
        let s = &self.client.settings;
        ClientSettings {
            view_distance: Some(s.render_distance.max(0) as u32),
            loaded_view_distance: Some(self.client.streaming.radius.max(0) as u32),
            simulation_distance: Some(s.simulation_distance.max(0) as u32),
            far_terrain_distance: Some(self.lod_distance()),
            far_terrain_active: Some(self.client.lod.0.is_some()),
            graphics_preset: s.graphics_preset.clone(),
        }
    }

    pub(in crate::game::client_side) fn start_profile(
        &mut self,
        side: Option<ProfileSide>,
        options: &StartOptions,
        trigger: Trigger,
    ) -> Result<Started> {
        if self.profile_busy() {
            return Err("a capture is already running or still being saved".into());
        }
        if side == Some(ProfileSide::Server) {
            return self.start_server_only(options, trigger);
        }
        let trigger = if trigger == Trigger::Ctl && self.common.options.benchmark.is_some() {
            Trigger::Benchmark
        } else {
            trigger
        };
        self.client.profile.server_only = None;
        self.client.profile.stopped_after = None;
        let mut start = client_perf::start_message(
            self.client.integrated.is_some(),
            sunrise_client::telemetry::gpu_name().as_deref(),
        );
        start.mods = self.common.perf_mods();
        start.client_settings = Some(self.perf_client_settings());
        let t = self.client.profile.perf.now();
        let path = self.client.profile.perf.start(
            t,
            &reports_dir(),
            trigger,
            options,
            start,
            Vec::new(),
        )?;
        self.client.profile.started_by = trigger;
        self.client.profile.gpu = options.gpu;
        self.client.profile.trace_file = options.file.as_ref().map(|f| f.display().to_string());
        self.client.profile.spans_were_on = spans::recording_here();
        spans::record_here(true);
        self.client.profile.joined = None;
        self.client.profile.link = if side.is_none() {
            ServerLink::Pending
        } else {
            ServerLink::Off
        };
        self.join_server_capture();
        if side.is_none() {
            self.join_remote_server(options);
        }
        log::info!("Sunrise: profiling the client to {}", path.display());
        Ok(Started {
            ok: true,
            sides: [
                self.client.profile.served().then_some(ProfileSide::Server),
                Some(ProfileSide::Client),
            ]
            .into_iter()
            .flatten()
            .collect(),
            path: shown(&path),
            seconds: options.seconds,
            samples: options.samples,
            gpu: options.gpu,
        })
    }

    /// `profile server start` in single player: the server thread writes a file of its own.
    pub(in crate::game::client_side) fn start_server_only(
        &mut self,
        options: &StartOptions,
        trigger: Trigger,
    ) -> Result<Started> {
        let integrated = self
            .client
            .integrated
            .as_ref()
            .ok_or("profile server: no single-player world is open")?;
        let began = self.client.profile.perf.now();
        let live = Live::new();
        let target = Target::Own {
            options: options.clone(),
            trigger,
            dir: reports_dir(),
            viewer: Some(sunrise_client::client_perf::request::viewer),
        };
        let handoff = live.clone();
        let path = integrated
            .ask(|reply| OwnerRequest::StartServerCapture(target, handoff, Some(reply)))
            .ok_or(NO_ANSWER)??;
        let started = Started {
            ok: true,
            sides: vec![ProfileSide::Server],
            path: shown(&path),
            seconds: options.seconds,
            samples: options.samples,
            gpu: GpuSwitch::Off,
        };
        self.client.profile.server_only = Some(ServerOnly {
            live,
            path,
            since: Instant::now(),
            began,
        });
        Ok(started)
    }

    /// Asks the server thread to write its side into the running capture, once there is a server thread to ask.
    pub(in crate::game::client_side) fn join_server_capture(&mut self) {
        if !matches!(self.client.profile.link, ServerLink::Pending) {
            return;
        }
        let (Some(integrated), Some(join)) =
            (&self.client.integrated, self.client.profile.perf.join())
        else {
            return;
        };
        let live = Live::new();
        let handoff = live.clone();
        let posted = integrated.tell(OwnerRequest::StartServerCapture(
            Target::Join(join),
            handoff,
            None,
        ));
        if posted {
            self.client.profile.joined = Some(live);
            self.client.profile.link = ServerLink::Off;
        }
    }

    /// Ends the window's capture and, when the server thread joined it, the server's side too.
    pub(in crate::game::client_side) fn stop_profile(&mut self, reason: StopReason) -> Result<()> {
        self.stop_profile_with(reason, Vec::new())
    }

    pub(in crate::game::client_side) fn stop_profile_with(
        &mut self,
        reason: StopReason,
        bench: Vec<BenchResult>,
    ) -> Result<()> {
        // Ending a capture never requests or waits for a final memory sample.
        self.client.profile.memory.cancel();
        let t = self.client.profile.perf.now();
        if self.client.profile.stopped_after.is_none() {
            self.client.profile.stopped_after =
                self.client.profile.perf.status(t).map(|s| s.elapsed_s);
        }
        let gpu = self.gpu_readout();
        let server = self.client.profile.server_live();
        if !self.client.profile.perf.stop_with(t, reason, gpu, bench) {
            return Err("no capture is running".into());
        }
        if server && let Some(integrated) = &self.client.integrated {
            integrated.tell(OwnerRequest::StopServerCapture(reason, None));
        }
        self.stop_joined_server();
        self.client.profile.link = ServerLink::Off;
        self.client.profile.gpu = GpuSwitch::Off;
        Ok(())
    }

    /// The pass timings the running capture asked for, as the renderer shows them now.
    pub(in crate::game::client_side) fn gpu_readout(
        &self,
    ) -> Option<sunrise_client::gfx::pack_timing::GpuReadout> {
        gpu_readout(self.client.profile.gpu, self.client.renderer.0.as_deref())
    }

    fn profiler_syntax_failure(&mut self, e: &CommandError, command: &str) {
        self.client_system_message(
            sunrise_gameplay_common::game::commands_common::formatted_command_failure(
                e.message.clone(),
                None,
            ),
        );
        if let Some(context) =
            sunrise_gameplay_common::game::commands_common::format_syntax_failure_context(
                e, command,
            )
        {
            self.client_system_message(
                sunrise_gameplay_common::game::commands_common::formatted_command_failure(
                    context, None,
                ),
            );
        }
    }

    /// `/profiler start|stop|status|mark`: the requests of `sunrise-ctl profile`, answered in chat.
    pub(in crate::game::client_side) fn profiler_chat(&mut self, args: &str) {
        let line = format!("profiler{args}");
        let tree = Dispatcher::profiler();
        let parse = tree.parse(&line, &Registries::default());
        let (literals, name, id) = match parse.executable() {
            Ok((context, _)) => (
                context.literals.clone(),
                match context.get("name") {
                    Some(Arg::Str(name)) => Some(name.clone()),
                    _ => None,
                },
                match context.get("id") {
                    Some(Arg::Str(id)) => Some(id.clone()),
                    _ => None,
                },
            ),
            Err(e) => return self.profiler_syntax_failure(&e, &line),
        };
        if let Some(job) = match literals.get(1).map(String::as_str) {
            Some("open") => Some(Job::Open),
            Some("share") => Some(Job::Share),
            _ => None,
        } {
            self.profiler_job(job, id.as_deref());
            return;
        }
        let owned = profiler_words(&literals, name.as_deref());
        let words: Vec<&str> = owned.iter().map(String::as_str).collect();
        let parsed = match request::parse(&words) {
            Ok(parsed) => parsed,
            Err(e) => return self.notify(lines::error(&e.to_string())),
        };
        let running = self.client.profile.status().capturing;
        let idle_answer = match parsed.request {
            Request::Start(_) => running,
            Request::Stop { .. } | Request::Mark(_) => !running,
            _ => false,
        };
        if idle_answer {
            self.profiler_status();
            return;
        }
        let stopping = matches!(parsed.request, Request::Stop { .. });
        if stopping {
            self.client.profile.stopped_after = self.client.profile.status().elapsed_s;
        }
        let (reply, answer) = std::sync::mpsc::channel();
        match self.run_profile(&words, &reply, Trigger::Command) {
            Ok(None) => {
                if stopping {
                    self.profiler_work(Working::Saving);
                }
                self.client.profile.chat_answer = Some(answer);
            }
            Ok(Some(_)) => match parsed.request {
                Request::Start(_) => self.notify(lines::started(parsed.side)),
                Request::Status => self.profiler_status(),
                Request::Mark(name) => self.notify(lines::marked(&name)),
                Request::Stop { .. } | Request::Open(_) | Request::Share(_) => {}
            },
            Err(e) => self.notify(lines::error(&e.to_string())),
        }
    }

    /// Puts `notice` in the chat history or on the action bar.
    pub(in crate::game::client_side) fn notify(&mut self, notice: Notice) {
        match notice {
            Notice::Chat(line) => self.client_system_message(line),
            Notice::Bar(line) => self.set_hud_overlay(line),
        }
    }

    /// Shows the notice for `working` now and again every [`PROGRESS_EVERY`] until [`Game::profiler_done`].
    pub(in crate::game::client_side) fn profiler_work(&mut self, working: Working) {
        self.client.profile.progress = Some(Progress {
            working,
            due: Instant::now() + PROGRESS_EVERY,
        });
        self.notify(Self::progress_notice(working));
    }

    pub(in crate::game::client_side) fn progress_notice(working: Working) -> Notice {
        match working {
            Working::Saving => lines::saving(),
            Working::Uploading => lines::uploading(),
        }
    }

    pub(in crate::game::client_side) fn profiler_done(&mut self, working: Working) {
        if self
            .client
            .profile
            .progress
            .as_ref()
            .is_some_and(|p| p.working == working)
        {
            self.client.profile.progress = None;
        }
    }

    pub(in crate::game::client_side) fn poll_progress(&mut self) {
        let Some(progress) = &mut self.client.profile.progress else {
            return;
        };
        if Instant::now() < progress.due {
            return;
        }
        progress.due += PROGRESS_EVERY;
        let working = progress.working;
        self.notify(Self::progress_notice(working));
    }

    pub(in crate::game::client_side) fn profiler_status(&mut self) {
        let status = self.client.profile.status();
        self.notify(lines::status(
            &status.sides,
            status.elapsed_s.filter(|_| status.capturing),
        ));
    }

    /// A capture from chat that asked for both sides, told the server refused to join it.
    pub(in crate::game::client_side) fn profiler_client_only(&mut self, why: &str) {
        if self.client.profile.started_by == Trigger::Command {
            self.notify(lines::client_only(why));
        }
    }

    pub(in crate::game::client_side) fn profiler_saved(&mut self, path: &str) {
        let full = sunrise_level::logging::game_home().join(path);
        self.client.profile.last_capture = Some(full.clone());
        let secs = self.client.profile.stopped_after.take().unwrap_or_default();
        let can_open_and_share = sunrise_hooks::engine::captures().is_some();
        self.notify(lines::saved(secs, &full, path, can_open_and_share));
    }

    pub(in crate::game::client_side) fn capture_named(
        &self,
        id: Option<&str>,
    ) -> std::result::Result<PathBuf, Notice> {
        resolve_capture(
            self.client.profile.last_capture.as_deref(),
            &reports_dir(),
            id,
        )
        .map_err(|missing| match missing {
            Missing::Last => lines::share_missing(),
            Missing::Named(id) => lines::not_found(&id),
        })
    }

    /// `/profiler open|share [ID]`: runs the captures hook on a saved capture, off the game thread.
    pub(in crate::game::client_side) fn profiler_job(&mut self, job: Job, id: Option<&str>) {
        if sunrise_hooks::engine::captures().is_none() {
            return self.notify(lines::unavailable());
        }
        match self.capture_named(id) {
            Ok(path) => self.queue_profile_job(path, job),
            Err(refused) => self.notify(refused),
        }
    }

    pub(in crate::game::client_side) fn queue_profile_job(&mut self, path: PathBuf, job: Job) {
        let queued = self
            .client
            .profile
            .jobs
            .iter()
            .any(|(p, j)| *p == path && *j == job);
        let active = self
            .client
            .profile
            .job
            .as_ref()
            .is_some_and(|(p, j, _)| *p == path && *j == job);
        if !queued && !active {
            self.client.profile.jobs.push_back((path, job));
            if job == Job::Share {
                self.profiler_work(Working::Uploading);
            }
        }
    }

    pub(in crate::game::client_side) fn poll_profile_job(&mut self) {
        if let Some((file, job, receiver)) = &self.client.profile.job {
            let (file, job) = (file.clone(), *job);
            let result = match receiver.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(sunrise_game_data::lang::get(
                    "sunrise.perf.upload_interrupted",
                )),
            };
            self.client.profile.job = None;
            let shown = shown(&file);
            match (job, result) {
                (Job::Open, Ok(_)) => self.notify(lines::opened()),
                (Job::Open, Err(error)) => {
                    self.notify(lines::open_failed(&error, &file, &shown));
                }
                (Job::Share, Ok(url)) => {
                    self.profiler_done(Working::Uploading);
                    log::info!("Sunrise: shared performance capture: {url}");
                    self.notify(lines::shared(&url));
                }
                (Job::Share, Err(error)) => {
                    self.profiler_done(Working::Uploading);
                    self.notify(lines::upload_failed(&error, &file));
                }
            }
        }
        if let Some((path, job)) = self.client.profile.jobs.pop_front() {
            let (send, receive) = std::sync::mpsc::channel();
            let file = path.clone();
            match std::thread::Builder::new()
                .name("perf-share".into())
                .spawn(move || {
                    let _ = send.send(run_job(job, &file));
                }) {
                Ok(_) => self.client.profile.job = Some((path, job, receive)),
                Err(error) => {
                    self.profiler_done(Working::Uploading);
                    self.notify(lines::error(&error.to_string()));
                }
            }
        }
    }

    /// Start a memory census on schedule; publish it only if it finishes in this capture.
    pub(in crate::game::client_side) fn poll_memory_snapshot(&mut self) {
        if let Some((label, t, pending)) = self.client.profile.memory.pending.take() {
            match pending.poll() {
                Poll::Pending => {
                    self.client.profile.memory.pending = Some((label, t, pending));
                    return;
                }
                Poll::Ready(Some(rows)) => {
                    self.client.profile.perf.memory_snapshot(t, &label, &rows);
                }
                Poll::Ready(None) => {}
            }
        }
        let Some(cost) = self.client.profile.perf.memory_cost() else {
            return;
        };
        let now = self.client.profile.perf.now();
        let label = match self.client.profile.memory.due {
            None => {
                self.client.profile.memory.due = Some((now + MEMORY_EVERY_S, 1));
                "start".to_owned()
            }
            Some((due, period)) if now >= due => {
                // A slow reply is not a reason to build a queue of stale samples.
                let elapsed = ((now - due) / MEMORY_EVERY_S) as u32;
                let period = period + elapsed;
                self.client.profile.memory.due =
                    Some((due + f64::from(elapsed + 1) * MEMORY_EVERY_S, period + 1));
                format!("{}s", u64::from(period) * MEMORY_EVERY_S as u64)
            }
            Some(_) => return,
        };
        self.client.profile.memory.pending =
            memory_client::start(self, cost).map(|job| (label, now, job));
    }

    pub(in crate::game::client_side) fn poll_chat_answer(&mut self) {
        let Some(answer) = &self.client.profile.chat_answer else {
            return;
        };
        let text = match answer.try_recv() {
            Ok(text) => text,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                self.client.profile.chat_answer = None;
                self.profiler_done(Working::Saving);
                return;
            }
        };
        self.client.profile.chat_answer = None;
        self.profiler_done(Working::Saving);
        match serde_json::from_str::<ChatAnswer>(&text) {
            Ok(ChatAnswer { error: Some(e), .. }) => self.notify(lines::error(&e)),
            Ok(ChatAnswer {
                removed: Some(_),
                path: Some(path),
                ..
            }) => self.profiler_saved(&path),
            _ => self.notify(lines::started(Some(ProfileSide::Server))),
        }
    }

    /// Client-visible entity count across common entity families (excludes server-only `others`).
    fn client_perf_entity_count(&self) -> usize {
        self.common.dropped.len()
            + self.common.orbs.len()
            + self.common.carts.len()
            + self.common.boats.len()
            + self.common.falling.len()
            + self.common.paintings.len()
            + self.common.item_frames.len()
            + self.common.armor_stands.len()
            + self.common.tridents.len()
            + self.common.rockets.len()
            + self.common.tnts.len()
            + self.common.crystals.len()
            + self.common.resident_entities.knots.len()
            + self.common.ominous_spawners.len()
            + self.common.cushions.len()
            + self.common.breath_clouds.len()
            + self.common.bolts.len()
            + self.common.fangs.list.len()
            + usize::from(self.common.dragon.as_ref().is_some())
            + self.common.mobs.len()
            + self.common.creatures.projectiles.len()
            + self.common.resident_entities.general.rows.len()
    }

    /// One iteration of the game loop, rendered or not: `wall_ms` long, `cpu_ms` of it the
    /// CPU's own work, ending at `t`.
    pub(in crate::game::client_side) fn record_perf_frame(
        &mut self,
        wall_ms: f32,
        cpu_ms: f32,
        gpu_ms: Option<f32>,
        frame_spans: &[Span],
    ) {
        // Spans exist only during a capture, which keeps the worst frames whatever the slow threshold is.
        let detail = (!frame_spans.is_empty()).then(|| TickDetail {
            cpu_ms: Some(cpu_ms),
            spans: frame_spans.to_vec(),
        });
        let t = self.client.profile.perf.now();
        if self.client.profile.gpu != GpuSwitch::Off
            && let Some(renderer) = self.client.renderer.0.as_mut()
        {
            let frames = renderer.take_timed_frames();
            self.client.profile.perf.gpu_frames(t, cpu_ms, &frames);
        }
        let connection = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.session.counters.lock().unwrap().clone());
        self.client.profile.perf.set_connection(connection);
        let (wanted, renderer) = (self.client.profile.gpu, self.client.renderer.0.as_deref());
        let entities = self.client_perf_entity_count() as u32;
        let memory = &mut self.client.profile.memory;
        self.client.profile.perf.frame_with_gpu(
            t,
            wall_ms,
            cpu_ms,
            gpu_ms,
            detail,
            || WorldCounts {
                players: self
                    .client
                    .remote
                    .as_ref()
                    .map_or(0, |r| r.listed_player_count() as u32),
                chunks: self.common.world.chunks.len() as u32,
                entities,
            },
            || {
                // This callback runs just before the timed capture stops.
                memory.cancel();
                gpu_readout(wanted, renderer)
            },
        );
        if !self.client.profile.perf.capturing() {
            self.client.profile.gpu = GpuSwitch::Off;
        }
        if matches!(self.client.profile.link, ServerLink::Pending) {
            self.join_server_capture();
        }
        if !self.client.profile.hardware_warmed {
            self.client.profile.hardware_warmed = true;
            server_perf::warm_hardware();
        }
        if !self.client.profile.env_tried && server_perf::hardware().is_some() {
            self.client.profile.env_tried = true;
            let named = |var: &str| std::env::var_os(var).filter(|v| !v.is_empty());
            let gpu = client_perf::env_gpu(
                std::env::var_os(client_perf::ENV_PASS_TIMING).as_deref(),
                std::env::var_os(client_perf::ENV_PASS_DETAIL).as_deref(),
            );
            let (var, options) = if named(ENV_DIR).is_some() || gpu != GpuSwitch::Off {
                (
                    ENV_DIR,
                    StartOptions {
                        gpu,
                        ..StartOptions::default()
                    },
                )
            } else if let Some(file) = named(ENV_TRACE) {
                let options = StartOptions {
                    samples: false,
                    timeline: true,
                    file: Some(PathBuf::from(file)),
                    ..StartOptions::default()
                };
                (ENV_TRACE, options)
            } else {
                return;
            };
            if let Err(e) = self.start_profile(None, &options, Trigger::Env) {
                log::warn!("Sunrise: {var}: {e}");
            }
        }
    }

    /// Once per loop turn: the captures whose files are whole, told to whoever waits for them.
    pub(in crate::game::client_side) fn poll_profile(&mut self) {
        self.poll_server_link();
        self.poll_chat_answer();
        self.poll_progress();
        self.poll_memory_snapshot();
        for saved in self.client.profile.perf.take_saved(false) {
            self.profile_saved(saved);
        }
        self.poll_profile_job();
    }

    /// Ends the capture a closing game still has, and waits for its file.
    pub(in crate::game::client_side) fn finish_profile(&mut self) {
        if self.client.profile.perf.capturing() {
            let _ = self.stop_profile(StopReason::Shutdown);
        }
        self.abandon_part();
        // The file is whole only once the server thread's side is, which a closing game's server has no more ticks for.
        if let Some(integrated) = &self.client.integrated {
            integrated.request_stop();
        }
        for saved in self.client.profile.perf.take_saved(true) {
            self.profile_saved(saved);
        }
    }

    pub(in crate::game::client_side) fn profile_saved(
        &mut self,
        saved: std::io::Result<sunrise_sim::server_perf::Saved>,
    ) {
        self.stop_joined_server();
        spans::record_here(self.client.profile.spans_were_on);
        self.client.profile.trace_file = None;
        let reply = self.client.profile.reply.take();
        let in_chat = self.client.profile.started_by == Trigger::Command && reply.is_none();
        match saved {
            Ok(saved) => {
                for path in &saved.deleted {
                    log::info!("Sunrise: removed the oldest capture {}", path.display());
                }
                log::info!("Sunrise: {}", files::saved_line(&saved.path));
                self.client.profile.last_capture =
                    Some(sunrise_level::logging::game_home().join(shown(&saved.path)));
                if let Some(reply) = reply {
                    let _ = reply.send(reply_text(&Stopped {
                        ok: true,
                        sides: [
                            self.client.profile.served().then_some(ProfileSide::Server),
                            Some(ProfileSide::Client),
                        ]
                        .into_iter()
                        .flatten()
                        .collect(),
                        path: shown(&saved.path),
                        open_at: sunrise_client::client_perf::request::viewer(),
                        removed: saved.deleted.iter().map(|p| shown(p)).collect(),
                    }));
                }
                if in_chat {
                    self.profiler_done(Working::Saving);
                    self.profiler_saved(&shown(&saved.path));
                }
            }
            Err(e) => {
                log::error!("Sunrise: could not write the performance capture: {e}");
                if let Some(reply) = reply {
                    let _ = reply.send(reply_text(&Failed {
                        error: e.to_string(),
                    }));
                }
                if in_chat {
                    self.profiler_done(Working::Saving);
                    self.notify(lines::error(&e.to_string()));
                }
            }
        }
    }
}

#[cfg(feature = "client")]
#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;

    fn chat_request(line: &str) -> std::result::Result<request::Parsed, String> {
        let tree = Dispatcher::profiler();
        let parse = tree.parse(line, &Registries::default());
        let (context, _) = parse.executable().map_err(|e| e.message.plain())?;
        let name = match context.get("name") {
            Some(Arg::Str(name)) => Some(name.as_str()),
            _ => None,
        };
        let owned = profiler_words(&context.literals, name);
        let words: Vec<&str> = owned.iter().map(String::as_str).collect();
        request::parse(&words).map_err(|e| e.to_string())
    }

    #[test]
    fn a_chat_profiler_line_becomes_the_request_it_names() {
        let started = |line| match chat_request(line).unwrap() {
            request::Parsed {
                side,
                request: Request::Start(options),
            } => (side, options.gpu),
            other => panic!("{line}: {other:?}"),
        };
        assert_eq!(started("profiler start"), (None, GpuSwitch::Off));
        assert_eq!(started("profiler start gpu"), (None, GpuSwitch::Passes));
        assert_eq!(
            started("profiler start client gpu_detail"),
            (Some(ProfileSide::Client), GpuSwitch::Detail)
        );
        assert_eq!(
            chat_request("profiler stop server").unwrap().side,
            Some(ProfileSide::Server)
        );
        assert_eq!(
            chat_request("profiler mark slow chunk").unwrap().request,
            Request::Mark("slow chunk".into())
        );
        for line in [
            "profiler start --gpu",
            "profiler start --seconds 30",
            "profiler stop gpu",
        ] {
            assert!(chat_request(line).is_err(), "{line}");
        }
    }

    #[test]
    fn open_and_share_find_a_named_capture_in_the_reports_folder_or_the_last_one() {
        let dir = std::env::temp_dir().join(format!("sunrise-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "profile-20251107-143844Z";
        std::fs::write(dir.join(format!("{name}.hzperf")), b"x").unwrap();
        std::fs::write(dir.join("notes.hzperf"), b"x").unwrap();
        let found = |id| resolve_capture(None, &dir, id);
        assert_eq!(found(Some(name)), Ok(dir.join(format!("{name}.hzperf"))));
        assert_eq!(found(None), Err(Missing::Last));
        for id in ["notes", "profile-20251107-000000Z", "..", "../notes"] {
            assert_eq!(found(Some(id)), Err(Missing::Named(id.into())), "{id}");
        }
        let last = dir.join("trace.hzperf");
        assert_eq!(resolve_capture(Some(&last), &dir, None), Ok(last.clone()));
        assert_eq!(resolve_capture(Some(&last), &dir, Some("trace")), Ok(last));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn start<'a>(
        file: &'a str,
        seconds: Option<u32>,
    ) -> std::result::Result<Trace<'a>, &'static str> {
        Ok(Trace::Start { file, seconds })
    }

    #[test]
    fn trace_words() {
        assert_eq!(parse_trace(&[]), Ok(Trace::Status));
        assert_eq!(parse_trace(&["stop"]), Ok(Trace::Stop));
        assert_eq!(
            parse_trace(&["/tmp/a.hzperf"]),
            start("/tmp/a.hzperf", None)
        );
        assert_eq!(parse_trace(&["a", "2"]), start("a", Some(2)));
        assert_eq!(parse_trace(&["a", "1"]), start("a", Some(1)));
        assert_eq!(parse_trace(&["a", "600"]), start("a", Some(600)));
    }

    #[test]
    fn trace_seconds_out_of_range_are_usage_errors() {
        for bad in ["0", "601", "99999", "-1", "1.5", "x", ""] {
            assert_eq!(
                parse_trace(&["a", bad]),
                Err(TRACE_USAGE),
                "seconds {bad:?}"
            );
        }
        assert_eq!(parse_trace(&["a", "2", "3"]), Err(TRACE_USAGE));
    }
    use std::time::Duration;
    use sunrise_gameplay_common::game::profile_capture_common::Phase;

    fn live(phase: Phase) -> Arc<Live> {
        let live = Live::new();
        live.set(phase);
        live
    }

    fn server_only(phase: Phase, running_for: Duration, path: PathBuf) -> ServerOnly {
        ServerOnly {
            live: live(phase),
            path,
            since: Instant::now() - running_for,
            began: 0.0,
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("hz-profile-route-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A window capture that the server thread joined, in `phase`.
    fn joined(dir: &Path, phase: Phase) -> Profile {
        let mut profile = Profile::new(50.0);
        let options = StartOptions {
            samples: false,
            ..StartOptions::default()
        };
        std::thread::sleep(Duration::from_millis(30));
        let t = profile.perf.now();
        profile
            .perf
            .start(
                t,
                dir,
                Trigger::Ctl,
                &options,
                Default::default(),
                Vec::new(),
            )
            .unwrap();
        profile.joined = Some(live(phase));
        profile
    }

    #[test]
    fn a_server_mark_in_a_joined_capture_is_stamped_on_the_window_clock() {
        let dir = scratch("joined-mark");
        let mut profile = joined(&dir, Phase::Running);
        // A finished server-only capture, 1000 s old, must not lend its clock.
        profile.server_only = Some(server_only(
            Phase::Ended,
            Duration::from_secs(1000),
            dir.join("old.hzperf"),
        ));
        let before = profile.perf.now();
        let (client, server) = profile.mark(Some(ProfileSide::Server), "boss");
        assert_eq!(client, None);
        let t = server.expect("a joined capture takes a server mark");
        // The capture began 30 ms into the game, so a capture-relative `t` would fall short of `before`.
        assert!(
            (before..=profile.perf.now()).contains(&t),
            "{t} vs {before}"
        );

        let (client, server) = profile.mark(None, "both");
        assert_eq!(client, server);
        assert!(client.is_some());

        let (client, server) = profile.mark(Some(ProfileSide::Client), "mine");
        assert!(client.is_some());
        assert_eq!(server, None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mark_is_refused_when_no_running_capture_has_the_side() {
        let dir = scratch("no-mark");
        let mut profile = joined(&dir, Phase::Failed);
        assert_eq!(
            profile.mark(Some(ProfileSide::Server), "x"),
            (None, None),
            "the server never joined"
        );
        profile.server_only = Some(server_only(
            Phase::Ended,
            Duration::from_secs(5),
            dir.join("a"),
        ));
        assert_eq!(profile.mark(Some(ProfileSide::Server), "x"), (None, None));
        assert_eq!(Profile::new(50.0).mark(None, "x"), (None, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_server_only_mark_is_stamped_on_the_window_clock() {
        let mut profile = Profile::new(50.0);
        let mut alone = server_only(Phase::Running, Duration::from_secs(30), PathBuf::from("a"));
        alone.began = 100.0;
        profile.server_only = Some(alone);
        let (client, server) = profile.mark(None, "x");
        assert_eq!(client, None);
        assert!((130.0..140.0).contains(&server.unwrap()), "{server:?}");
    }

    #[test]
    fn status_reports_only_captures_that_are_running() {
        let dir = scratch("status");
        let file = dir.join("capture.hzperf");
        std::fs::write(&file, [0u8; 7]).unwrap();
        let mut profile = Profile::new(50.0);
        profile.server_only = Some(server_only(
            Phase::Running,
            Duration::from_secs(4),
            file.clone(),
        ));
        let running = profile.status();
        assert!(running.capturing);
        assert_eq!(running.sides, [ProfileSide::Server]);
        assert_eq!(running.path, Some(shown(&file)));
        assert_eq!(running.bytes, Some(7));
        assert!(running.elapsed_s.unwrap() >= 4.0);

        // After `profile server stop`, or when a timed capture ends, the old file is not reported.
        profile.server_only.as_ref().unwrap().live.set(Phase::Ended);
        let done = profile.status();
        assert!(!done.capturing);
        assert!(done.sides.is_empty());
        assert_eq!((done.path, done.elapsed_s, done.bytes), (None, None, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_joined_capture_lists_the_server_first_until_the_server_ends() {
        let dir = scratch("sides");
        let profile = joined(&dir, Phase::Running);
        let status = profile.status();
        assert!(status.capturing);
        assert_eq!(status.sides, [ProfileSide::Server, ProfileSide::Client]);
        assert!(status.path.is_some());
        profile.joined.as_ref().unwrap().set(Phase::Ended);
        assert_eq!(profile.status().sides, [ProfileSide::Client]);
        assert!(
            profile.served(),
            "an ended server side still holds its part of the file"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stop_goes_to_the_capture_that_is_running() {
        let dir = scratch("stop");
        let none = Profile::new(50.0);
        for side in [None, Some(ProfileSide::Client), Some(ProfileSide::Server)] {
            assert!(none.stop_route(side).is_err(), "{side:?}");
        }

        let both = joined(&dir, Phase::Running);
        for side in [None, Some(ProfileSide::Client), Some(ProfileSide::Server)] {
            assert_eq!(both.stop_route(side), Ok(StopRoute::Window), "{side:?}");
        }

        let alone = joined(&dir, Phase::Failed);
        assert_eq!(alone.stop_route(None), Ok(StopRoute::Window));
        assert_eq!(
            alone.stop_route(Some(ProfileSide::Client)),
            Ok(StopRoute::Window)
        );
        assert!(alone.stop_route(Some(ProfileSide::Server)).is_err());

        let mut server = Profile::new(50.0);
        server.server_only = Some(server_only(
            Phase::Running,
            Duration::ZERO,
            PathBuf::from("a"),
        ));
        assert_eq!(server.stop_route(None), Ok(StopRoute::ServerOnly));
        assert_eq!(
            server.stop_route(Some(ProfileSide::Server)),
            Ok(StopRoute::ServerOnly)
        );
        assert!(server.stop_route(Some(ProfileSide::Client)).is_err());

        server.server_only.as_ref().unwrap().live.set(Phase::Ended);
        for side in [None, Some(ProfileSide::Server)] {
            assert!(server.stop_route(side).is_err(), "{side:?}");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
