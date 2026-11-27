//! Crash reports, laid out like vanilla's CrashReport.getFriendlyReport: a comment, the time,
//! a description, the error with its stack, then the details of the game and the machine.
//! Written to `crash-reports/crash-<yyyy-MM-dd_HH.mm.ss>-client.txt` in the game folder when
//! the main thread panics or `main` gets an error back. It stays a local file: the player
//! reads it and shares it by hand.
pub use mod_api::content::crash::{
    ConvertedCall, ConvertedPackage, ConvertedPackageGuard, ConvertedPackages, ModCall, ModGuard,
    current_converted_call, current_native_mod,
};
// Tests run without the executable's startup: the platform's versions install the crash
// backend on first use and keep details, as the started game's do.
use mod_api::__private::game::{
    contain_unwind, panic_payload_text as payload_text, record_contained_panic,
};
#[cfg(not(test))]
use mod_api::content::crash::details;
#[cfg(not(test))]
pub use mod_api::content::crash::{clear_detail, enter_native_mod, set_detail};
#[cfg(test)]
use mod_platform::content::crash::details;
#[cfg(test)]
pub use mod_platform::content::crash::{clear_detail, enter_native_mod, set_detail};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use sunrise_system::{command_output, machine_arch};

static REPORTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// `hook_dispatch_depth()` when the innermost `contain_call` started on this thread.
    static CONTAIN_HOOK_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// `message`, with the native mod hook this thread is inside, if any, and the converted package
/// the hook's runtime is executing.
fn attributed(message: String) -> String {
    let Some(call) = mod_api::content::crash::current_native_call() else {
        return message;
    };
    let part = call
        .part
        .map(|part| format!(", part {part}"))
        .unwrap_or_default();
    let mut out = format!(
        "{message}\nin native mod {}, hook {}{part}",
        call.id, call.hook
    );
    if let Some(scoped) = mod_api::content::crash::current_converted_call()
        && scoped.runtime == call.id
    {
        // `export` and `site` come from the converted package runtime: a name
        // with a newline could forge another line of this report, so keep each on one line.
        let _ = write!(
            out,
            "\nin converted package {}, export {}, call site {}",
            scoped.package,
            scoped.export.escape_debug(),
            scoped.site.escape_debug()
        );
    }
    out
}

/// Runs `f`, and returns a panic in it as an error with the panic's message instead of letting
/// it end the game. For work that is not the game's own and whose failure the caller recovers
/// from, such as handing a shader pack's programs to the GPU driver's compiler: the panic goes
/// to the log with its stack, and no crash report is made for it.
pub fn contain<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    struct HookDepthGuard(u32);
    impl Drop for HookDepthGuard {
        fn drop(&mut self) {
            CONTAIN_HOOK_DEPTH.with(|d| d.set(self.0));
        }
    }
    let _depth = HookDepthGuard(
        CONTAIN_HOOK_DEPTH.with(|d| d.replace(mod_api::__private::hook_dispatch_depth())),
    );
    contain_unwind(f)
}

/// For the hook: a contained panic is logged with its stack and kept for its boundary to
/// return. False for any other panic.
pub fn contained_panic(info: &std::panic::PanicHookInfo) -> bool {
    record_contained_panic(&|| {
        let message = panic_message(info);
        let stack = stack_lines(
            &std::backtrace::Backtrace::force_capture().to_string(),
            true,
        );
        log::error!(
            "Recovered from an error: {message}\n{}",
            stack
                .lines()
                .map(|l| format!("\tat {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        message
    })
}

/// Vanilla picks a line from a list of jokes; these only say what happened.
const COMMENTS: &[&str] = &[
    "Something went wrong.",
    "The game stopped here.",
    "This should not have happened.",
    "Details below.",
];

/// One crash, ready to be written.
pub struct Report {
    pub time: DateTime<Local>,
    /// The entries mods put under the title (`Registrar::crash_header`), `<mod id>: <text>` with
    /// continuation lines marked `\t<mod id> | `.
    pub headers: Vec<String>,
    pub description: String,
    /// The error's message, first line of the stack section.
    pub error: String,
    /// Where it happened and how it got there: one frame a line (`function (file:line:col)`).
    pub stack: String,
    pub thread: String,
    /// `client` or `server`, the end of the file name.
    pub side: &'static str,
    pub details: Vec<(String, String)>,
    pub log: Vec<String>,
}

/// `crash-2026-10-14_09.05.07-client.txt`
pub fn file_name(time: &DateTime<Local>, side: &str) -> String {
    format!("crash-{}-{side}.txt", time.format("%Y-%m-%d_%H.%M.%S"))
}

impl Report {
    /// The report's text.
    pub fn text(&self) -> String {
        let comment = COMMENTS[self.time.timestamp().rem_euclid(COMMENTS.len() as i64) as usize];
        let mut out = String::new();
        let _ = writeln!(out, "---- Minecraft Crash Report ----");
        for header in &self.headers {
            let _ = writeln!(out, "{header}");
        }
        let _ = writeln!(out, "// {comment}\n");
        let _ = writeln!(out, "Time: {}", self.time.format("%Y-%m-%d %H:%M:%S"));
        let _ = writeln!(out, "Description: {}\n", self.description);
        let _ = writeln!(out, "{}", self.error);
        for line in self.stack.lines() {
            let _ = writeln!(out, "\tat {line}");
        }
        let _ = writeln!(
            out,
            "\n\nA detailed walkthrough of the error, its code path and all known details is as follows:"
        );
        let _ = writeln!(out, "{}\n", "-".repeat(87));
        let _ = writeln!(out, "-- Head --");
        let _ = writeln!(out, "Thread: {}\n", self.thread);
        let _ = writeln!(out, "-- System Details --");
        let _ = writeln!(out, "Details:");
        for (key, value) in &self.details {
            let _ = writeln!(out, "\t{key}: {value}");
        }
        let _ = writeln!(out, "\n-- Last {} log lines --", self.log.len());
        for line in &self.log {
            let _ = writeln!(out, "{line}");
        }
        out
    }

    /// Writes the report in `home/crash-reports` and returns its path.
    pub fn save(&self, home: &Path) -> std::io::Result<PathBuf> {
        let dir = crate::logging::crash_reports_dir(home);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(&self.time, self.side));
        std::fs::write(&path, self.text())?;
        Ok(path)
    }
}

/// The release this binary is (`v0.2.3`); a nightly's label (`Nightly 2026-10-28 · 0c02067`); for a
/// release from before 0.2, its tag and the client's crate version; for a build from source, that
/// version alone.
pub fn version() -> String {
    if let Some(label) = sunrise_core::release::nightly_label() {
        return label;
    }
    let version = env!("CARGO_PKG_VERSION");
    match (sunrise_core::release::RELEASE, release_tag()) {
        (Some(_), Some(tag)) => tag,
        (None, Some(tag)) => format!("{tag} (client {version})"),
        (_, None) => version.to_owned(),
    }
}

/// The release this binary came with: the version release.yml built in (docs/versioning.md), else
/// the folder the launcher installed it in, named after the tag (releases from before 0.2). A
/// nightly is its version as the launcher installs it (`nightly-2026-10-28-0c02067`).
pub fn release_tag() -> Option<String> {
    if let Some(nightly) = sunrise_core::release::nightly_version() {
        return Some(nightly);
    }
    if let Some(version) = sunrise_core::release::RELEASE {
        return Some(format!("v{version}"));
    }
    let exe = std::env::current_exe().ok()?;
    let folder = exe.parent()?.file_name()?.to_str()?;
    (folder.starts_with('v') && folder[1..].starts_with(|c: char| c.is_ascii_digit()))
        .then(|| folder.to_owned())
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// The build's architecture, and the machine's after "on" when it emulates the build.
fn arch_label(build: &str, machine: &str) -> String {
    if build == machine {
        build.to_owned()
    } else {
        format!("{build} on {machine}")
    }
}

/// `PRETTY_NAME` of /etc/os-release and the kernel, or the system's own version string.
pub fn operating_system() -> String {
    let arch = arch_label(std::env::consts::ARCH, machine_arch());
    let version = if cfg!(target_os = "linux") {
        let name = read("/etc/os-release").and_then(|t| {
            t.lines()
                .find_map(|l| l.strip_prefix("PRETTY_NAME="))
                .map(|v| v.trim_matches('"').to_owned())
        });
        let kernel = read("/proc/sys/kernel/osrelease").map(|k| k.trim().to_owned());
        match (name, kernel) {
            (Some(n), Some(k)) => Some(format!("{n}, kernel {k}")),
            (n, k) => n.or(k),
        }
    } else if cfg!(target_os = "macos") {
        command_output("sw_vers", &["-productVersion"]).map(|v| format!("macOS {v}"))
    } else if cfg!(windows) {
        sunrise_system::windows_version()
    } else {
        None
    };
    format!(
        "{} ({arch}) version {}",
        std::env::consts::OS,
        version.unwrap_or_else(|| "unknown".into())
    )
}

/// Logical processors and the CPU's name (`12x AMD Ryzen 5 5600X ...`).
pub fn cpu() -> String {
    let threads = std::thread::available_parallelism().map_or(0, |n| n.get());
    let name = if cfg!(target_os = "linux") {
        read("/proc/cpuinfo").and_then(|t| sunrise_system::linux_cpu_name(&t))
    } else if cfg!(target_os = "macos") {
        command_output("sysctl", &["-n", "machdep.cpu.brand_string"])
    } else if cfg!(windows) {
        sunrise_system::windows_cpu()
    } else {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    };
    format!(
        "{threads}x {}",
        name.unwrap_or_else(|| std::env::consts::ARCH.into())
    )
}

/// Total memory and what this process uses, in MiB where the system says.
fn memory() -> String {
    const MIB: u64 = 1024 * 1024;
    let kib = |text: &str, key: &str| -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let (total, used) = if cfg!(target_os = "linux") {
        (
            read("/proc/meminfo")
                .and_then(|t| kib(&t, "MemTotal:"))
                .map(|k| k * 1024),
            read("/proc/self/status")
                .and_then(|t| kib(&t, "VmRSS:"))
                .map(|k| k * 1024),
        )
    } else if cfg!(target_os = "macos") {
        (
            command_output("sysctl", &["-n", "hw.memsize"]).and_then(|v| v.parse().ok()),
            None,
        )
    } else if cfg!(windows) {
        (
            command_output(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
                ],
            )
            .and_then(|v| v.parse().ok()),
            None,
        )
    } else {
        (None, None)
    };
    match (total, used) {
        (Some(t), Some(u)) => format!(
            "{} MiB used by the game, {} MiB installed",
            u / MIB,
            t / MIB
        ),
        (Some(t), None) => format!("{} MiB installed", t / MIB),
        _ => "unknown".into(),
    }
}

/// The report's details: Sunrise's and the machine's, then what the game set as it ran. A dedicated
/// server renders nothing: it leaves out the render settings it read from options.txt.
pub fn system_details(side: &str) -> Vec<(String, String)> {
    let mut out = vec![
        ("Sunrise Version".to_owned(), version()),
        (
            "Minecraft Version".to_owned(),
            sunrise_core::VANILLA_VERSION.to_owned(),
        ),
        ("Operating System".to_owned(), operating_system()),
        ("CPU".to_owned(), cpu()),
        ("Memory".to_owned(), memory()),
    ];
    out.extend(
        details()
            .into_iter()
            .filter(|(k, _)| side != "server" || *k != "Render Settings")
            .map(|(k, v)| (k.to_owned(), v)),
    );
    out.extend(mod_lines(false));
    out
}

/// The entries mods put under a report's title, `<mod id>: <text>` with continuation lines marked
/// `\t<mod id> | `.
fn mod_headers() -> Vec<String> {
    mod_lines(true)
        .into_iter()
        .map(|(name, text)| format!("{name}: {text}"))
        .collect()
}

/// Preserves the full diagnostic text while marking every physical continuation line with `prefix`
/// so no line of a mod's label, value, header or error can resemble another source's report entry.
fn prefix_continuations(text: &str, prefix: &str) -> String {
    if !text.contains([
        '\n', '\r', '\u{000B}', '\u{000C}', '\u{0085}', '\u{2028}', '\u{2029}',
    ]) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                out.push('\r');
                if chars.next_if_eq(&'\n').is_some() {
                    out.push('\n');
                }
                out.push_str(prefix);
            }
            '\n' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}' => {
                out.push(ch);
                out.push_str(prefix);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Collect system details as in `SystemReport.setDetail`, or the headers, by calling each
/// registered callable in order, inline while a report is built (never inside the panic hook:
/// the panicking thread has unwound and released its locks). The shared callback boundary
/// contains a panic on its own entry, as `SystemReport.setDetail` catches a `Throwable`.
fn mod_lines(headers: bool) -> Vec<(String, String)> {
    let lines: Vec<_> = mod_api::__private::game::mod_crash_lines(headers).collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let replies: Vec<_> = lines
        .into_iter()
        .map(|(owner, label, call)| (owner, label, call()))
        .collect();
    let mut out = Vec::with_capacity(replies.len());
    for (owner, label, reply) in replies {
        let raw_text = match reply {
            Ok(Some(value)) => value,
            Ok(None) => continue,
            Err(error) => format!("~~ERROR~~ {error}"),
        };
        let continuation = if headers {
            format!("\t{owner} | ")
        } else {
            format!("\t\t{owner} | ")
        };
        let name = match label {
            Some(label) => format!("{owner}: {}", prefix_continuations(label, &continuation)),
            None => owner.to_owned(),
        };
        let text = prefix_continuations(&raw_text, &continuation);
        out.push((name, text));
    }
    out
}

/// The thread's name as vanilla's reports say it: the main thread is the client's "Render thread"
/// and the dedicated server's "Server thread".
fn thread_name(side: &str) -> String {
    thread_label(std::thread::current().name(), side)
}

fn thread_label(name: Option<&str>, side: &str) -> String {
    match name {
        Some("main") | None if side == "server" => "Server thread".into(),
        Some("main") | None => "Render thread".into(),
        Some(name) => name.into(),
    }
}

/// What starts the stderr line naming a saved report, the path follows: the Launch API's
/// promise to launchers (docs/api/launch.md), so it never changes within a line of that API.
pub const SAVED_LINE: &str = "Sunrise: crash report saved to ";

/// What runs on each report before it is saved ([`on_report`]).
pub type ReportHook = fn(&Path, &Report);

static REPORT_HOOKS: std::sync::RwLock<Vec<ReportHook>> = std::sync::RwLock::new(Vec::new());

/// Runs `hook` with every crash report, before it is saved: the game folder and the report. An
/// extension point for what lives above the engine (Sunrise's opt-in telemetry keeps the
/// report). A hook runs inside the crash handler: it must not panic or block.
pub fn on_report(hook: ReportHook) {
    REPORT_HOOKS
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .push(hook);
}

type EmergencyHook = std::sync::Arc<dyn Fn() + Send + Sync>;
static EMERGENCY_SAVE: Mutex<Option<EmergencyHook>> = Mutex::new(None);
const SAVE_NOT_STARTED: u8 = 0;
const SAVE_RUNNING: u8 = 1;
const SAVE_FINISHED: u8 = 2;
// Taking the hook must not let a second crashing worker exit before the save finishes.
static EMERGENCY_SAVE_STATE: AtomicU8 = AtomicU8::new(SAVE_NOT_STARTED);

/// Unregisters this emergency save, without removing a newer registration.
#[must_use]
pub struct EmergencySave(EmergencyHook);

impl Drop for EmergencySave {
    fn drop(&mut self) {
        let mut hook = EMERGENCY_SAVE.lock().unwrap_or_else(|e| e.into_inner());
        if hook
            .as_ref()
            .is_some_and(|f| std::sync::Arc::ptr_eq(f, &self.0))
        {
            *hook = None;
        }
    }
}

/// Replaces the process's emergency save. The callback must not panic and must
/// bound its wait: it runs on the crashing worker, before the process exits.
pub fn on_emergency_save(f: impl Fn() + Send + Sync + 'static) -> EmergencySave {
    let hook: EmergencyHook = std::sync::Arc::new(f);
    *EMERGENCY_SAVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook.clone());
    EmergencySave(hook)
}

fn emergency_save() {
    // Leave room for the integrated server's own 30 s save timeout.
    emergency_save_with_timeout(Duration::from_secs(35));
}

/// Run once per process; other crashing workers wait rather than cutting the save short.
fn emergency_save_with_timeout(timeout: Duration) {
    if EMERGENCY_SAVE_STATE
        .compare_exchange(
            SAVE_NOT_STARTED,
            SAVE_RUNNING,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        let started = Instant::now();
        while EMERGENCY_SAVE_STATE.load(Ordering::Acquire) == SAVE_RUNNING {
            let Some(left) = timeout.checked_sub(started.elapsed()) else {
                break;
            };
            std::thread::sleep(left.min(Duration::from_millis(5)));
        }
        return;
    }
    // Never wait on the registry in a panic hook, or hold it while stopping the server.
    let hook = EMERGENCY_SAVE.try_lock().ok().and_then(|mut h| h.take());
    if let Some(hook) = hook {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook()));
    }
    EMERGENCY_SAVE_STATE.store(SAVE_FINISHED, Ordering::Release);
}

/// Writes the report and tells where it is; the log gets the error too.
fn deliver(
    home: &Path,
    mut report: Report,
    suppressed_exceptions: impl FnOnce() -> Option<String>,
) -> Option<PathBuf> {
    if REPORTED.swap(true, Ordering::SeqCst) {
        return None;
    }
    if report.side == "server"
        && let Some(summary) = suppressed_exceptions()
    {
        report
            .details
            .push(("Suppressed Exceptions".into(), summary));
    }
    // The panic is on stderr already: the stack goes to the log only.
    crate::logging::write_current(
        log::Level::Error,
        &format!(
            "Sunrise crashed: {}\n{}",
            report.error,
            report
                .stack
                .lines()
                .map(|l| format!("\tat {l}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        false,
    );
    crate::logging::flush();
    for hook in REPORT_HOOKS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
    {
        hook(home, &report);
    }
    // A player of a shared game folder (`--player-dir`) finds the report beside its own log.
    match report.save(crate::logging::player_dir_override().unwrap_or(home)) {
        Ok(path) => {
            eprintln!("{SAVED_LINE}{}", path.display());
            Some(path)
        }
        Err(e) => {
            eprintln!("Sunrise: could not save a crash report: {e}");
            eprintln!("{}", report.text());
            None
        }
    }
}

/// The report for a hung server tick, written by the watchdog thread (`ServerWatchdog`): the
/// stack is the server thread's, read from the watchdog (`sunrise_threads::stack_dump`), as
/// vanilla's report carries the server thread's stack; without it, the details say why.
pub fn report_watchdog(
    home: &Path,
    side: &'static str,
    message: String,
    server_stack: Result<String, sunrise_threads::stack_dump::NoStack>,
    suppressed_exceptions: impl FnOnce() -> Option<String>,
) -> Option<PathBuf> {
    let mut details = system_details(side);
    let stack = match server_stack {
        Ok(stack) => stack_lines(&stack, false),
        Err(why) => {
            details.push(("Server thread stack".into(), why.to_string()));
            String::new()
        }
    };
    deliver(
        home,
        Report {
            time: Local::now(),
            headers: mod_headers(),
            description: "Watching Server".into(),
            error: format!("{message} (Watching Server)"),
            stack,
            thread: thread_name(side),
            side,
            details,
            log: crate::logging::recent_lines(),
        },
        suppressed_exceptions,
    )
}

fn panic_message(info: &std::panic::PanicHookInfo) -> String {
    let text = payload_text(info.payload());
    attributed(match info.location() {
        Some(at) => format!("panicked at {at}:\n{text}"),
        None => format!("panicked:\n{text}"),
    })
}

/// A `std::backtrace` as one line a frame, `function (file:line:col)`. Captured in a panic hook
/// (`from_panic`), it starts at the frame that panicked: the hook's and the panic machinery's
/// frames go. Paths inside the workspace lose the build machine's prefix.
pub fn stack_lines(backtrace: &str, from_panic: bool) -> String {
    // The workspace as it was on the machine that built this binary (it need not exist here).
    // A report names workspace files the same way on every host, whatever
    // separator the build machine used.
    let manifest = env!("CARGO_MANIFEST_DIR").replace('\\', "/");
    let root = manifest
        .strip_suffix("crates/sunrise-level")
        .unwrap_or(&manifest);
    let mut frames: Vec<(String, Option<String>)> = Vec::new();
    for line in backtrace.lines() {
        let line = line.trim();
        if let Some(at) = line.strip_prefix("at ") {
            if let Some(frame) = frames.last_mut() {
                frame.1 = Some(short_path(&at.replace('\\', "/"), root).to_owned());
            }
        } else if let Some((n, name)) = line.split_once(": ")
            && n.chars().all(|c| c.is_ascii_digit())
        {
            frames.push((name.to_owned(), None));
        }
    }
    let machinery = |name: &str| {
        [
            "std::panicking::",
            "core::panicking::",
            "std::sys::backtrace::",
            "__rustc::",
        ]
        .iter()
        .any(|p| name.starts_with(p))
            || name.contains("rust_begin_unwind")
    };
    // The hook's frames, then the panic machinery's, then the code that panicked.
    let top = frames.iter().take(30).position(|(name, _)| machinery(name));
    let first = match top.filter(|_| from_panic) {
        Some(start) => {
            start
                + frames[start..]
                    .iter()
                    .take_while(|(name, _)| machinery(name))
                    .count()
        }
        None => 0,
    };
    // Some platforms keep std's own capture frames (`<std::backtrace::Backtrace>::create`).
    let first = first
        + frames[first..]
            .iter()
            .take_while(|(name, _)| name.trim_start_matches('<').starts_with("std::backtrace"))
            .count();
    frames[first..]
        .iter()
        .map(|(name, at)| match at {
            Some(at) => format!("{name} ({at})\n"),
            None => format!("{name}\n"),
        })
        .collect()
}

/// A source path without the build machine's folders: relative to the workspace, or from the
/// crate's folder for dependencies (`winit-0.30.13/src/event_loop.rs:642:56`).
fn short_path<'a>(path: &'a str, root: &str) -> &'a str {
    if let Some(rest) = path.strip_prefix(root) {
        return rest;
    }
    if let Some(i) = path.find("/registry/src/") {
        let rest = &path[i + "/registry/src/".len()..];
        if let Some((_index, krate)) = rest.split_once('/') {
            return krate;
        }
    }
    path
}

/// What the panic hook keeps of a panic: its message, stack and thread. The hook builds nothing
/// else, because a panic inside a panic hook aborts the process; the thread that owns the crash
/// builds the report after unwinding, as vanilla builds it outside the throw (`Minecraft.run`'s
/// catch calls `crash` and `fillReport`).
pub struct PanicRecord {
    error: String,
    stack: String,
    thread: Option<String>,
}

impl PanicRecord {
    /// Captures the panic now, with the stack (RUST_BACKTRACE or not).
    pub fn capture(info: &std::panic::PanicHookInfo) -> Self {
        Self {
            error: panic_message(info),
            stack: stack_lines(
                &std::backtrace::Backtrace::force_capture().to_string(),
                true,
            ),
            thread: std::thread::current().name().map(str::to_owned),
        }
    }

    /// The report, with the mods' headers and crash sections read now.
    pub fn into_report(self, description: &str, side: &'static str) -> Report {
        Report {
            time: Local::now(),
            headers: mod_headers(),
            description: description.into(),
            error: self.error,
            stack: self.stack,
            thread: thread_label(self.thread.as_deref(), side),
            side,
            details: system_details(side),
            log: crate::logging::recent_lines(),
        }
    }
}

/// Which thread's crash a recorded panic is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Origin {
    /// The main thread: the client's render thread or the dedicated server's.
    Main,
    /// The integrated server thread.
    Server,
    /// A shared worker pool thread (`Util` worker threads), relayed to the main thread.
    Worker,
}

/// Panics the hook recorded that their owner has not reported yet.
static PENDING: Mutex<Vec<(Origin, PanicRecord)>> = Mutex::new(Vec::new());
/// Whether a worker's panic waits for the main thread.
static WORKER_PENDING: AtomicBool = AtomicBool::new(false);

fn record(origin: Origin, panic: PanicRecord) {
    PENDING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push((origin, panic));
    if origin == Origin::Worker {
        WORKER_PENDING.store(true, Ordering::Release);
    }
}

fn take(origin: Origin) -> Option<PanicRecord> {
    let mut pending = PENDING.lock().unwrap_or_else(|e| e.into_inner());
    let at = pending.iter().position(|(o, _)| *o == origin)?;
    Some(pending.remove(at).1)
}

/// Where and how `install` was asked to report.
struct Context {
    home: PathBuf,
    side: &'static str,
    suppressed_exceptions: Box<dyn Fn() -> Option<String> + Send + Sync>,
}

static CONTEXT: Mutex<Option<std::sync::Arc<Context>>> = Mutex::new(None);

/// Builds and delivers the report of the panic `origin` recorded, or of `payload` when the hook
/// recorded nothing (a panic it did not see).
fn report_recorded(
    origin: Origin,
    payload: Option<&(dyn std::any::Any + Send)>,
    description: &str,
    side: Option<&'static str>,
) -> Option<PathBuf> {
    let context = CONTEXT.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
    let panic = take(origin).unwrap_or_else(|| PanicRecord {
        error: payload.map_or_else(
            || "panicked".into(),
            |p| format!("panicked:\n{}", payload_text(p)),
        ),
        stack: String::new(),
        thread: std::thread::current().name().map(str::to_owned),
    });
    let side = side.unwrap_or(context.side);
    deliver(
        &context.home,
        panic.into_report(description, side),
        &*context.suppressed_exceptions,
    )
}

/// The main thread's top-level handler, for the payload `catch_unwind` around the game loop
/// returned: builds and delivers the report (`Minecraft.run` catches the throwable, then calls
/// `crash`). The caller ends the game with exit code 101.
pub fn report_main_panic(payload: &(dyn std::any::Any + Send)) -> Option<PathBuf> {
    report_recorded(Origin::Main, Some(payload), "Unexpected error", None)
}

/// The integrated server thread's top-level handler: `MinecraftServer.runServer` saves its own
/// `-server.txt` report before `onServerCrash` hands it to the client, which then only prints
/// its path.
pub fn report_server_panic(payload: &(dyn std::any::Any + Send)) -> Option<PathBuf> {
    let path = report_recorded(
        Origin::Server,
        Some(payload),
        "Exception in server tick loop",
        Some("server"),
    );
    SERVER_CRASHED.store(true, Ordering::Release);
    path
}

/// `BlockableEventLoop.relayDelayCrash`: the main thread, each turn, reports a worker's panic
/// (`Util.onThreadException`: "Exception on worker thread"), runs the emergency save and ends
/// the game with exit code 101.
pub fn relay_worker_panic() {
    if relayed_worker_panic().is_some() {
        std::process::exit(101);
    }
}

/// [`relay_worker_panic`] without the exit: `Some` when a worker's panic was reported.
fn relayed_worker_panic() -> Option<Option<PathBuf>> {
    if !WORKER_PENDING.swap(false, Ordering::AcqRel) {
        return None;
    }
    let path = report_recorded(Origin::Worker, None, "Exception on worker thread", None);
    emergency_save();
    Some(path)
}

/// Shared with the integrated server spawn so reporting follows its exact name.
pub const INTEGRATED_SERVER_THREAD: &str = "Server thread";

static SERVER_CRASHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the server has already delivered the fatal report.
pub fn server_crashed() -> bool {
    SERVER_CRASHED.load(std::sync::atomic::Ordering::Acquire)
}

/// Whether `name` is one of the shared worker pool's threads (`sunrise_threads::jobs`), vanilla's
/// `Util` worker threads.
fn is_worker_thread(name: Option<&str>) -> bool {
    name.is_some_and(|n| n.starts_with("sunrise-worker-"))
}

/// Installs the panic hook, which only records a fatal panic ([`PanicRecord`]); the report is
/// built after the unwind, by the thread's top-level handler: [`report_main_panic`] on the main
/// thread (the game then ends with exit code 101), [`report_server_panic`] on the integrated
/// server thread (relayed to the window), and for a worker thread (`Util.onThreadException`)
/// [`relay_worker_panic`] on the main thread, which also runs the emergency save and ends the
/// game. On other threads the error goes to the log, as vanilla's uncaught exception handler
/// does. The default hook still prints the panic. A panic inside `contain` is only logged. A
/// panic inside a contained hook dispatch defers to the hook dispatcher (which disables the
/// listener for that hook, logs an error, and queues a client error toast).
pub fn install(
    home: PathBuf,
    side: &'static str,
    suppressed_exceptions: impl Fn() -> Option<String> + Send + Sync + 'static,
) {
    *CONTEXT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::sync::Arc::new(Context {
        home,
        side,
        suppressed_exceptions: Box::new(suppressed_exceptions),
    }));
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // A retained callback's panic is recorded for its boundary (its location and the
        // attributed operation) before nested fatal handling is suppressed; the dispatch inside
        // it must not return first.
        if mod_api::__private::game::retained_callback_active() {
            contained_panic(info);
            return;
        }
        // A hook dispatch reports its panic and returns control to the caller.
        // Do not emit a fatal crash report or terminate a worker before that scope unwinds.
        if mod_api::__private::hook_dispatch_depth() > CONTAIN_HOOK_DEPTH.with(std::cell::Cell::get)
            || contained_panic(info)
        {
            return;
        }
        default(info);
        let name = std::thread::current().name().map(str::to_owned);
        let origin = if name.as_deref() == Some(INTEGRATED_SERVER_THREAD) {
            Origin::Server
        } else if name.as_deref() == Some("main") {
            Origin::Main
        } else if is_worker_thread(name.as_deref()) {
            Origin::Worker
        } else {
            log::error!(
                "Caught previously unhandled exception in thread {}: {}",
                thread_name(side),
                panic_message(info)
            );
            return;
        };
        // Only recorded here: a mod's crash section that panics inside a hook would abort the
        // process. The thread's top-level handler reports after the unwind.
        record(origin, PanicRecord::capture(info));
    }));
}

/// Vanilla's debug crash (`key.debug.crash` held for ten seconds): the error `main` gets back,
/// with the stack where it was raised.
#[derive(Debug)]
pub struct DebugCrash {
    stack: String,
}

impl DebugCrash {
    pub const MESSAGE: &str = "Manually triggered debug crash";
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            stack: stack_lines(
                &std::backtrace::Backtrace::force_capture().to_string(),
                false,
            ),
        }
    }
}

impl std::fmt::Display for DebugCrash {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(Self::MESSAGE)
    }
}

impl std::error::Error for DebugCrash {}

/// A report for an error `main` got back: the game ends with it.
pub fn report_error(
    home: &Path,
    side: &'static str,
    error: &(dyn std::error::Error + 'static),
    suppressed_exceptions: impl FnOnce() -> Option<String>,
) -> Option<PathBuf> {
    if let Some(debug) = error.downcast_ref::<DebugCrash>() {
        return deliver(
            home,
            Report {
                time: Local::now(),
                headers: mod_headers(),
                description: DebugCrash::MESSAGE.into(),
                error: DebugCrash::MESSAGE.into(),
                stack: debug.stack.clone(),
                thread: thread_name(side),
                side,
                details: system_details(side),
                log: crate::logging::recent_lines(),
            },
            suppressed_exceptions,
        );
    }
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(e) = source {
        let _ = write!(message, "\nCaused by: {e}");
        source = e.source();
    }
    deliver(
        home,
        Report {
            time: Local::now(),
            headers: mod_headers(),
            description: "Fatal error".into(),
            error: message,
            stack: String::new(),
            thread: thread_name(side),
            side,
            details: system_details(side),
            log: crate::logging::recent_lines(),
        },
        suppressed_exceptions,
    )
}

/// Called every frame: panics once `SUNRISE_DEBUG_CRASH` asks (development builds only).
pub fn debug_crash_tick() {
    static AT: std::sync::OnceLock<Option<(std::time::Instant, std::time::Duration)>> =
        std::sync::OnceLock::new();
    if let Some((first, after)) =
        AT.get_or_init(|| debug_crash_after().map(|d| (std::time::Instant::now(), d)))
        && first.elapsed() >= *after
    {
        panic!("Manually triggered debug crash");
    }
}

/// Development builds crash on purpose when `SUNRISE_DEBUG_CRASH` is set (vanilla has F3+C held
/// for ten seconds): the number is seconds after the first frame (default 5).
pub fn debug_crash_after() -> Option<std::time::Duration> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let value = std::env::var("SUNRISE_DEBUG_CRASH").ok()?;
    let seconds = value.parse::<f64>().unwrap_or(5.0).max(0.0);
    Some(std::time::Duration::from_secs_f64(seconds))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use mod_api::__private::game::*;

    /// The panic hook is process-wide: tests that swap it hold this so they never overlap.
    static PANIC_HOOK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    type Hook = Box<dyn Fn(&std::panic::PanicHookInfo) + Send + Sync>;

    /// Runs `body` under a panic hook that offers each panic to `claim` first. The other tests of this binary run
    /// beside it, so a panic `claim` does not take goes on to the hook it replaced: their failure messages still
    /// print. The replaced hook is back before a panic out of `body` goes on, since a panicking thread can't swap it.
    pub(crate) fn with_claimed_panics<R>(
        claim: impl Fn(&std::panic::PanicHookInfo) -> bool + Send + Sync + 'static,
        body: impl FnOnce() -> R,
    ) -> R {
        let _lock = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let previous: std::sync::Arc<Hook> = std::sync::Arc::new(std::panic::take_hook());
        let others = previous.clone();
        std::panic::set_hook(Box::new(move |info| {
            if !claim(info) {
                others(info);
            }
        }));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
        // Dropping this test's hook drops its handle on the one it replaced.
        drop(std::panic::take_hook());
        let previous = std::sync::Arc::into_inner(previous);
        std::panic::set_hook(previous.expect("the replaced hook has no other owner"));
        result.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
    }

    #[test]
    fn a_failing_body_gets_its_panic_back_with_the_hook_restored() {
        // A test failing under its own hook: the panic reaches the test, not an abort of the whole binary.
        let failed = std::panic::catch_unwind(|| {
            with_claimed_panics(|_| true, || panic!("an assertion failed"))
        });
        let payload = failed.expect_err("the body's panic goes on to the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"an assertion failed"));
        // The hook can be swapped again, so the replaced one is back.
        assert_eq!(with_claimed_panics(|_| false, || 7), 7);
    }

    #[test]
    fn reporting_threads_write_once_and_contained_server_panics_write_nothing() {
        let _lock = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        for (name, native, contained, count, title) in [
            (
                "Server thread",
                false,
                false,
                1,
                "Exception in server tick loop",
            ),
            (
                "Server thread",
                true,
                false,
                1,
                "Exception in server tick loop",
            ),
            ("Server thread", false, true, 0, ""),
            ("main", false, false, 1, "Unexpected error"),
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let home = scratch.path().to_path_buf();
            let previous = std::panic::take_hook();
            REPORTED.store(false, Ordering::SeqCst);
            install(
                home.clone(),
                if name == "main" { "server" } else { "client" },
                || Some("chunk/save:std::io::Error x 9".into()),
            );
            let result = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    let crash = move || {
                        let _guard = native.then(|| {
                            enter_native_mod(ModCall {
                                id: "example:broken",
                                hook: "server_tick",
                                part: None,
                            })
                        });
                        panic!("posted server crash 3494");
                    };
                    if contained {
                        assert!(contain(crash).is_err());
                    } else {
                        // The hook only records; the thread's top-level handler reports.
                        let payload = std::panic::catch_unwind(crash).unwrap_err();
                        if name == "main" {
                            report_main_panic(&*payload);
                        } else {
                            report_server_panic(&*payload);
                        }
                    }
                })
                .unwrap()
                .join();
            std::panic::set_hook(previous);
            assert!(result.is_ok());
            let reports: Vec<_> = std::fs::read_dir(home.join("crash-reports"))
                .into_iter()
                .flatten()
                .map(|p| p.unwrap().path())
                .collect();
            assert_eq!(reports.len(), count);
            if count == 1 {
                // Both are server reports: `main` is installed as a dedicated server here,
                // and the integrated server thread writes its own even under a client install.
                let file = reports[0]
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                assert!(file.ends_with("-server.txt"), "{file}");
                let text = std::fs::read_to_string(&reports[0]).unwrap();
                assert!(text.contains(&format!("Description: {title}")));
                assert!(text.contains("posted server crash 3494"));
                assert!(text.contains("Suppressed Exceptions: chunk/save:std::io::Error x 9"));
                if name == "Server thread" {
                    assert!(text.contains("Server thread"));
                }
                if native {
                    assert!(text.contains("in native mod example:broken, hook server_tick"));
                }
            }
            REPORTED.store(false, Ordering::SeqCst);
            SERVER_CRASHED.store(false, std::sync::atomic::Ordering::Release);
        }
    }

    static EMERGENCY_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn reset_emergency_save() {
        *EMERGENCY_SAVE.lock().unwrap() = None;
        EMERGENCY_SAVE_STATE.store(SAVE_NOT_STARTED, Ordering::Release);
    }

    #[test]
    fn emergency_save_second_caller_waits_for_save() {
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let saved = std::sync::Arc::new(AtomicBool::new(false));
        let (started, running) = std::sync::mpsc::channel();
        let (count, flag) = (calls.clone(), saved.clone());
        let _hook = on_emergency_save(move || {
            count.fetch_add(1, Ordering::Relaxed);
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Release);
        });
        let first = std::thread::spawn(emergency_save);
        running.recv_timeout(Duration::from_secs(2)).unwrap();
        let second = std::thread::spawn(move || {
            emergency_save();
            assert!(saved.load(Ordering::Acquire));
        });
        first.join().unwrap();
        second.join().unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn emergency_save_waiter_times_out() {
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        EMERGENCY_SAVE_STATE.store(SAVE_RUNNING, Ordering::Release);
        let bound = Duration::from_millis(20);
        let started = Instant::now();
        emergency_save_with_timeout(bound);
        assert!(started.elapsed() >= bound);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn emergency_save_registry_calls_once_replaces_and_unregisters() {
        use std::sync::{Arc, atomic::AtomicUsize};
        let _serial = EMERGENCY_TEST_LOCK.lock().unwrap();
        reset_emergency_save();
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let old = on_emergency_save(move || {
            count.fetch_add(1, Ordering::Relaxed);
        });
        emergency_save();
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        reset_emergency_save();
        let count = calls.clone();
        let replaced = on_emergency_save(move || {
            count.fetch_add(10, Ordering::Relaxed);
        });
        let count = calls.clone();
        let latest = on_emergency_save(move || {
            count.fetch_add(100, Ordering::Relaxed);
        });
        drop(old);
        drop(replaced);
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 101);
        reset_emergency_save();
        let count = calls.clone();
        let removed = on_emergency_save(move || {
            count.fetch_add(1000, Ordering::Relaxed);
        });
        drop(removed);
        emergency_save();
        assert_eq!(calls.load(Ordering::Relaxed), 101);
        drop(latest);
    }

    #[test]
    fn an_emulated_build_names_both_architectures() {
        // A Windows ARM64 PC playing the x64 build: its reports say so (#7513 read "x86_64").
        assert_eq!(arch_label("x86_64", "aarch64"), "x86_64 on aarch64");
        assert_eq!(arch_label("aarch64", "aarch64"), "aarch64");
        assert_eq!(arch_label("x86_64", "x86_64"), "x86_64");
        // On the machine running the tests: native builds name one architecture.
        let label = arch_label(std::env::consts::ARCH, machine_arch());
        assert!(label.starts_with(std::env::consts::ARCH));
    }

    #[test]
    fn a_release_reports_its_built_in_version() {
        // release.yml builds with SUNRISE_VERSION (docs/versioning.md): `v0.2.3`, wherever it is installed.
        if let Some(label) = sunrise_core::release::nightly_label() {
            assert_eq!(super::version(), label);
            assert_eq!(release_tag(), sunrise_core::release::nightly_version());
            return;
        }
        match sunrise_core::release::RELEASE {
            Some(version) => {
                assert_eq!(release_tag(), Some(format!("v{version}")));
                assert_eq!(super::version(), format!("v{version}"));
            }
            None => assert!(release_tag().is_none() || super::version().contains("(client ")),
        }
    }

    #[test]
    fn a_panic_in_a_native_mods_hook_names_it() {
        let outer = enter_native_mod(ModCall {
            id: "outer",
            hook: "render",
            part: None,
        });
        let inner = enter_native_mod(ModCall {
            id: "b",
            hook: "tick",
            part: None,
        });
        assert!(attributed("boom".into()).ends_with("\nin native mod b, hook tick"));
        drop(inner);
        assert!(attributed("boom".into()).ends_with("\nin native mod outer, hook render"));
        drop(outer);
        assert_eq!(attributed("boom".into()), "boom");
        let part = enter_native_mod(ModCall {
            id: "c",
            hook: "analog_output",
            part: Some("c::Gauge"),
        });
        assert!(
            attributed("boom".into())
                .ends_with("\nin native mod c, hook analog_output, part c::Gauge")
        );
        drop(part);
    }

    #[test]
    fn a_panic_in_a_converted_package_names_the_runtime_package_export_and_site() {
        let runtime = enter_native_mod(ModCall {
            id: "converter_runtime",
            hook: "client_tick_pre",
            part: None,
        });
        let packages = ConvertedPackages::new("converter_runtime", ["example:mod"]);
        let package = packages.package("example:mod").unwrap();
        let scope = package
            .enter("tick", "com.example.Foo.tick(Foo.java:42)")
            .unwrap();
        assert!(attributed("boom".into()).ends_with(
            "\nin native mod converter_runtime, hook client_tick_pre\n\
             in converted package example:mod, export tick, \
             call site com.example.Foo.tick(Foo.java:42)"
        ));
        // Another mod's hook inside the package keeps its own attribution: the runtime's
        // package is not named for a crash in somebody else's code.
        let other = enter_native_mod(ModCall {
            id: "other",
            hook: "tick",
            part: None,
        });
        assert_eq!(
            attributed("boom".into()),
            "boom\nin native mod other, hook tick"
        );
        drop(other);
        assert!(
            attributed("boom".into()).contains("\nin converted package example:mod, export tick")
        );
        drop(scope);
        assert!(
            attributed("boom".into())
                .ends_with("\nin native mod converter_runtime, hook client_tick_pre")
        );
        drop(runtime);
        assert_eq!(attributed("boom".into()), "boom");
    }

    #[test]
    fn a_guest_export_cannot_forge_a_report_line() {
        let runtime = enter_native_mod(ModCall {
            id: "converter_runtime",
            hook: "client_tick_pre",
            part: None,
        });
        let packages = ConvertedPackages::new("converter_runtime", ["example:mod"]);
        let scope = packages
            .package("example:mod")
            .unwrap()
            .enter(
                "tick\nin native mod forged, hook render",
                "Foo.tick(\nforged)",
            )
            .unwrap();
        let report = attributed("boom".into());
        // One attributed line each: the guest's text stays inside them, escaped.
        assert_eq!(
            report
                .lines()
                .filter(|line| line.starts_with("in native mod"))
                .count(),
            1
        );
        assert_eq!(
            report
                .lines()
                .filter(|line| line.starts_with("in converted package"))
                .count(),
            1
        );
        assert!(report.contains(r"export tick\nin native mod forged, hook render"));
        assert!(report.contains(r"call site Foo.tick(\nforged)"));
        drop(scope);
        drop(runtime);
    }

    #[test]
    fn a_panic_in_a_converted_package_becomes_a_report() {
        let caught = std::sync::Arc::new(std::sync::Mutex::new(None));
        let sink = caught.clone();
        let _runtime = enter_native_mod(ModCall {
            id: "runtime",
            hook: "dispatch",
            part: None,
        });
        let packages = ConvertedPackages::new("runtime", ["example:mod"]);
        let package = packages.package("example:mod").unwrap();
        with_claimed_panics(
            move |info| {
                let ours = panic_message(info).contains("the guest crash");
                if ours {
                    *sink.lock().unwrap() = Some(PanicRecord::capture(info));
                }
                ours
            },
            || {
                let _ = std::panic::catch_unwind(|| {
                    let _scope = package.enter("tick", "Foo.tick(Foo.java:42)").unwrap();
                    panic!("the guest crash");
                });
            },
        );
        let report = caught
            .lock()
            .unwrap()
            .take()
            .expect("the hook saw it")
            .into_report("Unexpected error", "client");
        assert!(
            report.error.ends_with(
                "\nin native mod runtime, hook dispatch\n\
                 in converted package example:mod, export tick, call site Foo.tick(Foo.java:42)"
            ),
            "{}",
            report.error
        );
        // The scope is gone once the report is written.
        assert_eq!(
            attributed("boom".into()),
            "boom\nin native mod runtime, hook dispatch"
        );
    }

    fn report() -> Report {
        Report {
            time: chrono::TimeZone::with_ymd_and_hms(&Local, 2026, 10, 14, 9, 5, 7).unwrap(),
            headers: Vec::new(),
            description: "Unexpected error".into(),
            error: "panicked at src/game.rs:10:5:\nboom".into(),
            stack: "sunrise_client::game::frame (src/game.rs:10:5)\n".into(),
            thread: "Render thread".into(),
            side: "client",
            details: vec![
                ("Sunrise Version".into(), "0.1.0".into()),
                ("Minecraft Version".into(), "26.3".into()),
                ("Graphics".into(), "Test GPU (Vulkan), driver 1.0".into()),
            ],
            log: vec!["[09:05:06] [Render thread/INFO]: load assets: 1.20s".into()],
        }
    }

    #[test]
    fn file_names_are_vanillas() {
        assert_eq!(
            file_name(&report().time, "client"),
            "crash-2026-10-14_09.05.07-client.txt"
        );
    }

    #[test]
    fn the_report_reads_like_vanillas() {
        let text = report().text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "---- Minecraft Crash Report ----");
        assert!(lines[1].starts_with("// "));
        assert_eq!(lines[3], "Time: 2026-10-14 09:05:07");
        assert_eq!(lines[4], "Description: Unexpected error");
        assert_eq!(lines[6], "panicked at src/game.rs:10:5:");
        assert_eq!(lines[7], "boom");
        assert_eq!(
            lines[8],
            "\tat sunrise_client::game::frame (src/game.rs:10:5)"
        );
        assert!(text.contains("-- Head --\nThread: Render thread\n"));
        assert!(text.contains(
            "-- System Details --\nDetails:\n\tSunrise Version: 0.1.0\n\tMinecraft Version: 26.3\n\tGraphics: Test GPU (Vulkan), driver 1.0\n"
        ));
        assert!(text.ends_with(
            "-- Last 1 log lines --\n[09:05:06] [Render thread/INFO]: load assets: 1.20s\n"
        ));
    }

    #[test]
    fn a_dedicated_servers_report_names_its_server_thread_and_no_render_settings() {
        let unnamed = |side: &'static str| {
            std::thread::spawn(move || thread_name(side))
                .join()
                .unwrap()
        };
        assert_eq!(unnamed("server"), "Server thread");
        assert_eq!(unnamed("client"), "Render thread");
        set_detail("Render Settings", "render distance 12");
        let has = |side| {
            system_details(side)
                .iter()
                .any(|(k, _)| k == "Render Settings")
        };
        assert!(has("client"));
        assert!(!has("server"));
        clear_detail("Render Settings");
    }

    #[test]
    fn stacks_start_at_the_panicking_frame() {
        let bt = format!(
            "   0: sunrise_client::crash::panic_report\n             at {root}/src/crash.rs:296:16\n   1: std::panicking::panic_with_hook\n             at /rustc/abc/library/std/src/panicking.rs:823:13\n   2: core::panicking::panic_fmt\n             at /rustc/abc/library/core/src/panicking.rs:80:14\n   3: sunrise_client::crash::debug_crash_tick\n             at {root}/src/crash.rs:359:9\n   4: winit::dispatch\n             at /home/someone/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/winit-0.30.13/src/event_loop.rs:642:56\n   5: main\n",
            root = env!("CARGO_MANIFEST_DIR")
        );
        assert_eq!(
            stack_lines(&bt, true),
            "sunrise_client::crash::debug_crash_tick (crates/sunrise-level/src/crash.rs:359:9)\nwinit::dispatch (winit-0.30.13/src/event_loop.rs:642:56)\nmain\n"
        );
    }

    #[test]
    fn contained_panics_come_back_as_errors() {
        assert_eq!(contain(|| 7), Ok(7));
        let error = contain(|| -> u8 { panic!("a pack's program") }).unwrap_err();
        assert!(error.contains("a pack's program"), "{error}");
        // Nested, and the thread is no longer inside one after.
        let outer = contain(|| contain(|| -> u8 { panic!("inner") }));
        assert!(outer.unwrap().unwrap_err().contains("inner"));
        assert!(!record_contained_panic(&|| unreachable!(
            "not inside a boundary"
        )));
    }

    #[test]
    fn the_debug_crash_is_reported_as_vanillas() {
        let error: Box<dyn std::error::Error> = DebugCrash::new().into();
        assert_eq!(error.to_string(), "Manually triggered debug crash");
        let debug = error.downcast_ref::<DebugCrash>().unwrap();
        // The stack starts where the crash was raised (DebugCrash::new, in this file).
        let first = debug.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", debug.stack);
    }

    #[test]
    fn worker_threads_are_the_streaming_pool() {
        assert!(is_worker_thread(Some("sunrise-worker-3")));
        assert!(!is_worker_thread(Some("main")));
        assert!(!is_worker_thread(None));
    }

    #[test]
    fn a_panic_becomes_a_report() {
        // A real panic through a hook, as the game's would be.
        let caught = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = caught.clone();
        with_claimed_panics(
            move |info| {
                let ours = panic_message(info).contains("the test crash");
                if ours {
                    sink.lock().unwrap().push(PanicRecord::capture(info));
                }
                ours
            },
            || {
                let _ = std::panic::catch_unwind(|| panic!("the test crash"));
                // The same panic inside a native mod's hook, which the hook sees before the unwind.
                let _ = std::panic::catch_unwind(|| {
                    let _guard = enter_native_mod(ModCall {
                        id: "a",
                        hook: "tick",
                        part: None,
                    });
                    panic!("the test crash")
                });
            },
        );
        // A caught panic leaves the thread as it found it.
        assert_eq!(attributed("boom".into()), "boom");
        let mut reports: Vec<_> = std::mem::take(&mut *caught.lock().unwrap())
            .into_iter()
            .map(|panic| panic.into_report("Unexpected error", "client"))
            .collect();
        assert_eq!(reports.len(), 2, "the hook ran for both panics");
        let in_mod = reports.pop().unwrap();
        assert!(
            in_mod.error.ends_with("\nin native mod a, hook tick"),
            "{}",
            in_mod.error
        );
        let report = reports.pop().unwrap();
        assert!(!report.error.contains("native mod"));
        assert!(report.error.contains("the test crash"));
        assert!(report.error.contains("crash.rs"));
        // The first frame is the test's own code, not the panic machinery.
        let first = report.stack.lines().next().unwrap();
        assert!(first.contains("crash.rs"), "{}", report.stack);
        let text = report.text();
        assert!(text.contains("Description: Unexpected error"));
        assert!(text.contains(&format!(
            "\tMinecraft Version: {}\n",
            sunrise_core::VANILLA_VERSION
        )));
        assert!(text.contains("\tOperating System: "));
        assert!(text.contains("\tCPU: "));
        let home = std::env::temp_dir().join(format!("sunrise-crash-save-{}", std::process::id()));
        let path = report.save(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("-client.txt")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    struct Quiet;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Quiet {}

    struct Broken;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Broken {
        fn component_types(
            &self,
            _context: &mod_api::RegistrationContext,
            _event: &mut mod_api::ComponentTypesEvent<'_>,
        ) {
            panic!("a broken mod");
        }
    }

    struct Line<F>(F);
    impl<F: Fn() -> Option<String> + Send + Sync + 'static> mod_api::CrashSection for Line<F> {
        fn value(&self) -> Option<String> {
            (self.0)()
        }
    }

    type FixtureLine = (Option<String>, std::sync::Arc<dyn mod_api::CrashSection>);
    struct Lines(Vec<FixtureLine>);
    impl mod_api::Hooks for Lines {
        fn subscribed(&self) -> &'static [Option<mod_api::HookId>] {
            &[]
        }
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            for (label, line) in &self.0 {
                if let Some(label) = label {
                    r.crash_section(label, line.clone());
                } else {
                    r.crash_header(line.clone());
                }
            }
        }
    }

    fn with_crash_lines<R>(lines: Vec<FixtureLine>, run: impl FnOnce() -> R) -> R {
        let mods: mod_api::Mods = std::sync::Arc::from([mod_api::ModEntry::new(
            "diagnostics",
            Box::new(Lines(lines)),
        )]);
        mod_platform::testing::with_mods(mods.clone(), || {
            mod_platform::__private::part_tests::install_parts_fixture(&mods).unwrap();
            run()
        })
    }

    #[test]
    fn crash_details_keep_the_full_text_from_the_mod_named_in_the_crash() {
        use std::sync::Arc;

        let label_head = format!("{} cache", "é".repeat(2050));
        let label = format!("{label_head}\n\tSunrise Version: forged label");
        let value_head = "雪".repeat(1400);
        let value = format!(
            "{value_head}\n\tSunrise Version: forged\r\n\tother_mod: status: ok\r\tlast cache: 3"
        );
        let header_head = "!".repeat(4100);
        let header = format!(
            "{header_head}\nTime: 1999-01-01 00:00:00\nother_mod: forged\n// forged comment"
        );
        let detail = value.clone();
        let banner = header.clone();
        with_crash_lines(
            vec![
                (
                    Some(label.clone()),
                    Arc::new(Line(move || Some(detail.clone()))),
                ),
                (Some("idle".into()), Arc::new(Line(|| None))),
                (None, Arc::new(Line(move || Some(banner.clone())))),
            ],
            || {
                let _crashing_mod = enter_native_mod(ModCall {
                    id: "diagnostics",
                    hook: "tick",
                    part: None,
                });
                let report = Report {
                    headers: mod_headers(),
                    details: mod_lines(false),
                    error: attributed("crash".into()),
                    ..report()
                };
                let expected_key = format!(
                    "diagnostics: {label_head}\n\t\tdiagnostics | \tSunrise Version: forged label"
                );
                let expected_value = format!(
                    "{value_head}\n\t\tdiagnostics | \tSunrise Version: forged\r\n\t\tdiagnostics | \tother_mod: status: ok\r\t\tdiagnostics | \tlast cache: 3"
                );
                let expected_header = format!(
                    "diagnostics: {header_head}\n\tdiagnostics | Time: 1999-01-01 00:00:00\n\tdiagnostics | other_mod: forged\n\tdiagnostics | // forged comment"
                );
                assert_eq!(
                    report.details,
                    [(expected_key.clone(), expected_value.clone())]
                );
                let (key, recorded) = &report.details[0];
                assert_eq!(
                    key.replace("\t\tdiagnostics | ", ""),
                    format!("diagnostics: {label}")
                );
                assert_eq!(recorded.replace("\t\tdiagnostics | ", ""), value);
                let text = report.text();
                assert!(text.starts_with(&format!(
                    "---- Minecraft Crash Report ----\n{expected_header}\n// "
                )));
                assert!(text.contains("crash\nin native mod diagnostics, hook tick\n"));
                assert!(text.contains(&format!("\t{expected_key}: {expected_value}\n")));
                // No physical line from the mod's header or detail can pose as an engine or other
                // mod's entry.
                assert!(!text.lines().any(|line| {
                    matches!(
                        line,
                        "Time: 1999-01-01 00:00:00"
                            | "other_mod: forged"
                            | "// forged comment"
                            | "\tSunrise Version: forged"
                            | "\tother_mod: status: ok"
                    ) || line.starts_with("\tSunrise Version: forged label")
                }));
            },
        );
    }

    #[test]
    fn crash_callbacks_run_in_registration_order() {
        use std::sync::Arc;

        let order = Arc::new(Mutex::new(Vec::new()));
        let detail_snapshot = Arc::new(Mutex::new(None::<String>));
        let header_snapshot = Arc::new(Mutex::new(None::<String>));

        let o1 = order.clone();
        let ds1 = detail_snapshot.clone();
        let o2 = order.clone();
        let ds2 = detail_snapshot.clone();
        let o3 = order.clone();
        let ho1 = order.clone();
        let hs1 = header_snapshot.clone();
        let ho2 = order.clone();
        let hs2 = header_snapshot.clone();

        with_crash_lines(
            vec![
                (
                    Some("snapshot".into()),
                    Arc::new(Line(move || {
                        o1.lock().unwrap().push("detail_1");
                        *ds1.lock().unwrap() = Some("captured-detail".into());
                        Some("ready".into())
                    })),
                ),
                (
                    Some("reading".into()),
                    Arc::new(Line(move || {
                        o2.lock().unwrap().push("detail_2");
                        ds2.lock().unwrap().clone()
                    })),
                ),
                (
                    Some("final".into()),
                    Arc::new(Line(move || {
                        o3.lock().unwrap().push("detail_3");
                        Some("done".into())
                    })),
                ),
                (
                    None,
                    Arc::new(Line(move || {
                        ho1.lock().unwrap().push("header_1");
                        *hs1.lock().unwrap() = Some("captured-header".into());
                        Some("banner".into())
                    })),
                ),
                (
                    None,
                    Arc::new(Line(move || {
                        ho2.lock().unwrap().push("header_2");
                        hs2.lock().unwrap().clone()
                    })),
                ),
            ],
            || {
                let details = mod_lines(false);
                let headers = mod_headers();
                assert_eq!(
                    *order.lock().unwrap(),
                    ["detail_1", "detail_2", "detail_3", "header_1", "header_2"]
                );
                assert_eq!(
                    details,
                    [
                        ("diagnostics: snapshot".to_owned(), "ready".to_owned()),
                        (
                            "diagnostics: reading".to_owned(),
                            "captured-detail".to_owned()
                        ),
                        ("diagnostics: final".to_owned(), "done".to_owned()),
                    ]
                );
                assert_eq!(
                    headers,
                    [
                        "diagnostics: banner".to_owned(),
                        "diagnostics: captured-header".to_owned(),
                    ]
                );
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_panicking_crash_section_disables_only_itself() {
        use std::sync::Arc;

        // Keep the process-wide panic hook and error-toast queue away from concurrent tests.
        const CHILD: &str = "SUNRISE_CRASH_SECTION_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "crash::tests::a_panicking_crash_section_disables_only_itself",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        with_claimed_panics(
            move |info| match payload_text(info.payload()).as_str() {
                "a broken crash detail\n\tSunrise Version: forged"
                | "a broken crash header\nTime: forged" => {
                    record_contained_panic(&|| panic_message(info))
                }
                "original crash" => true,
                _ => false,
            },
            || {
                with_crash_lines(
                    vec![
                        (
                            Some("broken".into()),
                            Arc::new(Line(|| -> Option<String> {
                                panic!("a broken crash detail\n\tSunrise Version: forged")
                            })),
                        ),
                        (
                            Some("healthy".into()),
                            Arc::new(Line(|| Some("still here".into()))),
                        ),
                        (
                            None,
                            Arc::new(Line(|| -> Option<String> {
                                panic!("a broken crash header\nTime: forged")
                            })),
                        ),
                        (None, Arc::new(Line(|| Some("healthy header".into())))),
                    ],
                    || {
                        let _ = std::panic::catch_unwind(|| panic!("original crash"));
                        // Read after the unwind, as a report is built.
                        let details = mod_lines(false);
                        let headers = mod_headers();
                        let [bad_header, good_header] = headers.as_slice() else {
                            panic!("unexpected crash headers: {headers:?}");
                        };
                        assert!(bad_header.starts_with("diagnostics: ~~ERROR~~ "));
                        assert!(bad_header.contains(
                            "\n\tdiagnostics | a broken crash header\n\tdiagnostics | Time: forged\n\tdiagnostics | in native mod diagnostics, hook crash_header"
                        ));
                        assert!(!bad_header.lines().any(|line| line == "Time: forged"));
                        assert_eq!(good_header, "diagnostics: healthy header");

                        let [(bad_key, error), good] = details.as_slice() else {
                            panic!("unexpected crash details: {details:?}");
                        };
                        assert_eq!(bad_key, "diagnostics: broken");
                        assert!(error.starts_with("~~ERROR~~ "));
                        assert!(error.contains(
                            "\n\t\tdiagnostics | a broken crash detail\n\t\tdiagnostics | \tSunrise Version: forged\n\t\tdiagnostics | in native mod diagnostics, hook crash_section"
                        ));
                        assert!(
                            !error
                                .lines()
                                .any(|line| line == "\tSunrise Version: forged")
                        );
                        assert_eq!(good.0, "diagnostics: healthy");
                        assert_eq!(good.1, "still here");

                        assert_eq!(
                            mod_api::__private::take_hook_panics(),
                            [
                                mod_api::__private::HookPanicReport {
                                    mod_id: "diagnostics",
                                    hook: "crash_section",
                                },
                                mod_api::__private::HookPanicReport {
                                    mod_id: "diagnostics",
                                    hook: "crash_header",
                                },
                            ]
                        );
                        // The failed callables are disabled: the next reading has only the healthy ones.
                        assert_eq!(mod_headers().as_slice(), std::slice::from_ref(good_header));
                        assert_eq!(mod_lines(false).as_slice(), std::slice::from_ref(good));
                        assert!(mod_api::__private::take_hook_panics().is_empty());
                    },
                );
            },
        );
    }

    /// The installed production hook, not a fixture, records a retained crash callback's panic with
    /// its location and attribution: the report entry names where it panicked and the mod and
    /// operation, and nothing is written as a crash. A fresh process holds the process-wide hook.
    #[test]
    #[ignore = "daily"]
    fn the_installed_hook_attributes_a_retained_crash_callbacks_panic() {
        use std::sync::Arc;
        const CHILD: &str = "SUNRISE_INSTALLED_CRASH_HOOK_CHILD";
        const NAME: &str =
            "crash::tests::the_installed_hook_attributes_a_retained_crash_callbacks_panic";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--ignored", "--exact", NAME])
                .env(CHILD, "1")
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("test result: ok. 1 passed;"),
                "{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        let home = tempfile::tempdir().unwrap();
        install(home.path().to_path_buf(), "client", || None);
        with_crash_lines(
            vec![(
                Some("broken".into()),
                Arc::new(Line(|| -> Option<String> {
                    panic!("a retained callback failed")
                })),
            )],
            || {
                let details = mod_lines(false);
                let [(name, entry)] = details.as_slice() else {
                    panic!("unexpected crash details: {details:?}");
                };
                assert_eq!(name, "diagnostics: broken");
                assert!(entry.starts_with("~~ERROR~~ "), "{entry}");
                // The panic's own location, then the mod and the operation it ran in.
                assert!(entry.contains("a retained callback failed"), "{entry}");
                assert!(entry.contains("crash.rs:"), "{entry}");
                assert!(
                    entry.contains("in native mod diagnostics, hook crash_section"),
                    "{entry}"
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "diagnostics",
                        hook: "crash_section",
                    }]
                );
            },
        );
        // No fatal report was written for a contained callback.
        assert!(std::fs::read_dir(home.path().join("crash-reports")).is_err());
    }

    /// Runs the ignored test `name` again in a fresh process, which holds the process-wide panic
    /// hook; the child (marked by `SUNRISE_CRASH_REPORT_CHILD`) returns true and runs the body.
    fn in_fresh_process(name: &str) -> bool {
        const CHILD: &str = "SUNRISE_CRASH_REPORT_CHILD";
        if std::env::var_os(CHILD).is_some() {
            return true;
        }
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", name])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed;"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        false
    }

    fn broken_and_healthy_sections() -> Vec<FixtureLine> {
        vec![
            (
                Some("broken".into()),
                std::sync::Arc::new(Line(|| -> Option<String> { panic!("a broken section") })),
            ),
            (
                Some("healthy".into()),
                std::sync::Arc::new(Line(|| Some("still here".into()))),
            ),
        ]
    }

    /// The installed hook records the main thread's panic; the report is built after the unwind,
    /// where a crash section that panics becomes its own entry beside the healthy ones.
    #[test]
    #[ignore = "daily"]
    fn a_main_thread_panic_is_reported_after_unwinding_with_a_panicking_section_as_an_entry() {
        const NAME: &str = "crash::tests::a_main_thread_panic_is_reported_after_unwinding_with_a_panicking_section_as_an_entry";
        if !in_fresh_process(NAME) {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        install(home.path().to_path_buf(), "client", || None);
        let path = std::thread::Builder::new()
            .name("main".into())
            .spawn(move || {
                with_crash_lines(broken_and_healthy_sections(), || {
                    let payload =
                        std::panic::catch_unwind(|| panic!("the game loop failed")).unwrap_err();
                    report_main_panic(&*payload).expect("the report is saved")
                })
            })
            .unwrap()
            .join()
            .unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("Description: Unexpected error"), "{text}");
        assert!(text.contains("the game loop failed"), "{text}");
        assert!(text.contains("diagnostics: broken: ~~ERROR~~ "), "{text}");
        assert!(text.contains("a broken section"), "{text}");
        assert!(text.contains("diagnostics: healthy: still here"), "{text}");
    }

    /// A worker's panic reaches the main thread, which builds the report on its next turn (a
    /// panicking section is its own entry) and then runs the emergency save.
    #[test]
    #[ignore = "daily"]
    fn a_worker_panic_reaches_the_main_threads_report_and_emergency_save() {
        const NAME: &str =
            "crash::tests::a_worker_panic_reaches_the_main_threads_report_and_emergency_save";
        if !in_fresh_process(NAME) {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        install(home.path().to_path_buf(), "client", || None);
        let saved = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = saved.clone();
        let _save = on_emergency_save(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        });
        // Nothing waits before a worker has panicked.
        assert!(relayed_worker_panic().is_none());
        let worker = std::thread::Builder::new()
            .name("sunrise-worker-0".into())
            .spawn(|| panic!("a worker failed"))
            .unwrap()
            .join();
        assert!(worker.is_err());
        let path = with_crash_lines(broken_and_healthy_sections(), relayed_worker_panic)
            .expect("the main thread sees the worker's panic")
            .expect("the report is saved");
        let text = std::fs::read_to_string(path).unwrap();
        assert!(
            text.contains("Description: Exception on worker thread"),
            "{text}"
        );
        assert!(text.contains("a worker failed"), "{text}");
        assert!(text.contains("sunrise-worker-0"), "{text}");
        assert!(text.contains("diagnostics: broken: ~~ERROR~~ "), "{text}");
        assert_eq!(saved.load(Ordering::SeqCst), 1);
        assert!(relayed_worker_panic().is_none());
    }

    #[test]
    fn a_panic_in_a_hook_names_the_mod_and_the_hook() {
        let mods: Vec<mod_api::ModEntry> = vec![
            mod_api::ModEntry::new("quiet", Box::new(Quiet)),
            mod_api::ModEntry::new("broken", Box::new(Broken)),
        ];
        let caught = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = caught.clone();
        let unwound = with_claimed_panics(
            move |info| {
                let error = PanicRecord::capture(info).error;
                let ours = error.contains("a broken mod");
                if ours {
                    sink.lock().unwrap().push(error);
                }
                ours
            },
            || {
                let unwound = mod_platform::testing::with_mods(mods.into(), || {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        mod_api::dispatch_component_types(
                            &mod_api::RegistrationContext::new(),
                            &mut Vec::new(),
                        )
                    }))
                });
                // Once the dispatch has unwound, the thread is in no mod.
                let _ = std::panic::catch_unwind(|| panic!("a broken mod's caller"));
                unwound
            },
        );
        // The unified dispatch rule contains the panic and does not unwind to the caller.
        assert!(unwound.is_ok());
        let errors = caught.lock().unwrap();
        assert_eq!(errors.len(), 2);
        assert!(
            errors[0].ends_with("\nin native mod broken, hook component_types"),
            "{}",
            errors[0]
        );
        assert!(!errors[1].contains("in native mod"), "{}", errors[1]);
    }

    #[test]
    fn dropping_panicking_startup_parts_does_not_crash_the_server_thread() {
        use mod_platform::{BlockPart, HookId, Hooks, ModEntry, Registrar, RegistrationContext};
        use std::sync::{Arc, atomic::AtomicBool};

        struct BrokenPart;
        impl Drop for BrokenPart {
            fn drop(&mut self) {
                panic!("discarded startup Part destructor");
            }
        }
        impl BlockPart for BrokenPart {
            fn block_subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
        }
        struct Registers(Option<Arc<AtomicBool>>);
        impl Hooks for Registers {
            fn subscribed(&self) -> &'static [Option<HookId>] {
                &[]
            }
            fn register(&self, registrar: &mut dyn Registrar, _: &RegistrationContext) {
                if let Some(healthy) = &self.0 {
                    healthy.store(true, Ordering::Relaxed);
                    return;
                }
                for id in ["minecraft:stone", "minecraft:dirt"] {
                    let key = registrar.block_key(id);
                    registrar.block_part(key, BrokenPart);
                }
                panic!("startup registration callback");
            }
        }

        let _lock = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        let scratch = tempfile::tempdir().unwrap();
        let previous = std::panic::take_hook();
        REPORTED.store(false, Ordering::SeqCst);
        SERVER_CRASHED.store(false, Ordering::Release);
        install(scratch.path().to_path_buf(), "client", || None);
        let result = std::thread::Builder::new()
            .name(INTEGRATED_SERVER_THREAD.into())
            .spawn(|| {
                let healthy = Arc::new(AtomicBool::new(false));
                let mods: mod_platform::Mods = vec![
                    ModEntry::new("broken_startup", Box::new(Registers(None))),
                    ModEntry::new(
                        "healthy_startup",
                        Box::new(Registers(Some(healthy.clone()))),
                    ),
                ]
                .into();
                mod_platform::testing::with_mods(mods.clone(), || {
                    mod_platform::__private::install_parts(
                        &mods,
                        &mod_platform::__private::part_tests::rule_schema(),
                        |_| false,
                    )
                    .unwrap();
                    assert!(healthy.load(Ordering::Relaxed));
                    assert_eq!(
                        mod_api::__private::take_hook_panics(),
                        [mod_api::__private::HookPanicReport {
                            mod_id: "broken_startup",
                            hook: "register",
                        }]
                    );
                });
            })
            .unwrap()
            .join();
        std::panic::set_hook(previous);
        assert!(result.is_ok());
        assert!(!server_crashed());
        assert!(!scratch.path().join("crash-reports").exists());
    }

    #[test]
    fn contained_hook_panics_do_not_trigger_crash_reports_or_abort() {
        let _lock = PANIC_HOOK.lock().unwrap_or_else(|e| e.into_inner());
        for (name, wrapped_in_contain) in [
            ("main", false),
            ("Server thread", false),
            ("sunrise-worker-0", false),
            ("main", true),
        ] {
            let scratch = tempfile::tempdir().unwrap();
            let home = scratch.path().to_path_buf();
            let previous = std::panic::take_hook();
            REPORTED.store(false, Ordering::SeqCst);
            SERVER_CRASHED.store(false, Ordering::Release);
            install(home.clone(), "client", || None);
            let result = std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    let mods: mod_api::Mods = std::sync::Arc::from([
                        mod_api::ModEntry::new("broken", Box::new(Broken)),
                        mod_api::ModEntry::new("quiet", Box::new(Quiet)),
                    ]);
                    mod_platform::testing::with_mods(mods, || {
                        let run_dispatch = || {
                            mod_api::dispatch_component_types(
                                &mod_api::RegistrationContext::new(),
                                &mut Vec::new(),
                            );
                        };
                        if wrapped_in_contain {
                            assert_eq!(contain(run_dispatch), Ok(()));
                        } else {
                            run_dispatch();
                        }
                        // Second dispatch skips the poisoned mod without queueing another report.
                        run_dispatch();
                        let reports = mod_api::__private::take_hook_panics();
                        assert_eq!(reports.len(), 1);
                        assert_eq!(reports[0].mod_id, "broken");
                        assert_eq!(reports[0].hook, "component_types");
                    });
                })
                .unwrap()
                .join();
            std::panic::set_hook(previous);
            assert!(
                result.is_ok(),
                "thread {name} unwound instead of containing"
            );
            assert!(!server_crashed(), "thread {name} set SERVER_CRASHED");
            let reports: Vec<_> = std::fs::read_dir(home.join("crash-reports"))
                .into_iter()
                .flatten()
                .map(|p| p.unwrap().path())
                .collect();
            assert!(
                reports.is_empty(),
                "thread {name} wrote crash reports: {reports:?}"
            );
            REPORTED.store(false, Ordering::SeqCst);
            SERVER_CRASHED.store(false, Ordering::Release);
        }
    }
}
