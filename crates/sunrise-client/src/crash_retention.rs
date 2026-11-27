use std::path::Path;
use sunrise_telemetry::{
    Item, Payload, Source, outbox,
    payload::{Crash, System},
    scrub::Scrubber,
};

/// The details of a crash report that say what the game was doing, and only those: not the
/// world, the server or the resource packs.
const CRASH_DETAILS: [&str; 3] = ["Sunrise Version", "Graphics Adapter", "Render Settings"];

/// What a crash report becomes when it may leave: the error and stack, the thread, and the
/// details in [`CRASH_DETAILS`], with the world's name and folder and the server's address
/// scrubbed wherever they appear. No log lines.
pub(super) fn crash_item(
    report: &crate::crash::Report,
    system: impl FnOnce() -> System,
    version: impl FnOnce() -> String,
) -> Item {
    let mut scrubber = Scrubber::new();
    for (key, value) in &report.details {
        match key.as_str() {
            // `Name (saves/folder)`, as game.rs sets it.
            "World" => {
                let (name, folder) = value.rsplit_once(" (saves/").unwrap_or((value, ""));
                scrubber = scrubber.term(name).term(folder.trim_end_matches(')'));
            }
            "Server" => {
                let host = value.rsplit_once(':').map_or(value.as_str(), |(h, _)| h);
                scrubber = scrubber.term(value.as_str()).term(host);
            }
            _ => {}
        }
    }
    let crash = Crash {
        system: system(),
        error: scrubber.scrub(&report.error),
        stack: scrubber.scrub(&report.stack),
        thread: report.thread.clone(),
        details: report
            .details
            .iter()
            .filter(|(k, _)| CRASH_DETAILS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), scrubber.scrub(v)))
            .collect(),
    };
    let mut item = Item::new(Source::Game, version(), Payload::Crash(crash));
    // A huge stack keeps its top, where the error is.
    while item.validate().is_err() {
        let Payload::Crash(crash) = &mut item.payload else {
            break;
        };
        let keep = crash.stack.lines().count() * 3 / 4;
        if keep == 0 {
            crash.stack.clear();
            break;
        }
        crash.stack = crash
            .stack
            .lines()
            .take(keep)
            .map(|l| format!("{l}\n"))
            .collect();
    }
    item
}

/// Keeps the crash for the launcher to send, if the player allows crash reports.
pub(super) fn keep_crash(
    home: &Path,
    report: &crate::crash::Report,
    system: impl FnOnce() -> System,
    version: impl FnOnce() -> String,
) {
    if let Err(e) = outbox::push(home, &crash_item(report, system, version)) {
        log::warn!("Sunrise: could not keep the crash report to send: {e}");
    }
}
