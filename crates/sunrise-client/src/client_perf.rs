//! The client's .hzperf captures (docs/perf-platform.md §4.1): the window's frames feed the always-on health history, and a capture writes it with the samples of the render thread.
use crate::gfx::pack_timing::{
    Counters as PackCounters, DetailSupport, FragmentUnit, GpuReadout, PartsMode, PassCounters,
    Readings, TimedFrame, TimedFrames,
};
use crate::perf_types::{
    GpuBackendKind, GpuCounters, GpuDetailSupport, GpuFragmentUnitKind, GpuPartsModeKind,
    GpuPassCounters, GpuPassReadings, GpuPerfSnapshot, TimedFrameDto, TimedFramesDto,
};
use crate::server_perf::traffic::{Conn, Wire};
use crate::server_perf::{
    self, CaptureEnd, CaptureOptions, CaptureStatus, MAX_WINDOW_S, Role, SamplerOptions, Saved,
    ServerPerf, TickDetail, WorldCounts,
};
use request::{GpuSwitch, StartOptions, Threads};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use sunrise_perf_format::v1::{
    BenchResult, Machine, Side, Start, StopReason, Trigger, WarningCode,
};
use sunrise_protocol::connection::Counters;

pub mod request;

#[cfg(test)]
mod tests;

/// A frame this slow enters the slow ring when `SUNRISE_SPIKES` names no other threshold.
pub const SLOW_FRAME_MS: f32 = 50.0;

/// The variables that start a capture with the GPU passes timed: pass timing, and pass timing with its parts.
pub const ENV_PASS_TIMING: &str = "SUNRISE_SHADER_PACK_TIMING";
pub const ENV_PASS_DETAIL: &str = "SUNRISE_GPU_PASS_DETAIL";

/// The GPU switch a run's environment asks for; the detail variable wins over the timing one.
pub fn env_gpu(timing: Option<&OsStr>, detail: Option<&OsStr>) -> GpuSwitch {
    if detail.is_some() {
        GpuSwitch::Detail
    } else if timing.is_some() {
        GpuSwitch::Passes
    } else {
        GpuSwitch::Off
    }
}

pub use sunrise_sim::perf_types::{Join, reports_dir};

/// The client's health history, always on, and the capture running from it, if any.
pub struct ClientPerf {
    perf: ServerPerf,
    epoch: Instant,
    cpu_ms: Vec<f32>,
    gpu_ms: Vec<f32>,
    /// When the running capture's window ends, and why it stops then.
    window: Option<(f64, StopReason)>,
    /// The running capture's switches, which a side that joins it follows.
    running: Option<(StartOptions, Trigger)>,
    /// The server connection's byte counts while the window plays on one: the capture's own traffic.
    connection: Option<Arc<Counters>>,
}

/// The window's one connection as the traffic recorder's wire list.
fn wire_of(connection: Option<&Counters>) -> Wire {
    let order = Ordering::Relaxed;
    connection
        .map(|c| {
            let count = |n: &std::sync::atomic::AtomicI64| n.load(order).max(0) as u64;
            vec![Conn {
                id: 1,
                bytes: (count(&c.bytes_in), count(&c.bytes_out)),
                ..Conn::default()
            }]
        })
        .unwrap_or_default()
}

impl ClientPerf {
    pub fn new(slow_ms: f32) -> Self {
        Self {
            perf: ServerPerf::with_role(Role::CLIENT, 0.0, slow_ms),
            epoch: Instant::now(),
            cpu_ms: Vec::new(),
            gpu_ms: Vec::new(),
            window: None,
            running: None,
            connection: None,
        }
    }

    /// The connection whose bytes a capture records, or None in a world of the window's own.
    pub fn set_connection(&mut self, connection: Option<Arc<Counters>>) {
        self.connection = connection;
    }

    /// Seconds since the game began, the clock every other time here is on.
    pub fn now(&self) -> f64 {
        self.epoch.elapsed().as_secs_f64()
    }

    pub fn capturing(&self) -> bool {
        self.perf.capturing()
    }

    pub fn health(&self) -> &sunrise_perf_format::HealthRecorder {
        self.perf.health()
    }

    /// One rendered frame that took `wall_ms`, of which `cpu_ms` was the CPU's own work, ending at `t`.
    pub fn frame(
        &mut self,
        t: f64,
        wall_ms: f32,
        cpu_ms: f32,
        gpu_ms: Option<f32>,
        detail: Option<TickDetail>,
        world: impl FnOnce() -> WorldCounts,
    ) {
        self.frame_with_gpu(t, wall_ms, cpu_ms, gpu_ms, detail, world, || None);
    }

    /// [`ClientPerf::frame`], where `gpu` is asked for the pass timings only when this frame ends the capture's window.
    #[allow(clippy::too_many_arguments)]
    pub fn frame_with_gpu(
        &mut self,
        t: f64,
        wall_ms: f32,
        cpu_ms: f32,
        gpu_ms: Option<f32>,
        detail: Option<TickDetail>,
        world: impl FnOnce() -> WorldCounts,
        gpu: impl FnOnce() -> Option<GpuReadout>,
    ) {
        self.cpu_ms.push(cpu_ms);
        self.gpu_ms.extend(gpu_ms);
        if self.perf.gauges_due(t) {
            self.perf
                .set_frame_medians(median(&mut self.cpu_ms), median(&mut self.gpu_ms));
        }
        let connection = self.connection.as_deref();
        self.perf
            .tick(t, wall_ms, detail, world, || wire_of(connection));
        if let Some((end, reason)) = self.window
            && t >= end
        {
            self.stop_with_gpu(t, reason, gpu());
        }
    }

    /// The GPU frames timed since the last frame, with this frame's CPU ms, for a capture that times the GPU.
    pub fn gpu_frames(&mut self, t: f64, cpu_ms: f32, frames: &TimedFrames) {
        self.perf.gpu_frames(t, cpu_ms, &timed_frames_dto(frames));
    }

    /// Opens a capture in `dir`; call it on the render thread, which it samples.
    pub fn start(
        &mut self,
        t: f64,
        dir: &Path,
        trigger: Trigger,
        options: &StartOptions,
        start: Start,
        block_names: Vec<String>,
    ) -> std::io::Result<PathBuf> {
        let capture = CaptureOptions {
            window_s: options.seconds.map(f64::from),
            trigger,
            start,
            samples: options.samples,
            sampler: SamplerOptions {
                hz: options.hz,
                over_ms: options.over_ms,
                wall: options.wall,
                all_threads: options.threads == Threads::All,
            },
            block_names,
            levels: Vec::new(),
            wire: wire_of(self.connection.as_deref()),
            gpu: options.gpu.mode(),
            timeline: options.timeline.then(Default::default),
            file: options.file.clone(),
        };
        let path = self.perf.start_capture(t, dir, capture)?;
        self.window = Some(match options.seconds {
            Some(s) => (t + f64::from(s), StopReason::Normal),
            None => (t + MAX_WINDOW_S, StopReason::WindowCap),
        });
        self.running = Some((options.clone(), trigger));
        Ok(path)
    }

    /// What the server thread needs to write its side into the running capture; None when none runs.
    pub fn join(&self) -> Option<Join> {
        let (sender, path, origin) = self.perf.joinable()?;
        let (options, trigger) = self.running.clone()?;
        let at = |t: f64| self.epoch + Duration::from_secs_f64(t.max(0.0));
        Some(Join {
            sender,
            path,
            origin: at(origin),
            ends: self.window.map(|(end, reason)| (at(end), reason)),
            options,
            trigger,
        })
    }

    /// Ends the running capture; its file is finished off this thread and comes back from [`ClientPerf::take_saved`].
    pub fn stop(&mut self, t: f64, reason: StopReason) -> bool {
        self.stop_with(t, reason, None, Vec::new())
    }

    /// [`ClientPerf::stop`], recording `gpu`'s pass timings if the capture asked for them.
    pub fn stop_with_gpu(&mut self, t: f64, reason: StopReason, gpu: Option<GpuReadout>) -> bool {
        self.stop_with(t, reason, gpu, Vec::new())
    }

    /// [`ClientPerf::stop_with_gpu`] with benchmark reports embedded in the capture.
    pub fn stop_with(
        &mut self,
        t: f64,
        reason: StopReason,
        gpu: Option<GpuReadout>,
        bench: Vec<BenchResult>,
    ) -> bool {
        self.window = None;
        self.running = None;
        let end = CaptureEnd {
            gpu: gpu.map(gpu_perf_snapshot),
            bench,
            wire: wire_of(self.connection.as_deref()),
            ..CaptureEnd::default()
        };
        self.perf.begin_stop(t, reason, end)
    }

    /// Whether a stopped capture's file is still being finished or waits to be taken.
    pub fn saving(&self) -> bool {
        self.perf.finishing()
    }

    /// The captures whose files are done; with `wait`, every one begun.
    pub fn take_saved(&mut self, wait: bool) -> Vec<std::io::Result<Saved>> {
        self.perf.take_saved(wait)
    }

    pub fn mark(&mut self, t: f64, name: &str) -> bool {
        self.perf.mark(t, name)
    }

    /// Records a memory snapshot in the running capture; false when none runs.
    pub fn memory_snapshot(&mut self, t: f64, label: &str, rows: &[(String, f64)]) -> bool {
        self.perf.memory_snapshot(t, label, rows)
    }

    pub fn memory_cost(&self) -> Option<crate::server_perf::MemoryCost> {
        self.perf.memory_cost()
    }

    /// Records `code` in the running capture; false when none runs.
    pub fn warn(&mut self, t: f64, code: WarningCode) -> bool {
        self.perf.warn(t, code)
    }

    pub fn status(&self, t: f64) -> Option<CaptureStatus> {
        self.perf.status(t)
    }
}

/// A benchmark report as the capture embeds it, minus its local-only label and paths; anything but a `sunrise-benchmark-report` is refused.
pub fn bench_result(report_json: String) -> Result<BenchResult, String> {
    let mut report = sunrise_bench::Report::parse(&report_json)?;
    report.label = None;
    report.local.clear();
    Ok(BenchResult {
        side: Side::Client as i32,
        format: sunrise_bench::FORMAT.to_owned(),
        report_json: report.to_json(),
        corpus_id: String::new(),
    })
}

/// The median of the values, which are cleared; None when there were none.
fn median(values: &mut Vec<f32>) -> Option<f32> {
    values.sort_by(f32::total_cmp);
    let rank = values.len().div_ceil(2);
    let m = rank.checked_sub(1).map(|i| values[i]);
    values.clear();
    m
}

/// `Start` for a client capture: its build and machine; the capture fills the rest.
pub fn start_message(same_process: bool, gpu: Option<&str>) -> Start {
    let mut machine: Machine = server_perf::machine(Some(
        server_perf::hardware().unwrap_or_else(server_perf::hardware_now),
    ));
    if let Some(gpu) = gpu {
        machine.gpu = gpu.to_owned();
    }
    Start {
        build: Some(server_perf::build()),
        machine: Some(machine),
        same_process,
        ..Start::default()
    }
}

fn timed_frame_dto(frame: &TimedFrame) -> TimedFrameDto {
    TimedFrameDto {
        at: frame.at,
        total_ms: frame.total_ms,
        passes: frame.passes.clone(),
        parts: frame.parts.clone(),
    }
}

fn timed_frames_dto(frames: &TimedFrames) -> TimedFramesDto {
    TimedFramesDto {
        own: frames.own.iter().map(timed_frame_dto).collect(),
        pack: frames.pack.iter().map(timed_frame_dto).collect(),
    }
}

fn pack_counters(c: PackCounters) -> GpuCounters {
    GpuCounters {
        vertex_shader_invocations: c.vertex_shader_invocations,
        clipper_invocations: c.clipper_invocations,
        clipper_primitives_out: c.clipper_primitives_out,
        fragment_shader_invocations: c.fragment_shader_invocations,
    }
}

fn pass_counters(c: PassCounters) -> GpuPassCounters {
    GpuPassCounters {
        total: pack_counters(c.total),
        pixels: c.pixels,
        parts: c
            .parts
            .into_iter()
            .map(|(name, part)| (name, pack_counters(part)))
            .collect(),
    }
}

fn detail_support(s: DetailSupport) -> GpuDetailSupport {
    GpuDetailSupport {
        parts: match s.parts {
            PartsMode::InsidePasses => GpuPartsModeKind::InsidePasses,
            PartsMode::SeparatePasses => GpuPartsModeKind::SeparatePasses,
        },
        encoder_stamps: s.encoder_stamps,
        counters: s.counters,
        fragment_unit: s.fragment_unit.map(|u| match u {
            FragmentUnit::Pixel => GpuFragmentUnitKind::Pixel,
            FragmentUnit::Quad => GpuFragmentUnitKind::Quad,
        }),
    }
}

fn pass_readings(r: Readings) -> GpuPassReadings {
    GpuPassReadings {
        passes: r.passes,
        parts: r.parts,
        counters: r
            .counters
            .into_iter()
            .map(|(pass, c)| (pass, pass_counters(c)))
            .collect(),
        support: detail_support(r.support),
    }
}

fn gpu_perf_snapshot(r: GpuReadout) -> GpuPerfSnapshot {
    let driver = format!("{} {}", r.adapter.driver, r.adapter.driver_info);
    GpuPerfSnapshot {
        adapter: r.adapter.name,
        backend: match r.adapter.backend {
            wgpu::Backend::Vulkan => GpuBackendKind::Vulkan,
            wgpu::Backend::Metal => GpuBackendKind::Metal,
            wgpu::Backend::Dx12 => GpuBackendKind::Dx12,
            wgpu::Backend::Gl => GpuBackendKind::Gl,
            _ => GpuBackendKind::Other,
        },
        driver: driver.trim().to_owned(),
        own: r.own.map(pass_readings),
        pack: r.pack.map(pass_readings),
    }
}
