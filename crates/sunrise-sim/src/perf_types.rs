//! Portable performance DTOs, GPU timing serialization, profile request/reply
//! parsing, and cross-crate perf test locks shared by `sunrise-sim` and `sunrise-client-lib`.
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;
use sunrise_perf_format::v1::{
    GpuBackend, GpuCounter, GpuCounts, GpuDevice, GpuFragmentUnit, GpuFrame, GpuFrames, GpuMode,
    GpuPart, GpuPartCounts, GpuPartsMode, GpuPass, GpuPassSplit, GpuPasses, GpuSource, Side,
    StopReason, Trigger,
};
use sunrise_perf_format::{FieldSender, Interner};

/// Block-entity static-cache counters reported per frame into `control::FrameStats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RenderCacheStats {
    pub snapshot_rebuilds: u64,
    pub snapshot_bytes_built: u64,
    pub static_bytes_uploaded: u64,
    pub static_entities_drawn: u64,
    pub dynamic_entities_drawn: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum GpuBackendKind {
    Vulkan,
    Metal,
    Dx12,
    Gl,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum GpuPartsModeKind {
    #[default]
    InsidePasses,
    SeparatePasses,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum GpuFragmentUnitKind {
    Pixel,
    Quad,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GpuDetailSupport {
    pub parts: GpuPartsModeKind,
    pub encoder_stamps: bool,
    pub counters: bool,
    pub fragment_unit: Option<GpuFragmentUnitKind>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GpuCounters {
    pub vertex_shader_invocations: u64,
    pub clipper_invocations: u64,
    pub clipper_primitives_out: u64,
    pub fragment_shader_invocations: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct GpuPassCounters {
    pub total: GpuCounters,
    pub pixels: u64,
    pub parts: Vec<(String, GpuCounters)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GpuPassReadings {
    pub passes: Vec<(String, f32)>,
    pub parts: Vec<(String, Vec<(String, f32)>)>,
    pub counters: Vec<(String, GpuPassCounters)>,
    pub support: GpuDetailSupport,
}

impl GpuPassReadings {
    /// Whether the timer has shown nothing yet.
    pub fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.parts.is_empty() && self.counters.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GpuPerfSnapshot {
    pub adapter: String,
    pub backend: GpuBackendKind,
    pub driver: String,
    pub own: Option<GpuPassReadings>,
    pub pack: Option<GpuPassReadings>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimedFrameDto {
    pub at: Instant,
    pub total_ms: Option<f32>,
    pub passes: Vec<(String, f32)>,
    pub parts: Vec<(String, Vec<(String, f32)>)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimedFramesDto {
    pub own: Vec<TimedFrameDto>,
    pub pack: Vec<TimedFrameDto>,
}

/// Every time one timer's frames gave a pass or a part during a capture.
#[derive(Default)]
struct Times {
    passes: Vec<(String, Vec<f32>)>,
    parts: Vec<(String, Vec<(String, Vec<f32>)>)>,
}

fn times_of<'a>(list: &'a mut Vec<(String, Vec<f32>)>, name: &str) -> &'a mut Vec<f32> {
    let i = match list.iter().position(|e| e.0 == name) {
        Some(i) => i,
        None => {
            list.push((name.to_owned(), Vec::new()));
            list.len() - 1
        }
    };
    &mut list[i].1
}

impl Times {
    fn add(&mut self, frame: &TimedFrameDto) {
        for (pass, ms) in &frame.passes {
            times_of(&mut self.passes, pass).push(*ms);
        }
        for (pass, parts) in &frame.parts {
            let i = match self.parts.iter().position(|e| e.0 == *pass) {
                Some(i) => i,
                None => {
                    self.parts.push((pass.clone(), Vec::new()));
                    self.parts.len() - 1
                }
            };
            for (part, ms) in parts {
                times_of(&mut self.parts[i].1, part).push(*ms);
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.passes.is_empty() && self.parts.is_empty()
    }

    /// `readings` with its times the medians over the whole capture instead of its last frames.
    fn medians_into(&self, readings: &GpuPassReadings) -> GpuPassReadings {
        let median = |v: &[f32]| {
            let mut v = v.to_vec();
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        GpuPassReadings {
            passes: self
                .passes
                .iter()
                .map(|(name, v)| (name.clone(), median(v)))
                .collect(),
            parts: self
                .parts
                .iter()
                .map(|(pass, parts)| {
                    let parts = parts.iter().map(|(n, v)| (n.clone(), median(v))).collect();
                    (pass.clone(), parts)
                })
                .collect(),
            ..readings.clone()
        }
    }
}

/// A capture's timed frames on their way into `GpuFrames` blocks, every time kept for the whole capture's medians.
#[derive(Default)]
pub struct GpuFrameLog {
    pending: Vec<GpuFrame>,
    cpu_frame_ms: Vec<f32>,
    own: Times,
    pack: Times,
}

impl GpuFrameLog {
    /// The CPU's own ms of one frame the window drew.
    pub fn cpu_frame(&mut self, ms: f32) {
        self.cpu_frame_ms.push(ms);
    }

    /// The frames timed since the last call; `t_of` puts a frame's start on the capture clock.
    pub fn add(
        &mut self,
        frames: &TimedFramesDto,
        t_of: impl Fn(Instant) -> f64,
        strings: &mut Interner,
    ) {
        for (source, list, times) in [
            (GpuSource::Sunrise, &frames.own, &mut self.own),
            (GpuSource::ShaderPack, &frames.pack, &mut self.pack),
        ] {
            for frame in list {
                times.add(frame);
                self.pending
                    .push(frame_message(frame, source, t_of(frame.at), strings));
            }
        }
    }

    /// The frames and CPU frame times since the last block; none when there are neither.
    pub fn take_block(&mut self, side: Side) -> Option<GpuFrames> {
        if self.pending.is_empty() && self.cpu_frame_ms.is_empty() {
            return None;
        }
        Some(GpuFrames {
            side: side as i32,
            frames: std::mem::take(&mut self.pending),
            cpu_frame_ms: std::mem::take(&mut self.cpu_frame_ms),
        })
    }
}

fn frame_message(
    frame: &TimedFrameDto,
    source: GpuSource,
    t: f64,
    strings: &mut Interner,
) -> GpuFrame {
    let mut passes: Vec<GpuPass> = frame
        .passes
        .iter()
        .map(|(name, ms)| GpuPass {
            name: strings.intern(name),
            ms: Some(*ms),
            parts: Vec::new(),
        })
        .collect();
    for (pass, parts) in &frame.parts {
        let parts = parts
            .iter()
            .map(|(part, ms)| GpuPart {
                name: strings.intern(part),
                ms: *ms,
            })
            .collect();
        let name = strings.intern(pass);
        match passes.iter_mut().find(|p| p.name == name) {
            Some(p) => p.parts = parts,
            None => passes.push(GpuPass {
                name,
                ms: None,
                parts,
            }),
        }
    }
    GpuFrame {
        source: source as i32,
        t,
        total_ms: frame.total_ms,
        passes,
    }
}

fn backend(backend: GpuBackendKind) -> GpuBackend {
    match backend {
        GpuBackendKind::Vulkan => GpuBackend::Vulkan,
        GpuBackendKind::Metal => GpuBackend::Metal,
        GpuBackendKind::Dx12 => GpuBackend::Dx12,
        GpuBackendKind::Gl => GpuBackend::Gl,
        GpuBackendKind::Other => GpuBackend::Other,
    }
}

fn counts(c: GpuCounters) -> GpuCounts {
    GpuCounts {
        vertex_shader_invocations: c.vertex_shader_invocations,
        clipper_invocations: c.clipper_invocations,
        clipper_primitives_out: c.clipper_primitives_out,
        fragment_shader_invocations: c.fragment_shader_invocations,
    }
}

fn counter(strings: &mut Interner, pass: &str, c: &GpuPassCounters) -> GpuCounter {
    GpuCounter {
        pass: strings.intern(pass),
        total: Some(counts(c.total)),
        pixels: c.pixels,
        parts: c
            .parts
            .iter()
            .map(|(name, part)| GpuPartCounts {
                name: strings.intern(name),
                counts: Some(counts(*part)),
            })
            .collect(),
    }
}

fn device(readout: &GpuPerfSnapshot, readings: &GpuPassReadings) -> GpuDevice {
    GpuDevice {
        adapter: readout.adapter.clone(),
        backend: backend(readout.backend) as i32,
        driver: readout.driver.clone(),
        parts_mode: match readings.support.parts {
            GpuPartsModeKind::InsidePasses => GpuPartsMode::InsidePasses,
            GpuPartsModeKind::SeparatePasses => GpuPartsMode::SeparatePasses,
        } as i32,
        pass_split: match readings.support.parts {
            GpuPartsModeKind::InsidePasses => GpuPassSplit::DrawGroups,
            GpuPartsModeKind::SeparatePasses => GpuPassSplit::PassesOnly,
        } as i32,
        counters: readings.support.counters,
        fragment_unit: match readings.support.fragment_unit {
            None => GpuFragmentUnit::Unspecified,
            Some(GpuFragmentUnitKind::Pixel) => GpuFragmentUnit::Pixel,
            Some(GpuFragmentUnitKind::Quad) => GpuFragmentUnit::Quad,
        } as i32,
    }
}

fn section(
    readout: &GpuPerfSnapshot,
    side: Side,
    source: GpuSource,
    readings: &GpuPassReadings,
    strings: &mut Interner,
) -> GpuPasses {
    let mut passes: Vec<GpuPass> = readings
        .passes
        .iter()
        .map(|(name, ms)| GpuPass {
            name: strings.intern(name),
            ms: Some(*ms),
            parts: Vec::new(),
        })
        .collect();
    for (pass, parts) in &readings.parts {
        let name = strings.intern(pass);
        let at = passes
            .iter()
            .position(|p| p.name == name)
            .unwrap_or_else(|| {
                passes.push(GpuPass {
                    name,
                    ms: None,
                    parts: Vec::new(),
                });
                passes.len() - 1
            });
        passes[at].parts = parts
            .iter()
            .map(|(part, ms)| GpuPart {
                name: strings.intern(part),
                ms: *ms,
            })
            .collect();
    }
    GpuPasses {
        side: side as i32,
        device: Some(device(readout, readings)),
        passes,
        counters: readings
            .counters
            .iter()
            .map(|(pass, c)| counter(strings, pass, c))
            .collect(),
        source: source as i32,
    }
}

/// One `GpuPasses` per timer that has timed something, its times the medians of `log` where it has them; the caller appends `strings`' new entries first.
pub fn gpu_passes(
    readout: &GpuPerfSnapshot,
    log: &GpuFrameLog,
    side: Side,
    strings: &mut Interner,
) -> Vec<GpuPasses> {
    [
        (GpuSource::Sunrise, &readout.own, &log.own),
        (GpuSource::ShaderPack, &readout.pack, &log.pack),
    ]
    .into_iter()
    .filter_map(|(source, readings, times)| {
        let readings = readings.as_ref().filter(|r| !r.is_empty())?;
        let whole = (!times.is_empty()).then(|| times.medians_into(readings));
        let readings = whole.as_ref().unwrap_or(readings);
        Some(section(readout, side, source, readings, strings))
    })
    .collect()
}

/// The longest name a mark takes; it is the one free text in a capture.
pub const MARK_NAME_MAX: usize = 100;
/// The sampler's rate bounds, per second.
pub const HZ_RANGE: RangeInclusive<u32> = 10..=1000;

/// Which side `profile` addresses; no side means every side the game can reach.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProfileSide {
    Client,
    Server,
}

impl FromStr for ProfileSide {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "client" => Ok(Self::Client),
            "server" => Ok(Self::Server),
            other => Err(format!("unknown side {other}; use client or server")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Threads {
    Main,
    All,
}

impl FromStr for Threads {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "main" => Ok(Self::Main),
            "all" => Ok(Self::All),
            other => Err(format!("unknown threads {other}; use main or all")),
        }
    }
}

/// What `--gpu` records: nothing, the passes' times, or also their parts and counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GpuSwitch {
    #[default]
    Off,
    Passes,
    Detail,
}

impl FromStr for GpuSwitch {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "passes" => Ok(Self::Passes),
            "detail" => Ok(Self::Detail),
            other => Err(format!(
                "unknown --gpu value {other}; use --gpu, --gpu=passes or --gpu=detail"
            )),
        }
    }
}

impl GpuSwitch {
    pub fn mode(self) -> GpuMode {
        match self {
            Self::Off => GpuMode::Off,
            Self::Passes => GpuMode::Passes,
            Self::Detail => GpuMode::Detail,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Start,
    Stop,
    Status,
    Mark,
    Open,
    Share,
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "start" => Ok(Self::Start),
            "stop" => Ok(Self::Stop),
            "status" => Ok(Self::Status),
            "mark" => Ok(Self::Mark),
            "open" => Ok(Self::Open),
            "share" => Ok(Self::Share),
            other => Err(format!("unknown profile action {other}")),
        }
    }
}

/// The switches of `profile start`.
#[derive(Clone, Debug, PartialEq)]
pub struct StartOptions {
    /// Unset runs until `stop`, or the longest window.
    pub seconds: Option<u32>,
    pub hz: Option<u32>,
    pub over_ms: Option<f32>,
    pub threads: Threads,
    pub wall: bool,
    pub samples: bool,
    pub gpu: GpuSwitch,
    /// Also record every thread's spans (`--timeline`), for 60 s at most.
    pub timeline: bool,
    /// Write the capture here instead of into the reports folder: `trace` and `SUNRISE_TRACE` only, never a profile switch.
    pub file: Option<PathBuf>,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            seconds: None,
            hz: None,
            over_ms: None,
            threads: Threads::Main,
            wall: false,
            samples: true,
            gpu: GpuSwitch::Off,
            timeline: false,
            file: None,
        }
    }
}

/// Where a client's captures go: `SUNRISE_PROFILE` when set, else `reports/` in the game home.
pub fn reports_dir(profile: Option<&OsStr>, home: &Path) -> PathBuf {
    match profile.filter(|p| !p.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home.join("reports"),
    }
}

/// What a side on another thread needs to write into the running capture's file.
#[derive(Clone)]
pub struct Join {
    pub sender: FieldSender,
    pub path: PathBuf,
    /// The instant every `t` of the capture counts from.
    pub origin: Instant,
    /// When the capture's window ends, and why it stops then.
    pub ends: Option<(Instant, StopReason)>,
    pub options: StartOptions,
    pub trigger: Trigger,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    Start(StartOptions),
    /// With the path of a benchmark report to embed in the capture.
    Stop {
        bench: Option<String>,
    },
    Status,
    Mark(String),
    /// Open the saved capture in the viewer, through the captures hook.
    Open(PathBuf),
    /// Upload the saved capture, through the captures hook; the reply is the link.
    Share(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub side: Option<ProfileSide>,
    pub request: Request,
}

const USAGE: &str = "profile [client|server] start [--seconds S] [--hz N] [--over MS] [--threads main|all] [--wall] [--gpu[=detail]] [--no-samples] [--timeline] | stop [--bench REPORT] | status | mark NAME | open FILE | share FILE";

/// The words after `profile`; the side comes first or as `--side`, and the switches follow `start`.
pub fn parse(words: &[&str]) -> Result<Parsed, String> {
    let (mut side, words) = match words.first().map(|w| w.parse::<ProfileSide>()) {
        Some(Ok(side)) => (Some(side), &words[1..]),
        _ => (None, words),
    };
    let Some((action, rest)) = words.split_first() else {
        return Err(format!("usage: {USAGE}"));
    };
    let action: Action = action.parse().map_err(|e| format!("{e}; usage: {USAGE}"))?;
    if action == Action::Mark {
        return mark(side, rest);
    }
    if matches!(action, Action::Open | Action::Share) {
        let file = rest.join(" ");
        if file.is_empty() {
            return Err(format!("{} needs a FILE; usage: {USAGE}", words[0]));
        }
        let file = PathBuf::from(file);
        let request = match action {
            Action::Open => Request::Open(file),
            _ => Request::Share(file),
        };
        return Ok(Parsed {
            side: None,
            request,
        });
    }
    let mut options = StartOptions::default();
    let mut switched = false;
    let mut bench = None;
    let mut words = rest.iter().copied().peekable();
    while let Some(word) = words.next() {
        let mut value = || {
            words
                .next()
                .ok_or_else(|| format!("{word} needs a value; usage: {USAGE}"))
        };
        match word {
            "--side" => side = Some(value()?.parse()?),
            "--seconds" => options.seconds = Some(number(word, value()?, 1..=600)?),
            "--hz" => options.hz = Some(number(word, value()?, HZ_RANGE)?),
            "--over" => {
                let ms: f32 = value()?
                    .parse()
                    .map_err(|_| "--over must be a number of milliseconds".to_owned())?;
                if !ms.is_finite() || ms <= 0.0 {
                    return Err("--over must be more than 0".into());
                }
                options.over_ms = Some(ms);
            }
            "--threads" => options.threads = value()?.parse()?,
            "--wall" => options.wall = true,
            "--gpu" => {
                options.gpu = match words.next_if(|w| w.parse::<GpuSwitch>().is_ok()) {
                    Some(kind) => kind.parse()?,
                    None => GpuSwitch::Passes,
                }
            }
            _ if word.starts_with("--gpu=") => options.gpu = word["--gpu=".len()..].parse()?,
            "--no-samples" => options.samples = false,
            "--timeline" => options.timeline = true,
            "--bench" => {
                // The line arrives split on whitespace, so a run of spaces or a tab in the path becomes one space.
                let path = words.by_ref().collect::<Vec<_>>().join(" ");
                if path.is_empty() {
                    return Err(format!("--bench needs a report path; usage: {USAGE}"));
                }
                bench = Some(path);
                continue;
            }
            other => return Err(format!("unexpected {other}; usage: {USAGE}")),
        }
        switched |= word != "--side";
    }
    let request = match action {
        Action::Start if bench.is_some() => {
            return Err(format!("only stop takes --bench; usage: {USAGE}"));
        }
        Action::Start => Request::Start(options),
        _ if switched => return Err(format!("only start takes options; usage: {USAGE}")),
        Action::Stop => Request::Stop { bench },
        _ if bench.is_some() => return Err(format!("only stop takes --bench; usage: {USAGE}")),
        _ => Request::Status,
    };
    Ok(Parsed { side, request })
}

fn mark(mut side: Option<ProfileSide>, mut words: &[&str]) -> Result<Parsed, String> {
    if let [flag @ "--side", value, rest @ ..] = words {
        let _ = flag;
        side = Some(value.parse()?);
        words = rest;
    }
    let name = words.join(" ");
    if name.is_empty() {
        return Err("mark needs a NAME".into());
    }
    if name.chars().count() > MARK_NAME_MAX || name.chars().any(char::is_control) {
        return Err(format!(
            "a mark name is at most {MARK_NAME_MAX} characters without control characters"
        ));
    }
    Ok(Parsed {
        side,
        request: Request::Mark(name),
    })
}

fn number(flag: &str, text: &str, range: RangeInclusive<u32>) -> Result<u32, String> {
    match text.parse::<u32>() {
        Ok(n) if range.contains(&n) => Ok(n),
        _ => Err(format!(
            "{flag} must be a whole number from {} to {}",
            range.start(),
            range.end()
        )),
    }
}

/// The reply to `profile start`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Started {
    pub ok: bool,
    /// The sides asked for, server first; the server thread's join is confirmed by `status` and `stop`.
    pub sides: Vec<ProfileSide>,
    pub path: String,
    pub seconds: Option<u32>,
    pub samples: bool,
    pub gpu: GpuSwitch,
}

/// The reply to `profile stop`, once the file is whole.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stopped {
    pub ok: bool,
    pub sides: Vec<ProfileSide>,
    pub path: String,
    /// The viewer page, when the build has a part that opens captures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_at: Option<String>,
    pub removed: Vec<String>,
}

/// The reply to `profile status`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub ok: bool,
    pub capturing: bool,
    pub sides: Vec<ProfileSide>,
    pub path: Option<String>,
    pub elapsed_s: Option<f64>,
    pub bytes: Option<u64>,
}

/// The reply of a deferred `profile stop` that failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Failed {
    pub error: String,
}

/// The reply to `trace`: whether a timeline capture runs, and where it writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Traced {
    pub ok: bool,
    pub tracing: bool,
    pub file: Option<String>,
}

/// The reply to `profile open`: the viewer the capture was opened in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Opened {
    pub ok: bool,
    pub viewer: String,
}

/// The reply to `profile share`: the capture's share link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shared {
    pub ok: bool,
    pub url: String,
}

/// The reply to `profile mark`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Marked {
    pub ok: bool,
    pub name: String,
    /// Seconds on the game clock, which counts from when the game began, for every side.
    pub t: f64,
}

/// Tests that sample threads take turns: a sampler session arms every thread of the name it asks for, the other tests' too.
#[cfg(all(any(test, feature = "test-support"), target_os = "linux"))]
pub static SAMPLED_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The process has one timeline: tests that run one take this first.
#[cfg(any(test, feature = "test-support"))]
pub fn timeline_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(own: Option<GpuPassReadings>, pack: Option<GpuPassReadings>) -> GpuPerfSnapshot {
        GpuPerfSnapshot {
            adapter: "Test GPU".into(),
            backend: GpuBackendKind::Vulkan,
            driver: "NVIDIA 570.1".into(),
            own,
            pack,
        }
    }

    fn support(unit: Option<GpuFragmentUnitKind>) -> GpuDetailSupport {
        GpuDetailSupport {
            parts: GpuPartsModeKind::InsidePasses,
            encoder_stamps: true,
            counters: true,
            fragment_unit: unit,
        }
    }

    #[test]
    fn known_medians_parts_and_counters_come_through_with_their_names() {
        let c = |v, f| GpuCounters {
            vertex_shader_invocations: v,
            clipper_invocations: 2_050_000,
            clipper_primitives_out: 1_200_000,
            fragment_shader_invocations: f,
        };
        let own = GpuPassReadings {
            passes: vec![("World".into(), 1.5), ("HUD".into(), 0.25)],
            parts: vec![("GPU culling".into(), vec![("cull".into(), 0.125)])],
            counters: vec![(
                "World".into(),
                GpuPassCounters {
                    total: c(4_100_000, 6_300_000),
                    pixels: 2_073_600,
                    parts: vec![
                        ("before translucent".into(), c(3_000_000, 5_100_000)),
                        ("translucent terrain".into(), c(1_100_000, 1_200_000)),
                    ],
                },
            )],
            support: support(Some(GpuFragmentUnitKind::Quad)),
        };
        let readout = snapshot(Some(own), None);
        let mut strings = Interner::default();
        let got = gpu_passes(
            &readout,
            &GpuFrameLog::default(),
            Side::Client,
            &mut strings,
        );
        assert_eq!(got.len(), 1);
        let g = &got[0];
        assert_eq!((g.side(), g.source()), (Side::Client, GpuSource::Sunrise));
        let d = g.device.as_ref().unwrap();
        assert_eq!(d.adapter, "Test GPU");
        assert_eq!(d.backend(), GpuBackend::Vulkan);
        assert_eq!(d.driver, "NVIDIA 570.1");
        assert_eq!(d.parts_mode(), GpuPartsMode::InsidePasses);
        assert!(d.counters);
        assert_eq!(d.fragment_unit(), GpuFragmentUnit::Quad);

        // World, HUD, then the pass known only through its parts.
        let entries = strings.take_new(Side::Client);
        let name = |id: u32| entries.iter().find(|e| e.id == id).unwrap().text.as_str();
        let named: Vec<_> = g.passes.iter().map(|p| name(p.name)).collect();
        assert_eq!(named, ["World", "HUD", "GPU culling"]);
        let shown: Vec<_> = g.passes.iter().map(|p| p.ms).collect();
        assert_eq!(shown, [Some(1.5), Some(0.25), None]);
        assert_eq!(name(g.passes[2].parts[0].name), "cull");
        assert_eq!(g.passes[2].parts[0].ms, 0.125);
        let counted = &g.counters[0];
        assert_eq!(counted.pixels, 2_073_600);
        assert_eq!(
            counted.total.as_ref().unwrap().fragment_shader_invocations,
            6_300_000
        );
        assert_eq!(counted.parts.len(), 2);
        assert_eq!(
            counted.parts[1]
                .counts
                .as_ref()
                .unwrap()
                .vertex_shader_invocations,
            1_100_000
        );
        // The pass's counters and its time name the same string.
        assert_eq!(counted.pass, g.passes[0].name);
    }

    #[test]
    fn a_timer_that_timed_nothing_adds_no_section() {
        let idle = GpuPassReadings {
            passes: Vec::new(),
            parts: Vec::new(),
            counters: Vec::new(),
            support: support(None),
        };
        let pack = GpuPassReadings {
            passes: vec![("composite".into(), 0.75)],
            ..idle.clone()
        };
        let readout = snapshot(Some(idle), Some(pack));
        let got = gpu_passes(
            &readout,
            &GpuFrameLog::default(),
            Side::Client,
            &mut Interner::default(),
        );
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source(), GpuSource::ShaderPack);
        assert_eq!(
            got[0].device.as_ref().unwrap().fragment_unit(),
            GpuFragmentUnit::Unspecified
        );
    }

    #[test]
    fn the_end_section_takes_the_whole_capture_median_and_each_block_its_frames() {
        let start = Instant::now();
        let frame = |i: u32, world: f32| TimedFrameDto {
            at: start + std::time::Duration::from_millis(u64::from(i) * 10),
            total_ms: Some(world + 1.0),
            passes: vec![("World".into(), world), ("HUD".into(), 0.5)],
            parts: vec![("World".into(), vec![("sky".into(), world / 4.0)])],
        };
        let mut log = GpuFrameLog::default();
        let mut strings = Interner::default();
        let worlds = [1.0, 2.0, 3.0, 4.0, 100.0];
        let frames = TimedFramesDto {
            own: worlds
                .iter()
                .enumerate()
                .map(|(i, &w)| frame(i as u32, w))
                .collect(),
            pack: Vec::new(),
        };
        log.add(
            &frames,
            |at| at.duration_since(start).as_secs_f64(),
            &mut strings,
        );
        log.cpu_frame(6.0);
        let block = log.take_block(Side::Client).expect("a block");
        assert_eq!(block.frames.len(), 5);
        assert_eq!(block.cpu_frame_ms, [6.0]);
        assert!((block.frames[4].t - 0.04).abs() < 1e-9);
        assert_eq!(block.frames[4].total_ms, Some(101.0));
        assert_eq!(block.frames[4].passes[0].parts[0].ms, 25.0);
        assert!(log.take_block(Side::Client).is_none());

        // The timer's own median (9.0) is of its last frames; the capture's is the third of five sorted.
        let own = GpuPassReadings {
            passes: vec![("World".into(), 9.0)],
            parts: Vec::new(),
            counters: Vec::new(),
            support: support(None),
        };
        let readout = snapshot(Some(own), None);
        let got = gpu_passes(&readout, &log, Side::Client, &mut strings);
        let world = &got[0].passes[0];
        assert_eq!(world.ms, Some(3.0));
        assert_eq!(world.parts[0].ms, 0.75);
        assert_eq!(
            got[0].device.as_ref().unwrap().pass_split(),
            GpuPassSplit::DrawGroups
        );
    }

    fn p(line: &str) -> Result<Parsed, String> {
        parse(&line.split_whitespace().collect::<Vec<_>>())
    }

    #[test]
    fn open_and_share_take_a_file_with_spaces_kept() {
        assert_eq!(
            p("open reports/a.hzperf").unwrap().request,
            Request::Open(PathBuf::from("reports/a.hzperf"))
        );
        assert_eq!(
            p("share /tmp/my  capture.hzperf").unwrap().request,
            Request::Share(PathBuf::from("/tmp/my capture.hzperf"))
        );
        assert!(p("open").is_err());
        assert!(p("share").is_err());
    }

    #[test]
    fn the_side_comes_first_or_as_a_flag() {
        let a = p("client start --seconds 30").unwrap();
        let b = p("start --side client --seconds 30").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.side, Some(ProfileSide::Client));
        assert_eq!(p("start").unwrap().side, None);
        assert_eq!(p("server stop").unwrap().side, Some(ProfileSide::Server));
        assert_eq!(
            p("stop --side client").unwrap().side,
            Some(ProfileSide::Client)
        );
    }

    #[test]
    fn every_start_switch_lands_in_its_field() {
        let Request::Start(o) = p(
            "start --hz 500 --over 12.5 --threads all --wall --gpu=detail --no-samples --timeline --seconds 9",
        )
        .unwrap()
        .request
        else {
            panic!("not a start");
        };
        assert_eq!(
            o,
            StartOptions {
                seconds: Some(9),
                hz: Some(500),
                over_ms: Some(12.5),
                threads: Threads::All,
                wall: true,
                samples: false,
                gpu: GpuSwitch::Detail,
                timeline: true,
                file: None,
            }
        );
    }

    #[test]
    fn a_mark_takes_the_rest_of_the_line_as_its_name() {
        assert_eq!(
            p("mark   loaded the nether").unwrap().request,
            Request::Mark("loaded the nether".into())
        );
        assert_eq!(
            p("client mark --wall").unwrap(),
            Parsed {
                side: Some(ProfileSide::Client),
                request: Request::Mark("--wall".into())
            }
        );
        assert_eq!(
            p("mark --side client the nether").unwrap(),
            Parsed {
                side: Some(ProfileSide::Client),
                request: Request::Mark("the nether".into())
            }
        );
        assert!(p("mark").is_err());
        assert!(p("mark --side client").is_err());
        assert!(p(&format!("mark {}", "x".repeat(MARK_NAME_MAX + 1))).is_err());
        assert!(p(&format!("mark {}", "x".repeat(MARK_NAME_MAX))).is_ok());
    }

    #[test]
    fn stop_takes_the_rest_of_the_line_as_a_report_path() {
        assert_eq!(p("stop").unwrap().request, Request::Stop { bench: None });
        let with = p("client stop --bench /home/a b/games/benchmarks/game-1.json").unwrap();
        assert_eq!(with.side, Some(ProfileSide::Client));
        assert_eq!(
            with.request,
            Request::Stop {
                bench: Some("/home/a b/games/benchmarks/game-1.json".into())
            }
        );
        for line in [
            "stop --bench",
            "start --bench x.json",
            "status --bench x.json",
        ] {
            assert!(p(line).is_err(), "{line:?} was accepted");
        }
    }

    #[test]
    fn bad_words_are_refused_with_the_usage() {
        for line in [
            "",
            "client",
            "begin",
            "start --seconds 0",
            "start --seconds 601",
            "start --seconds",
            "start --hz 5",
            "start --over 0",
            "start --over nan",
            "start --threads some",
            "start --gpu=all",
            "start --gpu=",
            "stop now",
            "stop --wall",
            "status --hz 100",
            "start --side everyone",
        ] {
            assert!(p(line).is_err(), "{line:?} was accepted");
        }
        assert!(p("start --gpu=all").unwrap_err().contains("--gpu=detail"));
        assert!(
            p("start --gpu fast")
                .unwrap_err()
                .contains("usage: profile")
        );
    }

    #[test]
    fn gpu_alone_records_the_passes_and_detail_asks_for_more() {
        let gpu = |line: &str| match p(line).unwrap().request {
            Request::Start(o) => o.gpu,
            other => panic!("{other:?}"),
        };
        assert_eq!(gpu("start"), GpuSwitch::Off);
        assert_eq!(gpu("start --gpu"), GpuSwitch::Passes);
        assert_eq!(
            gpu("client start --seconds 5 --gpu=detail"),
            GpuSwitch::Detail
        );
        assert_eq!(gpu("start --gpu detail --wall"), GpuSwitch::Detail);
        assert_eq!(gpu("start --gpu --wall"), GpuSwitch::Passes);
        assert!(p("stop --gpu").is_err(), "only start takes options");
    }
}
