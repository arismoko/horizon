//! `sunrise-report [SECONDS]` and `sunrise-trace [SECONDS]` on the dedicated server's console, and `SUNRISE_TRACE`: record a window of ticks into `reports/profile-<time>Z.hzperf`.
use super::super::memory::{MEMORY_EVERY_S, flatten_report};
use super::super::{
    Dim,
    perf_remote::{Asked, Done},
    profile_capture_common::shown,
    tick_profile,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(in crate::game) use sunrise_gameplay::dedicated::report::*;
use sunrise_level::chunk_streamer::job_stats;
use sunrise_perf_format::{
    files,
    v1::{GpuMode, Start, StopReason, SunriseOptions, Trigger},
};
use sunrise_sim::perf_net::Refusal;
use sunrise_sim::perf_types::Threads;
use sunrise_sim::server_perf::{
    self, CaptureEnd, CaptureOptions, SamplerOptions, ServerPerf, WorldCounts,
    traffic::{Conn, Wire},
    world::{LevelCounts, MobCost, MobCosts, WorkerJobs},
};
use sunrise_world::level::BlockId;

/// What a capture is asked to record, and how it was asked.
pub struct Plan {
    pub seconds: u32,
    pub samples: bool,
    pub timeline: bool,
    /// Where the capture is written when it is not `reports/`.
    pub file: Option<PathBuf>,
    pub trigger: Trigger,
    pub sampler: SamplerOptions,
}

/// Logs the captures whose files are done; with `wait`, every one still being finished.
pub(in crate::game::server_side) fn log_saved(perf: &mut ServerPerf, wait: bool) -> Vec<Done> {
    perf.take_saved(wait)
        .into_iter()
        .map(|saved| match saved {
            Ok(saved) => {
                for path in &saved.deleted {
                    log::info!("Sunrise: removed the oldest capture {}", path.display());
                }
                log::info!("Sunrise: {}", files::saved_line(&saved.path));
                Done::Saved {
                    path: shown(&saved.path),
                    file: saved.path.clone(),
                    removed: saved.deleted.iter().map(|p| shown(p)).collect(),
                }
            }
            Err(e) => {
                log::error!("Sunrise: could not write the performance capture: {e}");
                Done::Failed
            }
        })
        .collect()
}

impl crate::game::ServerGame<'_> {
    /// Starts a report: the recording the game side needs and the capture beside it.
    pub(super) fn record_report(
        &self,
        state: &mut super::ServerState,
        plan: &Plan,
    ) -> Option<PathBuf> {
        state.report = Some(Recording::start(plan.seconds));
        self.start_perf_capture(state, plan)
    }

    /// Begins and ends the captures guests asked for, between two ticks.
    pub(super) fn serve_perf_requests(&mut self, state: &mut super::ServerState) {
        self.pump_perf_files();
        for asked in self.take_perf_asked() {
            match asked {
                Asked::Start { conn, name, wanted } => {
                    if state.report.is_some() || state.perf.capturing() || state.perf.finishing() {
                        self.perf_refused(conn, Refusal::AlreadyRunning);
                        continue;
                    }
                    let options = wanted.options();
                    let plan = Plan {
                        seconds: options.seconds.unwrap_or(MAX_SECONDS),
                        samples: options.samples,
                        timeline: options.timeline,
                        file: None,
                        trigger: Trigger::Remote,
                        sampler: SamplerOptions {
                            hz: options.hz,
                            over_ms: options.over_ms,
                            wall: options.wall,
                            all_threads: options.threads == Threads::All,
                        },
                    };
                    match self.record_report(state, &plan) {
                        Some(path) => self.perf_started(conn, name, &wanted, &shown(&path)),
                        None => {
                            state.report = None;
                            log::error!("Sunrise: {name}'s server profile did not start");
                            self.perf_refused(conn, Refusal::Failed);
                        }
                    }
                }
                Asked::Stop(reason) => self.finish_perf_capture(state, reason),
            }
        }
    }

    /// Opens the `.hzperf` that records beside a report, and logs where it goes.
    pub(super) fn start_perf_capture(
        &self,
        state: &mut super::ServerState,
        plan: &Plan,
    ) -> Option<PathBuf> {
        let options = self.capture_options(
            plan,
            (state.view, state.max_players, state.compression),
            state.sunrise,
        );
        let t = state.perf_epoch.elapsed().as_secs_f64();
        match state.perf.start_capture(t, Path::new("reports"), options) {
            Ok(path) => {
                log::info!("Sunrise: capturing to {}", path.display());
                Some(path)
            }
            Err(e) => {
                log::error!("Sunrise: could not start the performance capture: {e}");
                None
            }
        }
    }

    /// Every block's name by the id the sampler's context holds.
    pub(in crate::game::server_side) fn block_names(&self) -> Vec<String> {
        let table = &self.common.res.behaviours().table;
        (0..table.block_count() as u16)
            .map(|id| table.info(BlockId(id)).name.to_string())
            .collect()
    }

    /// What a capture of `plan` records of the server as it starts: the typed allowlists, the world and the connections.
    pub fn capture_options(
        &self,
        plan: &Plan,
        (view, max_players, compression): ((u32, u32), usize, i32),
        sunrise: SunriseOptions,
    ) -> CaptureOptions {
        let start = Start {
            build: Some(server_perf::build()),
            machine: Some(server_perf::machine(server_perf::hardware())),
            settings: Some(server_perf::settings(
                view.0,
                view.1,
                max_players as u32,
                compression,
            )),
            mods: self.common.perf_mods(),
            game_rules: Some(server_perf::game_rules(&self.common.ui.rules)),
            sunrise_options: Some(sunrise),
            players: self.server.lan.as_ref().map_or(0, |h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| g.participates_online())
                    .count() as u32
            }),
            ..Start::default()
        };
        CaptureOptions {
            window_s: Some(f64::from(plan.seconds)),
            trigger: plan.trigger,
            start,
            samples: plan.samples,
            sampler: plan.sampler,
            block_names: self.block_names(),
            levels: self.level_counts(),
            wire: self.wire_bytes(),
            gpu: GpuMode::Unspecified,
            timeline: plan.timeline.then(Default::default),
            file: plan.file.clone(),
        }
    }

    fn active_perf_view(&self) -> super::super::levels::EntityCountView<'_> {
        super::super::levels::entity_count_view!(
            self,
            &self.creatures.projectiles,
            &self.resident_entities.general.rows,
            &self.resident_entities.knots,
            &self.server.others.0
        )
    }

    /// All typed entities in the active dimension, without allocating.
    pub(in crate::game) fn perf_entity_count(&self) -> usize {
        self.active_perf_view().total()
    }

    /// Health samples count every typed family using lengths only.
    pub fn perf_world_counts(&self) -> WorldCounts {
        let (mut chunks, mut entities) = (self.common.world.chunks.len(), self.perf_entity_count());
        for (_, world, view) in self.server.levels.0.parked_perf_views() {
            chunks += world.chunks.len();
            entities += view.total();
        }
        WorldCounts {
            players: self.server.lan.as_ref().map_or(0, |h| {
                h.hub
                    .guests
                    .iter()
                    .filter(|g| g.participates_online())
                    .count() as u32
            }),
            chunks: chunks as u32,
            entities: entities as u32,
        }
    }

    /// Capture snapshots resolve registry labels; gauges use the same view without labels.
    pub(in crate::game::server_side) fn level_counts(&self) -> Vec<LevelCounts> {
        let level = |dim: Dim,
                     world: &sunrise_level::world::World,
                     view: super::super::levels::EntityCountView<'_>| {
            let mut block_entities = BTreeMap::<String, u32>::new();
            for entity in world.block_entities.values() {
                *block_entities.entry(entity.id().to_owned()).or_default() += 1;
            }
            LevelCounts {
                dimension: dim.id().to_owned(),
                chunks: world.chunks.len() as u32,
                entities: view.registry_counts(),
                block_entities: block_entities.into_iter().collect(),
            }
        };
        std::iter::once(level(
            self.common.dim,
            &self.common.world,
            self.active_perf_view(),
        ))
        .chain(
            self.server
                .levels
                .0
                .parked_perf_views()
                .map(|(dim, world, view)| level(dim, world, view)),
        )
        .collect()
    }

    /// Each player's connection: wire bytes (in, out), name, brand and view distance.
    pub fn wire_bytes(&self) -> Wire {
        self.server.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter_map(|g| {
                    Some(Conn {
                        id: g.conn,
                        bytes: g.sender.wire_bytes()?,
                        name: g.name.clone(),
                        brand: g.brand.clone(),
                        view_distance: g.view_distance.max(0) as u32,
                    })
                })
                .collect()
        })
    }

    fn take_server_memory_snapshot(&self, state: &mut super::ServerState, label: &str) {
        if let Some(cost) = state.perf.memory_cost() {
            let rows = cost.measure(|| flatten_report(&self.memory_value()));
            let t = state.perf_epoch.elapsed().as_secs_f64();
            state.perf.memory_snapshot(t, label, &rows);
        }
    }

    /// A capture's memory snapshots: one as it starts, then every `MEMORY_EVERY_S`.
    pub(super) fn poll_server_memory_snapshot(&self, state: &mut super::ServerState) {
        if !state.perf.capturing() {
            state.memory_due = None;
            return;
        }
        let now = state.perf_epoch.elapsed().as_secs_f64();
        match state.memory_due {
            None => {
                self.take_server_memory_snapshot(state, "start");
                state.memory_due = Some((now + MEMORY_EVERY_S, 0));
            }
            Some((due, taken)) if now >= due => {
                let taken = taken + 1;
                let label = format!("{}s", u64::from(taken) * MEMORY_EVERY_S as u64);
                self.take_server_memory_snapshot(state, &label);
                state.memory_due = Some((due + MEMORY_EVERY_S, taken));
            }
            Some(_) => {}
        }
    }

    /// Ends the capture and the recording beside it, if they run; `log_saved` reports the file when it is done.
    pub(super) fn finish_perf_capture(&self, state: &mut super::ServerState, reason: StopReason) {
        let recording = state.report.take();
        if !state.perf.capturing() {
            return;
        }
        self.take_server_memory_snapshot(state, "stop");
        let end = recording.map_or_else(CaptureEnd::default, |r| self.capture_end(&r));
        let t = state.perf_epoch.elapsed().as_secs_f64();
        state.perf.begin_stop(t, reason, end);
    }

    /// What a finished recording leaves for the capture: the world as it stands, the sampled mob costs and the workers' time.
    pub fn capture_end(&self, recording: &Recording) -> CaptureEnd {
        let span_ns = span_cost_nanos();
        let mob_ns = tick_profile::sample_cost_nanos();
        // Release this recording's span request; other recordings or SUNRISE_SPIKES may remain.
        if recording.all_threads {
            tick_profile::want_spans(tick_profile::SPANS_REPORT, false);
        }
        job_stats::enable(false);
        CaptureEnd {
            levels: self.level_counts(),
            mob_costs: MobCosts {
                costs: tick_profile::take()
                    .into_iter()
                    .map(|c| MobCost {
                        dimension: c.dimension.to_owned(),
                        kind: c.kind,
                        nanos: c.nanos,
                        mob_ticks: c.mob_ticks,
                    })
                    .collect(),
                sampled_ticks: recording.sampled_ticks,
            },
            jobs: job_stats::take()
                .into_iter()
                .map(|(kind, nanos, count)| WorkerJobs {
                    kind: kind.to_owned(),
                    nanos,
                    count,
                })
                .collect(),
            wire: self.wire_bytes(),
            span_ns,
            mob_ns,
            gpu: None,
            bench: Vec::new(),
        }
    }
}

use sunrise_gameplay_common::game::memory_common::MemoryCommonGameExt1 as _;
