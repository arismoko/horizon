//! Native mods compiled against a signed prebuilt SDK by `cargo sunrise-mod`.
use crate::host::{tr, trf};
use crate::rust::Toolchain;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};
use sunrise_mod_package::dependencies;

pub mod api;
mod api_route;
pub mod check;
pub mod contract;
mod crates;
mod diagnostics;
pub mod discover;
pub mod game_uses;
pub mod host;
pub mod internals;
pub mod library_record;
mod mixins_patch;
pub mod package;
pub mod patch;
pub mod prebuilt;
pub mod present;
pub mod registry_compile;
pub mod registry_generate;
mod registry_inventory;
mod registry_lock;
pub mod registry_plan;
mod registry_resolve;
mod registry_runtime;
pub mod registry_rustc;
mod registry_shared;
pub mod rust;

pub use mixins_patch::MixinsIdentity;
pub use prebuilt::{Refusal, compiling};
pub use registry_inventory::Inventory as RegistryInventory;

/// What a native build compiles: the game, or the headless engine a dedicated server runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    /// The game: `sunrise-client` as the release ships it.
    #[default]
    Client,
    /// The headless engine, built without the default features (`sunrise-engine`).
    Server,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

/// The game's executable as an installed version holds it.
pub const CLIENT: &str = if cfg!(windows) {
    "sunrise-client.exe"
} else {
    "sunrise-client"
};

/// The horizon-mixins revision this tool ships with.
pub const MIXINS_REV: &str = sunrise_mod_package::mixins_tool::REV;

/// `native-build --mixins-from-source`: this process builds the horizon-mixins tool from its
/// pinned source and never looks for the prebuilt one, so CI keeps that path proven.
pub fn mixins_from_source(on: bool) {
    mixins_patch::FROM_SOURCE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// `line` without terminal escape sequences (`ESC [ ... letter`).
fn without_escapes(line: &str) -> String {
    let mut plain = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                for d in chars.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain
}

/// What of `line` the build log keeps: all of it but cargo's progress bar, whose redraws are
/// separated by carriage returns and may be followed on the same line by a real message
/// (`Building [=>] 3/9: a\r\x1b[Kerror[E0425]: ...`). None when nothing else is left.
fn log_text(line: &str) -> Option<String> {
    let kept: Vec<String> = without_escapes(line)
        .split('\r')
        .filter(|piece| {
            let piece = piece.trim();
            !piece.is_empty() && !piece.starts_with("Building [")
        })
        .map(str::to_owned)
        .collect();
    (!kept.is_empty()).then(|| kept.join("\n"))
}

/// The build's own folder ([`host::Host::root`]).
pub fn root() -> PathBuf {
    host::root()
}

/// The build's log, kept until the next build.
pub fn log_path() -> PathBuf {
    root().join("build.log")
}

/// Makes the build log fresh for a new attempt: its folder made, the last build's log
/// emptied. The error says why it couldn't, so the Build log page can show it instead of
/// the old file's contents.
pub fn clear_log() -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| e.to_string())?;
    fs::write(log_path(), "").map_err(|e| format!("{}: {e}", log_path().display()))
}

/// Appends `message` (with a newline) to the build log, so a failure that happens before any
/// command runs is still recorded.
pub fn append_log(message: &str) -> Result<(), String> {
    fs::create_dir_all(root()).map_err(|e| format!("{}: {e}", root().display()))?;
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| format!("{}: {e}", log_path().display()))?;
    writeln!(log, "{message}").map_err(|e| e.to_string())
}

pub const MANIFEST: &str = "horizon-mixin.toml";
/// A registry download's `sunrise-mod.toml` at the `.smod` package root.
pub const NATIVE_MANIFEST: &str = sunrise_mod_package::manifest::MANIFEST;

/// A validated flat mod project. The package manifest owns identity and build pins.
#[derive(Clone, Debug, PartialEq)]
pub struct Bundle {
    pub manifest: sunrise_mod_package::manifest::Manifest,
    pub dir: PathBuf,
}

impl Bundle {
    pub fn native(&self) -> &sunrise_mod_package::manifest::Native {
        self.manifest
            .native
            .as_ref()
            .expect("validated code package")
    }

    /// Whether horizon-mixins reads source to transform the game.
    pub fn needs_mixins(&self) -> bool {
        self.manifest.native.as_ref().is_some_and(|native| {
            !native.mixin.is_empty() || !native.module.is_empty() || !native.bridge.is_empty()
        })
    }
}

/// The flat native project in `folder`, or the native projects directly inside it.
pub fn scan_bundles(folder: &Path) -> Result<Vec<Bundle>, String> {
    if !folder.exists() {
        return Ok(Vec::new());
    }
    if let Some(found) = bundle_in(folder)? {
        return Ok(vec![found]);
    }
    let mut dirs: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(|e| format!("Could not read {}: {e}", folder.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut found = Vec::new();
    for dir in dirs {
        found.extend(bundle_in(&dir)?);
    }
    Ok(found)
}

pub fn bundle_in(dir: &Path) -> Result<Option<Bundle>, String> {
    package::read(dir)
}

/// Every file under `dir`, by its path relative to `dir` (with `/`), sorted. A symlink is refused:
/// horizon-mixins refuses them too.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<(), String> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let (path, kind) = (entry.path(), entry.file_type().map_err(|e| e.to_string())?);
            if kind.is_symlink() {
                return Err(format!(
                    "{} is a link: copy the file instead.",
                    path.display()
                ));
            }
            if kind.is_dir() {
                walk(root, &path, out)?;
            } else if kind.is_file() {
                out.push((
                    path.strip_prefix(root)
                        .map_err(|e| e.to_string())?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    path,
                ));
            } else {
                return Err(format!("{}: unsupported file type", path.display()));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Bundle files omit only the API package's Cargo build output.
pub fn bundle_files(bundle: &Bundle) -> Result<Vec<(String, PathBuf)>, String> {
    package::files(bundle)
}

/// A digest of a mod's files (name, length, bytes, sorted by name), kept in `native-mods.json` so
/// the Mods page sees a mod edited or reinstalled without a version bump.
pub fn digest<'a>(files: impl Iterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in files {
        hash.update(name.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The selected flat files, in the same order as `native_status::enabled` hashes an archive.
/// A build records this digest so an unchanged installed mod reads as up to date.
pub fn source_digest(bundle: &Bundle) -> Option<String> {
    let paths = bundle_files(bundle);
    let read: Vec<_> = paths
        .ok()?
        .into_iter()
        .map(|(name, path)| Some((name, fs::read(path).ok()?)))
        .collect::<Option<_>>()?;
    Some(digest(read.iter().map(|(n, b)| (n.as_str(), b.as_slice()))))
}

/// Runs `cmd`, its output appended to the build log.
/// An error ends with its first `error` lines and the last lines: a failed build script's output
/// alone could push the line naming its crate out of the last ones.
pub fn run(cmd: Command, what: &str) -> Result<(), String> {
    run_checked(cmd, what, None, None)
}

/// [`run`] for a compiler started with `--error-format=json`: a diagnostic it prints is shown at
/// the author's files (`sources`), and the build log keeps what the compiler printed.
pub fn run_diagnosed(cmd: Command, what: &str, sources: &present::Sources) -> Result<(), String> {
    run_checked(cmd, what, None, Some(sources))
}

fn run_checked(
    mut cmd: Command,
    what: &str,
    stdout_file: Option<fs::File>,
    sources: Option<&present::Sources>,
) -> Result<(), String> {
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .map_err(|e| e.to_string())?;
    let _ = writeln!(log, "$ {cmd:?}");
    cmd.stdout(match stdout_file {
        Some(file) => file,
        None => log.try_clone().map_err(|e| e.to_string())?,
    });
    // Keep the child in the CLI's group on Unix so the caller can stop the whole build.
    let started = std::time::Instant::now();
    let mut child = cmd
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| app_control(format!("Could not start {what}: {e}")))?;
    let mut tail = std::collections::VecDeque::new();
    let mut errors = Vec::new();
    let mut blocked = false;
    // Noted as the output arrives: later lines can push the linker's note out of `tail`.
    let mut libraries = false;
    let mut stderr = BufReader::new(child.stderr.take().expect("piped"));
    let mut bytes = Vec::new();
    let stderr_result = loop {
        bytes.clear();
        match stderr.read_until(b'\n', &mut bytes) {
            Ok(0) => break Ok(()),
            Ok(_) => (),
            Err(error) => break Err(format!("Could not read {what}'s stderr: {error}")),
        }
        // Strip LF or CRLF as `lines` does, leaving Cargo's interior carriage returns for
        // `log_text`. A compiler or linker can print non-UTF-8 bytes; keep reading after them.
        let line = if let Some(line) = bytes.strip_suffix(b"\n") {
            line.strip_suffix(b"\r").unwrap_or(line)
        } else {
            &bytes
        };
        let text = String::from_utf8_lossy(line);
        // A diagnostic is one line of JSON: its rendered text goes on as lines, each as the
        // compiler printed it (for the log and the matchers) and as shown to the author.
        let lines = match sources.zip(present::Diagnostic::parse(&text)) {
            Some((sources, diagnostic)) => sources
                .lines(&diagnostic)
                .into_iter()
                .map(|(raw, shown)| (raw.to_owned(), shown))
                .collect(),
            None => vec![(text.to_string(), text.into_owned())],
        };
        for (raw, shown) in lines {
            if let Some(kept) = log_text(&raw) {
                let _ = writeln!(log, "{kept}");
            }
            blocked |= rust::blocked_by_app_control(&raw);
            libraries |= rust::missing_system_libraries(&raw);
            if shown.starts_with("error") && errors.len() < 4 {
                errors.push(shown.clone());
            }
            if tail.len() == 12 {
                tail.pop_front();
            }
            tail.push_back(shown);
        }
    };
    // Close the pipe before waiting even if a read failed.
    drop(stderr);
    let status = child.wait();
    stderr_result?;
    let status = status.map_err(|e| e.to_string())?;
    let took = started.elapsed();
    if status.success() {
        // Every command of a native build says how long it took, so the log accounts for
        // the whole build and not only the steps that time themselves. The command's stdout
        // went into this log and may not have ended its last line.
        let mut last = *b"\n";
        if let Ok(mut read) = fs::File::open(log_path())
            && read.seek(SeekFrom::End(-1)).is_ok()
        {
            let _ = read.read_exact(&mut last);
        }
        let _ = writeln!(
            log,
            "{}{}",
            if last[0] == b'\n' { "" } else { "\n" },
            trf(
                "{step}: {ms} ms",
                &[("step", what), ("ms", &took.as_millis().to_string())],
            )
        );
        return Ok(());
    }
    let tail: Vec<String> = tail.into();
    let error =
        diagnostics::with_internals_hint(failure(what, &status.to_string(), &errors, &tail));
    Err(advised(error, blocked, libraries))
}

/// `error` led by what to do when Windows' Smart App Control refused one of the build's
/// programs (#5911): cargo's own lines only say a build script was "never executed".
fn app_control(error: String) -> String {
    match rust::blocked_by_app_control(&error) {
        true => with_app_control(error),
        false => error,
    }
}

fn with_app_control(error: String) -> String {
    match error.starts_with(&*rust::smart_app_control()) {
        true => error,
        false => format!("{}\n\n{error}", rust::smart_app_control()),
    }
}

/// `error` led by what to install when the build's link could not find udev or ALSA.
fn with_system_libraries(error: String) -> String {
    match error.starts_with(&*rust::system_libraries()) {
        true => error,
        false => format!("{}\n\n{error}", rust::system_libraries()),
    }
}

/// `error` led by the advice the build's output (`blocked`: Application Control refused a
/// program, `libraries`: the link could not find udev or ALSA) and the error itself call for.
pub(crate) fn advised(error: String, blocked: bool, libraries: bool) -> String {
    let error = match libraries {
        true => with_system_libraries(error),
        false => error,
    };
    match blocked {
        true => with_app_control(error),
        false => app_control(error),
    }
}

/// [`run`]'s error: the `errors` lines not already among the last ones (`tail`), then those.
pub fn failure(what: &str, status: &str, errors: &[String], tail: &[String]) -> String {
    let end = diagnostic(&tail.join("\n")).unwrap_or_else(|| tail.join("\n"));
    let first: Vec<&str> = errors
        .iter()
        .filter(|e| !tail.contains(e))
        .map(String::as_str)
        .collect();
    let (shown, first) = if first.is_empty() {
        ("The end of its output", String::new())
    } else {
        (
            "Its first errors and the end of its output",
            format!("{}\n…\n", first.join("\n")),
        )
    };
    format!(
        "{what} failed ({status}). {shown}, the whole of it in {}:\n{first}{end}",
        log_path().display()
    )
}

/// Extracts the mod id from a mixin manifest or mod file path (e.g. `.../mixins.incoming/<mod_id>/...`
/// or `.../mods.incoming/<mod_id>/...`).
fn mod_id_from_file(file: &str) -> Option<&str> {
    for prefix in ["mixins.incoming", "mods.incoming"] {
        if let Some(pos) = file.find(prefix) {
            let rest = &file[pos + prefix.len()..];
            let sub = rest.trim_start_matches(['/', '\\']);
            if let Some(end) = sub.find(['/', '\\']) {
                let id = &sub[..end];
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// A horizon-mixins diagnostic (`{"code", "message", "file"}`) in one line, led by the mod it
/// names (`mod_id`, its `injection_id`, or extracted from its `file` path for `E_MODULE_CONFLICT`),
/// so a player with several mods sees which one doesn't apply to the build's Sunrise: their files'
/// paths are deep in the launcher's folder.
fn diagnostic(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let value: serde_json::Value = serde_json::from_str(&text[start..]).ok()?;
    let mod_id = value["mod_id"]
        .as_str()
        .or_else(|| value["file"].as_str().and_then(mod_id_from_file));
    let culprit = match (mod_id, value["injection_id"].as_str()) {
        (Some(m), Some(i)) => format!("{m} ({i}): "),
        (Some(m), None) => format!("{m}: "),
        _ => String::new(),
    };
    let mut line = format!(
        "{culprit}{}: {}",
        value["code"].as_str()?,
        value["message"].as_str()?
    );
    if let Some(file) = value["file"].as_str() {
        line += &format!(" ({file})");
    }
    Some(line)
}

/// External mixins read source, not generated OUT_DIR includes: a copy of `source` with the
/// generated host forwarders written in, so neither the download nor a cached snapshot changes.
pub fn prepare_mixins_source(source: &Path, prepared: &Path) -> Result<(), String> {
    let result = sync(source, prepared)
        .and_then(|()| sunrise_platform::mod_build::api_gen::materialize_generated_host(prepared));
    if result.is_err() {
        let _ = fs::remove_dir_all(prepared);
    }
    result
}

/// `horizon-mixins apply`: the mods in `mods` applied to `source`, into `out`. It runs
/// `cargo metadata`, so it gets the launcher's toolchain like a build does: on a computer with no
/// Rust of its own, a plain command found no cargo (E_METADATA).
pub fn apply(rust: &dyn Toolchain, tool: &Path, source: &Path, mods: &Path, out: &Path) -> Command {
    let mut cmd = rust.command(tool);
    cmd.arg("apply").arg("--workspace").arg(source);
    cmd.arg("--mods").arg(mods).arg("--out").arg(out);
    cmd
}

/// The launcher's cargo running `subcommand` on the workspace `dir`, at low priority, with only
/// the workspace's own Cargo configuration ([`compile`]).
pub fn cargo(rust: &dyn Toolchain, dir: &Path, subcommand: &str) -> Command {
    let mut cmd = rust.command(&rust.cargo());
    rust::low_priority(&mut cmd);
    rust.configure_cargo(&mut cmd);
    cmd.arg(subcommand);
    let config = dir.join(".cargo").join("config.toml");
    if config.is_file() {
        cmd.arg("--config").arg(config);
    }
    cmd.arg("--manifest-path").arg(dir.join("Cargo.toml"));
    cmd
}

fn read_lock(dir: &Path) -> Result<String, String> {
    let path = dir.join("Cargo.lock");
    fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn source(revision: &str) -> Result<PathBuf, String> {
    let dir = root().join("source").join(revision);
    if dir.join(".horizon-revision").is_file() {
        host::mark_used(&dir)?;
        return Ok(dir);
    }
    let archive = format!(
        "https://codeload.github.com/{}/tar.gz/{revision}",
        sunrise_mod_package::index::native::PREBUILT_REPO
    );
    let partial = root().join("source").join(format!(".{revision}.partial"));
    let top = unpack_archive(&archive, &partial, u64::MAX)?;
    fs::write(top.join(".horizon-revision"), revision).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&top, &dir).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir_all(&partial);
    host::mark_used(&dir)?;
    Ok(dir)
}

/// GitHub's archive (`.tar.gz`) at `url`, unpacked into the fresh folder `partial`: the archive's
/// one top folder, which is returned. Links are left out; at most `limit` bytes are unpacked.
fn unpack_source(reader: impl std::io::Read, partial: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry =
            entry.map_err(|e| trf("Source interrupted: {error}", &[("error", &e.to_string())]))?;
        let kind = entry.header().entry_type();
        // Match downloaded sources: horizon-mixins refuses links, which the build does not need.
        if !(kind.is_file() || kind.is_dir()) {
            continue;
        }
        if !entry.unpack_in(partial).map_err(|e| {
            trf(
                "Could not unpack the source: {error}",
                &[("error", &e.to_string())],
            )
        })? {
            return Err(tr("The source archive contains a path outside its folder.").into());
        }
    }
    Ok(())
}

pub fn unpack_archive(url: &str, partial: &Path, limit: u64) -> Result<PathBuf, String> {
    use std::io::Read;
    let _ = fs::remove_dir_all(partial);
    fs::create_dir_all(partial).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(host::get(url)?).take(limit);
    unpack_source(reader, partial)?;
    // The archive's one top folder (<repo>-<commit>/) is the source's root.
    fs::read_dir(partial)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or_else(|| "The source archive is empty.".into())
}

/// Makes `to` hold `from`'s files, writing only those that changed, so Cargo, which goes by file
/// times, recompiles only what the mods changed. A file written gets the current time, not
/// `from`'s: a copy keeps the original's time on Windows and macOS, and an unpacked source has
/// its commit's, which can be older than the last build. Cargo would take that file for one it
/// already compiled and link the older crate (#10421).
pub fn sync(from: &Path, to: &Path) -> Result<(), String> {
    let new = files(from)?;
    let old = if to.is_dir() { files(to)? } else { Vec::new() };
    for (name, path) in &new {
        copy_changed(path, &to.join(name))?;
    }
    for (name, path) in old {
        if new.binary_search_by(|(n, _)| n.cmp(&name)).is_err() {
            let _ = fs::remove_file(path);
        }
    }
    Ok(())
}

/// Writes `from` over `to` when their bytes differ, with the current time (see [`sync`]).
fn copy_changed(from: &Path, to: &Path) -> Result<(), String> {
    if fs::read(to).ok() == fs::read(from).ok() {
        return Ok(());
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::copy(from, to)
        .and_then(|_| fs::File::options().write(true).open(to))
        .and_then(|file| file.set_modified(std::time::SystemTime::now()))
        .map_err(|e| format!("{}: {e}", to.display()))?;
    Ok(())
}

/// Copy a bundle without its API package's Cargo build output.
pub fn copy_bundle(bundle: &Bundle, to: &Path) -> Result<(), String> {
    package::copy(bundle, to)
}

/// Stage supporting modules for horizon-mixins, before applying mixins.
pub fn stage_mixins(mods: &[Bundle], to: &Path) -> Result<Vec<Bundle>, String> {
    if to.exists() {
        fs::remove_dir_all(to).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    let mut staged = Vec::new();
    for bundle in mods.iter().filter(|bundle| bundle.needs_mixins()) {
        let dir = to.join(&bundle.manifest.id);
        package::copy_tool_input(bundle, &dir)?;
        let copied = Bundle {
            dir,
            ..bundle.clone()
        };
        package::tool_manifest(&copied)?;
        staged.push(copied);
    }
    Ok(staged)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    pub const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    static LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The one build log of this test process, held while a test reads it.
    fn log_lock() -> std::sync::MutexGuard<'static, ()> {
        LOG.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sunrise-native-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) fn split_source(root: &Path) -> PathBuf {
        let write = |path: &Path, text: &str| {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        for package in [
            "sunrise-client",
            "sunrise-gameplay-common",
            "sunrise-gameplay",
            "sunrise-client-gameplay",
        ] {
            let root = root.join("crates").join(package);
            write(
                &root.join("Cargo.toml"),
                &format!("[package]\nname = '{package}'\n"),
            );
            if package == "sunrise-client" {
                write(&root.join("src/lib.rs"), "mod entry;");
                write(&root.join("src/entry.rs"), "");
            } else {
                write(&root.join("src/lib.rs"), "pub mod game;");
                write(&root.join("src/game.rs"), "");
            }
        }
        let generator = root.join("crates/sunrise-mod-api-gen");
        write(&generator.join("source-layout"), "split\n");
        write(&generator.join("internals.txt"), "");
        root.join("crates/sunrise-gameplay/src")
    }

    pub fn project_manifest(id: &str, version: &str, sunrise: &str, mixins: &str) -> String {
        format!(
            "id='{id}'\nname='{id}'\nversion='{version}'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nsunrise='{sunrise}'\nmixins='{mixins}'\n"
        )
    }

    pub fn write_bundle(project: &Path, id: &str, helper: &str) {
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(
            project.join(NATIVE_MANIFEST),
            project_manifest(id, "0.1.0", A, B),
        )
        .unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='example'\nversion='9.0.0'\n[dependencies]\nsunrise-client='0.1'\n",
        )
        .unwrap();
        fs::write(project.join("src/lib.rs"), helper).unwrap();
    }

    #[test]
    fn source_archive_escape_is_refused_without_writing_outside() {
        let folder = tempfile::tempdir().unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_mode(0o644);
        header.as_mut_bytes()[..10].copy_from_slice(b"../outside");
        header.set_cksum();
        let mut archive = tar::Builder::new(Vec::new());
        archive.append(&header, &b"x"[..]).unwrap();
        let bytes = archive.into_inner().unwrap();
        let inside = folder.path().join("inside");
        fs::create_dir(&inside).unwrap();
        assert!(unpack_source(bytes.as_slice(), &inside).is_err());
        assert!(!folder.path().join("outside").exists());
    }

    #[test]
    fn mixins_input_copies_the_source_without_changing_the_download() {
        let root = dir("mixins-input");
        let source = root.join("download");
        let prepared = root.join("prepared");
        let src = split_source(&source);
        let area = src.join("game/items");
        fs::create_dir_all(&area).unwrap();
        let game = "struct ServerGame;\n#[path = \"game/items/using_item.rs\"]\nmod using_item;\n";
        let helper = "use super::*; impl ServerGame { fn area_helper(&mut self) {} }";
        fs::write(src.join("game.rs"), game).unwrap();
        fs::write(area.join("using_item.rs"), helper).unwrap();
        prepare_mixins_source(&source, &prepared).unwrap();
        let output = prepared.join("crates/sunrise-gameplay/src");
        assert_eq!(fs::read_to_string(output.join("game.rs")).unwrap(), game);
        assert_eq!(
            fs::read_to_string(output.join("game/items/using_item.rs")).unwrap(),
            helper
        );
        // A file the next source no longer has leaves the copy.
        fs::remove_file(area.join("using_item.rs")).unwrap();
        fs::write(src.join("game.rs"), "struct ServerGame;\n").unwrap();
        prepare_mixins_source(&source, &prepared).unwrap();
        assert!(!output.join("game/items/using_item.rs").exists());
        assert_eq!(
            fs::read_to_string(src.join("game.rs")).unwrap(),
            "struct ServerGame;\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_build_log_keeps_messages_but_not_progress_redraws() {
        assert_eq!(log_text("   Building [=>   ] 3/9: a"), None);
        assert_eq!(log_text("\x1b[K"), None);
        assert_eq!(
            log_text("   Building [=>   ] 3/9: a\r   Compiling b v1").as_deref(),
            Some("   Compiling b v1")
        );
        assert_eq!(
            log_text("   Building [=>   ] 3/9: a\r\x1b[Kerror[E0425]: cannot find `x`").as_deref(),
            Some("error[E0425]: cannot find `x`")
        );
        assert_eq!(
            log_text("\x1b[K\x1b[33mwarning\x1b[0m: unused").as_deref(),
            Some("warning: unused")
        );
        assert_eq!(log_text("warning: x").as_deref(), Some("warning: x"));
    }

    #[test]
    fn only_changed_files_are_written_again() {
        let (from, to) = (dir("sync-from"), dir("sync-to"));
        fs::create_dir_all(from.join("src")).unwrap();
        fs::write(from.join("src/a.rs"), "a").unwrap();
        fs::write(from.join("src/b.rs"), "b2").unwrap();
        fs::create_dir_all(to.join("src")).unwrap();
        fs::write(to.join("src/a.rs"), "a").unwrap();
        fs::write(to.join("src/b.rs"), "b").unwrap();
        fs::write(to.join("gone.rs"), "x").unwrap();
        let before = fs::metadata(to.join("src/a.rs"))
            .unwrap()
            .modified()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sync(&from, &to).unwrap();
        assert_eq!(
            fs::metadata(to.join("src/a.rs"))
                .unwrap()
                .modified()
                .unwrap(),
            before
        );
        assert_eq!(fs::read_to_string(to.join("src/b.rs")).unwrap(), "b2");
        assert!(!to.join("gone.rs").exists());
    }

    /// #10421: a Nightly's source carries its commit's time, older than a build made since on
    /// another Nightly. Its changed files must reach the stage newer than that build, or Cargo
    /// keeps the old crate.
    #[test]
    fn a_changed_file_is_newer_than_the_last_build_even_from_an_older_source() {
        use std::time::{Duration, SystemTime};
        let (from, to) = (dir("sync-old-from"), dir("sync-old-to"));
        let commit = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        fs::create_dir_all(from.join("crates/sunrise-core/src")).unwrap();
        let file = from.join("crates/sunrise-core/src/storage.rs");
        fs::write(&file, "pub fn unpack_into() {}").unwrap();
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(commit)
            .unwrap();
        fs::create_dir_all(to.join("crates/sunrise-core/src")).unwrap();
        fs::write(
            to.join("crates/sunrise-core/src/storage.rs"),
            "pub fn unpack() {}",
        )
        .unwrap();
        // The older Nightly's build: Cargo records when it started.
        let built = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        sync(&from, &to).unwrap();
        let staged = to.join("crates/sunrise-core/src/storage.rs");
        assert_eq!(
            fs::read_to_string(&staged).unwrap(),
            "pub fn unpack_into() {}"
        );
        assert!(fs::metadata(&staged).unwrap().modified().unwrap() > built);
        // A file new to the stage too.
        fs::remove_dir_all(&to).unwrap();
        sync(&from, &to).unwrap();
        assert!(fs::metadata(&staged).unwrap().modified().unwrap() > built);
        fs::remove_dir_all(from).unwrap();
        fs::remove_dir_all(to).unwrap();
    }
    #[test]
    fn tool_errors_read_in_one_line() {
        let err = "{\n  \"code\": \"E_REVISION\",\n  \"message\": \"requires exact Sunrise revision a\",\n  \"file\": \"m.toml\"\n}";
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("E_REVISION: requires exact Sunrise revision a (m.toml)")
        );
    }

    #[test]
    fn a_tool_error_starts_with_the_mod_that_does_not_apply() {
        // horizon-mixins e8cd0aa on Sunrise 998b1d8 with fast_items 0.1.0 and appleskin 1.0.0
        // (made for ba6344a), as a player's build was on 2026-11-04.
        let err = r#"{
  "code": "E_TARGET_NOT_FOUND",
  "message": "no source function 'sunrise_client::game::titles::Game<'_>::draw_titles'",
  "file": "/home/p/.local/share/sunrise-launcher/native/mixins.incoming/appleskin/horizon-mixin.toml",
  "mod_id": "appleskin",
  "injection_id": "hud",
  "target": "sunrise_client::game::titles::Game<'_>::draw_titles"
}"#;
        let line = diagnostic(err).unwrap();
        assert!(
            line.starts_with("appleskin (hud): E_TARGET_NOT_FOUND: no source function"),
            "{line}"
        );
        // About a whole mod, no injection in it.
        let err = r#"{"code": "E_REVISION", "message": "pinned to a", "mod_id": "chop"}"#;
        assert_eq!(
            diagnostic(err).as_deref(),
            Some("chop: E_REVISION: pinned to a")
        );
    }

    #[test]
    fn a_tool_error_without_mod_id_extracts_culprit_from_file_path() {
        // horizon-mixins e8cd0aa emits E_MODULE_CONFLICT without mod_id (issue #9391).
        let err = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module sunrise_physics",
  "file": "/home/p/.local/share/sunrise-launcher/native/mixins.incoming/sunrise_physics/horizon-mixin.toml"
}"#;
        let line = diagnostic(err).unwrap();
        assert!(
            line.starts_with(
                "sunrise_physics: E_MODULE_CONFLICT: conflict in module sunrise_physics"
            ),
            "{line}"
        );

        // Windows path with backslashes
        let err_win = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module biomecompass",
  "file": "C:\\Users\\p\\AppData\\Local\\sunrise-launcher\\native\\mixins.incoming\\biomecompass\\horizon-mixin.toml"
}"#;
        let line_win = diagnostic(err_win).unwrap();
        assert!(
            line_win
                .starts_with("biomecompass: E_MODULE_CONFLICT: conflict in module biomecompass"),
            "{line_win}"
        );

        // mods.incoming path
        let err_mods = r#"{
  "code": "E_MODULE_CONFLICT",
  "message": "conflict in module worldedit",
  "file": "/home/p/.local/share/sunrise-launcher/native/mods.incoming/worldedit/sunrise-mod.toml"
}"#;
        let line_mods = diagnostic(err_mods).unwrap();
        assert!(
            line_mods.starts_with("worldedit: E_MODULE_CONFLICT: conflict in module worldedit"),
            "{line_mods}"
        );
    }

    #[test]
    fn a_failed_build_names_the_crate_its_output_pushed_out() {
        // cargo's output on Linux without ALSA's development files (Rust 1.99.1, 2026-10-31):
        // the build script's report fills the last lines.
        let errors = ["error: failed to run custom build command for `alsa-sys v0.3.1`".to_owned()];
        let tail: Vec<String> = [
            "  The system library `alsa` required by crate `alsa-sys` was not found.",
            "  HINT: you may need to install a package such as alsa, alsa-dev or alsa-devel.",
            "warning: build failed, waiting for other jobs to finish...",
        ]
        .map(str::to_owned)
        .into();
        let e = failure("Compiling the game", "exit status: 101", &errors, &tail);
        assert!(e.starts_with("Compiling the game failed (exit status: 101). Its first errors"));
        assert!(
            e.ends_with(&format!("{}\n…\n{}", errors[0], tail.join("\n"))),
            "{e}"
        );
        // An error still among the last lines isn't said twice.
        let e = failure("horizon-mixins", "exit status: 1", &errors, &errors);
        assert!(e.contains("The end of its output") && e.matches("alsa-sys").count() == 1);
    }

    /// The linker's note comes before more output than the error keeps, so the hint is decided
    /// as the output arrives.
    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn a_link_missing_udev_or_alsa_leads_with_what_to_install() {
        let _guard = log_lock();
        clear_log().unwrap();
        // rustc's output linking a native mod's patch library in a shell without either library
        // (Rust 1.99.1 with rust-lld, 2026-11-19), then more lines than the error's end keeps.
        let script = |library: &str| {
            let mut cmd = Command::new("sh");
            cmd.args(["-c", &format!(
                "echo 'error: linking with `cc` failed: exit status: 1' >&2
                 echo '  = note: rust-lld: error: unable to find library {library}' >&2
                 for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo \"warning: build failed, waiting ($i)\" >&2; done
                 exit 101"
            )]);
            run(cmd, "Compiling the mod").unwrap_err()
        };
        for library in ["-ludev", "-lasound"] {
            let e = script(library);
            assert!(e.starts_with(&rust::system_libraries()), "{e}");
            assert!(!e.contains("unable to find library"), "{e}");
        }
        let e = script("-lasound_extra");
        assert!(e.starts_with("Compiling the mod failed"), "{e}");
    }

    /// Smart App Control refusing a build script (#5911): cargo names the crate, Windows' reason
    /// is only in a `Caused by` line in the middle of its output, so the error leads with what to do.
    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn a_build_smart_app_control_blocked_says_how_to_build() {
        let _guard = log_lock();
        clear_log().unwrap();
        // cargo's output on Windows 11 25H2 with Smart App Control on (the report in #5911).
        let mut cmd = Command::new("sh");
        cmd.args(["-c", r#"
            echo 'error: failed to run custom build command for `quote v1.0.47`' >&2
            echo 'Caused by:' >&2
            echo '  could not execute process `C:\target\build-script-build` (never executed)' >&2
            echo 'Caused by:' >&2
            echo '  An Application Control policy has blocked this file. (os error 4551)' >&2
            for i in 1 2 3 4 5 6 7 8 9 10 11 12 13; do echo "warning: build failed, waiting ($i)" >&2; done
            exit 101
        "#]);
        let e = run(cmd, "Building horizon-mixins").unwrap_err();
        assert!(e.starts_with(&rust::smart_app_control()), "{e}");
        assert!(e.contains("quote v1.0.47"), "{e}");
        // Another failure says nothing about it.
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "echo 'error: could not compile `sunrise-client`' >&2; exit 101",
        ]);
        let e = run(cmd, "Compiling the game").unwrap_err();
        assert!(!e.contains("Smart App Control"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn malformed_stderr_does_not_hide_later_diagnostics() {
        const ACTOR: &str = "SUNRISE_MALFORMED_STDERR_TEST_ACTOR";
        if std::env::var_os(ACTOR).is_none() {
            let scratch = tempfile::tempdir().unwrap();
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tests::malformed_stderr_does_not_hide_later_diagnostics",
                    "--include-ignored",
                    "--nocapture",
                ])
                .env(ACTOR, "1")
                // The unhosted build log lives under the child's temp dir and PID.
                .env("TMPDIR", scratch.path())
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(output.status.success(), "{stdout}\n{stderr}");
            assert!(
                stdout.contains("test result: ok. 1 passed;"),
                "exact-name child did not run one test:\n{stdout}\n{stderr}"
            );
            return;
        }
        clear_log().unwrap();
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            r"printf 'bad byte \377\r\nBuilding [=>] 3/9: a\r\033[Kerror: later\r\nlast warning' >&2; exit 17",
        ]);
        let error = run(cmd, "Compiling the mod").unwrap_err();
        assert!(
            error.starts_with("Compiling the mod failed (exit status: 17)"),
            "{error}"
        );
        assert!(error.contains("bad byte �\n"), "{error}");
        assert!(error.ends_with("error: later\nlast warning"), "{error}");
        let log = fs::read_to_string(log_path()).unwrap();
        assert!(
            log.ends_with("bad byte �\nerror: later\nlast warning\n"),
            "{log}"
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn build_commands_inherit_the_tools_process_group() {
        let _guard = log_lock();
        clear_log().unwrap();
        // SAFETY: getpgrp only reads this process's group and takes no arguments.
        let group = unsafe { libc::getpgrp() }.to_string();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "ps -o pgid= -p \"$$\""]);
        run(cmd, "Reading the build command's process group").unwrap();
        let log = fs::read_to_string(log_path()).unwrap();
        assert!(
            log.lines().any(|line| line.trim() == group.as_str()),
            "{log}"
        );
    }

    /// A command that succeeds leaves its time in the build log; one that fails leaves none.
    #[cfg(unix)]
    #[test]
    #[ignore = "daily"]
    fn each_command_logs_its_time() {
        let _guard = log_lock();
        clear_log().unwrap();
        // Its stdout, which goes to the log, does not end its line.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "printf output; sleep 0.05"]);
        run(cmd, "Linking the game").unwrap();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "exit 1"]);
        run(cmd, "Compiling fnr_alpha").unwrap_err();
        let log = fs::read_to_string(log_path()).unwrap();
        let ms: u64 = log
            .lines()
            .find_map(|line| line.strip_prefix("Linking the game: ")?.strip_suffix(" ms"))
            .unwrap_or_else(|| panic!("no timing line in {log}"))
            .parse()
            .unwrap();
        assert!(ms >= 50, "{log}");
        assert!(log.lines().any(|line| line == "output"), "{log}");
        assert!(!log.contains("Compiling fnr_alpha: "), "{log}");
    }
}
