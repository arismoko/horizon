//! `sunrise-ctl profile`: the words the game receives, as typed requests, and the typed replies.
pub use sunrise_sim::perf_types::{
    Failed, GpuSwitch, HZ_RANGE, MARK_NAME_MAX, Marked, Opened, Parsed, ProfileSide, Request,
    Shared, StartOptions, Started, Status, Stopped, Threads, Traced, parse,
};

/// The viewer page a registered captures hook offers.
pub fn viewer() -> Option<String> {
    sunrise_hooks::engine::captures().map(|c| c.viewer().to_owned())
}
