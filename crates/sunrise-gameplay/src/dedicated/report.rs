//! The dedicated server's performance report, the part that holds no `Game`: the console words, the recording's clock and the report's own overhead.
use crate::tick_profile;
use std::time::{Duration, Instant};
use sunrise_level::chunk_streamer::job_stats;
use sunrise_level::spans;

/// The longest window a report records.
pub const MAX_SECONDS: u32 = 600;

/// What `sunrise-report` was asked.
#[derive(Debug, PartialEq)]
pub enum Command {
    Record {
        seconds: u32,
        samples: bool,
        timeline: bool,
    },
    Stop,
    Status,
}

/// The longest `sunrise-trace` records when no seconds are given.
const TRACE_DEFAULT_SECONDS: u32 = 30;

/// The words after `sunrise-trace`: `[SECONDS | stop]`, a timeline-only capture.
#[allow(clippy::result_unit_err)]
pub fn parse_trace(words: &[&str]) -> Result<Command, ()> {
    match words {
        [] => Ok(Command::Record {
            seconds: TRACE_DEFAULT_SECONDS,
            samples: false,
            timeline: true,
        }),
        ["stop"] => Ok(Command::Stop),
        [seconds] => match seconds.parse::<u32>() {
            Ok(seconds) if (1..=MAX_SECONDS).contains(&seconds) => Ok(Command::Record {
                seconds,
                samples: false,
                timeline: true,
            }),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// The words after `sunrise-report`: `[SECONDS | stop | status] [--no-samples] [--timeline] [--profile]`, flags anywhere.
#[allow(clippy::result_unit_err)]
pub fn parse(words: &[&str]) -> Result<Command, ()> {
    let mut samples = true;
    let mut timeline = false;
    let mut positional = Vec::new();
    for &word in words {
        match word {
            "--no-samples" => samples = false,
            "--timeline" => timeline = true,
            // Samples are on by default now; the old switch is accepted and does nothing.
            "--profile" => {}
            flag if flag.starts_with("--") => return Err(()),
            word => positional.push(word),
        }
    }
    match positional[..] {
        [] => Ok(Command::Record {
            seconds: 60,
            samples,
            timeline,
        }),
        ["stop"] => Ok(Command::Stop),
        ["status"] => Ok(Command::Status),
        [seconds] => match seconds.parse::<u32>() {
            Ok(seconds) if (1..=MAX_SECONDS).contains(&seconds) => Ok(Command::Record {
                seconds,
                samples,
                timeline,
            }),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// Registry ids for the loose entities a level counts apart from its mobs: items, orbs, projectiles, minecarts, boats.
pub const LOOSE_ENTITIES: [&str; 5] = [
    "minecraft:item",
    "minecraft:experience_orb",
    "sunrise:projectile",
    "minecraft:minecart",
    "minecraft:boat",
];

/// A report being recorded: what the game side needs while the capture runs.
pub struct Recording {
    pub seconds: u32,
    /// Whether it turned the spans of every thread on, which it turns off again.
    pub all_threads: bool,
    pub started: Instant,
    pub ticks: u64,
    /// The ticks whose mobs were timed one by one ([`tick_profile::SAMPLE_EVERY`]).
    pub sampled_ticks: u64,
}

impl Recording {
    pub fn start(seconds: u32) -> Self {
        tick_profile::want_spans(tick_profile::SPANS_REPORT, true);
        Self::new(seconds, true)
    }
    /// A recording beside a window: the calling thread's spans are on already (`spans::record_here`), the other threads' stay off.
    pub fn here(seconds: u32) -> Self {
        Self::new(seconds, false)
    }
    fn new(seconds: u32, all_threads: bool) -> Self {
        job_stats::enable(true);
        job_stats::take();
        tick_profile::take();
        Self {
            seconds,
            all_threads,
            started: Instant::now(),
            ticks: 0,
            sampled_ticks: 0,
        }
    }
    /// Whether the next tick's mobs are timed one by one.
    pub fn sample_next(&self) -> bool {
        self.ticks.is_multiple_of(tick_profile::SAMPLE_EVERY as u64)
    }
    pub fn add(&mut self, sampled: bool) {
        self.ticks += 1;
        self.sampled_ticks += u64::from(sampled);
    }
    pub fn done(&self) -> bool {
        self.started.elapsed() >= Duration::from_secs(u64::from(self.seconds))
    }
    /// `sunrise-report status`: how far the recording is.
    pub fn status(&self) -> String {
        format!(
            "Recording a performance report: {} of {} s, {} ticks (sunrise-report stop writes it now)",
            self.started.elapsed().as_secs(),
            self.seconds,
            self.ticks
        )
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        if self.all_threads {
            tick_profile::want_spans(tick_profile::SPANS_REPORT, false);
        }
        job_stats::enable(false);
    }
}

/// What one span costs on this thread with spans on: the report's own overhead. The spans it
/// records are thrown away.
pub fn span_cost_nanos() -> f64 {
    const N: u32 = 5_000;
    let kept = spans::take_raw();
    let timer = Instant::now();
    for _ in 0..N {
        drop(spans::guard("calibration"));
    }
    let per = timer.elapsed().as_nanos() as f64 / f64::from(N);
    spans::take_raw();
    debug_assert!(kept.is_empty(), "the tick takes its spans");
    per
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_words_pick_the_window_and_the_samples() {
        let record = |seconds, samples| {
            Ok(Command::Record {
                seconds,
                samples,
                timeline: false,
            })
        };
        assert_eq!(parse(&[]), record(60, true));
        assert_eq!(parse(&["120"]), record(120, true));
        assert_eq!(parse(&["--no-samples", "30"]), record(30, false));
        assert_eq!(parse(&["30", "--no-samples"]), record(30, false));
        assert_eq!(parse(&["--profile"]), record(60, true));
        assert_eq!(
            parse(&["--timeline", "10"]),
            Ok(Command::Record {
                seconds: 10,
                samples: true,
                timeline: true
            })
        );
        assert_eq!(parse(&["stop"]), Ok(Command::Stop));
        assert_eq!(parse(&["status", "--no-samples"]), Ok(Command::Status));
        for bad in [
            &["0"][..],
            &["601"],
            &["abc"],
            &["--fast"],
            &["30", "60"],
            &["stop", "5"],
        ] {
            assert_eq!(parse(bad), Err(()), "{bad:?}");
        }
    }

    #[test]
    fn trace_words_pick_a_timeline_only_window() {
        let trace = |seconds| {
            Ok(Command::Record {
                seconds,
                samples: false,
                timeline: true,
            })
        };
        assert_eq!(parse_trace(&[]), trace(30));
        assert_eq!(parse_trace(&["45"]), trace(45));
        assert_eq!(parse_trace(&["600"]), trace(600));
        assert_eq!(parse_trace(&["stop"]), Ok(Command::Stop));
        for bad in [&["0"][..], &["601"], &["x"], &["10", "20"], &["stop", "1"]] {
            assert_eq!(parse_trace(bad), Err(()), "{bad:?}");
        }
    }

    /// One tick in four has its mobs timed one by one, starting with the first.
    #[test]
    fn one_tick_in_four_is_sampled() {
        let mut r = Recording {
            seconds: 1,
            all_threads: false,
            started: Instant::now(),
            ticks: 0,
            sampled_ticks: 0,
        };
        for _ in 0..20 {
            let sampled = r.sample_next();
            r.add(sampled);
        }
        assert_eq!((r.ticks, r.sampled_ticks), (20, 5));
    }

    /// A recording that ends without `capture_end` (the capture failed to start, or shutdown) leaves spans off.
    #[test]
    fn a_dropped_recording_turns_spans_off() {
        let _spans = tick_profile::spans_test_lock();
        drop(Recording::start(1));
        assert!(!tick_profile::wanted(tick_profile::SPANS_REPORT));
    }

    /// On this machine, timing a span and a mob each cost well under a millisecond.
    #[test]
    fn the_prices_of_a_span_and_a_mob_sample_are_small() {
        let (span_ns, mob_ns) = (span_cost_nanos(), tick_profile::sample_cost_nanos());
        assert!(span_ns > 0.0 && span_ns < 50_000.0, "{span_ns}");
        assert!(mob_ns > 0.0 && mob_ns < 50_000.0, "{mob_ns}");
    }
}
