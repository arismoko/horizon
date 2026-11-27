//! `/random` (RandomCommand) and `/teammsg` (TeamMsgCommand).
use super::{Arg, Context, R, err};
use sunrise_core::java_number::format_fixed;
use sunrise_game_data::text::Text;
use sunrise_sim::clock::SprintReport;

pub(in crate::game) fn millis(nanos: u64) -> String {
    format_fixed(f64::from(nanos as f32 / 1_000_000.0), 1)
}

impl crate::game::ServerGame<'_> {
    /// `/stopwatch` (StopwatchCommand).
    pub(in crate::game::server_side) fn cmd_stopwatch(
        &mut self,
        id: &str,
        ctx: &Context,
    ) -> R<i32> {
        let Some(Arg::Resource(name)) = ctx.get("id").cloned() else {
            return Ok(0);
        };
        let arg = || Text::literal(name.clone());
        let missing = || err("commands.stopwatch.does_not_exist", &[&name]);
        let sw = &mut self.common.score.stopwatches;
        match id {
            "stopwatch.create" => {
                if !sw.create(&name) {
                    return Err(err("commands.stopwatch.already_exists", &[&name]));
                }
                self.feedback(Text::translate(
                    "commands.stopwatch.create.success",
                    &[arg()],
                ));
                Ok(1)
            }
            "stopwatch.query" => {
                let scale = match ctx.get("scale") {
                    Some(Arg::Double(v)) => *v,
                    _ => 1.0,
                };
                let t = sw.elapsed_seconds(&name).ok_or_else(missing)?;
                self.feedback(Text::translate(
                    "commands.stopwatch.query",
                    &[arg(), Text::literal(format!("{t:?}"))],
                ));
                Ok((t * scale) as i32)
            }
            "stopwatch.restart" => {
                if !sw.restart(&name) {
                    return Err(missing());
                }
                self.feedback(Text::translate(
                    "commands.stopwatch.restart.success",
                    &[arg()],
                ));
                Ok(1)
            }
            _ => {
                if !sw.remove(&name) {
                    return Err(missing());
                }
                self.feedback(Text::translate(
                    "commands.stopwatch.remove.success",
                    &[arg()],
                ));
                Ok(1)
            }
        }
    }
    /// `/tick` (TickCommand) over the game clock.
    pub(in crate::game::server_side) fn cmd_tick(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let t = |key: &str, args: &[Text]| Text::translate(key, args);
        let lit = |s: String| Text::literal(s);
        let time = match ctx.get("time") {
            Some(Arg::Int(v)) => *v,
            _ => 1,
        };
        match id {
            "tick.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(v)) => *v,
                    _ => 20.0,
                };
                self.set_tick_rate(rate);
                self.feedback(t(
                    "commands.tick.rate.success",
                    &[lit(format_fixed(f64::from(rate), 1))],
                ));
                Ok(rate as i32)
            }
            "tick.query" => {
                // TickCommand.tickQuery: every line passes `broadcast` false.
                let rate = self.common.clock.rate();
                let busy = millis(self.common.clock.average_nanos());
                let rate_text = format_fixed(f64::from(rate), 1);
                if self.common.clock.sprinting() {
                    self.feedback_unbroadcast(t("commands.tick.status.sprinting", &[]));
                    self.feedback_unbroadcast(t(
                        "commands.tick.query.rate.sprinting",
                        &[lit(rate_text), lit(busy)],
                    ));
                } else {
                    let target = self.common.clock.tick_duration().as_nanos() as u64;
                    let status = if self.common.clock.frozen() {
                        "commands.tick.status.frozen"
                    } else if target < self.common.clock.average_nanos() {
                        "commands.tick.status.lagging"
                    } else {
                        "commands.tick.status.running"
                    };
                    self.feedback_unbroadcast(t(status, &[]));
                    self.feedback_unbroadcast(t(
                        "commands.tick.query.rate.running",
                        &[lit(rate_text), lit(busy), lit(millis(target))],
                    ));
                }
                let samples = self.common.clock.sorted_samples();
                let at = |q: f32| {
                    samples
                        .get(
                            ((samples.len() as f32 * q) as usize)
                                .min(samples.len().saturating_sub(1)),
                        )
                        .copied()
                        .unwrap_or(0)
                };
                self.feedback_unbroadcast(t(
                    "commands.tick.query.percentiles",
                    &[
                        lit(millis(at(0.5))),
                        lit(millis(at(0.95))),
                        lit(millis(at(0.99))),
                        lit(samples.len().to_string()),
                    ],
                ));
                Ok(rate as i32)
            }
            "tick.sprint" => {
                if self.common.clock.start_sprint(time as u32) {
                    self.feedback(t("commands.tick.sprint.stop.success", &[]));
                }
                self.feedback(t("commands.tick.status.sprinting", &[]));
                Ok(1)
            }
            "tick.sprint.stop" => {
                let Some(report) = self.common.clock.stop_sprint() else {
                    return Err(err("commands.tick.sprint.stop.fail", &[]));
                };
                self.sprint_report(&report);
                self.on_tick_rate_changed();
                self.feedback(t("commands.tick.sprint.stop.success", &[]));
                Ok(1)
            }
            "tick.freeze" | "tick.unfreeze" => {
                let freeze = id == "tick.freeze";
                if freeze {
                    if let Some(report) = self.common.clock.stop_sprint() {
                        self.sprint_report(&report);
                        self.on_tick_rate_changed();
                    }
                    self.common.clock.stop_step();
                }
                self.common.clock.set_frozen(freeze);
                self.feedback(t(
                    if freeze {
                        "commands.tick.status.frozen"
                    } else {
                        "commands.tick.status.running"
                    },
                    &[],
                ));
                Ok(i32::from(freeze))
            }
            "tick.step" => {
                if !self.common.clock.start_step(time as u32) {
                    return Err(err("commands.tick.step.fail", &[]));
                }
                self.feedback(t("commands.tick.step.success", &[lit(time.to_string())]));
                Ok(1)
            }
            _ => {
                if !self.common.clock.stop_step() {
                    return Err(err("commands.tick.step.stop.fail", &[]));
                }
                self.feedback(t("commands.tick.step.stop.success", &[]));
                Ok(1)
            }
        }
    }
    /// The report `/tick sprint` prints when a sprint ends. Each caller then
    /// runs `on_tick_rate_changed`, as `ServerTickRateManager.finishTickSprint` does.
    pub(in crate::game::server_side) fn sprint_report(&mut self, report: &SprintReport) {
        self.feedback(Text::translate(
            "commands.tick.sprint.report",
            &[
                Text::literal(report.ticks_per_second.to_string()),
                Text::literal(report.milliseconds_per_tick.clone()),
            ],
        ));
    }
}
