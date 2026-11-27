use std::path::Path;
use sunrise_telemetry::{Item, Payload, Source, outbox};

/// The release (`v0.2.3`), or the client's crate version for a build from source.
pub fn version() -> String {
    crate::crash::release_tag().unwrap_or_else(|| format!("dev {}", env!("CARGO_PKG_VERSION")))
}

/// Keeps an opted-in benchmark report in the launcher's outbox.
pub fn keep_benchmark(home: &Path, report: &sunrise_bench::Report) {
    let Ok(data) = serde_json::to_value(report.for_upload()) else {
        return;
    };
    if let Err(e) = outbox::push(
        home,
        &Item::new(Source::Game, version(), Payload::Benchmark(data)),
    ) {
        log::warn!("Sunrise: could not keep the benchmark report to send: {e}");
    }
}
