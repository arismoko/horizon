use super::{Context, Dim, R};
use crate::game::commands::pregen::PregenTask;
use crate::game::commands::pregen::REPORT_EVERY;
use crate::game::commands::pregen::STALL;
use crate::game::commands::pregen::failure;
use std::time::{Duration, Instant};
use sunrise_game_data::text::{self, Text};
use sunrise_sim::pregen::{Pregen, Shape};

impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn cmd_pregen(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "pregen.start" => {
                if self.server.cmds.0.pregen.is_some() {
                    return Err(failure(
                        "Pregeneration is already running: /pregen cancel stops it",
                    ));
                }
                let radius = Self::int(ctx, "radius").unwrap_or(0);
                let shape = if ctx.literals.iter().any(|l| l == "circle") {
                    Shape::Circle
                } else {
                    Shape::Square
                };
                let [x, z] = if ctx.get("center").is_some() {
                    self.column_arg(ctx, "center")?
                } else {
                    let p = self.source_position();
                    [p[0].floor() as i32, p[2].floor() as i32]
                };
                let pregen = Pregen::new((x >> 4, z >> 4), radius >> 4, shape);
                let now = Instant::now();
                let total = pregen.total;
                self.feedback(Text::literal(format!(
                    "Pregenerating {total} chunks of {}: a {} of radius {radius} around {x} {z}",
                    self.dim.id(),
                    match shape {
                        Shape::Square => "square",
                        Shape::Circle => "circle",
                    },
                )));
                self.server.cmds.0.pregen = Some(PregenTask {
                    pregen,
                    dim: self.common.dim,
                    paused: false,
                    ran: Duration::ZERO,
                    resumed: now,
                    reported: now,
                    moved: now,
                    owner: self.server.lan_acting,
                });
                Ok(total as i32)
            }
            "pregen.pause" | "pregen.continue" => {
                let pause = id == "pregen.pause";
                let task = self
                    .server
                    .cmds
                    .0
                    .pregen
                    .as_mut()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                if task.paused != pause {
                    task.paused = pause;
                    if pause {
                        task.ran += task.resumed.elapsed();
                    } else {
                        task.resumed = Instant::now();
                    }
                }
                let (progress, dim) = (task.progress(), task.dim);
                if pause && self.common.dim == dim {
                    self.server.streaming.set_band(None);
                }
                self.feedback(
                    Text::literal(if pause { "Paused. " } else { "Continuing. " }).append(progress),
                );
                Ok(1)
            }
            "pregen.cancel" => {
                let task = self
                    .server
                    .cmds
                    .0
                    .pregen
                    .take()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                if self.common.dim == task.dim {
                    self.server.streaming.set_band(None);
                }
                self.feedback(Text::literal("Cancelled. ").append(task.progress()));
                Ok(task.pregen.done as i32)
            }
            _ => {
                let task = self
                    .server
                    .cmds
                    .0
                    .pregen
                    .as_ref()
                    .ok_or_else(|| failure("No pregeneration is running"))?;
                let progress = task.progress();
                let done = task.pregen.done as i32;
                self.feedback(progress);
                Ok(done)
            }
        }
    }
    /// The level a running task needs ticked, whoever is in it.
    pub(in crate::game::server_side) fn pregen_level(&self) -> Option<Dim> {
        self.server.cmds.0.pregen.as_ref().map(|t| t.dim)
    }
    /// Whether the level in place has a running task, which the server pumps
    /// between its ticks (`pump_chunks`).
    pub(in crate::game::server_side) fn pregen_runs_here(&self) -> bool {
        self.server
            .cmds
            .0
            .pregen
            .as_ref()
            .is_some_and(|t| t.dim == self.common.dim && !t.paused)
    }
    /// Keep the workers fed until `until`: collect what they finished and
    /// hand them the next jobs as soon as each one is done, rather than once
    /// a tick. Return time spent waiting for workers, not running chunk tasks.
    pub(in crate::game::server_side) fn pump_chunks(
        &mut self,
        until: Instant,
        paused: bool,
    ) -> Duration {
        // Force-loaded chunks too, a mod's held ones as `/forceload`'s: decorating in vanilla's order, a chunk
        // waits for its neighbours far more often than once a tick allows.
        // And the players' chunks while they load, as vanilla's server runs
        // its chunk tasks while it waits for the next tick
        // (`MinecraftServer.waitUntilNextTick`): once a tick, a few jobs per
        // worker (`schedule_generation`) capped joining a world at about 120
        // columns a second with two workers, the workers idle most of the
        // time. Not while a respawn search waits for a chunk: its area is
        // only asked for by the tick's own `update_world`.
        // A viewed area's chunks (#3716) as the forced ones.
        let players = !paused && self.server.spawn_wait.is_empty();
        let viewed = self.viewed_level(self.common.dim).is_some();
        let mut idle = Duration::ZERO;
        while self.pregen_runs_here()
            || ((players || viewed || !self.server.streaming.forced.is_empty())
                && self.server.streaming.busy())
        {
            let now = Instant::now();
            if now >= until {
                break;
            }
            idle += self
                .server
                .streaming
                .wait((until - now).min(Duration::from_millis(5)));
            self.server_update_world();
            self.run_tasks();
        }
        idle
    }
    /// Move the task's band past the rows that are lit and hand it to the
    /// level's streaming, and report the progress now and then. Runs before
    /// each streaming update of the level in place.
    pub(in crate::game::server_side) fn tick_pregen(&mut self) {
        let Some(task) = self.server.cmds.0.pregen.as_mut() else {
            return;
        };
        if task.dim != self.common.dim || task.paused {
            return;
        }
        // A row is done once its chunks are lit and post-processed
        // (`postprocess_chunks`, on the tick), as vanilla's are by the time
        // they are FULL: then they may unload and be saved.
        let (streaming, chunks) = (&mut self.server.streaming, &self.common.world.chunks);
        if task.pregen.advance(|x, z| {
            streaming.is_lit(x, z)
                && chunks
                    .get(&(x, z))
                    .is_some_and(|c| c.postprocess.is_empty())
        }) {
            task.moved = Instant::now();
        }
        streaming.set_band(task.pregen.band());
        if task.moved.elapsed() >= STALL {
            // Nothing advanced for a while: say what the first row waits
            // for, and queue light again for chunks that lost their job.
            task.moved = Instant::now();
            let row = task.pregen.first_row();
            let waiting = row
                .iter()
                .filter(|k| chunks.get(k).is_some_and(|c| !c.postprocess.is_empty()))
                .count();
            let [missing, lighting, unlit] = streaming.unstick_light(&mut self.common.world, &row);
            log::warn!(
                "Sunrise: pregeneration waits on row {:?}: {missing} not loaded, {lighting} being lit, \
                 {unlit} unlit with no light job (queued again), {waiting} not post-processed; {:?}",
                row.first(),
                streaming.stages()
            );
        }
        let finished = task.pregen.finished();
        if finished || task.reported.elapsed() >= REPORT_EVERY {
            task.reported = Instant::now();
            let (owner, mut message) = (task.owner, task.progress());
            if finished {
                message = Text::literal("Done. ")
                    .append(message)
                    .with_color(text::GREEN);
                self.server.cmds.0.pregen = None;
            }
            self.pregen_report(owner, message);
        }
    }
}
