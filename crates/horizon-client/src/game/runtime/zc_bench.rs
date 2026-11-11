//! REQ-R1-2 measurement only: per-tick and per-frame wall times, perf on/off around the measured ticks, point call counts.
use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

struct Ticks {
    warm: u64,
    count: u64,
    seen: u64,
    out: String,
    durations: Vec<u64>,
    counts_at_start: Vec<u64>,
    started: Option<Instant>,
}

static TICKS: Mutex<Option<Option<Ticks>>> = Mutex::new(None);

fn perf(command: &str) {
    let (Ok(ctl), Ok(ack)) = (std::env::var("HZ_PERF_CTL"), std::env::var("HZ_PERF_ACK")) else {
        return;
    };
    let mut fifo = std::fs::OpenOptions::new().write(true).open(ctl).expect("perf ctl fifo");
    fifo.write_all(format!("{command}\n").as_bytes()).unwrap();
    drop(fifo);
    let mut answer = [0u8; 5];
    let mut fifo = std::fs::File::open(ack).expect("perf ack fifo");
    let _ = fifo.read(&mut answer);
}

fn counts() -> Vec<u64> {
    ::mod_api::__private::points::COUNTS.iter().map(|c| c.load(Ordering::Relaxed)).collect()
}

/// `HZ_TICKLOG=OUT:WARM:COUNT`: records COUNT tick times after WARM ticks, writes OUT and exits.
pub(crate) fn tick(took: Duration) {
    let mut guard = TICKS.lock().unwrap();
    let state = guard.get_or_insert_with(|| {
        let spec = std::env::var("HZ_TICKLOG").ok()?;
        let mut parts = spec.rsplitn(3, ':');
        let count = parts.next()?.parse().ok()?;
        let warm = parts.next()?.parse().ok()?;
        let out = parts.next()?.to_string();
        Some(Ticks { warm, count, seen: 0, out, durations: Vec::new(), counts_at_start: Vec::new(), started: None })
    });
    let Some(t) = state.as_mut() else { return };
    t.seen += 1;
    if t.seen == t.warm {
        t.counts_at_start = counts();
        perf("enable");
        t.started = Some(Instant::now());
        return;
    }
    if t.seen <= t.warm {
        return;
    }
    t.durations.push(took.as_nanos() as u64);
    if t.durations.len() as u64 == t.count {
        perf("disable");
        let wall = t.started.unwrap().elapsed();
        let end = counts();
        let mut text = format!("# wall_ns {}\n", wall.as_nanos());
        for d in &t.durations {
            text.push_str(&format!("{d}\n"));
        }
        std::fs::write(&t.out, text).unwrap();
        let mut calls = String::new();
        for (i, (a, b)) in t.counts_at_start.iter().zip(&end).enumerate() {
            if b > a {
                calls.push_str(&format!("{i} {}\n", b - a));
            }
        }
        std::fs::write(format!("{}.calls", t.out), calls).unwrap();
        std::process::exit(0);
    }
}

static FRAMES: Mutex<Option<Option<(String, u64, Vec<Instant>)>>> = Mutex::new(None);

/// `HZ_FRAMELOG=OUT:COUNT`: the instants of the first COUNT world frames, written to OUT as intervals.
pub(crate) fn frame() {
    let mut guard = FRAMES.lock().unwrap();
    let state = guard.get_or_insert_with(|| {
        let spec = std::env::var("HZ_FRAMELOG").ok()?;
        let (out, count) = spec.rsplit_once(':')?;
        Some((out.to_string(), count.parse().ok()?, Vec::new()))
    });
    let Some((out, count, at)) = state.as_mut() else { return };
    at.push(Instant::now());
    if at.len() as u64 == *count {
        let text: String = at.windows(2).map(|w| format!("{}\n", (w[1] - w[0]).as_nanos())).collect();
        std::fs::write(out, text).unwrap();
    }
}
