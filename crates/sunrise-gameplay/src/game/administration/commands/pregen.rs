//! `/pregen`, Sunrise's own command: generate, light and save every chunk of a
//! square or a circle ahead of the players (docs/pregeneration.md). The task
//! lives on the server and hands its band to the level's streaming, whose
//! workers do the work; chunks behind the band unload and are saved as any
//! chunk the players leave.
use super::{CommandError, Dim};
use std::time::{Duration, Instant};
use sunrise_game_data::text::Text;
use sunrise_protocol::packets::play::{self, Clientbound as C};
use sunrise_sim::{lan::data::component_nbt, pregen::Pregen};

/// How often the progress is reported.
pub(in crate::game) const REPORT_EVERY: Duration = Duration::from_secs(5);
/// How long the band may stay put before the task looks for what holds it.
pub(in crate::game) const STALL: Duration = Duration::from_secs(30);

pub(in crate::game) struct PregenTask {
    pub(in crate::game) pregen: Pregen,
    pub(in crate::game) dim: Dim,
    pub(in crate::game) paused: bool,
    /// Time spent running, up to the last pause, and when it last resumed.
    pub(in crate::game) ran: Duration,
    pub(in crate::game) resumed: Instant,
    pub(in crate::game) reported: Instant,
    /// When the band last moved, to notice a task that stopped advancing.
    pub(in crate::game) moved: Instant,
    /// The player who started it, told the progress too (a guest's connection).
    pub(in crate::game) owner: Option<u32>,
}

impl PregenTask {
    pub(in crate::game::server_side) fn running_for(&self) -> Duration {
        if self.paused {
            self.ran
        } else {
            self.ran + self.resumed.elapsed()
        }
    }
    pub(in crate::game) fn progress(&self) -> Text {
        let p = &self.pregen;
        let seconds = self.running_for().as_secs_f64();
        let rate = p.done as f64 / seconds.max(1e-3);
        let left = p.total - p.done;
        let eta = if rate > 0.0 {
            duration(left as f64 / rate)
        } else {
            "unknown".into()
        };
        Text::literal(format!(
            "Pregenerating {}: {} of {} chunks ({:.1}%), {rate:.1} chunks/s, {} left",
            self.dim.id(),
            p.done,
            p.total,
            100.0 * p.done as f64 / p.total.max(1) as f64,
            if p.done == p.total {
                "nothing".into()
            } else {
                eta
            },
        ))
    }
}

pub(in crate::game) fn failure(message: &str) -> CommandError {
    CommandError::new(Text::literal(message))
}

/// `1h 02m 03s`.
pub(in crate::game) fn duration(seconds: f64) -> String {
    let s = seconds.round() as u64;
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s:02}s"),
        (h, m, s) => format!("{h}h {m:02}m {s:02}s"),
    }
}

impl crate::game::ServerGame<'_> {
    /// The console and the player who started the task get the progress.
    pub(in crate::game::server_side) fn pregen_report(
        &mut self,
        owner: Option<u32>,
        message: Text,
    ) {
        if self.common.is_dedicated_server() {
            super::super::dedicated::log("INFO", &message.plain());
        } else {
            log::info!("{}", message.plain());
        }
        if let Some(host) = &self.server.lan {
            if let Some(g) = owner.and_then(|c| host.hub.guest(c)) {
                g.send(&C::SystemChat(play::SystemChat {
                    content: component_nbt(&message.component()),
                    overlay: false,
                }));
            }
        } else if !self.common.is_dedicated_server() {
            self.server_system_message(message);
        }
    }
}
