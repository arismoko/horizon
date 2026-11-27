//! The game's log, as vanilla's log4j configuration writes it (client-1.21.2.xml, the one
//! 26.3's version manifest names): `logs/latest.log` in the game folder with the pattern
//! `[%d{HH:mm:ss}] [%t/%level]: %msg%n`, rolled over at start (OnStartupTriggeringPolicy, a
//! non-empty file only) and when the day changes (TimeBasedTriggeringPolicy) to
//! `logs/%d{yyyy-MM-dd}-%i.log.gz`, dated by the old file's last write. DefaultRolloverStrategy
//! keeps 7 archives a day, the newest with the highest index.
//!
//! Messages go through the `log` facade. Sunrise's own still print their plain text on stderr,
//! as they did before the log existed. Other crates, including native mods and their helper
//! crates, reach the file through INFO too, with their target prefixed to identify the source.
//! `SUNRISE_LOG_STDERR` also echoes selected targets to stderr for development.
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, mpsc};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDate};

/// DefaultRolloverStrategy's default `max`: archives kept for one date.
pub const MAX_ARCHIVES_PER_DAY: u32 = 7;
/// Formatted lines kept in memory for crash reports.
const RECENT: usize = 50;
/// Warnings and errors one line of code may log in a [`FLOOD_WINDOW`]; past that they are counted,
/// not written, so a warning repeated every packet can't fill the disk. Vanilla has no such limit:
/// below it the log is vanilla's, line for line.
const FLOOD_LINES: u32 = 100;
const FLOOD_WINDOW: Duration = Duration::from_secs(60);

/// The platform data folder, shared with launcher command preparation.
pub fn data_dir() -> PathBuf {
    sunrise_mod_package::selection::data_dir_with(|key| std::env::var_os(key))
}

/// The resolved game and overlay cache homes, set once at startup.
static HOME: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();

/// Records the game home and the base folder for the content overlay cache.
pub fn set_game_home(home: PathBuf, cache_home: PathBuf) {
    let _ = HOME.set((home, cache_home));
}

/// `--player-dir DIR` (#4952), set once at start: one player's own files, when several games
/// play in one game folder at the same time (the launcher's `play --account`).
static PLAYER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Makes [`player_dir`] return `dir` for the rest of the run (`--player-dir`), creating it.
/// What is a player's own goes there: `options.txt` and the settings files beside it,
/// `logs/`, `crash-reports/`, `sunrise-player.txt`, `servers.dat`, `hotbar.nbt` and
/// `screenshots/`. What the game folder's players share stays in it: saves, resource and
/// shader packs, mods and their config, caches.
pub fn set_player_dir(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    let _ = PLAYER.set(dir);
}

/// The folder of this player's own files: `--player-dir`, else the game folder (the working
/// directory, where the launcher runs the game).
pub fn player_dir() -> PathBuf {
    PLAYER
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// `--player-dir`, if given.
pub fn player_dir_override() -> Option<&'static Path> {
    PLAYER.get().map(PathBuf::as_path)
}

/// Where the files all players share live, for settings kept beside `options.txt` in
/// `options_dir` (the enabled mods): that folder, unless `--player-dir` moved `options.txt`
/// out of the game folder.
pub fn shared_dir(options_dir: &Path) -> PathBuf {
    shared_dir_of(PLAYER.get().map(PathBuf::as_path), options_dir, || {
        std::env::current_dir().unwrap_or_default()
    })
}

/// [`shared_dir`] for a given `--player-dir` and game folder.
fn shared_dir_of(
    player: Option<&Path>,
    options_dir: &Path,
    game_dir: impl FnOnce() -> PathBuf,
) -> PathBuf {
    match player {
        Some(player) if player == options_dir => game_dir(),
        _ => options_dir.to_path_buf(),
    }
}

/// Where the launcher runs the game and keeps its saves: `--home`, else `SUNRISE_HOME`, else
/// `sunrise` in the platform data folder (https://github.com/sunrise-org/sunrise-launcher/blob/main/src/backend.rs `game_home`).
pub fn game_home() -> PathBuf {
    if let Some((home, _)) = HOME.get() {
        return home.clone();
    }
    match std::env::var_os("SUNRISE_HOME").filter(|h| !h.is_empty()) {
        Some(h) => PathBuf::from(h),
        None => data_dir().join("sunrise"),
    }
}

/// The content overlay's `.sunrise-cache`: in the private home when `--home` or
/// `SUNRISE_HOME` names one, else in the game folder. Without startup (e.g. in tests),
/// use a named `SUNRISE_HOME` or the working directory.
pub fn cache_dir() -> PathBuf {
    if let Some((_, cache_home)) = HOME.get() {
        return cache_home.join(".sunrise-cache");
    }
    match std::env::var_os("SUNRISE_HOME").filter(|home| !home.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => std::env::current_dir().unwrap_or_default(),
    }
    .join(".sunrise-cache")
}

/// `logs/` and `crash-reports/` of a game folder.
pub fn logs_dir(home: &Path) -> PathBuf {
    home.join("logs")
}
pub fn crash_reports_dir(home: &Path) -> PathBuf {
    home.join("crash-reports")
}

/// `2026-10-14-3.log.gz`
pub fn archive_name(date: NaiveDate, index: u32) -> String {
    format!("{}-{index}.log.gz", date.format("%Y-%m-%d"))
}

/// The archive index of `name` if it is one of `date`'s archives.
fn archive_index(name: &str, date: NaiveDate) -> Option<u32> {
    let prefix = format!("{}-", date.format("%Y-%m-%d"));
    name.strip_prefix(&prefix)?
        .strip_suffix(".log.gz")?
        .parse()
        .ok()
        .filter(|&i| i > 0)
}

/// What one rollover does to the archives already there.
#[derive(Debug, PartialEq, Eq)]
pub struct Rollover {
    pub delete: Option<String>,
    /// In order: each target is free when its rename runs.
    pub renames: Vec<(String, String)>,
    pub archive: String,
}

/// DefaultRolloverStrategy with fileIndex "max": the new archive takes the next index; once
/// `max` are there the lowest goes and the others move down one.
pub fn rollover(existing: &[String], date: NaiveDate, max: u32) -> Rollover {
    let mut present: Vec<u32> = existing
        .iter()
        .filter_map(|n| archive_index(n, date))
        .collect();
    present.sort_unstable();
    present.dedup();
    let highest = present.last().copied().unwrap_or(0);
    if highest < max {
        return Rollover {
            delete: None,
            renames: Vec::new(),
            archive: archive_name(date, highest + 1),
        };
    }
    let delete = Some(archive_name(date, present[0]));
    let kept = &present[1..];
    let renames = kept
        .iter()
        .enumerate()
        .map(|(i, &from)| (from, i as u32 + 1))
        .filter(|(from, to)| from != to)
        .map(|(from, to)| (archive_name(date, from), archive_name(date, to)))
        .collect();
    Rollover {
        delete,
        renames,
        archive: archive_name(date, kept.len() as u32 + 1),
    }
}

/// `[12:04:31] [Render thread/INFO]: message`
pub fn format_line(
    time: &DateTime<Local>,
    thread: &str,
    level: log::Level,
    message: &str,
) -> String {
    format!(
        "[{}] [{thread}/{level}]: {message}",
        time.format("%H:%M:%S")
    )
}

/// Gzips `text` into the next archive for `date` in `dir`.
fn archive(dir: &Path, date: NaiveDate, text: &[u8]) -> io::Result<()> {
    let names: Vec<String> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    let plan = rollover(&names, date, MAX_ARCHIVES_PER_DAY);
    if let Some(name) = &plan.delete {
        fs::remove_file(dir.join(name))?;
    }
    for (from, to) in &plan.renames {
        fs::rename(dir.join(from), dir.join(to))?;
    }
    let mut gz = flate2::write::GzEncoder::new(
        File::create(dir.join(&plan.archive))?,
        flate2::Compression::default(),
    );
    gz.write_all(text)?;
    gz.finish()?.sync_all()
}

/// Moves what `file` holds into an archive dated `date` and empties it. The file stays open
/// (and locked), so this works on Windows too.
fn roll(file: &mut File, dir: &Path, date: NaiveDate) -> io::Result<()> {
    let mut text = Vec::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut text)?;
    if !text.is_empty() {
        archive(dir, date, &text)?;
    }
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(())
}

/// The open `latest.log` and the lock that keeps it this game's.
pub struct Latest {
    pub file: File,
    _lock: File,
}

/// Opens `dir/latest.log` for this run, archiving the last run's. None when another Sunrise
/// using the same folder is writing it: that game keeps it, this one logs to stderr only. The
/// lock is a separate file (`.lock`): Windows locks are mandatory, and a locked latest.log could
/// not be read while the game runs.
pub fn open_latest(dir: &Path) -> io::Result<Option<Latest>> {
    fs::create_dir_all(dir)?;
    let lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(".lock"))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Ok(None),
        Err(fs::TryLockError::Error(e)) => return Err(e),
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("latest.log"))?;
    let modified = file.metadata()?.modified()?;
    let date = DateTime::<Local>::from(modified).date_naive();
    roll(&mut file, dir, date)?;
    Ok(Some(Latest { file, _lock: lock }))
}

/// Where the message appears besides the log file.
enum Echo {
    Quiet,
    /// The caller prints the line itself, e.g. on the dedicated server's stdout.
    Mirrored,
    Plain(String),
    Record,
}

/// What the writer thread is told.
enum Out {
    /// Lines for the file, dated, with their stderr policy.
    Write {
        date: NaiveDate,
        lines: Vec<String>,
        stderr: Echo,
    },
    /// Answer once everything before is written.
    Flush(mpsc::SyncSender<()>),
}

/// The file side of the log, on its own thread: a line costs the thread that logs it a
/// format and a channel send, never a write that can wait (a full stderr pipe, a slow disk):
/// written on the game thread, a line could hold the tick for milliseconds while the CPU sat
/// idle.
struct Writer {
    file: Option<Latest>,
    dir: PathBuf,
    /// The day the open file's lines belong to.
    date: NaiveDate,
}

impl Writer {
    fn channel<'a>(
        &self,
        tap: &'a Tap,
        main_thread: &'static str,
        stderr_targets: Vec<String>,
    ) -> (Logger<'a>, mpsc::Receiver<Out>) {
        let (out, receive) = mpsc::channel();
        let logger = Logger {
            out,
            tap,
            has_file: self.file.is_some(),
            recent: Mutex::new(VecDeque::new()),
            main_thread,
            stderr_targets,
            floods: Mutex::new(Floods::default()),
        };
        (logger, receive)
    }

    fn run(mut self, out: mpsc::Receiver<Out>, mut stderr: impl Write) {
        for msg in out {
            match msg {
                Out::Write {
                    date,
                    lines,
                    stderr: echo,
                } => {
                    if let Echo::Plain(message) = &echo {
                        let _ = writeln!(stderr, "{message}");
                    }
                    if date != self.date {
                        if let Some(latest) = &mut self.file
                            && let Err(e) = roll(&mut latest.file, &self.dir, self.date)
                        {
                            let _ = writeln!(stderr, "Sunrise: could not roll over the log: {e}");
                        }
                        self.date = date;
                    }
                    let echo_record = matches!(echo, Echo::Record);
                    if self.file.is_some() || echo_record {
                        let mut text = lines.join("\n");
                        text.push('\n');
                        if let Some(latest) = &mut self.file {
                            let _ = latest.file.write_all(text.as_bytes());
                        }
                        if echo_record {
                            let _ = stderr.write_all(text.as_bytes());
                        }
                    }
                }
                Out::Flush(done) => {
                    if let Some(latest) = &mut self.file {
                        let _ = latest.file.flush();
                    }
                    let _ = done.send(());
                }
            }
        }
    }
}

struct Logger<'a> {
    out: mpsc::Sender<Out>,
    tap: &'a Tap,
    has_file: bool,
    /// The last lines, kept here (not on the writer) so a crash report has them at once.
    recent: Mutex<VecDeque<String>>,
    /// What the main thread is called in the log: "Render thread", or "Server thread" for the
    /// dedicated server, as in vanilla.
    main_thread: &'static str,
    stderr_targets: Vec<String>,
    floods: Mutex<Floods>,
}

/// What the flood limit does with one warning or error.
#[derive(Debug, PartialEq, Eq)]
enum Gate {
    /// Write it, after saying how many lines like it were left out in the window before.
    Write {
        left_out: u32,
    },
    /// Write it, then say the next ones from this line of code wait for the next window.
    Last,
    Skip,
}

/// Warnings and errors per line of code, in windows of [`FLOOD_WINDOW`].
#[derive(Default)]
struct Floods {
    sites: HashMap<(String, u32), Flood>,
}

struct Flood {
    since: Instant,
    written: u32,
    left_out: u32,
}

impl Floods {
    fn gate(&mut self, file: &str, line: u32, now: Instant) -> Gate {
        let flood = self.sites.entry((file.to_owned(), line)).or_insert(Flood {
            since: now,
            written: 0,
            left_out: 0,
        });
        let mut left_out = 0;
        if now.duration_since(flood.since) >= FLOOD_WINDOW {
            left_out = flood.left_out;
            *flood = Flood {
                since: now,
                written: 0,
                left_out: 0,
            };
        }
        flood.written += 1;
        match flood.written.cmp(&FLOOD_LINES) {
            std::cmp::Ordering::Less => Gate::Write { left_out },
            std::cmp::Ordering::Equal => Gate::Last,
            std::cmp::Ordering::Greater => {
                flood.left_out += 1;
                Gate::Skip
            }
        }
    }
}

static LOGGER: std::sync::OnceLock<Logger<'static>> = std::sync::OnceLock::new();

/// The most lines the tap keeps between two drains (`take_tapped`); new overflow is dropped.
pub const TAP_LINES: usize = 4096;

/// The most bytes of a line's message the tap keeps; the rest is cut off at a character boundary.
pub const TAP_MESSAGE_BYTES: usize = 8192;
/// The most bytes of a thread name the tap keeps; the rest is cut off at a character boundary.
pub const TAP_THREAD_BYTES: usize = 128;

/// A line the log tap kept: its level, the thread it names and the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TappedLine {
    pub level: log::Level,
    pub thread: String,
    pub message: String,
    /// The batch's overflow count on its first line; zero on every later line.
    pub dropped: u64,
}

#[derive(Default)]
struct TapQueue {
    lines: Vec<TappedLine>,
    dropped: u64,
}

impl TapQueue {
    fn push(&mut self, thread: &str, level: log::Level, message: &str) {
        if self.lines.len() == TAP_LINES {
            self.dropped += 1;
            return;
        }
        self.lines.push(TappedLine {
            level,
            thread: capped(thread, TAP_THREAD_BYTES),
            message: capped(message, TAP_MESSAGE_BYTES),
            dropped: 0,
        });
    }

    fn drain(&mut self) -> Vec<TappedLine> {
        if let Some(first) = self.lines.first_mut() {
            first.dropped = std::mem::take(&mut self.dropped);
        }
        std::mem::take(&mut self.lines)
    }
}

struct Tap {
    /// Read before touching the tap queue. Only the server tick or shutdown changes it.
    active: AtomicBool,
    queue: Mutex<TapQueue>,
}

impl Tap {
    const fn new() -> Self {
        Self {
            active: AtomicBool::new(false),
            queue: Mutex::new(TapQueue {
                lines: Vec::new(),
                dropped: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, TapQueue> {
        #[cfg(test)]
        TAP_LOCKS.set(TAP_LOCKS.get() + 1);
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_if_capturing(&self) -> Option<MutexGuard<'_, TapQueue>> {
        if !self.active.load(Ordering::Relaxed) || UNTAPPED.get() {
            return None;
        }
        let queue = self.lock();
        // Shutdown may have disabled the tap while this producer waited for the queue.
        self.active.load(Ordering::Relaxed).then_some(queue)
    }

    fn take(&self, listened: bool) -> Vec<TappedLine> {
        if !listened {
            self.stop();
        } else if self.active.load(Ordering::Relaxed) {
            return self.lock().drain();
        } else {
            self.active.store(true, Ordering::Relaxed);
        }
        Vec::new()
    }

    fn stop(&self) {
        if self.active.load(Ordering::Relaxed) {
            let mut queue = self.lock();
            self.active.store(false, Ordering::Relaxed);
            *queue = TapQueue::default();
        }
    }
}

static TAP: Tap = Tap::new();

thread_local! {
    /// Set while this thread runs `untapped`: its lines are not kept.
    static UNTAPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    static TAP_LOCKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Called by the server at tick start. The first listening tick enables capture; later ones
/// drain the previous tick's batch in order. With no listener, only the transition from active
/// to inactive touches the queue, to discard any undelivered lines and release its memory.
pub fn take_tapped(listened: bool) -> Vec<TappedLine> {
    TAP.take(listened)
}

/// Stop capture and forget any undelivered lines. Already idle means no tap lock.
pub fn stop_tap() {
    TAP.stop();
}

/// Runs `f` without tapping the lines this thread logs meanwhile (a `console_line`
/// listener's own lines).
pub fn untapped<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            UNTAPPED.set(self.0);
        }
    }
    let _restore = Restore(UNTAPPED.replace(true));
    f()
}

/// `text` cut to at most `max` bytes, at a character boundary.
fn capped(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn ours(target: &str) -> bool {
    target.starts_with("sunrise")
}

/// A target is the named crate or one of its modules, not just a shared prefix.
fn target_matches(target: &str, name: &str) -> bool {
    target
        .strip_prefix(name)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

/// Crates logged from ERROR only, their warnings being known noise. `calloop` 0.13, winit 0.30's
/// Wayland event loop, warns once per key press: smithay-client-toolkit re-registers the key
/// repeat timer on each press and the removed timer's last event still arrives ("Received an
/// event for non-existence source"). Neither crate has a release without it.
const ERRORS_ONLY: &[&str] = &["calloop"];

impl log::Log for Logger<'_> {
    fn enabled(&self, meta: &log::Metadata) -> bool {
        let target = meta.target();
        let floor = if ERRORS_ONLY.iter().any(|name| target_matches(target, name)) {
            log::Level::Error
        } else {
            log::Level::Info
        };
        meta.level() <= floor
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let target = record.target();
        let sunrise = ours(target);
        let message = if sunrise {
            record.args().to_string()
        } else {
            format!("[{target}] {}", record.args())
        };
        let current = std::thread::current();
        let thread = match current.name() {
            Some("main") => self.main_thread,
            Some(name) => name,
            None => "unnamed",
        };
        let gate = if record.level() <= log::Level::Warn {
            let mut floods = self.floods.lock().unwrap_or_else(|e| e.into_inner());
            floods.gate(
                record.file().unwrap_or(record.target()),
                record.line().unwrap_or(0),
                Instant::now(),
            )
        } else {
            Gate::Write { left_out: 0 }
        };
        let level = record.level();
        match gate {
            Gate::Skip => return,
            Gate::Write { left_out } if left_out > 0 => self.send(
                thread,
                level,
                &format!(
                    "{left_out} more lines like the next one were not logged in the last minute"
                ),
                Echo::Quiet,
            ),
            _ => {}
        }
        let stderr = if sunrise {
            Echo::Plain(message.clone())
        } else if self
            .stderr_targets
            .iter()
            .any(|name| target_matches(target, name))
        {
            Echo::Record
        } else {
            Echo::Quiet
        };
        self.send(thread, level, &message, stderr);
        if gate == Gate::Last {
            self.send(
                thread,
                level,
                &format!("{FLOOD_LINES} lines like this one in a minute: the next ones are counted, not logged"),
                Echo::Quiet,
            );
        }
    }

    /// Waits (a second at most) until every line logged before is in the file.
    fn flush(&self) {
        let (done, wait) = mpsc::sync_channel(1);
        if self.out.send(Out::Flush(done)).is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

impl Logger<'_> {
    fn send(&self, thread: &str, level: log::Level, message: &str, stderr: Echo) {
        let now = Local::now();
        // As log4j writes a multi-line message: the pattern once, the rest as it is.
        let lines: Vec<String> = message
            .lines()
            .enumerate()
            .map(|(i, line)| {
                if i == 0 {
                    format_line(&now, thread, level, line)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            for line in &lines {
                if recent.len() == RECENT {
                    recent.pop_front();
                }
                recent.push_back(line.clone());
            }
        }
        let visible_without_writer = matches!(stderr, Echo::Plain(_) | Echo::Mirrored);
        let visible = self.has_file || visible_without_writer || matches!(stderr, Echo::Record);
        // Keep the lock through publication so the tap and the writer see records in the
        // same order. Quiet records without a file only go into the crash report's history.
        let mut queue = if visible {
            self.tap.lock_if_capturing()
        } else {
            None
        };
        let result = self.out.send(Out::Write {
            date: now.date_naive(),
            lines,
            stderr,
        });
        if let Some(queue) = &mut queue
            && (result.is_ok() || visible_without_writer)
        {
            queue.push(thread, level, message);
        }
        drop(queue);
        if let Err(mpsc::SendError(Out::Write {
            stderr: Echo::Plain(message),
            ..
        })) = result
        {
            // No writer (it could not start): the terminal still gets Sunrise's messages.
            eprintln!("{message}");
        }
    }
}

/// Starts the log in `home/logs`. The main thread shows as `main_thread` in it. Messages are
/// still printed when the file cannot be opened.
pub fn init(home: &Path, main_thread: &'static str) {
    let dir = logs_dir(home);
    let file = match open_latest(&dir) {
        Ok(Some(latest)) => Some(latest),
        Ok(None) => {
            eprintln!(
                "Sunrise: another game is writing {}; this one logs to the terminal only",
                dir.join("latest.log").display()
            );
            None
        }
        Err(e) => {
            eprintln!(
                "Sunrise: could not open {}: {e}",
                dir.join("latest.log").display()
            );
            None
        }
    };
    let logger = LOGGER.get_or_init(|| {
        let writer = Writer {
            file,
            dir,
            date: Local::now().date_naive(),
        };
        let stderr_targets = std::env::var("SUNRISE_LOG_STDERR")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .map(str::to_owned)
            .collect();
        let (logger, receive) = writer.channel(&TAP, main_thread, stderr_targets);
        if let Err(e) = std::thread::Builder::new()
            .name("Log writer".into())
            .spawn(move || writer.run(receive, io::stderr()))
        {
            eprintln!("Sunrise: could not start the log writer: {e}");
        }
        logger
    });
    if log::set_logger(logger).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Writes to the log file under the current thread's log name. Set `printed_elsewhere` only
/// when the caller also prints this message to a terminal, so the tap can see it without a file.
pub fn write_current(level: log::Level, message: &str, printed_elsewhere: bool) {
    let current = std::thread::current();
    let main = LOGGER.get().map_or("main", |logger| logger.main_thread);
    let thread = match current.name() {
        Some("main") | None => main,
        Some(name) => name,
    };
    if printed_elsewhere {
        write_mirrored_as(thread, level, message);
    } else {
        write_as(thread, level, message);
    }
}

/// Writes `message` to the log file only, under the thread vanilla names for it.
pub fn write_as(thread: &str, level: log::Level, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.send(thread, level, message, Echo::Quiet);
    }
}

/// Writes a line that the caller also prints to a terminal, under the thread vanilla names for
/// it (`Server thread` for a login refused on a connection's own thread).
pub fn write_mirrored_as(thread: &str, level: log::Level, message: &str) {
    if let Some(logger) = LOGGER.get() {
        logger.send(thread, level, message, Echo::Mirrored);
    } else if let Some(mut queue) = TAP.lock_if_capturing() {
        queue.push(thread, level, message);
    }
}

/// The last lines written, oldest first (for crash reports).
pub fn recent_lines() -> Vec<String> {
    LOGGER.get().map_or_else(Vec::new, |l| {
        l.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    })
}

/// Flushes the log file (before the process exits).
pub fn flush() {
    if let Some(logger) = LOGGER.get() {
        log::Log::flush(logger);
    }
}

/// The last `lines` lines of a text file.
pub fn tail(path: &Path, lines: usize) -> io::Result<Vec<String>> {
    let text = fs::read(path)?;
    let text = String::from_utf8_lossy(&text);
    let all: Vec<&str> = text.lines().collect();
    Ok(all[all.len().saturating_sub(lines)..]
        .iter()
        .map(|l| l.to_string())
        .collect())
}

/// The newest `crash-*.txt` in a game folder's crash-reports (names sort by time).
pub fn newest_crash_report(home: &Path) -> Option<PathBuf> {
    fs::read_dir(crash_reports_dir(home))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .max_by_key(|p| p.file_name().map(|n| n.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tap_keeps_lines_while_on_and_skips_untapped_ones() {
        let tap = Tap::new();
        let writer = Writer {
            file: None,
            dir: PathBuf::new(),
            date: day(14),
        };
        let (logger, _receive) = writer.channel(&tap, "Server thread", Vec::new());
        let record = |thread, level, message| logger.send(thread, level, message, Echo::Record);
        let idle_locks = TAP_LOCKS.get();
        assert!(tap.take(false).is_empty());
        record("Server thread", log::Level::Info, "tap probe off");
        assert_eq!(
            TAP_LOCKS.get(),
            idle_locks,
            "an idle tick and log take no tap lock"
        );
        assert!(tap.take(true).is_empty());
        record("Server thread", log::Level::Warn, "tap probe 1");
        untapped(|| record("Server thread", log::Level::Info, "tap probe hidden"));
        record("Another thread", log::Level::Error, "tap probe 2");
        let kept = tap.take(true);
        let probes: Vec<_> = kept
            .iter()
            .map(|l| (l.thread.as_str(), l.level, l.message.as_str()))
            .collect();
        assert_eq!(
            probes,
            [
                ("Server thread", log::Level::Warn, "tap probe 1"),
                ("Another thread", log::Level::Error, "tap probe 2"),
            ]
        );
        // The message ends in a three-byte character that straddles the cap.
        let long = format!("tap probe long {}", "\u{20ac}".repeat(TAP_MESSAGE_BYTES));
        let thread = "t".repeat(TAP_THREAD_BYTES + 50);
        record(&thread, log::Level::Info, &long);
        let kept = tap.take(true);
        assert_eq!(kept.len(), 1);
        let line = &kept[0];
        assert_eq!(line.thread, "t".repeat(TAP_THREAD_BYTES));
        assert!(line.message.starts_with("tap probe long \u{20ac}"));
        assert!(line.message.len() <= TAP_MESSAGE_BYTES);
        assert!(
            TAP_MESSAGE_BYTES - line.message.len() < 3,
            "cut at the last whole character"
        );
        record("Server thread", log::Level::Info, "discarded on stop");
        assert!(tap.take(false).is_empty());
        record("Server thread", log::Level::Info, "tap probe after");
        assert!(tap.take(true).is_empty());
        assert!(tap.take(true).is_empty());
    }

    #[test]
    fn the_tap_keeps_the_oldest_lines_and_reports_overflow_once() {
        let mut queue = TapQueue::default();
        for i in 0..4099 {
            queue.push("Server thread", log::Level::Info, &format!("line {i}"));
        }
        let batch = queue.drain();
        assert_eq!(batch.len(), 4096);
        for (i, line) in batch.iter().enumerate() {
            assert_eq!(line.message, format!("line {i}"));
            assert_eq!(line.dropped, if i == 0 { 3 } else { 0 });
        }
        assert!(queue.drain().is_empty());
        queue.push("Server thread", log::Level::Info, "next batch");
        let batch = queue.drain();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].message, "next batch");
        assert_eq!(batch[0].dropped, 0);
    }

    #[test]
    fn native_mod_records_reach_the_file_with_their_targets() {
        use log::Log;
        let dir =
            std::env::temp_dir().join(format!("sunrise-native-log-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let tap = Tap::new();
        let writer = Writer {
            file: Some(open_latest(&dir).unwrap().unwrap()),
            dir: dir.clone(),
            date: Local::now().date_naive(),
        };
        let (logger, receive) = writer.channel(&tap, "Render thread", Vec::new());
        let handle = std::thread::spawn(move || writer.run(receive, io::sink()));
        for (target, level, message) in [
            ("meridian_sunrise::save", log::Level::Warn, "save failed"),
            ("meridian_core::map", log::Level::Info, "map opened"),
            ("wgpu_core::device", log::Level::Error, "device lost"),
            ("meridian_core::map", log::Level::Debug, "debug detail"),
            ("meridian_core::map", log::Level::Trace, "trace detail"),
            ("calloop::loop_logic", log::Level::Warn, "calloop noise"),
            ("calloop::loop_logic", log::Level::Error, "calloop failed"),
            ("calloopish", log::Level::Warn, "another crate"),
        ] {
            logger.log(
                &log::Record::builder()
                    .args(format_args!("{message}"))
                    .level(level)
                    .target(target)
                    .build(),
            );
        }
        logger.flush();
        drop(logger);
        handle.join().unwrap();
        let text = fs::read_to_string(dir.join("latest.log")).unwrap();
        fs::remove_dir_all(&dir).unwrap();
        assert!(
            text.contains("[meridian_sunrise::save] save failed"),
            "{text}"
        );
        assert!(text.contains("[meridian_core::map] map opened"), "{text}");
        assert!(text.contains("[wgpu_core::device] device lost"), "{text}");
        assert!(!text.contains("debug detail"));
        assert!(!text.contains("trace detail"));
        assert!(!text.contains("calloop noise"), "{text}");
        assert!(
            text.contains("[calloop::loop_logic] calloop failed"),
            "{text}"
        );
        assert!(text.contains("[calloopish] another crate"), "{text}");
    }

    #[test]
    fn sunrise_stderr_and_external_file_records_stay_separate() {
        use log::Log;
        let tap = Tap::new();
        let writer = Writer {
            file: None,
            dir: PathBuf::new(),
            date: day(14),
        };
        let (logger, receive) = writer.channel(&tap, "Render thread", Vec::new());
        for target in ["sunrise_client::game", "meridian_core", "wgpu_hal::vulkan"] {
            logger.log(
                &log::Record::builder()
                    .args(format_args!("diagnostic"))
                    .level(log::Level::Info)
                    .target(target)
                    .build(),
            );
            let Out::Write { stderr, lines, .. } = receive.try_recv().unwrap() else {
                panic!("a diagnostic must write a record")
            };
            if ours(target) {
                assert!(matches!(stderr, Echo::Plain(message) if message == "diagnostic"));
                assert!(lines.iter().any(|line| line.ends_with("diagnostic")));
            } else {
                assert!(matches!(stderr, Echo::Quiet));
                assert!(
                    lines
                        .iter()
                        .any(|line| line.ends_with(&format!("[{target}] diagnostic")))
                );
            }
        }
    }

    #[test]
    fn selected_targets_echo_whole_records_in_the_file_format() {
        use log::Log;
        let dir = tempfile::tempdir().unwrap();
        // With the file open, and when a different game owns it.
        for with_file in [true, false] {
            let tap = Tap::new();
            tap.take(true);
            let log_day = day(14);
            let writer = Writer {
                file: with_file.then(|| open_latest(dir.path()).unwrap().unwrap()),
                dir: dir.path().to_owned(),
                date: log_day,
            };
            let (logger, receive) = writer.channel(
                &tap,
                "Render thread",
                vec!["my_mod".into(), "another_mod".into()],
            );
            for (target, message) in [
                ("my_mod", "root record"),
                ("my_mod::map", "module record\ncontinued here"),
                ("another_mod", "second selected target"),
                (
                    "other",
                    "other record\n[12:34:56] [Render thread/INFO]: [my_mod] not ours",
                ),
                ("my_mod_else", "similar crate name"),
                ("sunrise_client::game", "Sunrise message"),
            ] {
                logger.log(
                    &log::Record::builder()
                        .args(format_args!("{message}"))
                        .level(log::Level::Info)
                        .target(target)
                        .build(),
                );
            }
            logger.send(
                "Server thread",
                log::Level::Info,
                "printed on stdout by the caller",
                Echo::Mirrored,
            );
            let mut expected = vec![
                "[my_mod] root record",
                "[my_mod::map] module record\ncontinued here",
                "[another_mod] second selected target",
            ];
            if with_file {
                expected.extend([
                    "[other] other record\n[12:34:56] [Render thread/INFO]: [my_mod] not ours",
                    "[my_mod_else] similar crate name",
                ]);
            }
            expected.extend(["Sunrise message", "printed on stdout by the caller"]);
            let batch = tap.take(true);
            assert_eq!(
                batch.iter().map(|l| l.message.as_str()).collect::<Vec<_>>(),
                expected
            );
            drop(logger);
            // Keep the byte comparison on one log day, even across midnight.
            let (fixed, queued) = mpsc::channel();
            for mut record in receive {
                let Out::Write { date, .. } = &mut record else {
                    panic!("a diagnostic must write a record");
                };
                *date = log_day;
                fixed.send(record).unwrap();
            }
            drop(fixed);
            let mut stderr = Vec::new();
            writer.run(queued, &mut stderr);
            let stderr = String::from_utf8(stderr).unwrap();
            let lines: Vec<_> = stderr.lines().collect();
            assert_eq!(lines.len(), 5);
            assert!(lines[0].ends_with("/INFO]: [my_mod] root record"));
            assert!(lines[1].ends_with("/INFO]: [my_mod::map] module record"));
            assert_eq!(lines[2], "continued here");
            assert!(lines[3].ends_with("/INFO]: [another_mod] second selected target"));
            assert_eq!(lines[4], "Sunrise message");
            assert!(stderr.ends_with('\n'));
            if with_file {
                let full = fs::read_to_string(dir.path().join("latest.log")).unwrap();
                assert!(full.starts_with(stderr.strip_suffix("Sunrise message\n").unwrap()));
                assert!(full.contains(
                    "[other] other record\n[12:34:56] [Render thread/INFO]: [my_mod] not ours\n"
                ));
                assert!(full.contains("[my_mod_else] similar crate name\n"));
                assert!(full.ends_with("/INFO]: printed on stdout by the caller\n"));
            }
        }
    }

    #[test]
    fn a_flood_notice_only_reaches_the_tap_when_written_to_the_file() {
        use log::Log;
        let dir = tempfile::tempdir().unwrap();
        let notice = "100 lines like this one in a minute: the next ones are counted, not logged";
        for with_file in [false, true] {
            let tap = Tap::new();
            tap.take(true);
            let writer = Writer {
                file: with_file.then(|| open_latest(dir.path()).unwrap().unwrap()),
                dir: dir.path().to_owned(),
                date: Local::now().date_naive(),
            };
            let (logger, receive) = writer.channel(&tap, "Server thread", Vec::new());
            for _ in 0..101 {
                logger.log(
                    &log::Record::builder()
                        .args(format_args!("repeated warning"))
                        .level(log::Level::Warn)
                        .target("sunrise_server")
                        .build(),
                );
            }
            let batch = tap.take(true);
            assert_eq!(batch.len(), if with_file { 101 } else { 100 });
            assert_eq!(batch[0].dropped, 0);
            assert_eq!(
                batch.last().unwrap().message,
                if with_file {
                    notice
                } else {
                    "repeated warning"
                }
            );
            drop(logger);
            let mut stderr = Vec::new();
            writer.run(receive, &mut stderr);
            assert_eq!(
                String::from_utf8(stderr).unwrap(),
                "repeated warning\n".repeat(100)
            );
            if with_file {
                let file = fs::read_to_string(dir.path().join("latest.log")).unwrap();
                assert!(file.ends_with(&format!("/WARN]: {notice}\n")));
            }
        }
    }

    #[test]
    fn a_player_dir_keeps_the_shared_files_in_the_game_folder() {
        let (game, player) = (Path::new("/games/shared"), Path::new("/games/players/alex"));
        // options.txt moved to the player's folder: the enabled mods stay the game folder's.
        assert_eq!(shared_dir_of(Some(player), player, || game.into()), game);
        // With or without --player-dir: beside options.txt.
        assert_eq!(shared_dir_of(None, game, || unreachable!()), game);
        let profile = Path::new("/games/shared/profiles/local");
        assert_eq!(
            shared_dir_of(Some(player), profile, || unreachable!()),
            profile
        );
    }

    #[test]
    fn a_warning_repeated_every_packet_is_counted_past_a_hundred_a_minute() {
        let (mut floods, start) = (Floods::default(), Instant::now());
        let gates: Vec<Gate> = (0..140)
            .map(|i| floods.gate("lan_vehicles.rs", 345, start + Duration::from_millis(i * 7)))
            .collect();
        assert!(
            gates[..99]
                .iter()
                .all(|g| *g == Gate::Write { left_out: 0 })
        );
        assert_eq!(gates[99], Gate::Last);
        assert!(gates[100..].iter().all(|g| *g == Gate::Skip));
        // Another line of code has its own count.
        assert_eq!(
            floods.gate("lan_vehicles.rs", 330, start),
            Gate::Write { left_out: 0 }
        );
        // The next minute writes again, and says how many were left out.
        assert_eq!(
            floods.gate("lan_vehicles.rs", 345, start + FLOOD_WINDOW),
            Gate::Write { left_out: 40 }
        );
        assert_eq!(
            floods.gate("lan_vehicles.rs", 345, start + FLOOD_WINDOW),
            Gate::Write { left_out: 0 }
        );
    }

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn archives_are_named_like_vanillas() {
        assert_eq!(archive_name(day(4), 1), "2026-10-04-1.log.gz");
        let none: Vec<String> = Vec::new();
        assert_eq!(
            rollover(&none, day(14), 7),
            Rollover {
                delete: None,
                renames: vec![],
                archive: "2026-10-14-1.log.gz".into()
            }
        );
        // Another day's archives and unrelated files do not count.
        let names: Vec<String> = [
            "2026-10-13-1.log.gz",
            "2026-10-13-2.log.gz",
            "2026-10-14-1.log.gz",
            "2026-10-14-2.log.gz",
            "latest.log",
            "debug-1.log.gz",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(rollover(&names, day(14), 7).archive, "2026-10-14-3.log.gz");
        assert_eq!(rollover(&names, day(13), 7).archive, "2026-10-13-3.log.gz");
        assert_eq!(rollover(&names, day(15), 7).archive, "2026-10-15-1.log.gz");
    }

    #[test]
    fn the_eighth_archive_of_a_day_drops_the_oldest() {
        let names: Vec<String> = (1..=7).map(|i| archive_name(day(14), i)).collect();
        let plan = rollover(&names, day(14), 7);
        assert_eq!(plan.delete.as_deref(), Some("2026-10-14-1.log.gz"));
        assert_eq!(plan.renames.len(), 6);
        assert_eq!(
            plan.renames[0],
            ("2026-10-14-2.log.gz".into(), "2026-10-14-1.log.gz".into())
        );
        assert_eq!(
            plan.renames[5],
            ("2026-10-14-7.log.gz".into(), "2026-10-14-6.log.gz".into())
        );
        assert_eq!(plan.archive, "2026-10-14-7.log.gz");
    }

    #[test]
    fn lines_look_like_log4js_pattern() {
        let time = chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap();
        assert_eq!(
            format_line(
                &time,
                "Render thread",
                log::Level::Info,
                "load assets: 1.20s"
            ),
            "[09:05:07] [Render thread/INFO]: load assets: 1.20s"
        );
        assert_eq!(
            format_line(&time, "Worker-1", log::Level::Warn, "x"),
            "[09:05:07] [Worker-1/WARN]: x"
        );
    }

    #[test]
    fn startup_archives_the_last_runs_log() {
        let dir = std::env::temp_dir().join(format!("sunrise-logs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // Empty: nothing is archived (OnStartupTriggeringPolicy's minSize is 1 byte).
        drop(open_latest(&dir).unwrap().unwrap());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::write(
            dir.join("latest.log"),
            "[10:00:00] [Render thread/INFO]: first run\n",
        )
        .unwrap();
        let mut latest = open_latest(&dir).unwrap().unwrap();
        // A second game in the same folder leaves the file alone.
        assert!(open_latest(&dir).unwrap().is_none());
        writeln!(latest.file, "second run").unwrap();
        drop(latest);
        let today = Local::now().date_naive();
        let archive = dir.join(archive_name(today, 1));
        let mut text = String::new();
        flate2::read::GzDecoder::new(File::open(&archive).unwrap())
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "[10:00:00] [Render thread/INFO]: first run\n");
        assert_eq!(
            fs::read_to_string(dir.join("latest.log")).unwrap(),
            "second run\n"
        );
        assert_eq!(
            tail(&dir.join("latest.log"), 100).unwrap(),
            vec!["second run"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_newest_crash_report_is_found_by_name() {
        let home = std::env::temp_dir().join(format!("sunrise-crash-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        assert!(newest_crash_report(&home).is_none());
        let dir = crash_reports_dir(&home);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "crash-2026-10-13_23.59.59-client.txt",
            "crash-2026-10-14_08.00.01-client.txt",
            "notes.txt",
        ] {
            fs::write(dir.join(name), "x").unwrap();
        }
        assert_eq!(
            newest_crash_report(&home).unwrap(),
            dir.join("crash-2026-10-14_08.00.01-client.txt")
        );
        let _ = fs::remove_dir_all(&home);
    }
}
