//! `IntegratedServer.reportChunkLoadFailure` / `reportChunkSaveFailure` /
//! `sendLowDiskSpaceWarning`: the region storage runs on the server thread and
//! its writer thread, and the toasts are the window's. Each failure is logged
//! and checked for low disk space (`MinecraftServer.warnOnLowDiskSpace`). The
//! window drains its queued notifications every frame (`Minecraft.execute`).
//! Save failures also leave server diagnostics without stopping the save.
//! `SystemToast.addOrUpdate` makes a repeat reset the toast it already shows,
//! so only the newest chunk of each kind is kept.
use crate::save;
use chrono::{DateTime, Local};
use std::{
    collections::VecDeque,
    error::Error,
    fmt::Write as _,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, Mutex, OnceLock, PoisonError, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

/// What the receiver has yet to display.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Pending {
    /// `SystemToast.onLowDiskSpace`.
    pub low_disk_space: bool,
    /// `SystemToast.onChunkLoadFailure`: the newest chunk.
    pub load_failure: Option<(i32, i32)>,
    /// `SystemToast.onChunkSaveFailure`: the newest chunk.
    pub save_failure: Option<(i32, i32)>,
    /// Mod IDs whose delayed callbacks panicked, in report order.
    pub scheduled_task_failures: Vec<&'static str>,
}

/// The receiver's inbox, shared with the integrated server and its region workers.
#[derive(Default)]
pub struct Reports(Mutex<Pending>);

/// Dedicated-server I/O reports; a client window has its own inbox.
pub static PROCESS_REPORTS: LazyLock<Arc<Reports>> = LazyLock::new(Arc::default);

impl Pending {
    /// `reportChunkLoadFailure` after `warnOnLowDiskSpace` found `low_disk_space`.
    fn load_failed(&mut self, x: i32, z: i32, low_disk_space: bool) {
        self.low_disk_space |= low_disk_space;
        self.load_failure = Some((x, z));
    }

    /// `reportChunkSaveFailure` after `warnOnLowDiskSpace` found `low_disk_space`.
    fn save_failed(&mut self, x: i32, z: i32, low_disk_space: bool) {
        self.low_disk_space |= low_disk_space;
        self.save_failure = Some((x, z));
    }
}

/// `MinecraftServer.warnOnLowDiskSpace` for the world folder `dir`.
fn warn_on_low_disk_space(dir: &Path, space: impl FnOnce(&Path) -> u64) -> bool {
    let low = space(dir) < save::LOW_DISK_SPACE;
    if low {
        // `MinecraftServer.sendLowDiskSpaceWarning`.
        log::warn!("Low disk space! Might not be able to save the world.");
    }
    low
}

const SAVE_LOCATION: &str = "chunk/save";
const LATEST_ENTRY_COUNT: usize = 8;
/// `Util.shutdownExecutors` gives the IO pool three seconds to finish its work.
const REPORT_DRAIN_TIMEOUT: Duration = Duration::from_secs(3);

/// A server's suppressed chunk save failures, shared by all its dimensions.
#[derive(Default)]
pub struct SaveFailureReporter {
    integrated: bool,
    suppressed: Arc<Mutex<SuppressedSaveFailures>>,
    target: Option<ReportTarget>,
}

type ReportJob = Box<dyn FnOnce() + Send>;

struct ReportTarget {
    debug_dir: PathBuf,
    level_id: String,
    worker: OnceLock<io::Result<ReportWorker>>,
}

struct ReportWorker {
    send: mpsc::Sender<ReportJob>,
    done: Mutex<mpsc::Receiver<()>>,
    abort: Arc<AtomicBool>,
}

impl ReportTarget {
    fn enqueue(&self, job: ReportJob) -> io::Result<()> {
        let worker = self.worker.get_or_init(|| {
            let (send, recv) = mpsc::channel::<ReportJob>();
            let (running, done) = mpsc::channel();
            let abort = Arc::new(AtomicBool::new(false));
            let aborted = Arc::clone(&abort);
            drop(
                thread::Builder::new()
                    .name("Chunk IO reports".into())
                    .spawn(move || {
                        // Disconnect the completion channel when the worker exits, even on panic.
                        let _running = running;
                        for job in recv {
                            if aborted.load(Ordering::Acquire) {
                                break;
                            }
                            job();
                        }
                    })?,
            );
            Ok(ReportWorker {
                send,
                done: Mutex::new(done),
                abort,
            })
        });
        match worker {
            Ok(worker) => worker
                .send
                .send(job)
                .map_err(|_| io::Error::other("chunk IO report worker exited")),
            Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }

    /// Drain at server teardown, then abandon queued reports at the deadline. An in-flight
    /// filesystem call cannot be interrupted here, but it cannot hold teardown open.
    fn drain(&mut self, timeout: Duration) {
        if let Some(Ok(ReportWorker { send, done, abort })) = self.worker.take() {
            drop(send);
            let done = done.into_inner().unwrap_or_else(PoisonError::into_inner);
            if done.recv_timeout(timeout) == Err(mpsc::RecvTimeoutError::Timeout) {
                abort.store(true, Ordering::Release);
                log::warn!("Timed out draining chunk IO reports after {timeout:?}");
            }
        }
    }
}

impl Drop for ReportTarget {
    fn drop(&mut self) {
        self.drain(REPORT_DRAIN_TIMEOUT);
    }
}

/// The launch's current server report source. The dedicated summary survives
/// server teardown for the fatal report, but the report worker does not.
#[derive(Clone, Default)]
pub struct SaveFailureDiagnostics(Arc<Mutex<Option<ReportSource>>>);

enum ReportSource {
    Integrated(Weak<Mutex<SuppressedSaveFailures>>),
    Dedicated(Arc<Mutex<SuppressedSaveFailures>>),
}

impl SaveFailureDiagnostics {
    pub fn register(&self, reporter: &SaveFailureReporter) {
        let source = if reporter.integrated {
            ReportSource::Integrated(Arc::downgrade(&reporter.suppressed))
        } else {
            ReportSource::Dedicated(Arc::clone(&reporter.suppressed))
        };
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(source);
    }

    /// A fatal report must not wait for locks held by a crashing server.
    pub fn suppressed_summary(&self) -> Option<String> {
        let source = match self.0.try_lock().ok()?.as_ref()? {
            ReportSource::Integrated(suppressed) => suppressed.upgrade()?,
            ReportSource::Dedicated(suppressed) => Arc::clone(suppressed),
        };
        Some(source.try_lock().ok()?.summary())
    }
}

#[derive(Default)]
struct SuppressedSaveFailures {
    latest: VecDeque<(i64, &'static str, String)>,
    // Small, most recently seen class first, as in SuppressedExceptionCollector.
    counts: Vec<(&'static str, u64)>,
}

struct SaveFailure<'a> {
    world_root: &'a Path,
    dimension: &'static str,
    storage: &'static str,
    pos: (i32, i32),
    error: &'a (dyn Error + 'static),
}

impl SaveFailureReporter {
    /// Write reports under the server's root, not the world's storage root.
    /// An integrated server also reports the failure to its local client.
    pub fn new(integrated: bool, server_root: &Path, level_id: &str) -> Self {
        Self {
            integrated,
            target: Some(ReportTarget {
                debug_dir: server_root.join("debug"),
                level_id: level_id.to_owned(),
                worker: OnceLock::new(),
            }),
            ..Self::default()
        }
    }

    /// `SuppressedExceptionCollector.addEntry` for `chunk/save`.
    fn record(&self, timestamp: i64, class: &'static str, message: String) {
        let mut suppressed = self
            .suppressed
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if suppressed.latest.len() == LATEST_ENTRY_COUNT {
            suppressed.latest.pop_front();
        }
        suppressed.latest.push_back((timestamp, class, message));
        let count = suppressed
            .counts
            .iter()
            .position(|&(key, _)| key == class)
            .map_or(0, |index| suppressed.counts.remove(index).1);
        suppressed.counts.insert(0, (class, count + 1));
    }

    /// Total failures reported by this server, including entries no longer in the summary.
    pub fn reported_failures(&self) -> u64 {
        self.suppressed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .counts
            .iter()
            .map(|&(_, count)| count)
            .sum()
    }

    /// `SuppressedExceptionCollector.dump`: the server crash report's `Suppressed Exceptions` detail.
    pub fn suppressed_summary(&self) -> String {
        self.suppressed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .summary()
    }

    fn report(
        &self,
        failure: SaveFailure<'_>,
        pending: &Mutex<Pending>,
        space: impl Fn(&Path) -> io::Result<u64> + Clone + Send + 'static,
    ) {
        let (x, z) = failure.pos;
        let message = failure.error.to_string();
        if failure.storage == "entities" {
            // `EntityStorage.reportSaveFailureIfPresent` logs before notifying the server.
            log::error!("Failed to store entity chunk [{x}, {z}]: {message}");
        }
        log::error!("Failed to save chunk {x},{z}: {message}");
        let time = Local::now();
        // The write path returns io::Error for record and filesystem failures;
        // opaque internal errors have no concrete Rust name available here.
        let class = if failure.error.is::<io::Error>() {
            "std::io::Error"
        } else {
            "Error"
        };
        self.record(time.timestamp_millis(), class, message.clone());
        if let Some(target) = &self.target {
            let debug = target.debug_dir.clone();
            let level = target.level_id.clone();
            let mut causes = Vec::new();
            let mut source = failure.error.source();
            while let Some(cause) = source {
                causes.push(cause.to_string());
                source = cause.source();
            }
            let report = ChunkIoReport {
                time,
                level,
                dimension: failure.dimension,
                storage: failure.storage,
                pos: failure.pos,
                message,
                causes,
            };
            let report_space = space.clone();
            if let Err(error) = target.enqueue(Box::new(move || {
                if let Err(error) = report.write_to(&debug, report_space) {
                    log::warn!(
                        "Failed to store chunk IO exception at {}: {error}",
                        debug.display()
                    );
                }
            })) {
                log::warn!("Failed to queue chunk IO report: {error}");
            }
        }
        let estimate = |path: &Path| space(path).unwrap_or(u64::MAX);
        let low = warn_on_low_disk_space(failure.world_root, estimate);
        // IntegratedServer.reportChunkSaveFailure checks again before showing the toast.
        let low_again = self.integrated && warn_on_low_disk_space(failure.world_root, estimate);
        let mut queued = pending.lock().unwrap_or_else(PoisonError::into_inner);
        if self.integrated {
            queued.save_failed(x, z, low || low_again);
        } else {
            // The dedicated server drains this flag to warn its operators.
            queued.low_disk_space |= low;
        }
    }
}

impl SuppressedSaveFailures {
    fn summary(&self) -> String {
        if self.latest.is_empty() {
            return "~~NONE~~".to_owned();
        }
        let now = Local::now().timestamp_millis();
        let mut out = String::from("\n\t\tLatest entries:\n");
        for (timestamp, class, message) in &self.latest {
            let _ = writeln!(
                out,
                "\t\t\t{SAVE_LOCATION}:{class}: {message} ({}ms ago)",
                now - timestamp
            );
        }
        out.push_str("\t\tEntry counts:\n");
        for (class, count) in &self.counts {
            let _ = writeln!(out, "\t\t\t{SAVE_LOCATION}:{class} x {count}");
        }
        out
    }
}

/// `SharedConstants.ILLEGAL_FILE_CHARACTERS`, in its order.
const ILLEGAL_FILE_CHARACTERS: [char; 15] = [
    '/', '\n', '\r', '\t', '\0', '\u{c}', '`', '?', '*', '\\', '<', '>', '|', '"', ':',
];

/// `FileUtil.sanitizeName`.
pub fn sanitize_world_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if ILLEGAL_FILE_CHARACTERS.contains(&c) || matches!(c, '.' | '/' | '"') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// An owned snapshot for `MinecraftServer.storeChunkIoError`, independent of the region writer.
struct ChunkIoReport {
    time: DateTime<Local>,
    level: String,
    dimension: &'static str,
    storage: &'static str,
    pos: (i32, i32),
    message: String,
    causes: Vec<String>,
}

impl ChunkIoReport {
    fn write_to(
        &self,
        debug: &Path,
        space: impl FnOnce(&Path) -> io::Result<u64>,
    ) -> io::Result<()> {
        fs::create_dir_all(debug)?;
        if space(debug)? < 8192 {
            log::warn!(
                "Not storing chunk IO report due to low space at {}",
                debug.display()
            );
            return Ok(());
        }
        let safe_level = sanitize_world_name(&self.level);
        let date = Local::now().format("%Y-%m-%d_%H.%M.%S");
        let path = debug.join(format!("chunk-{safe_level}-{date}-server.txt"));
        let (x, z) = self.pos;
        let mut text = format!(
            "---- Minecraft Chunk IO Error Report ----\n\nTime: {}\nDescription: Chunk save failure\n\n{}\n",
            self.time.format("%Y-%m-%d %H:%M:%S"),
            self.message,
        );
        for cause in &self.causes {
            let _ = writeln!(text, "Caused by: {cause}");
        }
        let _ = writeln!(
            text,
            "\n-- Chunk Info --\nDetails:\n\tLevel: {}\n\tDimension: {}\n\tStorage: {}\n\tPosition: [{x}, {z}]",
            self.level, self.dimension, self.storage,
        );
        fs::write(&path, text)?;
        log::info!("Saved chunk IO details to {}", path.display());
        Ok(())
    }
}

impl Reports {
    fn pending(&self) -> std::sync::MutexGuard<'_, Pending> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// `reportChunkLoadFailure` for the chunk at (`x`, `z`) of the dimension in `dir`.
    pub fn report_load_failure(&self, dir: &Path, x: i32, z: i32, error: &dyn std::fmt::Display) {
        log::error!("Failed to load chunk {x},{z}: {error}");
        let low = warn_on_low_disk_space(dir, save::estimate_disk_space);
        self.pending().load_failed(x, z, low);
    }

    /// `MinecraftServer.reportChunkSaveFailure`: record a failure without retrying or aborting the save.
    pub fn report_save_failure(
        &self,
        reporter: &SaveFailureReporter,
        world_root: &Path,
        dimension: &'static str,
        storage: &'static str,
        pos: (i32, i32),
        error: &(dyn Error + 'static),
    ) {
        reporter.report(
            SaveFailure {
                world_root,
                dimension,
                storage,
                pos,
                error,
            },
            &self.0,
            |path| fs4::available_space(path),
        );
    }

    /// `Minecraft.sendLowDiskSpaceWarning`: a server's `ClientboundLowDiskSpaceWarningPacket`
    /// (`ClientPacketListener.handleLowDiskSpaceWarning`) toasts like the local report.
    pub fn report_low_disk_space(&self) {
        self.pending().low_disk_space = true;
    }

    /// Report a mod's failed callback to the integrated server owner's client.
    pub fn report_scheduled_task_failure(&self, mod_id: &'static str) {
        self.pending().scheduled_task_failures.push(mod_id);
    }

    /// The reports since the last call.
    pub fn take(&self) -> Pending {
        std::mem::take(&mut *self.pending())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_keep_the_newest_chunk_of_each_kind() {
        let mut pending = Pending::default();
        pending.load_failed(1, 2, false);
        pending.load_failed(3, 4, false);
        pending.save_failed(5, 6, false);
        assert_eq!(
            pending,
            Pending {
                low_disk_space: false,
                load_failure: Some((3, 4)),
                save_failure: Some((5, 6)),
                ..Pending::default()
            }
        );
        // A later failure on a full disk raises the warning, and one with
        // space left does not lower it.
        pending.save_failed(7, 8, true);
        pending.load_failed(9, 10, false);
        assert_eq!(
            pending,
            Pending {
                low_disk_space: true,
                load_failure: Some((9, 10)),
                save_failure: Some((7, 8)),
                ..Pending::default()
            }
        );
    }

    #[test]
    fn a_servers_warning_packet_toasts_low_disk_space() {
        let reports = Reports::default();
        reports.report_low_disk_space();
        assert!(reports.take().low_disk_space);
    }

    // The capture logger is thread-local. Read the worker's records after its queued write.
    fn report_worker_records(reporter: &SaveFailureReporter) -> Vec<(log::Level, String)> {
        let (send, recv) = mpsc::channel();
        reporter
            .target
            .as_ref()
            .unwrap()
            .enqueue(Box::new(move || {
                send.send(crate::test_log::records()).unwrap()
            }))
            .unwrap();
        recv.recv().unwrap()
    }

    #[test]
    fn integrated_save_failure_logs_records_reports_and_toasts() {
        crate::test_log::install();
        crate::test_log::clear();
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path().join("game");
        let world = game_dir.join("saves").join("world-on-disk");
        let debug = game_dir.join("debug");
        let world_path = world.clone();
        let debug_path = debug.clone();
        let caller = thread::current().id();
        let pending = Mutex::new(Pending::default());
        let reporter = SaveFailureReporter::new(true, &game_dir, "My.World");
        reporter.report(
            SaveFailure {
                world_root: &world,
                dimension: "minecraft:the_nether",
                storage: "poi",
                pos: (-2, 5),
                error: &io::Error::other("refused POI encoding"),
            },
            &pending,
            move |path| {
                if path == world_path {
                    assert_eq!(thread::current().id(), caller);
                    Ok(save::LOW_DISK_SPACE - 1)
                } else {
                    assert_eq!(path, debug_path);
                    assert_ne!(thread::current().id(), caller);
                    Ok(8192)
                }
            },
        );

        let records = crate::test_log::records();
        assert!(
            records
                .iter()
                .any(|(level, text)| *level == log::Level::Error
                    && text.contains("-2,5")
                    && text.contains("refused POI encoding"))
        );
        assert_eq!(
            records
                .iter()
                .filter(|(level, _)| *level == log::Level::Warn)
                .count(),
            2
        );
        assert_eq!(
            pending.into_inner().unwrap(),
            Pending {
                low_disk_space: true,
                save_failure: Some((-2, 5)),
                load_failure: None,
                ..Pending::default()
            }
        );
        assert_eq!(reporter.reported_failures(), 1);
        assert!(
            reporter
                .suppressed_summary()
                .contains("chunk/save:std::io::Error: refused POI encoding")
        );

        drop(reporter); // The server's worker drains its queue before exiting.
        assert!(!world.join("debug").exists());
        let mut files = fs::read_dir(debug).unwrap();
        let path = files.next().unwrap().unwrap().path();
        assert!(files.next().is_none());
        let name = path.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("chunk-My_World-") && name.ends_with("-server.txt"));
        let text = fs::read_to_string(path).unwrap();
        for expected in [
            "Minecraft Chunk IO Error Report",
            "Description: Chunk save failure",
            "refused POI encoding",
            "Level: My.World",
            "Dimension: minecraft:the_nether",
            "Storage: poi",
            "Position: [-2, 5]",
        ] {
            assert!(text.contains(expected), "report is missing {expected}");
        }
    }

    #[test]
    fn dedicated_report_uses_working_directory_not_the_world_universe() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path().join("server");
        let world = tmp.path().join("universe/path/to/world");
        let pending = Mutex::new(Pending::default());
        let reporter = SaveFailureReporter::new(false, &working_dir, "path/to/world");
        reporter.report(
            SaveFailure {
                world_root: &world,
                dimension: "minecraft:overworld",
                storage: "chunk",
                pos: (3, 4),
                error: &io::Error::other("write refused"),
            },
            &pending,
            |_| Ok(save::LOW_DISK_SPACE),
        );
        drop(reporter);
        assert!(!world.join("debug").exists());
        let mut files = fs::read_dir(working_dir.join("debug")).unwrap();
        let entry = files.next().unwrap().unwrap();
        assert!(files.next().is_none());
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        assert!(name.starts_with("chunk-path_to_world-") && name.ends_with("-server.txt"));
        let text = fs::read_to_string(entry.path()).unwrap();
        assert!(text.contains("Level: path/to/world"));
        assert!(text.contains("Position: [3, 4]"));
        assert!(text.contains("write refused"));
        assert_eq!(pending.into_inner().unwrap(), Pending::default());
    }

    #[test]
    fn low_space_on_the_debug_filesystem_refuses_only_the_report_file() {
        crate::test_log::install();
        crate::test_log::clear();
        let server = tempfile::tempdir().unwrap();
        let debug = server.path().join("debug");
        let world = server.path().join("world");
        let debug_path = debug.clone();
        let pending = Mutex::new(Pending::default());
        let reporter = SaveFailureReporter::new(false, server.path(), "world");
        reporter.report(
            SaveFailure {
                world_root: &world,
                dimension: "minecraft:overworld",
                storage: "chunk",
                pos: (3, 4),
                error: &io::Error::other("write refused"),
            },
            &pending,
            move |path| {
                Ok(if path == debug_path {
                    8191
                } else {
                    save::LOW_DISK_SPACE
                })
            },
        );
        let records = report_worker_records(&reporter);
        assert!(fs::read_dir(&debug).unwrap().next().is_none());
        assert_eq!(reporter.reported_failures(), 1);
        assert_eq!(pending.into_inner().unwrap(), Pending::default());
        assert!(
            records
                .iter()
                .any(|(level, text)| *level == log::Level::Warn
                    && text.contains(debug.to_str().unwrap()))
        );
    }

    #[test]
    fn a_report_write_error_does_not_hide_the_chunk_failure() {
        crate::test_log::install();
        crate::test_log::clear();
        let server = tempfile::tempdir().unwrap();
        let world = server.path().join("world");
        let debug = server.path().join("debug");
        // An existing file blocks creation of the report directory on every platform.
        fs::write(&debug, "unrelated file").unwrap();
        let pending = Mutex::new(Pending::default());
        let reporter = SaveFailureReporter::new(true, server.path(), "world");
        reporter.report(
            SaveFailure {
                world_root: &world,
                dimension: "minecraft:overworld",
                storage: "entities",
                pos: (3, 4),
                error: &io::Error::other("entities refused"),
            },
            &pending,
            |_| Ok(save::LOW_DISK_SPACE),
        );
        assert_eq!(reporter.reported_failures(), 1);
        assert_eq!(
            pending.into_inner().unwrap(),
            Pending {
                low_disk_space: false,
                save_failure: Some((3, 4)),
                load_failure: None,
                ..Pending::default()
            }
        );
        let worker_records = report_worker_records(&reporter);
        assert!(
            worker_records
                .iter()
                .any(|(level, text)| *level == log::Level::Warn
                    && text.contains(debug.to_str().unwrap()))
        );
        let records = crate::test_log::records();
        for pos in ["[3, 4]", "3,4"] {
            assert!(
                records
                    .iter()
                    .any(|(level, text)| *level == log::Level::Error
                        && text.contains(pos)
                        && text.contains("entities refused"))
            );
        }
    }

    #[test]
    fn crash_diagnostics_follow_the_launch_servers_lifetime() {
        let diagnostics = SaveFailureDiagnostics::default();
        assert_eq!(diagnostics.suppressed_summary(), None);
        let integrated = Arc::new(SaveFailureReporter::new(true, Path::new("."), "world"));
        integrated.record(0, "Error", "integrated failure".into());
        diagnostics.register(&integrated);
        assert!(
            diagnostics
                .suppressed_summary()
                .unwrap()
                .contains("integrated failure")
        );
        drop(integrated);
        assert_eq!(diagnostics.suppressed_summary(), None);
        let tmp = tempfile::tempdir().unwrap();
        let dedicated = Arc::new(SaveFailureReporter::new(false, tmp.path(), "world"));
        let weak_reporter = Arc::downgrade(&dedicated);
        diagnostics.register(&dedicated);
        dedicated.report(
            SaveFailure {
                world_root: &tmp.path().join("world"),
                dimension: "minecraft:overworld",
                storage: "chunk",
                pos: (3, 4),
                error: &io::Error::other("dedicated failure"),
            },
            &Mutex::new(Pending::default()),
            |_| Ok(save::LOW_DISK_SPACE),
        );
        drop(dedicated);
        assert!(weak_reporter.upgrade().is_none());
        let report = fs::read_dir(tmp.path().join("debug"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert!(
            fs::read_to_string(report.path())
                .unwrap()
                .contains("dedicated failure")
        );
        assert!(
            diagnostics
                .suppressed_summary()
                .unwrap()
                .contains("dedicated failure")
        );
    }

    #[test]
    fn a_stalled_report_worker_cannot_delay_dedicated_crash_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let mut reporter = SaveFailureReporter::new(false, dir.path(), "world");
        let diagnostics = SaveFailureDiagnostics::default();
        diagnostics.register(&reporter);
        let target = reporter.target.as_ref().unwrap();
        let (started, entered) = mpsc::channel();
        let (release, stalled) = mpsc::channel();
        target
            .enqueue(Box::new(move || {
                started.send(()).unwrap();
                stalled.recv().unwrap();
            }))
            .unwrap();
        entered.recv_timeout(Duration::from_secs(1)).unwrap();
        reporter.report(
            SaveFailure {
                world_root: dir.path(),
                dimension: "minecraft:overworld",
                storage: "chunk",
                pos: (3, 4),
                error: &io::Error::other("dedicated failure while reporter was stalled"),
            },
            &Mutex::new(Pending::default()),
            |_| Ok(save::LOW_DISK_SPACE),
        );
        let (queued, ran) = mpsc::channel();
        target
            .enqueue(Box::new(move || queued.send(()).unwrap()))
            .unwrap();

        let (finished, drained) = mpsc::channel();
        let shutdown = thread::spawn(move || {
            reporter.target.as_mut().unwrap().drain(Duration::ZERO);
            drop(reporter);
            finished.send(()).unwrap();
        });
        // Clean up the worker before checking the result, even if shutdown blocked erroneously.
        let finished_first = drained.recv_timeout(Duration::from_secs(1));
        release.send(()).unwrap();
        shutdown.join().unwrap();
        assert_eq!(finished_first, Ok(()));
        assert_eq!(
            ran.recv_timeout(Duration::from_secs(1)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        assert!(
            diagnostics
                .suppressed_summary()
                .unwrap()
                .contains("dedicated failure while reporter was stalled")
        );
    }

    #[test]
    fn summary_keeps_last_eight_messages_but_counts_older_failures() {
        let reporter = SaveFailureReporter::default();
        assert_eq!(reporter.suppressed_summary(), "~~NONE~~");
        for n in 0..9 {
            reporter.record(0, "std::io::Error", format!("message {n}"));
        }
        reporter.record(0, "Error", "another class".into());
        let summary = reporter.suppressed_summary();
        assert_eq!(reporter.reported_failures(), 10);
        assert_eq!(summary.matches("ms ago)").count(), 8);
        assert!(!summary.contains("message 0"));
        assert!(!summary.contains("message 1"));
        assert!(summary.contains("message 2"));
        assert!(summary.contains("another class"));
        assert!(summary.contains("chunk/save:Error x 1\n\t\t\tchunk/save:std::io::Error x 9"));
        assert_eq!(SaveFailureReporter::default().reported_failures(), 0);
    }
}
