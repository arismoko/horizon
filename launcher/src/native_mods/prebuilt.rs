//! Native mods compile and link against the pinned prebuilt game.
//! Unsupported inputs and unavailable SDKs are refused, never rebuilt locally.
use super::{api, dependencies, internals, Target};
use crate::i18n::{tr, trf};
use crate::rust_toolchain::{self, Toolchain};
use horizon_mod_package::index::native;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

const IDENTITY: &str = "horizon-prebuilt.json";
/// Names the folder of a prebuilt game built from a developer install's checkout
/// (`tools/prebuilt_game.py build`). Read for developer installs only: their commit has no release.
pub(super) const DEVELOPER_SDK: &str = "HORIZON_LAUNCHER_DEVELOPER_SDK";
pub(super) const TEMPLATE: &str = "native_main.rs.template";
const SUMS: &str = "SHA256SUMS";
/// The optimization level of the `release` profile the prebuilt game is built with: the mods' code
/// matches the game's.
const OPT_LEVEL: &str = "opt-level=3";
/// The archive's largest plausible unpacked size: a corrupt or hostile archive stops there.
const UNPACKED_LIMIT: u64 = 8 << 30;

/// Why a build compiles the game from source instead of using the prebuilt game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A mod has a build script, which the prebuilt route doesn't run.
    BuildScript { mod_id: String },
    /// A mod declares Cargo features, or asks the mod API for features of its own.
    Features { mod_id: String },
    /// No prebuilt game for this computer.
    Host,
    /// The prebuilt game's release couldn't be downloaded.
    Download { why: String },
    /// No prebuilt game was released for this revision and target.
    NoRelease { revision: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::BuildScript { mod_id } => trf(
                "E_NATIVE_BUILD_SCRIPT: {mod} has a build script; remove it and ship generated source in the mod",
                &[("mod", mod_id)],
            ),
            Self::Features { mod_id } => trf(
                "E_NATIVE_FEATURES: {mod} requests Cargo features; use the pinned SDK feature set",
                &[("mod", mod_id)],
            ),
            Self::Host => {
                tr("E_NATIVE_HOST: no prebuilt game supports this computer; use a supported host").into()
            }
            Self::Download { why } => trf(
                "The prebuilt game couldn't be downloaded ({why}). Try again.",
                &[("why", why)],
            ),
            Self::NoRelease { revision } => trf(
                "E_NATIVE_NO_RELEASE: no prebuilt game for {revision}; pin a commit with a published SDK",
                &[("revision", &revision[..revision.len().min(10)])],
            ),
        };
        f.write_str(&text)
    }
}

/// The first structural reason these projects cannot use the prebuilt game.
pub(super) fn refusal(projects: &[api::Project<'_>]) -> Option<Refusal> {
    projects.iter().find_map(project_refusal)
}

/// Why `project` alone can't build against the prebuilt game, or `None` when it can.
pub(super) fn project_refusal(project: &api::Project<'_>) -> Option<Refusal> {
    {
        let mod_id = || project.bundle.manifest.id.clone();
        let cargo = &project.cargo;
        let package = cargo.get("package");
        let build = package.and_then(|p| p.get("build"));
        let has_script = build.is_some_and(|value| value.as_bool() != Some(false))
            || (build.and_then(toml::Value::as_bool) != Some(false)
                && project.bundle.dir.join("build.rs").is_file())
            || cargo
                .get("build-dependencies")
                .and_then(toml::Value::as_table)
                .is_some_and(|t| !t.is_empty());
        if has_script {
            return Some(Refusal::BuildScript { mod_id: mod_id() });
        }
        if cargo
            .get("features")
            .and_then(toml::Value::as_table)
            .is_some_and(|t| !t.is_empty())
        {
            return Some(Refusal::Features { mod_id: mod_id() });
        }
        for (_, dependencies) in project.dependency_tables() {
            for (key, value) in dependencies {
                let krate = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(key);
                if !api::PACKAGES.iter().any(|(package, _)| *package == krate) {
                    continue;
                }
                let own_features = value
                    .get("features")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|f| !f.is_empty())
                    || value.get("default-features").and_then(toml::Value::as_bool) == Some(false);
                if own_features {
                    return Some(Refusal::Features { mod_id: mod_id() });
                }
            }
        }
        None
    }
}

/// The prebuilt game for `revision` and `target`, downloaded and checked once, then kept in the
/// launcher's data (`downloading` is told before a download starts); `Err(reason)` inside when
/// there is none to use, or its download failed. An archive that arrives must match the release's
/// `SHA256SUMS` before it is unpacked, and its identity this revision, this computer and the
/// launcher's Rust: a mismatch is an error, never a source build. `local` is a developer
/// install's own prebuilt game ([`DEVELOPER_SDK`]): used where it is, under the same identity check.
pub(super) fn fetch(
    revision: &str,
    target: Target,
    local: Option<&Path>,
    downloading: impl FnOnce(),
) -> Result<Result<Package, Refusal>, String> {
    let Some(host) = rust_toolchain::host() else {
        return Ok(Err(Refusal::Host));
    };
    let name = native::prebuilt_archive(host.target, matches!(target, Target::Server));
    if let Some(local) = local {
        check_identity(local, revision, host.target, target)?;
        // Laid out afresh each build, by links: the developer's folder is never changed.
        let build = cache_dir(revision, &format!("{name}.developer"));
        let _ = fs::remove_dir_all(&build);
        return Ok(Ok(lay_out(local, &build, true)?));
    }
    let dir = cache_dir(revision, &name);
    if Package::at(&dir).runtime.join(IDENTITY).is_file() {
        crate::storage::mark_used(&dir).map_err(super::cache_error)?;
        return Ok(Ok(Package::at(&dir)));
    }
    let release = release_url(revision);
    let download = |why: String| Ok(Err(Refusal::Download { why }));
    let expected = match release_sum(revision, &name) {
        Ok(sum) => sum,
        Err(reason) => return Ok(Err(reason)),
    };
    let parent = dir.parent().expect("under prebuilt/");
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let partial = parent.join(format!(".{name}.partial"));
    let _ = fs::remove_dir_all(&partial);
    fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
    downloading();
    let file = partial.join(&name);
    let mut body = match crate::backend::get(&format!("{release}/{name}")) {
        Ok(response) => response.into_reader(),
        Err(why) => return download(why),
    };
    let mut out = fs::File::create(&file).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = match body.read(&mut buffer) {
            Ok(read) => read,
            Err(e) => {
                let _ = fs::remove_dir_all(&partial);
                return download(e.to_string());
            }
        };
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        std::io::Write::write_all(&mut out, &buffer[..read]).map_err(|e| e.to_string())?;
    }
    drop(out);
    let actual = format!("{:x}", hasher.finalize());
    if actual != expected {
        let _ = fs::remove_dir_all(&partial);
        return Err(trf(
            "The prebuilt game {name} doesn't match its release's checksum. Try again.",
            &[("name", &name)],
        ));
    }
    let _publication = crate::run_leases::installation().map_err(super::cache_error)?;
    let unpacked = partial.join("game");
    // `unpack_in` writes inside a folder that exists.
    fs::create_dir_all(&unpacked).map_err(|e| e.to_string())?;
    let reader = flate2::read::GzDecoder::new(fs::File::open(&file).map_err(|e| e.to_string())?)
        .take(UNPACKED_LIMIT);
    let mut tar = tar::Archive::new(reader);
    for entry in tar.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.header().entry_type();
        if kind.is_file() || kind.is_dir() {
            entry
                .unpack_in(&unpacked)
                .map_err(|e| format!("Could not unpack the prebuilt game: {e}"))?;
        }
    }
    check_identity(&unpacked, revision, host.target, target)?;
    let laid = partial.join("build");
    lay_out(&unpacked, &laid, false)?;
    let _ = fs::remove_dir_all(&dir);
    fs::rename(&laid, &dir).map_err(|e| e.to_string())?;
    crate::run_leases::publish(&dir).map_err(super::cache_error)?;
    let _ = fs::remove_dir_all(&partial);
    crate::storage::mark_used(&dir).map_err(super::cache_error)?;
    Ok(Ok(Package::at(&dir)))
}

fn release_url(revision: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/{}{revision}",
        native::PREBUILT_REPO,
        native::PREBUILT_TAG
    )
}

fn release_sum(revision: &str, name: &str) -> Result<String, Refusal> {
    let missing = || Refusal::NoRelease {
        revision: revision.to_owned(),
    };
    let download = |why: String| Refusal::Download { why };
    let sums = crate::backend::get_unless_missing(&format!("{}/{SUMS}", release_url(revision)))
        .map_err(download)?
        .ok_or_else(missing)?;
    let mut text = String::new();
    sums.into_reader()
        .read_to_string(&mut text)
        .map_err(|error| download(error.to_string()))?;
    expected_sum(&text, name).ok_or_else(missing)
}

/// Checks release metadata without downloading the prebuilt game. Run only on a worker.
/// Download failures are retryable, never permission to rebuild the game from source.
pub fn prebuilt_availability(revision: &str, target: Target) -> Option<Refusal> {
    let Some(host) = rust_toolchain::host() else {
        return Some(Refusal::Host);
    };
    let name = native::prebuilt_archive(host.target, matches!(target, Target::Server));
    let cached = cache_dir(revision, &name);
    if cached.join(IDENTITY).is_file() {
        return None;
    }
    release_sum(revision, &name).err()
}

fn cache_dir(revision: &str, name: &str) -> PathBuf {
    super::root()
        .join("prebuilt")
        .join(format!("{revision}-{}", name.trim_end_matches(".tar.gz")))
}

/// Read checked SDK metadata; the caller owns consent and the native-cache lease.
pub(super) fn planning_catalog(
    lease: &crate::native_cache::Cache,
    revision: &str,
) -> Result<internals::Catalog, Refusal> {
    let host = rust_toolchain::host().ok_or(Refusal::Host)?;
    let name = native::prebuilt_archive(host.target, false);
    let game = cache_dir(revision, &name);
    let package = Package::at(&game);
    if package.runtime.join(IDENTITY).is_file() {
        if let Ok(catalog) = internals::Catalog::read(&package.api) {
            crate::storage::mark_used(&game).map_err(|error| Refusal::Download {
                why: super::cache_error(error),
            })?;
            return Ok(catalog);
        }
    }
    let expected = release_sum(revision, &name)?;
    let download = |why| Refusal::Download { why };
    let unit = format!("catalog-{revision}-{}", host.target);
    let cache = super::root().join("internals-sync").join(&unit);
    let directory = lease
        .directory(&["internals-sync", &unit])
        .map_err(|error| download(super::cache_error(error)))?;
    crate::storage::mark_used(&cache).map_err(|error| download(super::cache_error(error)))?;
    catalog_pair(&directory, &cache, &name, &expected, || {
        crate::backend::get(&format!("{}/{name}", release_url(revision)))
            .map_err(download)
            .map(|response| response.into_reader())
    })
}

fn catalog_pair<R: Read>(
    directory: &cap_std::fs::Dir,
    path: &Path,
    name: &str,
    expected: &str,
    fetch: impl FnOnce() -> Result<R, Refusal>,
) -> Result<internals::Catalog, Refusal> {
    let download = |why| Refusal::Download { why };
    if directory.read_to_string("archive.sha256").ok().as_deref() == Some(expected) {
        if let Ok(catalog) = internals::Catalog::read(path) {
            return Ok(catalog);
        }
    }
    let reader = fetch()?;
    // A failed second rename must never leave an old marker authorizing a mixed pair.
    match directory.remove_file("archive.sha256") {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(download(error.to_string())),
    }
    horizon_mod_package::internals::cache_catalogs(directory, name, reader, expected)
        .map_err(|error| download(error.to_string()))?;
    let catalog = internals::Catalog::read(path).map_err(download)?;
    directory
        .write("archive.sha256", expected)
        .map_err(|error| download(error.to_string()))?;
    Ok(catalog)
}

/// `name`'s sum in a `sha256sum` listing.
pub(super) fn expected_sum(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| sum.to_ascii_lowercase())
    })
}

#[derive(serde::Deserialize)]
pub(super) struct Identity {
    commit: String,
    rustc: String,
    target: String,
    side: String,
    pub(super) link_args: Vec<String>,
    #[serde(default)]
    pub(super) registry_lock: Option<String>,
    #[serde(default)]
    pub(super) registry_patches: Option<toml::Table>,
    #[serde(default)]
    pub(super) native_links: Option<Vec<super::registry_inventory::NativeLink>>,
    #[serde(default)]
    facade_crates: std::collections::BTreeSet<String>,
    /// The game crate's features the SDK was built with: a patched function copied out of its
    /// source is compiled under the same `cfg`s.
    #[serde(default)]
    game_features: Vec<String>,
    /// The API route's mods library: its crate name and the metadata both builds of it use.
    #[serde(default)]
    pub(super) native_mods: Option<NativeMods>,
    /// Which package files the `runtime` and `api` slices hold; every other file is the `engine`'s.
    #[serde(default)]
    pub(super) slices: Option<Slices>,
}

#[derive(serde::Deserialize)]
pub(super) struct NativeMods {
    #[serde(rename = "crate")]
    pub(super) krate: String,
    pub(super) metadata: String,
}

#[derive(serde::Deserialize)]
pub(super) struct Slices {
    pub(super) runtime: Vec<String>,
    pub(super) api: Vec<String>,
}

/// The prebuilt game as the launcher keeps one build of it: each of the SDK's three compiled
/// slices in a folder of its own under the build's folder (`runtime`, `api`, `engine`), each
/// unpacked on its own. The API route reads `runtime` and `api`; the other routes read all three.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Package {
    pub(super) runtime: PathBuf,
    pub(super) api: PathBuf,
    pub(super) engine: PathBuf,
}

impl Package {
    /// The slices under the build folder `build`.
    pub(super) fn at(build: &Path) -> Self {
        Self {
            runtime: build.join("runtime"),
            api: build.join("api"),
            engine: build.join("engine"),
        }
    }

    /// The three slices as one folder at `into`, linked file by file (copied where a link can't
    /// be made): the folder the engine and registry routes build against.
    pub(super) fn whole(&self, into: &Path) -> Result<PathBuf, String> {
        let _ = fs::remove_dir_all(into);
        for slice in [&self.runtime, &self.api, &self.engine] {
            link_tree(slice, slice, into)?;
        }
        Ok(into.to_owned())
    }
}

/// Link every file under `dir` (inside the slice `slice`) into `into` at the same relative path;
/// a file two slices both hold is refused.
fn link_tree(slice: &Path, dir: &Path, into: &Path) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        let to = into.join(path.strip_prefix(slice).expect("under its slice"));
        if path.is_dir() {
            link_tree(slice, &path, into)?;
            continue;
        }
        if to.exists() {
            return Err(trf(
                "The prebuilt game's slices both hold {file}.",
                &[("file", &to.display().to_string())],
            ));
        }
        fs::create_dir_all(to.parent().expect("a file has a folder")).map_err(|e| e.to_string())?;
        if fs::hard_link(&path, &to).is_err() {
            fs::copy(&path, &to).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

/// Lay the whole prebuilt game in `whole` out as its three slices under `build` ([`Package`]),
/// as its identity's `slices` lists them (the `runtime` and `api` files; every other file is the
/// `engine`'s), moving or linking each file; a listed file that isn't there is refused by name.
fn lay_out(whole: &Path, build: &Path, link: bool) -> Result<Package, String> {
    let sdk = identity(whole)?;
    let slices = sdk.slices.as_ref().ok_or_else(|| {
        tr("This prebuilt game has no slices. Update Horizon and rebuild the mods against this game.")
            .to_string()
    })?;
    let package = Package::at(build);
    let mut placed = std::collections::HashMap::new();
    for (folder, names) in [
        (&package.runtime, &slices.runtime),
        (&package.api, &slices.api),
    ] {
        for name in names {
            if !Path::new(name)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            {
                return Err(trf(
                    "The prebuilt game's file list names {file}, which is not a plain path.",
                    &[("file", name)],
                ));
            }
            placed.insert(whole.join(name), folder.join(name));
        }
    }
    let mut files = Vec::new();
    every_file(whole, &mut files)?;
    let missing: Vec<&String> = slices
        .runtime
        .iter()
        .chain(&slices.api)
        .filter(|name| !whole.join(name).is_file())
        .collect();
    if !missing.is_empty() {
        return Err(trf(
            "The prebuilt game is missing files its slices list: {files}.",
            &[(
                "files",
                &missing
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )],
        ));
    }
    for file in files {
        let to = placed.remove(&file).unwrap_or_else(|| {
            package
                .engine
                .join(file.strip_prefix(whole).expect("under the package"))
        });
        fs::create_dir_all(to.parent().expect("a file has a folder")).map_err(|e| e.to_string())?;
        let moved = if link {
            fs::hard_link(&file, &to).or_else(|_| fs::copy(&file, &to).map(drop))
        } else {
            fs::rename(&file, &to)
        };
        moved.map_err(|e| format!("{}: {e}", file.display()))?;
    }
    for folder in [&package.runtime, &package.api, &package.engine] {
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    Ok(package)
}

fn every_file(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            every_file(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

impl Identity {
    /// The commit the SDK was built from: the source a patched function is copied out of.
    pub(super) fn commit(&self) -> &str {
        &self.commit
    }

    pub(super) fn facade_crates(&self) -> impl Iterator<Item = &str> {
        self.facade_crates.iter().map(String::as_str)
    }

    pub(super) fn game_features(&self) -> &[String] {
        &self.game_features
    }
}

pub(super) fn identity(dir: &Path) -> Result<Identity, String> {
    let path = dir.join(IDENTITY);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Formats up to 5 named the number `format_version` (before 5: nothing, so 0).
    #[derive(serde::Deserialize)]
    struct Format {
        package_format: Option<u32>,
        format_version: Option<u32>,
    }
    let format: Format =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let found = format.package_format.or(format.format_version).unwrap_or(0);
    let supported = horizon_mod_package::sdk::PACKAGE_FORMAT;
    let (version, supported_text) = (found.to_string(), supported.to_string());
    let args = [
        ("version", version.as_str()),
        ("format", supported_text.as_str()),
    ];
    match found {
        found if found == supported => {
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
        }
        found if found < supported => Err(trf(
            "SDK format {version} is too old; install a newer SDK (format {format})",
            &args,
        )),
        _ => Err(trf(
            "SDK format {version} is newer than this launcher's supported format {format}; update the launcher",
            &args,
        )),
    }
}

/// Whether the unpacked game in `dir` is the one asked for: built from `revision` for `host` and
/// `target`'s side by the launcher's Rust, whose compiler is the only one that reads its metadata.
fn check_identity(dir: &Path, revision: &str, host: &str, target: Target) -> Result<(), String> {
    let found = identity(dir)?;
    let release = format!("release: {}", rust_toolchain::VERSION);
    let matches = found.commit == revision
        && found.target == host
        && found.side == target.name()
        && found.rustc.lines().any(|line| line == release);
    if matches {
        Ok(())
    } else {
        Err(trf(
            "The prebuilt game for {revision} was made for another build ({commit}, {target}, {side}, {rustc}).",
            &[
                ("revision", revision),
                ("commit", &found.commit),
                ("target", &found.target),
                ("side", &found.side),
                ("rustc", found.rustc.lines().next().unwrap_or_default()),
            ],
        ))
    }
}

/// The library file a crate named `krate` left in `deps`: its metadata, or its own dylib.
pub(super) fn library(deps: &Path, krate: &str) -> Result<PathBuf, String> {
    // A cached target can still contain metadata from an earlier static build. Prefer the dylib:
    // callers installing runtime libraries need its code, not a stale metadata-only artifact.
    let dylib = ["so", "dll", "dylib"]
        .iter()
        .flat_map(|ext| [format!("lib{krate}.{ext}"), format!("{krate}.{ext}")])
        .map(|name| deps.join(name))
        .find(|path| path.is_file());
    if let Some(path) = dylib {
        return Ok(path);
    }
    let prefix = format!("lib{krate}-");
    let candidates: Vec<_> = fs::read_dir(deps)
        .map_err(|e| e.to_string())?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".rmeta")
        })
        .collect();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(trf(
            "The prebuilt game has no {crate}.",
            &[("crate", krate)],
        )),
        _ => Err(facade_refused(krate)),
    }
}

/// Both compiler routes link the same SDK-owned executable template.
pub(super) fn wrapper_externs(
    deps: &Path,
    template: &str,
) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut externs = BTreeMap::new();
    for name in [
        "horizon_client",
        "mod_api",
        "mod_points",
        "horizon_mod_package",
    ] {
        externs.insert(name.to_owned(), library(deps, name)?);
    }
    if template.contains("horizon_mod_host::") {
        externs.insert(
            "horizon_mod_host".into(),
            library(deps, "horizon_mod_host")?,
        );
    }
    Ok(externs)
}

fn facade_refused(krate: &str) -> String {
    trf(
        "The prebuilt game cannot expose {crate}: it is build-only or has ambiguous metadata.",
        &[("crate", krate)],
    )
}

pub(super) fn facade_library(deps: &Path, krate: &str, sdk: &Identity) -> Result<PathBuf, String> {
    if !sdk.facade_crates.contains(krate) {
        return Err(facade_refused(krate));
    }
    library(deps, krate)
}

/// Rust's standard library as a dynamic library: what the prebuilt game and its executable load.
fn standard_library(rust: &Toolchain) -> Result<PathBuf, String> {
    let dir = rust
        .target_libdir()
        .ok_or("no Rust standard library for this computer")?;
    fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            (name.starts_with("libstd-") || name.starts_with("std-"))
                && [".so", ".dll", ".dylib"]
                    .iter()
                    .any(|ext| name.ends_with(ext))
        })
        .ok_or_else(|| format!("{}: no dynamic standard library", dir.display()))
}

/// rustc with the prebuilt game's settings, its libraries and `out`'s on the search path, and the
/// native libraries the game's build scripts link (`native/`: the `windows` crates' import
/// libraries).
fn rustc(rust: &Toolchain, game: &Path, out: &Path, edition: &str) -> Command {
    let mut cmd = rustc_with(
        rust,
        &[&game.join("deps"), &game.join("host"), out],
        edition,
    );
    cmd.arg("-L")
        .arg(format!("native={}", game.join("native").display()));
    cmd
}

/// rustc with the prebuilt game's settings and only `search` as dependency search paths.
pub(super) fn rustc_with(rust: &Toolchain, search: &[&Path], edition: &str) -> Command {
    let mut cmd = rust.command(&rust.rustc());
    rust_toolchain::low_priority(&mut cmd);
    cmd.args([
        "--edition",
        edition,
        "-C",
        "prefer-dynamic",
        "-C",
        OPT_LEVEL,
    ]);
    for folder in search {
        cmd.arg("-L")
            .arg(format!("dependency={}", folder.display()));
    }
    cmd
}

/// How an instance's native mods are built against the prebuilt game: chosen once, from the whole
/// set of mods, and never changed after a failure (SDK design §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// Every mod is API-only: the mods become the `horizon_native_mods` library, built from the
    /// package's `api` files alone and installed beside its prebuilt API executable
    /// ([`super::api_route`]). The `engine` archive is never read.
    Api,
    /// A mod has a `[native]` table row (mixin, module, bridge, state, loop) or game internals:
    /// the executable is generated and linked against the whole package.
    Engine,
    /// A mod depends on a crate outside the game's: the executable is built with Cargo against
    /// the whole package ([`super::registry_compile`]).
    Registry,
}

impl Route {
    /// The route of the set `projects`: the strongest one any of its mods needs.
    pub(super) fn of(projects: &[api::Project<'_>]) -> Self {
        if projects.iter().any(registry_dependency) {
            Self::Registry
        } else if projects.iter().any(|project| {
            let manifest = &project.bundle.manifest;
            // Every row kind by name: a new one does not compile until it is routed here.
            let rows = manifest.native.as_ref().is_some_and(|native| {
                let horizon_mod_package::manifest::Native {
                    game: _,
                    horizon: _,
                    mixins: _,
                    mixin,
                    module,
                    bridge,
                    state,
                    r#loop,
                } = native;
                !(mixin.is_empty()
                    && module.is_empty()
                    && bridge.is_empty()
                    && state.is_empty()
                    && r#loop.is_empty())
            });
            internals::needed(manifest)
                || rows
                || project
                    .api_dependencies()
                    .iter()
                    .any(|(_, package)| *package == "internals")
        }) {
            Self::Engine
        } else {
            Self::Api
        }
    }

    /// The route as the build record (`native-mods.json`) names it.
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Api => "prebuilt-api",
            Self::Engine | Self::Registry => "prebuilt",
        }
    }
}

/// Whether `project` depends on a crate that is not one of the game's own packages.
fn registry_dependency(project: &api::Project<'_>) -> bool {
    project.dependency_tables().any(|(_, dependencies)| {
        dependencies.iter().any(|(key, value)| {
            let name = value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            !api::PACKAGES.iter().any(|(package, _)| *package == name)
        })
    })
}

/// Compiles the mods (`resolution`'s order, each with its installed mod dependencies) and the
/// launcher's executable against the prebuilt game in `game`, into `out`, by `route`
/// ([`Route::of`] the same `projects`). Returns the executable and the dynamic libraries it
/// loads, to be installed beside it.
// Unlike the source workspace, this route invokes rustc with only selected --extern
// libraries; it does not resolve or compile arbitrary transitive Cargo dependencies.
#[allow(clippy::too_many_arguments)]
pub(super) fn compile(
    rust: &Toolchain,
    package: &Package,
    out: &Path,
    route: Route,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    consent: Option<&dyn Fn(&super::RegistryInventory) -> bool>,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    if route == Route::Api {
        return super::api_route::compile(
            rust,
            &package.runtime,
            &package.api,
            out,
            projects,
            declared,
            resolution,
        );
    }
    let game = &package.whole(&out.with_file_name("prebuilt-package"))?;
    if route == Route::Registry {
        return super::registry_compile::compile(
            rust,
            game,
            out,
            projects,
            declared,
            resolution,
            super::registry_compile::Options {
                consent,
                adapter: &std::env::current_exe().map_err(|e| e.to_string())?,
            },
        );
    }
    let catalog = internals::Catalog::read(game)?;
    let generated = catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    for project in projects {
        catalog.log_mixins(&project.bundle.manifest)?;
    }
    let _ = fs::remove_dir_all(out);
    fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let deps = game.join("deps");
    let sdk = identity(game)?;
    let mut libraries = std::collections::HashMap::new();
    for (package, name) in api::PACKAGES {
        if package != api::INTERNALS_PACKAGE {
            libraries.insert(name, library(&deps, name)?);
        }
    }
    let game_library = library(&deps, "horizon_client")?;
    let mut facades = std::collections::HashMap::new();
    let mut rlibs: std::collections::HashMap<&str, PathBuf> = std::collections::HashMap::new();
    let mut crate_names: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for &index in &resolution.order {
        let project = &projects[index];
        let id = project.bundle.manifest.id.as_str();
        let api_dependencies = project.api_dependencies();
        if internals::needed(&project.bundle.manifest)
            || api_dependencies
                .iter()
                .any(|(_, package)| *package == "internals")
        {
            let source = out.join(format!("internals_{id}.rs"));
            let facade = &generated[id];
            fs::write(&source, &facade.source).map_err(|e| e.to_string())?;
            let rlib = out.join(format!("libinternals_{id}.rlib"));
            let mut cmd = rustc(rust, game, out, "2024");
            cmd.args(["--crate-type", "rlib", "--crate-name", "internals"])
                .arg("-C")
                .arg(format!("metadata={id}"))
                .arg("--extern")
                .arg(format!("horizon_client={}", game_library.display()))
                .arg("-o")
                .arg(&rlib)
                .arg(source);
            for krate in &facade.crates {
                if krate != "horizon_client" {
                    cmd.arg("--extern").arg(format!(
                        "{krate}={}",
                        facade_library(&deps, krate, &sdk)?.display()
                    ));
                }
            }
            super::run(
                cmd,
                &trf("Compiling {mod}'s declared internals", &[("mod", id)]),
                |_| {},
            )?;
            facades.insert(id, rlib);
        }
        let (rlib, krate) = compile_mod(
            project,
            &declared[index],
            out,
            |edition| rustc(rust, game, out, edition),
            |package| match package {
                "internals" => &facades[id],
                _ => &libraries[package],
            },
            &rlibs,
        )?;
        rlibs.insert(id, rlib);
        crate_names.insert(id, krate);
    }
    // `[[native.mixin]]` rows: their functions copied, patched and compiled beside the mods.
    let patch = super::patch::build(
        rust,
        game,
        out,
        projects,
        &rlibs,
        &crate_names,
        &sdk,
        |edition| rustc(rust, game, out, edition),
    )?;
    let template = api::template(&game.join(TEMPLATE))?;
    let manifests: Vec<_> = resolution
        .order
        .iter()
        .map(|&i| &projects[i].bundle.manifest)
        .collect();
    let main = out.join("main.rs");
    let mut main_text = api::main_rs(&template, &manifests, &catalog);
    if let Some(patch) = &patch {
        let installs: String = patch
            .installs
            .iter()
            .map(|install| format!("        {install},\n"))
            .collect();
        main_text = main_text.replace("/* MOD_REDIRECTS */", installs.trim_end());
    }
    fs::write(&main, main_text).map_err(|e| e.to_string())?;
    let binary = out.join(crate::backend::CLIENT);
    let mut cmd = rustc(rust, game, out, "2024");
    cmd.args(["--crate-type", "bin", "--crate-name", "horizon_native_game"]);
    for (name, path) in wrapper_externs(&deps, &template)? {
        cmd.arg("--extern")
            .arg(format!("{name}={}", path.display()));
    }
    for (name, rlib) in patch.iter().flat_map(|patch| &patch.rlibs) {
        cmd.arg("--extern")
            .arg(format!("{name}={}", rlib.display()));
    }
    for (position, &index) in resolution.order.iter().enumerate() {
        let id = projects[index].bundle.manifest.id.as_str();
        cmd.arg("--extern")
            .arg(format!("mod_{position}={}", rlibs[id].display()));
        if let Some(facade) = facades.get(id) {
            cmd.arg("--extern")
                .arg(format!("mod_{position}_internals={}", facade.display()));
        }
    }
    // The game's libraries load from beside the executable.
    let origin = match std::env::consts::OS {
        "linux" => Some("-Wl,-rpath,$ORIGIN"),
        "macos" => Some("-Wl,-rpath,@executable_path"),
        _ => None,
    };
    for arg in origin.into_iter().map(str::to_owned).chain(sdk.link_args) {
        cmd.arg("-C").arg(format!("link-arg={arg}"));
    }
    cmd.arg("-o").arg(&binary).arg(&main);
    super::run(cmd, "Linking the game", |_| {})?;
    Ok((binary, runtime_libraries(rust, &deps)?))
}

/// Compiles `project`'s crate as an rlib into `out` with `rustc` (given the edition: rustc with
/// the route's search paths), its game packages from `library` and its installed mod dependencies
/// from `rlibs`. Returns the rlib and the crate's name.
pub(super) fn compile_mod<'a>(
    project: &api::Project<'_>,
    declared: &[dependencies::Dependency],
    out: &Path,
    rustc: impl Fn(&str) -> Command,
    library: impl Fn(&str) -> &'a PathBuf,
    rlibs: &std::collections::HashMap<&str, PathBuf>,
) -> Result<(PathBuf, String), String> {
    let id = project.bundle.manifest.id.as_str();
    let edition = project
        .cargo
        .get("package")
        .and_then(|p| p.get("edition"))
        .and_then(toml::Value::as_str)
        .unwrap_or("2015");
    let krate = project.crate_name();
    // Compile an isolated copy of the mod crate.
    let staged = out.join("staged").join(id);
    super::copy_bundle(project.bundle, &staged)?;
    let source = staged.join(
        project
            .cargo
            .get("lib")
            .and_then(|l| l.get("path"))
            .and_then(toml::Value::as_str)
            .unwrap_or("src/lib.rs"),
    );
    let mut cmd = rustc(edition);
    cmd.args(["--crate-type", "rlib", "--crate-name", &krate])
        .arg("-C")
        .arg(format!("metadata={id}"));
    for (name, package) in project.api_dependencies() {
        cmd.arg("--extern")
            .arg(format!("{name}={}", library(package).display()));
    }
    for dependency in declared {
        let Some(rlib) = rlibs.get(dependency.id.as_str()) else {
            continue;
        };
        cmd.arg("--extern").arg(format!(
            "{}={}",
            dependency.id.replace('-', "_"),
            rlib.display()
        ));
        if dependency.optional {
            cmd.arg("--cfg")
                .arg(format!("feature=\"{}\"", dependency.id));
        }
    }
    cmd.arg("--out-dir").arg(out).arg(&source);
    super::run(cmd, &format!("Compiling {id}"), |_| {})?;
    Ok((out.join(format!("lib{krate}.rlib")), krate))
}

/// The runtime shipped beside the prebuilt executable, for every native build route.
pub(super) fn runtime_libraries(rust: &Toolchain, deps: &Path) -> Result<Vec<PathBuf>, String> {
    let mut runtime = game_dylibs(deps)?;
    runtime.push(standard_library(rust)?);
    if rust.host.target == rust_toolchain::WINDOWS_X64_TARGET {
        // The SDK's GNU DLLs import libgcc, which in turn imports libwinpthread.
        // The compiler finds both through PATH during linking. The game loads them beside its executable.
        let mingw = rust.mingw_bin();
        for name in ["libgcc_s_seh-1.dll", "libwinpthread-1.dll"] {
            let library = mingw.join(name);
            if !library.is_file() {
                return Err(trf(
                    "The toolchain runtime library {file} is missing.",
                    &[("file", &library.display().to_string())],
                ));
            }
            runtime.push(library);
        }
    }
    Ok(runtime)
}

/// The game's own dynamic libraries, which the executable loads from beside itself: the game and
/// the wasm mod runtime it links (`tools/prebuilt_game.py` checks the archive holds both).
fn game_dylibs(deps: &Path) -> Result<Vec<PathBuf>, String> {
    let top = if library(deps, "horizon_mod_host").is_ok() {
        "horizon_mod_host"
    } else {
        "horizon_client"
    };
    let crates = [top, "horizon_mod_wasm"];
    crates
        .into_iter()
        .map(|krate| {
            let path = library(deps, krate)?;
            if path.extension().is_some_and(|ext| ext != "rmeta") {
                Ok(path)
            } else {
                Err(trf(
                    "The prebuilt game has no {crate}.",
                    &[("crate", krate)],
                ))
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_is_chosen_once_from_the_whole_set() {
        use horizon_mod_package::manifest::{loops, mixin, Internal, Module};
        let folder = super::super::tests::dir("prebuilt-route");
        let write = |name: &str, dependencies: &str| {
            let dir = folder.join(name);
            super::super::tests::write_bundle(&dir, name, "");
            fs::write(
                dir.join("Cargo.toml"),
                format!(
                    "[package]\nname='{name}'\nversion='0.1.0'\n[dependencies]\n{dependencies}\n"
                ),
            )
            .unwrap();
            super::super::bundle_in(&dir).unwrap().unwrap()
        };
        let api_only = write(
            "api_only",
            "horizon-mod-api='0.1'\nhorizon-mod-points='0.1'",
        );
        let facade = write("facade", "horizon-mod-api='0.1'\nhorizon-internals='0.1'");
        let registry = write("registry", "horizon-mod-api='0.1'\nserde='1'");
        let with = |change: &dyn Fn(&mut horizon_mod_package::manifest::Manifest)| {
            let mut bundle = api_only.clone();
            change(&mut bundle.manifest);
            bundle
        };
        let native = |change: &dyn Fn(&mut horizon_mod_package::manifest::Native)| {
            with(&|manifest| change(manifest.native.as_mut().unwrap()))
        };
        let forwarded = || mixin::Forwarded {
            id: "f".into(),
            fields: toml::Table::new(),
        };
        let engine = [
            native(&|n| {
                n.mixin.push(mixin::Mixin {
                    id: "m".into(),
                    target: "Game::tick".into(),
                    kind: mixin::Kind::Head,
                    helper: "m.rs".into(),
                    options: toml::Table::new(),
                })
            }),
            native(&|n| {
                n.module.push(Module {
                    id: "m".into(),
                    file: "game.rs".into(),
                    name: "m".into(),
                    source: "m.rs".into(),
                    visibility: None,
                })
            }),
            native(&|n| n.bridge.push(forwarded())),
            native(&|n| n.state.push(forwarded())),
            native(&|n| {
                n.r#loop.push(
                    toml::from_str::<loops::Loop>("id='l'\nloop='x.tick'\nhelper='l.rs'\nwith='L'")
                        .unwrap(),
                )
            }),
            with(&|m| {
                m.internals.push(Internal {
                    path: "horizon_client::Game".into(),
                    kind: None,
                    hash: None,
                })
            }),
            facade,
        ];
        let route = |bundles: &[&super::super::Bundle]| {
            let projects: Vec<_> = bundles
                .iter()
                .map(|bundle| api::Project::read(bundle).unwrap())
                .collect();
            Route::of(&projects)
        };
        assert_eq!(route(&[&api_only, &api_only]), Route::Api);
        assert_eq!(route(&[]), Route::Api);
        for needs in &engine {
            // One mod that needs the engine sends the whole set there, whichever its place.
            assert_eq!(
                route(&[&api_only, needs]),
                Route::Engine,
                "{:?}",
                needs.manifest
            );
            assert_eq!(route(&[needs, &api_only]), Route::Engine);
            assert_eq!(route(&[needs, &registry]), Route::Registry);
        }
        assert_eq!(route(&[&api_only, &registry]), Route::Registry);
        assert_eq!(Route::Api.name(), "prebuilt-api");
        assert_eq!(Route::Engine.name(), "prebuilt");
    }

    #[test]
    fn facades_refuse_build_only_crates_and_ambiguous_runtime_metadata() {
        let deps = super::super::tests::dir("prebuilt-facade-eligibility");
        for name in [
            "libfixture_meta-host.rmeta",
            "libfixture_meta-target.rmeta",
            "libbuild_tool-one.rmeta",
            "libhorizon_core-one.rmeta",
        ] {
            fs::write(deps.join(name), b"").unwrap();
        }
        let mut sdk: Identity = serde_json::from_value(serde_json::json!({
            "commit": "test", "rustc": "test", "target": "test", "side": "client",
            "link_args": [], "facade_crates": ["horizon_core"],
        }))
        .unwrap();
        assert_eq!(
            facade_library(&deps, "horizon_core", &sdk).unwrap(),
            deps.join("libhorizon_core-one.rmeta")
        );
        for krate in ["fixture_meta", "build_tool"] {
            let refusal = facade_library(&deps, krate, &sdk).unwrap_err();
            assert!(refusal.contains(krate), "{refusal}");
            assert!(
                refusal.contains("build-only or has ambiguous metadata"),
                "{refusal}"
            );
        }
        // Even an incorrect allowlist cannot make lookup select one of two variants.
        sdk.facade_crates.insert("fixture_meta".into());
        assert!(facade_library(&deps, "fixture_meta", &sdk).is_err());
        fs::remove_dir_all(deps).unwrap();
    }

    #[test]
    fn the_executable_carries_the_game_and_its_wasm_runtime() {
        let deps = super::super::tests::dir("prebuilt-dylibs");
        let ext = std::env::consts::DLL_EXTENSION;
        fs::write(deps.join(format!("libhorizon_client.{ext}")), b"").unwrap();
        fs::write(deps.join("libhorizon_mod_wasm-0123.rmeta"), b"").unwrap();
        // Metadata alone has no code to load: the archive is incomplete.
        assert!(game_dylibs(&deps).is_err());
        fs::write(deps.join(format!("libhorizon_mod_wasm.{ext}")), b"").unwrap();
        assert_eq!(game_dylibs(&deps).unwrap().len(), 2);
        fs::write(deps.join(format!("libhorizon_mod_host.{ext}")), b"").unwrap();
        let names: Vec<_> = game_dylibs(&deps)
            .unwrap()
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                format!("libhorizon_mod_host.{ext}"),
                format!("libhorizon_mod_wasm.{ext}")
            ]
        );
        fs::remove_dir_all(deps).unwrap();
    }

    #[test]
    fn prebuilt_runtime_carries_the_managed_gnu_dlls_on_windows_x64() {
        let scratch = tempfile::tempdir().unwrap();
        let deps = scratch.path().join("deps");
        fs::create_dir(&deps).unwrap();
        let game = deps.join("horizon_mod_host.dll");
        let wasm = deps.join("horizon_mod_wasm.dll");
        for library in [&game, &wasm] {
            fs::write(library, b"").unwrap();
        }
        for host in rust_toolchain::hosts() {
            let rust = Toolchain {
                dir: scratch.path().join(host.target),
                host,
            };
            let std = rust.target_libdir().unwrap().join("std-test.dll");
            fs::create_dir_all(std.parent().unwrap()).unwrap();
            fs::write(&std, b"").unwrap();
            let mut expected = vec![game.clone(), wasm.clone(), std];
            if host.target == rust_toolchain::WINDOWS_X64_TARGET {
                let bin = rust.mingw_bin();
                fs::create_dir_all(&bin).unwrap();
                for name in ["libgcc_s_seh-1.dll", "libwinpthread-1.dll"] {
                    assert!(runtime_libraries(&rust, &deps).unwrap_err().contains(name));
                    let library = bin.join(name);
                    fs::write(&library, b"").unwrap();
                    expected.push(library);
                }
            }
            assert_eq!(runtime_libraries(&rust, &deps).unwrap(), expected);
        }
    }

    /// What [`refusal`] says of one mod whose Cargo.toml is `cargo`.
    fn refusal_of(cargo: &str) -> Option<Refusal> {
        let name = std::thread::current().name().unwrap().replace(':', "-");
        let dir = super::super::tests::dir(&name);
        super::super::tests::write_bundle(&dir, "example", "");
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname='example-mod'\nversion='1.0.0'\nedition='2024'\n{cargo}\n"),
        )
        .unwrap();
        let bundle = super::super::bundle_in(&dir).unwrap().unwrap();
        refusal(&[api::Project::read(&bundle).unwrap()])
    }

    #[test]
    fn only_the_exact_build_of_the_prebuilt_game_is_accepted() {
        let dir = super::super::tests::dir("prebuilt-exact-build");
        let check = |commit: &str, rustc: &str, target: &str, side: &str| {
            let identity = serde_json::json!({
                "package_format": horizon_mod_package::sdk::PACKAGE_FORMAT,
                "commit": commit, "rustc": rustc, "target": target, "side": side, "link_args": [],
            });
            fs::write(dir.join(IDENTITY), identity.to_string()).unwrap();
            check_identity(&dir, "abc123", "x86_64-pc-windows-gnu", Target::Client)
        };
        let rustc = format!("rustc x\nrelease: {}\nhost: y", rust_toolchain::VERSION);
        let host = "x86_64-pc-windows-gnu";
        assert_eq!(check("abc123", &rustc, host, "client"), Ok(()));
        assert!(check("abc124", &rustc, host, "client").is_err());
        assert!(check("abc123", &rustc, "x86_64-unknown-linux-gnu", "client").is_err());
        assert!(check("abc123", &rustc, host, "server").is_err());
        assert!(check("abc123", "rustc x\nrelease: 0.0.1", host, "client").is_err());
        let longer = format!("release: {}.1", rust_toolchain::VERSION);
        assert!(check("abc123", &longer, host, "client").is_err());
        // A developer's own prebuilt game is used where it is, and only when it is that build.
        if let Some(this) = rust_toolchain::host() {
            assert_eq!(
                check("abc123", &rustc, this.target, "client").is_ok(),
                this.target == host
            );
            // Another build of a developer's own game is refused before anything is laid out.
            assert!(fetch("abc124", Target::Client, Some(&dir), || ()).is_err());
        }
        fs::remove_file(dir.join(IDENTITY)).unwrap();
        assert!(check_identity(&dir, "abc123", host, Target::Client).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_package_is_laid_out_as_its_slices_and_whole_again() {
        let dir = super::super::tests::dir("prebuilt-lay-out");
        let whole = dir.join("whole");
        let write = |slices: serde_json::Value| {
            let identity = serde_json::json!({
                "package_format": horizon_mod_package::sdk::PACKAGE_FORMAT,
                "commit": "abc123", "rustc": "", "target": "", "side": "client", "link_args": [],
                "slices": slices,
            });
            fs::write(whole.join(IDENTITY), identity.to_string()).unwrap();
        };
        for (name, text) in [
            ("bin/horizon-client", "exe"),
            ("deps/libmod_api-1.rmeta", "api"),
            ("deps/libhorizon_client-1.rlib", "engine"),
            ("points.json", "{}"),
        ] {
            fs::create_dir_all(whole.join(name).parent().unwrap()).unwrap();
            fs::write(whole.join(name), text).unwrap();
        }
        let runtime = serde_json::json!([IDENTITY, "bin/horizon-client"]);
        write(
            serde_json::json!({"runtime": runtime, "api": ["deps/libmod_api-1.rmeta", "points.json"]}),
        );
        let package = lay_out(&whole, &dir.join("linked"), true).unwrap();
        assert_eq!(package, Package::at(&dir.join("linked")));
        let read = |path: PathBuf| fs::read_to_string(path).unwrap();
        assert_eq!(read(package.runtime.join("bin/horizon-client")), "exe");
        assert_eq!(read(package.api.join("deps/libmod_api-1.rmeta")), "api");
        assert_eq!(
            read(package.engine.join("deps/libhorizon_client-1.rlib")),
            "engine"
        );
        assert!(!package.engine.join("points.json").exists());
        // Linking leaves the developer's folder as it was.
        assert_eq!(read(whole.join("bin/horizon-client")), "exe");
        let union = package.whole(&dir.join("union")).unwrap();
        for name in [
            "bin/horizon-client",
            "deps/libmod_api-1.rmeta",
            "deps/libhorizon_client-1.rlib",
            IDENTITY,
        ] {
            assert!(union.join(name).is_file(), "{name}");
        }
        // A listed file that is absent, or a name out of the package, is refused.
        write(serde_json::json!({"runtime": runtime, "api": ["deps/gone.rmeta"]}));
        let error = lay_out(&whole, &dir.join("absent"), true).unwrap_err();
        assert!(error.contains("deps/gone.rmeta"), "{error}");
        write(serde_json::json!({"runtime": runtime, "api": ["../points.json"]}));
        // Even when that file exists, outside the package.
        fs::write(dir.join("points.json"), "{}").unwrap();
        let error = lay_out(&whole, &dir.join("escape"), true).unwrap_err();
        assert!(error.contains("not a plain path"), "{error}");
        write(serde_json::json!(null));
        assert!(lay_out(&whole, &dir.join("none"), true).is_err());
        // Moving (an unpacked download) leaves nothing behind.
        write(
            serde_json::json!({"runtime": runtime, "api": ["deps/libmod_api-1.rmeta", "points.json"]}),
        );
        lay_out(&whole, &dir.join("moved"), false).unwrap();
        assert!(!whole.join("bin/horizon-client").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mods_on_the_mod_api_alone_take_the_prebuilt_game() {
        assert_eq!(refusal_of("[dependencies]\nhorizon-mod-api='0.1'"), None);
        assert_eq!(
            refusal_of("build=false\n[dependencies]\nhorizon-mod-api='0.1'"),
            None
        );
        assert_eq!(
            refusal_of("[dependencies.api]\npackage='horizon-mod-api'\nversion='0.1'"),
            None
        );
    }

    #[test]
    fn registry_dependencies_do_not_rebuild_the_game() {
        for cargo in [
            "[dependencies]\nhorizon-mod-api='0.1'\nserde='1'",
            "[dependencies]\nhorizon-mod-api='0.1'\n[target.'cfg(unix)'.dependencies]\nlibc='0.2'",
        ] {
            assert_eq!(refusal_of(cargo), None);
        }
    }

    #[test]
    fn anything_the_prebuilt_game_cant_build_names_why() {
        let id = || "example".to_owned();
        for (cargo, reason) in [
            (
                "[dependencies]\nhorizon-mod-api={version='0.1',features=['testing']}",
                Refusal::Features { mod_id: id() },
            ),
            (
                "[dependencies]\nhorizon-mod-api='0.1'\n[features]\nextra=[]",
                Refusal::Features { mod_id: id() },
            ),
            (
                "build='make.rs'\n[dependencies]\nhorizon-mod-api='0.1'",
                Refusal::BuildScript { mod_id: id() },
            ),
        ] {
            assert_eq!(refusal_of(cargo), Some(reason), "{cargo}");
        }
    }

    #[test]
    fn an_older_package_names_its_own_format() {
        let dir = super::super::tests::dir("prebuilt-older-format");
        let refusal = |text: &str| {
            fs::write(dir.join(IDENTITY), text).unwrap();
            identity(&dir).err().unwrap()
        };
        let error = refusal(r#"{"format_version": 5}"#);
        assert!(error.contains("SDK format 5 is too old"), "{error}");
        let error = refusal("{}");
        assert!(error.contains("SDK format 0 is too old"), "{error}");
        let newer = horizon_mod_package::sdk::PACKAGE_FORMAT + 1;
        let error = refusal(&format!(r#"{{"package_format": {newer}}}"#));
        assert!(error.contains("update the launcher"), "{error}");
    }

    #[test]
    fn the_archive_is_found_by_name_in_the_release_sums() {
        let name = native::prebuilt_archive("x86_64-unknown-linux-gnu", true);
        assert_eq!(
            name,
            "horizon-prebuilt-v6-x86_64-unknown-linux-gnu-server.tar.gz"
        );
        let listing =
            format!("AB12  horizon-prebuilt-v6-x86_64-unknown-linux-gnu.tar.gz\ncd34 *{name}\n");
        assert_eq!(expected_sum(&listing, &name).as_deref(), Some("cd34"));
        assert_eq!(expected_sum(&listing, "horizon-prebuilt-x.tar.gz"), None);
    }
}
