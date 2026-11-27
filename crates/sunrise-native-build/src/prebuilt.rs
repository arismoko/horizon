//! Native mods compile and link against the pinned prebuilt game.
//! Unsupported inputs and unavailable SDKs are refused, never rebuilt locally.
use super::{MixinsIdentity, Target, api, dependencies, internals};
use crate::host::{tr, trf};
use crate::rust::Toolchain;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use sunrise_mod_package::native_route::{self, Requirement};

const IDENTITY: &str = "sunrise-prebuilt.json";
#[cfg(test)]
pub const TEMPLATE: &str = "native_main.rs.template";
/// The optimization level of the `release` profile the prebuilt game is built with: the mods' code
/// matches the game's.
const OPT_LEVEL: &str = "opt-level=3";

/// Why a build can't use the prebuilt game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A mod declares Cargo features, or asks the mod API for features of its own.
    Features { mod_id: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Features { mod_id } => trf(
                "E_NATIVE_FEATURES: {mod} requests Cargo features; use the pinned SDK feature set",
                &[("mod", mod_id)],
            ),
        };
        f.write_str(&text)
    }
}

impl Refusal {
    /// What the player reads about it, in the words of a mod that doesn't compile: the code and
    /// its remedy for the mod's author stay in the build log ([`Display`](std::fmt::Display)).
    pub fn plain(&self) -> String {
        let why = match self {
            Self::Features { mod_id } => trf(
                "{mod} can't be built here: it asks for build options Sunrise's game doesn't offer.",
                &[("mod", mod_id)],
            ),
        };
        format!(
            "{why} {}",
            tr(
                "The problem is in the mod, not in your game. Switch it off to play, or send the log to its author."
            )
        )
    }
}

/// The first structural reason these projects cannot use the prebuilt game.
pub fn refusal(projects: &[api::Project<'_>]) -> Option<Refusal> {
    projects.iter().find_map(project_refusal)
}

/// Why `project` alone can't build against the prebuilt game, or `None` when it can.
pub fn project_refusal(project: &api::Project<'_>) -> Option<Refusal> {
    let mod_id = || project.bundle.manifest.id.clone();
    if project
        .cargo
        .get("features")
        .and_then(toml::Value::as_table)
        .is_some_and(|t| !t.is_empty())
    {
        return Some(Refusal::Features { mod_id: mod_id() });
    }
    for (_, dependencies) in native_route::dependency_tables(&project.cargo) {
        for (key, value) in dependencies {
            let krate = value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(key);
            if !native_route::PACKAGES
                .iter()
                .any(|(package, _)| *package == krate)
            {
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

/// `name`'s sum in a `sha256sum` listing.
pub fn expected_sum(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let (sum, file) = line.split_once(char::is_whitespace)?;
        (file.trim_start().trim_start_matches('*') == name).then(|| sum.to_ascii_lowercase())
    })
}

pub fn side(target: Target) -> sunrise_mod_package::sdk::Side {
    match target {
        Target::Client => sunrise_mod_package::sdk::Side::Client,
        Target::Server => sunrise_mod_package::sdk::Side::Server,
    }
}

/// The identity fields used by the native builder, present in every published format-6 package
/// except where noted.
#[derive(serde::Deserialize)]
pub struct Identity {
    commit: String,
    rustc: String,
    target: String,
    side: String,
    pub registry_lock: String,
    pub registry_patches: toml::Table,
    pub native_links: Vec<super::registry_inventory::NativeLink>,
    /// The prebuilt API exports the native log sink and the entry template must install it.
    /// Added within format 6; earlier packages do not export this sink.
    #[serde(default)]
    pub native_log: bool,
    facade_crates: std::collections::BTreeSet<String>,
    /// The game crate's features the SDK was built with: a patched function copied out of its
    /// source is compiled under the same `cfg`s.
    game_features: Vec<String>,
    #[serde(default)]
    source_layout: SdkSourceLayout,
    #[serde(default)]
    runtime_role: Option<RuntimeRole>,
    /// The cfg atoms of the lower libraries in this target and side's build. Absent on SDKs
    /// whose tool could build a lower mirror without these records.
    lower_crate_cfg: Option<BTreeMap<String, Vec<String>>>,
    slices: Slices,
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum SdkSourceLayout {
    #[default]
    Legacy,
    Split,
}

#[derive(serde::Deserialize)]
pub struct RuntimeRole {
    pub side: sunrise_mod_package::sdk::Side,
    pub libraries: std::collections::BTreeMap<String, RuntimeLibrary>,
}

#[derive(serde::Deserialize)]
pub struct RuntimeLibrary {
    pub package: String,
    pub library: String,
    pub features: Vec<String>,
    pub aliases: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct Slices {
    pub runtime: Vec<String>,
    pub api: Vec<String>,
}

/// The prebuilt game as the launcher keeps one build of it: each of the SDK's three compiled
/// slices in a folder of its own under the build's folder (`runtime`, `api`, `engine`), each
/// unpacked on its own. The API route reads `runtime` and `api`; the other routes read all three.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub runtime: PathBuf,
    pub api: PathBuf,
    pub engine: PathBuf,
}

impl Package {
    /// The slices under the build folder `build`.
    pub fn at(build: &Path) -> Self {
        Self {
            runtime: build.join("runtime"),
            api: build.join("api"),
            engine: build.join("engine"),
        }
    }

    /// The three slices as one folder at `into`, linked file by file (copied where a link can't
    /// be made): the folder the engine and registry routes build against.
    pub fn whole(&self, into: &Path) -> Result<PathBuf, String> {
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
/// using the already checked identity `sdk` of `whole`. Move or link each file; refuse a listed
/// file that isn't there by name.
pub fn lay_out(whole: &Path, build: &Path, link: bool, sdk: &Identity) -> Result<Package, String> {
    let slices = sdk.slices();
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
    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn facade_crates(&self) -> impl Iterator<Item = &str> {
        self.facade_crates.iter().map(String::as_str)
    }

    pub fn runtime_role(&self) -> Option<&RuntimeRole> {
        self.runtime_role.as_ref()
    }

    pub(crate) fn patch_library(&self, owner: &str) -> Result<Option<&RuntimeLibrary>, String> {
        let Some(role) = self.runtime_role() else {
            return Ok(None);
        };
        let package = owner.replace('_', "-");
        let mut matches = role
            .libraries
            .values()
            .filter(|fact| fact.package == package);
        match (matches.next(), matches.next()) {
            (Some(library), None) => Ok(Some(library)),
            _ => Err(format!(
                "split SDK has no unique runtime library for {owner}"
            )),
        }
    }

    pub(crate) fn patch_features(&self, owner: &str) -> Result<&[String], String> {
        match self.patch_library(owner)? {
            Some(library) => Ok(&library.features),
            None => self.legacy_game_features(),
        }
    }

    pub fn legacy_game_features(&self) -> Result<&[String], String> {
        match self.source_layout {
            SdkSourceLayout::Legacy => Ok(&self.game_features),
            SdkSourceLayout::Split => {
                Err(tr("This SDK requires per-library patch compilation.").to_string())
            }
        }
    }

    pub(crate) fn lower_crate_cfg(&self) -> Option<&BTreeMap<String, Vec<String>>> {
        self.lower_crate_cfg.as_ref()
    }

    /// Which package files the `runtime` and `api` slices hold.
    pub fn slices(&self) -> &Slices {
        &self.slices
    }
}

/// Read a published package's identity. Check its format before interpreting its fields.
pub fn identity(dir: &Path) -> Result<Identity, String> {
    let path = dir.join(IDENTITY);
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[derive(serde::Deserialize)]
    struct Format {
        package_format: u32,
    }
    let Format {
        package_format: found,
    } = serde_json::from_str(&text).map_err(|e| unreadable_identity(&path, &e))?;
    let supported = sunrise_mod_package::sdk::PACKAGE_FORMAT;
    let (version, supported_text) = (found.to_string(), supported.to_string());
    let args = [
        ("version", version.as_str()),
        ("format", supported_text.as_str()),
    ];
    match found {
        found if found == supported => {
            let identity: Identity =
                serde_json::from_str(&text).map_err(|e| unreadable_identity(&path, &e))?;
            match (&identity.source_layout, identity.runtime_role()) {
                (SdkSourceLayout::Legacy, None) => {}
                (SdkSourceLayout::Split, Some(role)) => {
                    let side = match role.side {
                        sunrise_mod_package::sdk::Side::Client => "client",
                        sunrise_mod_package::sdk::Side::Server => "server",
                    };
                    if side != identity.side
                        || role.libraries.is_empty()
                        || role.libraries.values().any(|library| {
                            library
                                .aliases
                                .values()
                                .any(|name| !role.libraries.contains_key(name))
                        })
                    {
                        return Err(format!(
                            "{}: invalid {side} SDK runtime library facts",
                            path.display()
                        ));
                    }
                }
                _ => {
                    return Err(format!(
                        "{}: SDK source layout and runtime facts disagree",
                        path.display()
                    ));
                }
            }
            Ok(identity)
        }
        found if found < supported => Err(trf(
            "SDK format {version} is too old for this tool (format {format}); the SDK is older than this tool: use the tool in the SDK's own release, or pin a newer SDK with cargo sunrise-mod pin",
            &args,
        )),
        _ => Err(trf(
            "SDK format {version} is newer than this tool's supported format {format}; this tool is older than the SDK: update cargo sunrise-mod, or use the tool in the SDK's own release",
            &args,
        )),
    }
}

/// The refusal for an identity file that does not parse. A field the file lacks or mistypes means
/// the SDK was written by an older layout of this format than this tool reads; a file that is not
/// JSON at all is a damaged download.
fn unreadable_identity(path: &Path, error: &serde_json::Error) -> String {
    let shown = path.display().to_string();
    let why = error.to_string();
    let args = [("path", shown.as_str()), ("why", why.as_str())];
    if error.is_data() {
        trf(
            "{path}: {why}; the SDK is older than this tool: use the tool in the SDK's own release, or pin a newer SDK with cargo sunrise-mod pin",
            &args,
        )
    } else {
        trf("{path}: {why}; fetch the SDK again", &args)
    }
}

/// Read the identity of the unpacked game in `dir`, refusing one not built from `revision`,
/// for `host` and `target`'s side, or with the Rust release that reads its metadata.
pub fn check_identity(
    dir: &Path,
    revision: &str,
    host: &str,
    target: Target,
) -> Result<Identity, String> {
    let found = identity(dir)?;
    let release = format!("release: {}", crate::rust::VERSION);
    let matches = found.commit == revision
        && found.target == host
        && found.side == target.name()
        && found.rustc.lines().any(|line| line == release);
    if matches {
        return Ok(found);
    }
    // Only the Rust release differs: say which side is older.
    let numbers = |release: &str| {
        release
            .split('.')
            .map(|part| part.parse::<u32>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let theirs = found
        .rustc
        .lines()
        .find_map(|line| line.strip_prefix("release: "));
    if found.commit == revision
        && found.target == host
        && found.side == target.name()
        && let Some(theirs) = theirs
        && let (Some(theirs_numbers), Some(ours_numbers)) =
            (numbers(theirs), numbers(crate::rust::VERSION))
    {
        let args = [("rust", theirs), ("this", crate::rust::VERSION)];
        return Err(if theirs_numbers > ours_numbers {
            trf(
                "The prebuilt game was made with Rust {rust}, newer than this tool's {this}; this tool is older than the SDK: update cargo sunrise-mod, or use the tool in the SDK's own release",
                &args,
            )
        } else {
            trf(
                "The prebuilt game was made with Rust {rust}, older than this tool's {this}; the SDK is older than this tool: use the tool in the SDK's own release, or pin a newer SDK with cargo sunrise-mod pin",
                &args,
            )
        });
    }
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

/// Does this plan need the SDK's own tool because it copies a lower-crate function from a
/// build that predates lower-crate cfg records? A present map belongs to the new tool, even
/// if an individual target's record is missing and the generator must refuse it.
pub fn needs_pinned_patch_tool(package: &Package, bundle: &crate::Bundle) -> Result<bool, String> {
    use sunrise_platform::mod_build::api_gen::functions;

    let copies_lower_crate = bundle.native().mixin.iter().any(|row| {
        functions::key(&row.target)
            .split_once("::")
            .is_some_and(|(krate, _)| krate != "sunrise_client")
    });
    if copies_lower_crate {
        let sdk = identity(&package.runtime)?;
        Ok(sdk.runtime_role().is_none() && sdk.lower_crate_cfg().is_none())
    } else {
        Ok(false)
    }
}

/// The library file a crate named `krate` left in `deps`: its metadata, or its own dylib.
pub fn library(deps: &Path, krate: &str) -> Result<PathBuf, String> {
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
            (name.starts_with(&prefix) && name.ends_with(".rmeta"))
                || name == format!("lib{krate}.rmeta")
        })
        .collect();
    match candidates.as_slice() {
        [path] => Ok(path.clone()),
        [] => Err(trf(
            "The prebuilt game has no {crate}, a library this tool needs: the SDK it comes from is older than the tool. Pin a current build (`cargo sunrise-mod pin nightly`), or build with the tool in that SDK's own release (`sdk-<build>/tool-<target>.tar.xz`).",
            &[("crate", krate)],
        )),
        _ => Err(facade_refused(krate)),
    }
}

pub(crate) fn role_library(deps: &Path, fact: &RuntimeLibrary) -> Result<PathBuf, String> {
    if fact.artifacts.is_empty() {
        return library(deps, &fact.library);
    }
    let mut metadata = None;
    for name in &fact.artifacts {
        let path = Path::new(name);
        if path.file_name().and_then(|name| name.to_str()) != Some(name.as_str()) {
            return Err(format!("invalid SDK library artifact {name:?}"));
        }
        let file = deps.join(path);
        if matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("so" | "dll" | "dylib")
        ) && file.is_file()
        {
            return Ok(file);
        }
        if path.extension().is_some_and(|ext| ext == "rmeta")
            && file.is_file()
            && metadata.replace(file).is_some()
        {
            return Err(facade_refused(&fact.library));
        }
    }
    metadata.ok_or_else(|| {
        format!(
            "SDK runtime library {} has no packaged artifact",
            fact.library
        )
    })
}

/// Both compiler routes link the same SDK-owned executable template, and the patch library, so `sunrise_mod_host`
/// is always an extern: the game crates it carries (`sunrise_client` and its closure, metadata only in `deps/`) are
/// code only inside that dylib.
pub fn wrapper_externs(deps: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    let mut externs = BTreeMap::new();
    for name in [
        "sunrise_client",
        "mod_api",
        "mod_vocab",
        "sunrise_mod_package",
    ] {
        externs.insert(name.to_owned(), library(deps, name)?);
    }
    externs.insert(
        "sunrise_mod_host".into(),
        library(deps, "sunrise_mod_host")?,
    );
    Ok(externs)
}

fn facade_refused(krate: &str) -> String {
    trf(
        "The prebuilt game cannot expose {crate}: it is build-only or has ambiguous metadata.",
        &[("crate", krate)],
    )
}

/// What the patch library's source ends with: edition 2024 loads an `--extern` only when the source names it, and
/// the dylib links the game's crates only through `sunrise_mod_host`.
pub const PATCH_HOST_LINK: &str = "\nextern crate sunrise_mod_host as _;\n";

pub fn facade_library(deps: &Path, krate: &str, sdk: &Identity) -> Result<PathBuf, String> {
    if !sdk.facade_crates.contains(krate) {
        return Err(facade_refused(krate));
    }
    library(deps, krate)
}

/// Rust's standard library as a dynamic library: what the prebuilt game and its executable load.
fn standard_library(rust: &dyn Toolchain) -> Result<PathBuf, String> {
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
fn rustc(rust: &dyn Toolchain, game: &Path, out: &Path, edition: &str) -> Command {
    let mut cmd = rustc_with(
        rust,
        &[&game.join("deps"), &game.join("host"), out],
        edition,
    );
    cmd.arg("-L")
        .arg(format!("native={}", game.join("native").display()));
    cmd
}

/// Find runtime dependencies beside the dylib, not relative to the executable. On Linux keep
/// DT_RPATH rather than DT_RUNPATH so the search path also applies to transitive dependencies.
/// The SDK release smoke build also reads this list.
pub(super) fn dylib_rpath_args(os: &str) -> impl Iterator<Item = &'static str> + '_ {
    include_str!("dylib-link-args.txt")
        .lines()
        .filter_map(move |line| line.strip_prefix(os)?.strip_prefix(' '))
}

/// rustc with the prebuilt game's settings and only `search` as dependency search paths.
pub fn rustc_with(rust: &dyn Toolchain, search: &[&Path], edition: &str) -> Command {
    let mut cmd = rust.command(&rust.rustc());
    crate::rust::low_priority(&mut cmd);
    cmd.args([
        "--edition",
        edition,
        "-C",
        "prefer-dynamic",
        "-C",
        OPT_LEVEL,
    ]);
    for arg in dylib_rpath_args(std::env::consts::OS) {
        cmd.arg("-C").arg(format!("link-arg={arg}"));
    }
    for folder in search {
        cmd.arg("-L")
            .arg(format!("dependency={}", folder.display()));
    }
    cmd
}

/// How an instance's native mods are built against the prebuilt game: chosen once, from the whole
/// set of mods, and never changed after a failure (SDK design §1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Route {
    /// Every mod is API-only: build its own dylib from the package's API metadata.
    /// The engine archive is never read.
    Api,
    /// A mod has a `[native]` table row (mixin, module, bridge, state, loop) or game internals:
    /// build its dylib against the whole package's metadata.
    Engine,
    /// A mod depends on a crate outside the game's, or has a build script: its dylib is built
    /// with Cargo against the whole package ([`super::registry_compile`]), which runs the build
    /// scripts.
    Registry,
}

impl Route {
    /// The route of the set `projects`: the strongest one any of its mods needs.
    pub fn of(projects: &[api::Project<'_>]) -> Self {
        projects
            .iter()
            .map(|project| {
                match native_route::requirement(
                    &project.bundle.manifest,
                    &project.cargo,
                    &project.bundle.dir,
                ) {
                    Requirement::Api => Self::Api,
                    Requirement::Engine(_) => Self::Engine,
                    Requirement::Registry(_) | Requirement::BuildScript => Self::Registry,
                }
            })
            .max()
            .unwrap_or(Self::Api)
    }

    /// The route's name in the SDK's input list (`sdk.json`'s `inputs.routes`): which slices
    /// it fetches.
    pub fn inputs(self) -> &'static str {
        match self {
            Self::Api => "api",
            Self::Engine | Self::Registry => "engine",
        }
    }

    /// The route as the build record (`native-mods.json`) names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Api => "prebuilt-api",
            Self::Engine | Self::Registry => "prebuilt",
        }
    }
}

/// Creates `out`, a build folder directly under `builds`, itself directly under the native work
/// folder, refusing either when it is a link (a symlink or a Windows junction) or resolves outside
/// its parent: the build clears folders inside `out`, and must never reach a folder elsewhere
/// through it.
fn own_folder(builds: &Path, out: &Path) -> Result<(), String> {
    let work = builds
        .parent()
        .ok_or_else(|| format!("{}: no work folder above it", builds.display()))?;
    for (parent, dir) in [(work, builds), (builds, out)] {
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let meta = fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let inside = || -> std::io::Result<bool> {
            Ok(fs::canonicalize(dir)?.parent() == Some(fs::canonicalize(parent)?.as_path()))
        };
        if meta.file_type().is_symlink() || !meta.is_dir() || !inside().unwrap_or(false) {
            return Err(format!(
                "{}: not a folder of its own under {}; remove it and build again",
                dir.display(),
                parent.display()
            ));
        }
    }
    Ok(())
}

/// The compiled outputs and the pinned catalog used to record the mods' internals.
pub struct Built {
    pub binary: PathBuf,
    pub libraries: Vec<PathBuf>,
    pub catalog: internals::Catalog,
}

/// Compiles the mods (`resolution`'s order, each with its installed mod dependencies) against
/// the prebuilt game `package`, by `route` ([`Route::of`] the same `projects`), under `builds`
/// (`native/prebuilt-build`), in folders keyed by the SDK's inputs ([`kept_build`]): the API route
/// in `api-<key>`, the engine route in `rustc-<key>`, each mod's library reused only while its own
/// inputs are unchanged, and the registry route's cargo build in `<key>`, staged per mods folder
/// (`mods`). Returns the SDK's unchanged game executable, the runtime and compiled mod libraries
/// to install beside it, and the build's validated internals catalog.
// Unlike the source workspace, this route invokes rustc with only selected --extern
// libraries; it does not resolve or compile arbitrary transitive Cargo dependencies.
#[allow(clippy::too_many_arguments)]
pub fn compile(
    rust: &dyn Toolchain,
    package: &Package,
    builds: &Path,
    mods: &Path,
    route: Route,
    projects: &[api::Project<'_>],
    declared: &[Vec<dependencies::Dependency>],
    resolution: &dependencies::Resolution,
    consent: Option<&dyn Fn(&super::RegistryInventory) -> bool>,
    // Each mod is its own library: told which one starts, and how far through them.
    on_mod: &dyn Fn(&str, usize, usize),
    check_only: bool,
) -> Result<Option<Built>, String> {
    let sdk = identity(&package.runtime)?;
    let catalog = internals::Catalog::read(&package.api, sdk.commit())?;
    let mixins = MixinsIdentity::resolve(projects.iter().map(|project| project.bundle), None)?;
    let mut names = reserved_library_names();
    for item in fs::read_dir(package.runtime.join("deps")).map_err(|e| e.to_string())? {
        names.insert(
            item.map_err(|e| e.to_string())?
                .file_name()
                .to_string_lossy()
                .to_ascii_lowercase(),
        );
    }
    for project in projects {
        check_library_name(
            &mut names,
            &dylib_name(&project.crate_name()),
            &project.bundle.manifest.id,
        )?;
    }
    // Every route's outputs are kept per SDK build (its files, the toolchain and the launcher's
    // settings): a library reused across builds was always compiled from the same SDK inputs.
    let key = kept_build(
        rust.target(),
        crate::contract::TOOL_COMMIT,
        package,
        sdk.commit(),
    )?;
    if route == Route::Api {
        let out = &builds.join(format!("api-{key}"));
        own_folder(builds, out)?;
        own_folder(out, &out.join("staged"))?;
        let built = super::api_route::compile(
            rust,
            package,
            &sdk,
            out,
            projects,
            declared,
            resolution,
            super::api_route::Options { on_mod, check_only },
        );
        // Stamped after the route has made its folder again, failed or not.
        if out.exists() {
            crate::host::mark_used(out)?;
        }
        return Ok(built?.map(|(binary, libraries)| Built {
            binary,
            libraries,
            catalog,
        }));
    }
    let game = &package.whole(&builds.with_file_name("prebuilt-package"))?;
    if route == Route::Registry {
        let kept = builds.join(&key);
        own_folder(builds, &kept)?;
        let target = kept.join("target");
        // Check an existing target before restaging. Cargo still creates an absent target
        // itself, after consent. Inspect the directory entry so dangling links are refused too.
        match fs::symlink_metadata(&target) {
            Ok(_) => own_folder(&kept, &target)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => return Err(format!("{}: {error}", target.display())),
        }
        // Stamped before it fills, so a build that fails still leaves an aged unit.
        crate::host::mark_used(&kept)?;
        let stage = kept.join(format!("stage-{}", short_hash(&mods_key(mods)?)));
        let built = super::registry_compile::stage_and_cargo(
            rust,
            game,
            &stage,
            &target,
            projects,
            declared,
            resolution,
            super::registry_compile::Options {
                consent,
                adapter: &std::env::current_exe().map_err(|e| e.to_string())?,
                mixins: &mixins,
                sdk: &sdk,
                catalog: &catalog,
            },
            check_only,
        )?;
        // The Storage page's Build files row ages this unit by its last use.
        crate::host::mark_used(&kept)?;
        return Ok(built.map(|(binary, libraries)| Built {
            binary,
            libraries,
            catalog,
        }));
    }
    let out = &builds.join(format!("rustc-{key}"));
    let generated = catalog
        .prepare(projects.iter().map(|project| &project.bundle.manifest))
        .map_err(|e| e.to_string())?;
    for project in projects {
        catalog.log_mixins(&project.bundle.manifest)?;
    }
    own_folder(builds, out)?;
    own_folder(out, &out.join("staged"))?;
    crate::host::mark_used(out)?;
    let deps = game.join("deps");
    let mut libraries = std::collections::HashMap::new();
    for (package, name) in native_route::PACKAGES {
        if package != native_route::INTERNALS_PACKAGE {
            libraries.insert(name, library(&deps, name)?);
        }
    }
    let game_library = library(&deps, "sunrise_client")?;
    let mut facades = std::collections::HashMap::new();
    libraries.insert("mod_vocab", library(&deps, "mod_vocab")?);
    let mut rlibs: std::collections::HashMap<&str, PathBuf> = std::collections::HashMap::new();
    let mut crate_names: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    let mut identities = super::library_record::Identities::default();
    for (n, &index) in resolution.order.iter().enumerate() {
        let project = &projects[index];
        let id = project.bundle.manifest.id.as_str();
        on_mod(id, n + 1, resolution.order.len());
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
                .arg(format!("sunrise_client={}", game_library.display()))
                .arg("-o")
                .arg(&rlib)
                .arg(source);
            for krate in &facade.crates {
                if krate != "sunrise_client" {
                    cmd.arg("--extern").arg(format!(
                        "{krate}={}",
                        facade_library(&deps, krate, &sdk)?.display()
                    ));
                }
            }
            super::run(
                cmd,
                &trf("Compiling {mod}'s declared internals", &[("mod", id)]),
            )?;
            facades.insert(id, rlib);
        }
        let source =
            super::source_digest(project.bundle).ok_or(tr("Could not hash native mod inputs"))?;
        let identity = identities.next(
            sdk.commit(),
            id,
            &project.bundle.manifest.version.to_string(),
            &source,
            declared[index]
                .iter()
                .map(|dependency| dependency.id.as_str()),
        );
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
            &library_entry(
                &api::template(&game.join(super::api_route::TEMPLATE))?,
                &[&project.bundle.manifest],
                &identity,
                &[],
            ),
            &identities,
            &source,
            check_only,
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
        &mixins,
        |edition| rustc(rust, game, out, edition),
        check_only,
    )?;
    if check_only {
        return Ok(None);
    }
    let mut installed = runtime_libraries(rust, &sdk, game)?;
    for &index in &resolution.order {
        installed.push(rlibs[projects[index].bundle.manifest.id.as_str()].clone());
    }
    if let Some(patch) = patch {
        let source = out.join("sunrise_patch.rs");
        let template = api::template(&game.join(super::api_route::TEMPLATE))?;
        let identity = identities.next(
            sdk.commit(),
            "sunrise_patch",
            "0",
            "none",
            patch.mods.iter().map(String::as_str),
        );
        let mut entry = library_entry(&template, &[], &identity, &patch.installs);
        entry.push_str(PATCH_HOST_LINK);
        fs::write(&source, &entry).map_err(|e| e.to_string())?;
        let library = out.join(dylib_name("sunrise_patch"));
        let mut hash = Sha256::new();
        hash.update(&entry);
        for (_, path) in &patch.rlibs {
            hash.update(fs::read(path).map_err(|e| e.to_string())?);
        }
        let fingerprint = format!("{:x}", hash.finalize());
        let saved = library.with_extension("fingerprint");
        let unchanged =
            library.is_file() && fs::read_to_string(&saved).ok().as_deref() == Some(&fingerprint);
        let mut cmd = rustc(rust, game, out, "2024");
        cmd.args(["--crate-type", "dylib", "--crate-name", "sunrise_patch"]);
        for (name, path) in wrapper_externs(&deps)? {
            cmd.arg("--extern")
                .arg(format!("{name}={}", path.display()));
        }
        for (name, path) in &patch.rlibs {
            cmd.arg("--extern")
                .arg(format!("{name}={}", path.display()));
        }
        cmd.arg("-o").arg(&library).arg(source);
        if !unchanged {
            super::run(cmd, tr("Compiling the patch library"))?;
            fs::write(saved, fingerprint).map_err(|e| e.to_string())?;
        }
        super::library_record::validate_exports(&library, "sunrise_patch")?;
        super::library_record::stamp(
            &library,
            "sunrise_patch",
            identities.recorded("sunrise_patch"),
            true,
        )?;
        installed.push(library);
    }
    Ok(Some(Built {
        binary: package.runtime.join("bin").join(crate::CLIENT),
        libraries: installed,
        catalog,
    }))
}

pub fn refuse_closure_rlib(id: &str, name: &str, path: &Path) -> Result<(), String> {
    if matches!(
        name,
        "mod_api" | "mod_vocab" | "sunrise_mod_runtime" | "sunrise_core"
    ) && path.extension().is_some_and(|ext| ext == "rlib")
    {
        return Err(trf(
            "Native mod {mod} links API closure crate {crate} as an rlib. Rebuild the SDK with a shared API closure.",
            &[("mod", id), ("crate", name)],
        ));
    }
    Ok(())
}

pub fn dylib_name(krate: &str) -> String {
    format!(
        "{}{krate}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    )
}

/// Where a library's entry template takes the library's identity.
pub(super) const IDENTITY_MARKER: &str = "/* NATIVE_BUILD */";

/// The library's entry module, exporting `identity` (as [`super::library_record::Identities::next`]
/// makes it) followed by the panic strategy the library is compiled with.
pub fn library_entry(
    template: &str,
    mods: &[&sunrise_mod_package::Manifest],
    identity: &str,
    installs: &[String],
) -> String {
    api::main_rs(template, mods)
        .replace("mod_0::", "crate::")
        .replace(IDENTITY_MARKER, identity)
        .replace(
            "/* MOD_REDIRECTS */",
            &installs
                .iter()
                .map(|i| format!("{i},\n"))
                .collect::<String>(),
        )
}

/// Add the SDK entry to the staged crate. The configured root and its generated sibling must
/// stay in the mod; neither is ever written through a link.
pub(super) fn stage_entry(
    staged: &Path,
    lib_path: Option<&toml::Value>,
    id: &str,
    entry: &str,
) -> Result<PathBuf, String> {
    let path = lib_path
        .map(|value| value.as_str().ok_or("[lib].path must be a string"))
        .transpose()?
        .unwrap_or("src/lib.rs");
    let relative = super::crates::local_path(staged, Path::new(""), path)
        .map_err(|error| format!("{id}: [lib].path {path:?}: {error}"))?;
    let source = staged.join(&relative);
    let mut root = fs::read_to_string(&source).map_err(|e| format!("{id}: {path}: {e}"))?;
    check_entry_root(&root, id)?;
    root.push_str(
        "\n#[allow(unsafe_code)]\n#[path = \"__sunrise_entry.rs\"] mod __sunrise_entry;\n",
    );
    if source
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("__sunrise_entry.rs"))
    {
        return Err(format!(
            "{id}: [lib].path names the reserved SDK entry file"
        ));
    }
    for (destination, text) in [
        (source.with_file_name("__sunrise_entry.rs"), entry),
        (source, root.as_str()),
    ] {
        // Replace the directory entry rather than opening an existing file for writing.
        let parent = destination.parent().expect("a library file below the mod");
        let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|e| e.to_string())?;
        file.persist(destination).map_err(|e| e.to_string())?;
    }
    Ok(relative)
}

/// Builds `project` against the route's SDK libraries and the compiled mod dependencies, from the
/// files `files` digests, stamping the identity `identities` recorded for it. A check emits only
/// Rust metadata; a build emits a dylib. Returns the artifact and crate name.
#[allow(clippy::too_many_arguments)]
pub fn compile_mod<'a>(
    project: &api::Project<'_>,
    declared: &[dependencies::Dependency],
    out: &Path,
    rustc: impl Fn(&str) -> Command,
    library: impl Fn(&str) -> &'a PathBuf,
    rlibs: &std::collections::HashMap<&str, PathBuf>,
    entry: &str,
    identities: &super::library_record::Identities,
    files: &str,
    check_only: bool,
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
    match fs::remove_dir_all(&staged) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
        _ => (),
    }
    super::copy_bundle(project.bundle, &staged)?;
    let lib_path = project.cargo.get("lib").and_then(|lib| lib.get("path"));
    let source = staged.join(stage_entry(&staged, lib_path, id, entry)?);
    let mut cmd = rustc(edition);
    // Diagnostics as JSON, which `run_diagnosed` shows at the author's files: the compiler's
    // inputs and outputs stay those of the staged copy.
    cmd.args([
        "--error-format=json",
        "--crate-type",
        "dylib",
        "--crate-name",
        &krate,
    ])
    // Two builds of one mod from different files never share mangled symbols or a `TypeId`.
    .arg("-C")
    .arg(format!("metadata={id}-{}", &files[..16]));
    for name in ["mod_api", "mod_vocab"] {
        refuse_closure_rlib(id, name, library(name))?;
        cmd.arg("--extern")
            .arg(format!("{name}={}", library(name).display()));
    }
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
    let library = if check_only {
        out.join(format!("lib{krate}.rmeta"))
    } else {
        out.join(dylib_name(&krate))
    };
    // The entry holds the identity: the build, the mod's files and its dependencies' identities.
    let mut hash = Sha256::new();
    hash.update(entry);
    for dependency in declared {
        if let Some(path) = rlibs.get(dependency.id.as_str()) {
            hash.update(fs::read(path).map_err(|e| e.to_string())?);
        }
    }
    let fingerprint = format!("{:x}", hash.finalize());
    let saved = library.with_extension("fingerprint");
    if !library.is_file() || fs::read_to_string(&saved).ok().as_deref() != Some(&fingerprint) {
        let sources = super::present::Sources::new(
            cmd.get_current_dir()
                .map(Path::to_path_buf)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_default(),
            vec![super::present::Staged {
                root: staged,
                author: project.bundle.dir.clone(),
            }],
        );
        if check_only {
            cmd.arg("--emit=metadata").arg("-o").arg(&library);
        } else {
            cmd.arg("--out-dir").arg(out);
        }
        cmd.arg(&source);
        super::run_diagnosed(cmd, &compiling(id), &sources)?;
    }
    if !check_only {
        super::library_record::validate_exports(&library, id)?;
        super::library_record::stamp(&library, id, identities.recorded(id), false)?;
    }
    fs::write(saved, fingerprint).map_err(|e| e.to_string())?;
    Ok((library, krate))
}

/// Refuses mod `id`'s library file `name` when another library already takes it. Names compare
/// without case, as Windows' and macOS's default file systems do.
/// The library files the build makes beside the mods': the patch library, and the one the registry
/// route links the mods' registry crates from when it writes one. Lowercase, as compared.
fn reserved_library_names() -> std::collections::HashSet<String> {
    [
        dylib_name("sunrise_patch"),
        dylib_name(super::registry_shared::CRATE),
    ]
    .into_iter()
    .map(|name| name.to_ascii_lowercase())
    .collect()
}

fn check_library_name(
    names: &mut std::collections::HashSet<String>,
    name: &str,
    id: &str,
) -> Result<(), String> {
    if !names.insert(name.to_ascii_lowercase()) {
        return Err(trf(
            "Native mod {mod} uses reserved or duplicate library file {file}. Choose a unique Cargo library name.",
            &[("mod", id), ("file", name)],
        ));
    }
    Ok(())
}

/// Refuses mod `id` when its crate root (`source`) forbids unsafe code: the generated library
/// entry appended to it exports `#[unsafe(no_mangle)]` functions, and `forbid` can't be lifted for
/// one module. A root this parser can't read is left to rustc, which reports it in its own words.
pub fn check_entry_root(source: &str, id: &str) -> Result<(), String> {
    let Ok(file) = syn::parse_file(source) else {
        return Ok(());
    };
    if file.attrs.iter().any(|attr| forbids_unsafe(&attr.meta)) {
        return Err(trf(
            "{mod}: its crate root forbids unsafe code, which the launcher's generated library entry needs; use #![deny(unsafe_code)] instead",
            &[("mod", id)],
        ));
    }
    Ok(())
}

/// Whether the attribute `meta` forbids unsafe code: `forbid(unsafe_code)`, or a `cfg_attr` (nested
/// ones too) carrying it. The launcher doesn't evaluate cfg predicates, so any predicate counts.
fn forbids_unsafe(meta: &syn::Meta) -> bool {
    let syn::Meta::List(list) = meta else {
        return false;
    };
    let Ok(items) = list.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    ) else {
        return false;
    };
    if list.path.is_ident("forbid") {
        items.iter().any(|lint| lint.path().is_ident("unsafe_code"))
    } else if list.path.is_ident("cfg_attr") {
        // The first item is the predicate; the rest are the attributes it applies.
        items.iter().skip(1).any(forbids_unsafe)
    } else {
        false
    }
}

/// What the build log and a failure call compiling mod `id`: a failure starts
/// "Compiling <id> failed".
pub fn compiling(id: &str) -> String {
    trf("Compiling {mod}", &[("mod", id)])
}

/// The tool's settings for a kept registry build: bumped whenever the tool changes the cargo
/// arguments, the compiler plan or how it stages, so a build made the old way is not reused.
const KEPT_BUILD_FLAGS: u32 = 3;

/// The folder a registry build's cargo output is kept in, under `native/prebuilt-build`: one per
/// game build, Rust toolchain and target, tool commit (the caller passes
/// [`crate::contract::TOOL_COMMIT`]) and tool settings ([`KEPT_BUILD_FLAGS`]), so two different
/// tools never share one. Inside one, cargo's own fingerprints decide what to rebuild.
fn kept_build(
    target: &str,
    tool_commit: &str,
    package: &Package,
    revision: &str,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(KEPT_BUILD_FLAGS.to_le_bytes());
    hasher.update(crate::rust::VERSION.as_bytes());
    hasher.update(tool_commit.as_bytes());
    hasher.update(target.as_bytes());
    // The package's files reach rustc and the linker through the compiler plan, which cargo
    // doesn't fingerprint: a package any of whose files differ is another key. Each file is known
    // by its slice, path, length and modified time, read from the slices themselves (the whole
    // game they are linked into is laid out again every build): a downloaded package was checked
    // against its release's sums when it was unpacked, and a rebuilt local SDK writes its files
    // anew.
    let mut files = Vec::new();
    for (name, slice) in [
        ("runtime", &package.runtime),
        ("api", &package.api),
        ("engine", &package.engine),
    ] {
        sdk_files(slice, Path::new(name), &mut files).map_err(|e| e.to_string())?;
    }
    files.sort();
    for (path, length, modified) in files {
        hasher.update(path.as_bytes());
        hasher.update(length.to_le_bytes());
        hasher.update(modified.to_le_bytes());
    }
    Ok(format!(
        "{}-{}",
        revision.get(..12).unwrap_or(revision),
        short_hash(&format!("{:x}", hasher.finalize()))
    ))
}

/// Each file under `folder` (shown as `shown`): its path, length and modified time in nanoseconds.
fn sdk_files(
    folder: &Path,
    shown: &Path,
    out: &mut Vec<(String, u64, u128)>,
) -> std::io::Result<()> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let meta = entry.metadata()?;
        let path = shown.join(entry.file_name());
        if meta.is_dir() {
            sdk_files(&entry.path(), &path, out)?;
        } else {
            let modified = meta
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_nanos());
            out.push((path.to_string_lossy().into_owned(), meta.len(), modified));
        }
    }
    Ok(())
}

/// The key of the mods folder a build stages: one staging folder per instance (or per CLI or
/// export folder), so instances never restage each other's files.
fn mods_key(mods: &Path) -> Result<String, String> {
    let path = mods
        .canonicalize()
        .map_err(|e| format!("{}: {e}", mods.display()))?;
    Ok(format!(
        "{:x}",
        Sha256::digest(path.to_string_lossy().as_bytes())
    ))
}

fn short_hash(hex: &str) -> &str {
    hex.get(..16).unwrap_or(hex)
}

/// The runtime shipped beside the prebuilt executable, for every native build route: every shared
/// library the package's `runtime` slice records under `deps/` (taken from `game`, the package or
/// its runtime slice), the standard library and, on Windows, the MinGW runtime the GNU DLLs import.
/// The package's own record is the list, so a library the game gains ships without a launcher
/// change.
pub fn runtime_libraries(
    rust: &dyn Toolchain,
    sdk: &Identity,
    game: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut runtime: Vec<PathBuf> = sdk
        .slices()
        .runtime
        .iter()
        .filter(|name| {
            name.starts_with("deps/")
                && [".so", ".dll", ".dylib"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        })
        .map(|name| game.join(name))
        .collect();
    if runtime.is_empty() {
        return Err(tr("The prebuilt game records no runtime libraries.").to_string());
    }
    runtime.push(standard_library(rust)?);
    if rust.target() == crate::rust::WINDOWS_X64_TARGET {
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

#[cfg(test)]
mod tests {
    #[test]
    fn role_facts_keep_aliases_and_features_separate_from_legacy_identity() {
        let dir = tempfile::tempdir().unwrap();
        let format = sunrise_mod_package::sdk::PACKAGE_FORMAT;
        let common = format!(
            r#""package_format":{format},"commit":"fixture","side":"server","rustc":"fixture","target":"host","registry_lock":"version = 4","registry_patches":{{}},"native_links":[],"facade_crates":[],"slices":{{"api":[],"runtime":[]}}"#
        );
        let write = |fields: &str| {
            fs::write(
                dir.path().join(super::IDENTITY),
                format!("{{{common},{fields}}}"),
            )
            .unwrap();
        };
        write(r#""game_features":["legacy"]"#);
        let legacy = super::identity(dir.path()).unwrap();
        assert_eq!(legacy.legacy_game_features().unwrap(), &["legacy"]);
        assert!(legacy.runtime_role().is_none());

        write(
            r#""game_features":["shared-shell"],"source_layout":"split","runtime_role":{"side":"server","libraries":{
                "common@0.1.0:fixture_common":{"package":"common","library":"fixture_common","features":["shared"],"aliases":{},"artifacts":["libfixture_common-role.rmeta"]},
                "server@0.1.0:fixture_server":{"package":"server","library":"fixture_server","features":["headless"],"aliases":{"shared_loan":"common@0.1.0:fixture_common"}},
                "sunrise-gameplay@0.1.0:sunrise_gameplay":{"package":"sunrise-gameplay","library":"sunrise_gameplay","features":[],"aliases":{},"artifacts":["libsunrise_gameplay-role.rmeta"]}
            }}"#,
        );
        #[derive(serde::Deserialize)]
        struct LegacyFeatureFields {
            game_features: Vec<String>,
        }
        let written = fs::read(dir.path().join(super::IDENTITY)).unwrap();
        let legacy_reader: LegacyFeatureFields = serde_json::from_slice(&written).unwrap();
        let sdk = super::identity(dir.path()).unwrap();
        assert_eq!(legacy_reader.game_features, ["shared-shell"]);
        assert_eq!(sdk.game_features, legacy_reader.game_features);
        assert!(sdk.legacy_game_features().is_err());
        let role = sdk.runtime_role().unwrap();
        assert_eq!(
            role.libraries["server@0.1.0:fixture_server"].features,
            ["headless"]
        );
        assert_eq!(
            role.libraries["server@0.1.0:fixture_server"].aliases["shared_loan"],
            "common@0.1.0:fixture_common"
        );

        assert_eq!(sdk.patch_features("server").unwrap(), ["headless"]);
        assert_eq!(sdk.patch_features("common").unwrap(), ["shared"]);
        assert!(sdk.patch_features("client").is_err());
        let deps = dir.path().join("deps");
        fs::create_dir(&deps).unwrap();
        for name in [
            "libsunrise_client.rmeta",
            "libfixture_common-role.rmeta",
            "libfixture_common-stale.rmeta",
            "libsunrise_gameplay-role.rmeta",
            "libsunrise_gameplay-stale.rmeta",
        ] {
            fs::write(deps.join(name), []).unwrap();
        }
        let externs = crate::patch::sdk_externs(dir.path(), &sdk, "server").unwrap();
        assert_eq!(
            externs["shared_loan"],
            deps.join("libfixture_common-role.rmeta")
        );
        assert_eq!(
            externs["sunrise_gameplay"],
            deps.join("libsunrise_gameplay-role.rmeta")
        );

        write(r#""source_layout":"split","game_features":["legacy"]"#);
        assert!(
            super::identity(dir.path())
                .err()
                .unwrap()
                .contains("runtime facts disagree")
        );

        write(
            r#""source_layout":"split","game_features":[],"runtime_role":{"side":"client","libraries":{}}"#,
        );
        assert!(
            super::identity(dir.path())
                .err()
                .unwrap()
                .contains("invalid client SDK")
        );
    }

    #[test]
    fn an_outside_library_root_is_refused_before_writing_generated_source() {
        let work = tempfile::tempdir().unwrap();
        let stage = work.path().join("mod");
        fs::create_dir(&stage).unwrap();
        let outside = work.path().join("outside.rs");
        let entry = work.path().join("__sunrise_entry.rs");
        fs::write(&outside, "fn untouched() {}\n").unwrap();
        for (path, reason) in [
            ("../outside.rs", "outside the mod"),
            ("src/C:/x.rs", "outside the mod"),
            (r"src\..\..", "portable separators"),
        ] {
            let path = toml::Value::String(path.into());
            let error =
                super::stage_entry(&stage, Some(&path), "example", "// generated").unwrap_err();
            assert!(
                error.contains("[lib].path") && error.contains(reason),
                "{error}"
            );
            assert_eq!(fs::read_to_string(&outside).unwrap(), "fn untouched() {}\n");
            assert!(!entry.exists());
        }
    }

    /// A linked build directory is refused before the files in it are cleared.
    #[test]
    fn a_build_folder_linked_elsewhere_is_refused_and_the_other_folder_kept() {
        #[cfg(unix)]
        fn link(to: &Path, at: &Path) -> std::io::Result<()> {
            std::os::unix::fs::symlink(to, at)
        }
        #[cfg(windows)]
        fn link(to: &Path, at: &Path) -> std::io::Result<()> {
            std::os::windows::fs::symlink_dir(to, at)
        }
        let work = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let builds = work.path().join("prebuilt-build");
        let out = builds.join("rustc-key");
        super::own_folder(&builds, &out).unwrap();
        super::own_folder(&builds, &out).unwrap();
        for stale in [
            outside.path().join("patch-applied"),
            outside.path().join("rustc-key/patch-applied"),
        ] {
            fs::create_dir_all(&stale).unwrap();
            fs::write(stale.join("keep"), "kept").unwrap();
        }
        // Windows grants symlinks only to developer mode or an administrator: a link it refuses
        // for that is skipped, and any other failure fails the test.
        let made = |link: std::io::Result<()>| match link {
            Ok(()) => true,
            Err(e) if cfg!(windows) && e.raw_os_error() == Some(1314) => false,
            Err(e) => panic!("{e}"),
        };
        fs::remove_dir(&out).unwrap();
        if made(link(outside.path(), &out)) {
            let refused = super::own_folder(&builds, &out).unwrap_err();
            assert!(refused.contains("not a folder of its own"), "{refused}");
            fs::remove_file(&out)
                .or_else(|_| fs::remove_dir(&out))
                .unwrap();
        }
        fs::remove_dir(&builds).unwrap();
        if made(link(outside.path(), &builds)) {
            let refused = super::own_folder(&builds, &out).unwrap_err();
            assert!(
                refused.contains("prebuilt-build: not a folder of its own"),
                "{refused}"
            );
        }
        for kept in ["patch-applied/keep", "rustc-key/patch-applied/keep"] {
            assert_eq!(
                fs::read_to_string(outside.path().join(kept)).unwrap(),
                "kept"
            );
        }
    }

    #[test]
    fn library_names_are_case_insensitive_and_entry_lints_are_checked() {
        let mut names = std::collections::HashSet::new();
        super::check_library_name(&mut names, "Example.dll", "one").unwrap();
        // A mod library taking the shared registry library's file is refused, naming both.
        let mut reserved = super::reserved_library_names();
        let file = super::dylib_name("Sunrise_Registry_Shared");
        let error = super::check_library_name(&mut reserved, &file, "taker").unwrap_err();
        assert!(error.contains("taker") && error.contains(&file), "{error}");
        assert!(
            super::check_library_name(&mut names, "example.dll", "two")
                .unwrap_err()
                .contains("two")
        );
        assert!(super::check_entry_root("#![deny(unsafe_code)]", "safe").is_ok());
        assert!(
            super::check_entry_root("#![forbid(unsafe_code, unused)]", "safe")
                .unwrap_err()
                .contains("safe")
        );
        assert!(super::check_entry_root("// #![forbid(unsafe_code)]", "safe").is_ok());
        let plain = super::check_entry_root("#![forbid(unsafe_code)]", "safe").unwrap_err();
        for root in [
            "#![cfg_attr(feature = \"x\", forbid(unsafe_code))]",
            "#![cfg_attr(all(unix, not(test)), allow(unused), forbid(unsafe_code))]",
            "#![cfg_attr(unix, cfg_attr(test, forbid(unsafe_code)))]",
        ] {
            assert_eq!(
                super::check_entry_root(root, "safe").unwrap_err(),
                plain,
                "{root}"
            );
        }
        assert!(super::check_entry_root("#![cfg_attr(unix, deny(unsafe_code))]", "safe").is_ok());
        assert!(super::check_entry_root("#![cfg_attr(unix, forbid(", "safe").is_ok());
    }

    #[test]
    fn unhashed_metadata_is_found_and_dylib_is_preferred_for_linking() {
        let dir = tempfile::tempdir().unwrap();
        let metadata = dir.path().join("libmod_api.rmeta");
        std::fs::write(&metadata, b"metadata").unwrap();
        assert_eq!(super::library(dir.path(), "mod_api").unwrap(), metadata);
        let dylib = dir.path().join(super::dylib_name("mod_api"));
        std::fs::write(&dylib, b"dylib").unwrap();
        assert_eq!(super::library(dir.path(), "mod_api").unwrap(), dylib);
    }

    #[test]
    fn closure_rlib_refusal_names_the_mod_and_crate() {
        let message = super::refuse_closure_rlib(
            "example",
            "mod_api",
            std::path::Path::new("libmod_api.rlib"),
        )
        .unwrap_err();
        assert!(message.contains("example"), "{message}");
        assert!(message.contains("mod_api"), "{message}");
        assert!(message.contains("rlib"), "{message}");
        assert!(
            super::refuse_closure_rlib(
                "example",
                "mod_api",
                std::path::Path::new("libmod_api.rmeta")
            )
            .is_ok()
        );
    }

    use super::*;

    #[test]
    fn the_route_is_chosen_once_from_the_whole_set() {
        use sunrise_mod_package::manifest::{Internal, Module, loops, mixin};
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
            "sunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}",
        );
        let facade = write(
            "facade",
            "sunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\nsunrise-internals={path='.sunrise/sdk/api/crates/sunrise-internals'}",
        );
        let registry = write(
            "registry",
            "sunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\nserde='1'",
        );
        let scripted = write(
            "scripted",
            "sunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}",
        );
        fs::write(folder.join("scripted/build.rs"), "fn main() {}").unwrap();
        let with = |change: &dyn Fn(&mut sunrise_mod_package::manifest::Manifest)| {
            let mut bundle = api_only.clone();
            change(&mut bundle.manifest);
            bundle
        };
        let native = |change: &dyn Fn(&mut sunrise_mod_package::manifest::Native)| {
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
            native(&|n| {
                n.r#loop.push(
                    toml::from_str::<loops::Loop>("id='l'\nloop='x.tick'\nhelper='l.rs'\nwith='L'")
                        .unwrap(),
                )
            }),
            with(&|m| {
                m.internals.push(Internal {
                    path: "sunrise_client::Game".into(),
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
        // A build script runs only under Cargo: the mod's own `build.rs` sends the set there,
        // and `build = false` does not.
        assert_eq!(route(&[&api_only, &scripted]), Route::Registry);
        let unscripted = write(
            "unscripted",
            "sunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}",
        );
        fs::write(folder.join("unscripted/build.rs"), "fn main() {}").unwrap();
        fs::write(
            folder.join("unscripted/Cargo.toml"),
            "[package]\nname='unscripted'\nversion='0.1.0'\nbuild=false\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n",
        )
        .unwrap();
        assert_eq!(route(&[&api_only, &unscripted]), Route::Api);
        // What ties a mod to its exact build is named by its table.
        let tie = |bundle: &super::super::Bundle| {
            let project = api::Project::read(bundle).unwrap();
            native_route::changes_game(&bundle.manifest, &project.cargo)
        };
        let ties: Vec<_> = engine.iter().map(tie).collect();
        assert_eq!(
            ties,
            [
                Some("[[native.mixin]]"),
                Some("[[native.module]]"),
                Some("[[native.bridge]]"),
                Some("[[native.loop]]"),
                Some("internals"),
                Some("internals"),
            ]
        );
        assert_eq!(tie(&api_only), None);
        assert_eq!(tie(&registry), None);
        assert_eq!(Route::Api.inputs(), "api");
        assert_eq!(Route::Engine.inputs(), "engine");
        assert_eq!(Route::Registry.inputs(), "engine");
        assert_eq!(Route::Api.name(), "prebuilt-api");
        assert_eq!(Route::Engine.name(), "prebuilt");
    }

    // The original format-6 publisher already wrote these fields, before native log forwarding.
    fn published_identity() -> serde_json::Value {
        serde_json::json!({
            "package_format": 6,
            "commit": "abc123", "rustc": "test", "target": "test", "side": "client",
            "registry_lock": "version = 4\n", "registry_patches": {}, "native_links": [],
            "facade_crates": ["sunrise_core"], "game_features": [],
            "slices": {"runtime": [], "api": []},
        })
    }

    #[test]
    fn facades_refuse_build_only_crates_and_ambiguous_runtime_metadata() {
        let deps = super::super::tests::dir("prebuilt-facade-eligibility");
        for name in [
            "libfixture_meta-host.rmeta",
            "libfixture_meta-target.rmeta",
            "libbuild_tool-one.rmeta",
            "libsunrise_core-one.rmeta",
        ] {
            fs::write(deps.join(name), b"").unwrap();
        }
        let mut sdk: Identity = serde_json::from_value(published_identity()).unwrap();
        assert_eq!(
            facade_library(&deps, "sunrise_core", &sdk).unwrap(),
            deps.join("libsunrise_core-one.rmeta")
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
    fn a_mod_library_compile_uses_dynamic_linking_and_platform_rpath() {
        let toolchain = crate::rust::Unpacked {
            dir: "toolchain".into(),
            target: "test".into(),
            cargo_home: "cargo-home".into(),
            cargo_config: Vec::new(),
            c_bin: None,
        };
        let cmd = rustc(&toolchain, Path::new("game"), Path::new("out"), "2024");
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|arg| arg == "prefer-dynamic"), "{args:?}");
        let link_args: Vec<_> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("link-arg="))
            .collect();
        for (os, expected) in [
            (
                "linux",
                &["-Wl,--disable-new-dtags", "-Wl,-rpath,$ORIGIN"][..],
            ),
            ("macos", &["-Wl,-rpath,@loader_path"][..]),
            ("windows", &[][..]),
        ] {
            assert!(dylib_rpath_args(os).eq(expected.iter().copied()), "{os}");
            if os == std::env::consts::OS {
                assert_eq!(link_args.as_slice(), expected, "{args:?}");
            }
        }
    }

    #[test]
    fn the_runtime_is_every_shared_library_the_package_records_plus_the_toolchains() {
        let scratch = tempfile::tempdir().unwrap();
        let game = scratch.path();
        // As a package records its runtime slice: shared libraries under deps/ (the engine slots
        // library among them, which no fixed list carried), the identity, the executable and
        // windows-gnu import libraries, which never ship.
        let mut document = published_identity();
        document["slices"]["runtime"] = serde_json::json!([
            "bin/sunrise-client.exe",
            "deps/sunrise_mod_host.dll",
            "deps/libsunrise_mod_host.dll.a",
            "deps/libmod_api.so",
            "sunrise-prebuilt.json",
        ]);
        let sdk: Identity = serde_json::from_value(document).unwrap();
        let recorded =
            ["sunrise_mod_host.dll", "libmod_api.so"].map(|name| game.join("deps").join(name));
        for target in crate::rust::tests::HOSTS {
            let rust = crate::rust::tests::unpacked(scratch.path().join(target), target);
            let std = rust.target_libdir().unwrap().join("std-test.dll");
            fs::create_dir_all(std.parent().unwrap()).unwrap();
            fs::write(&std, b"").unwrap();
            let mut expected = recorded.to_vec();
            expected.push(std);
            if target == crate::rust::WINDOWS_X64_TARGET {
                let bin = rust.mingw_bin();
                fs::create_dir_all(&bin).unwrap();
                for name in ["libgcc_s_seh-1.dll", "libwinpthread-1.dll"] {
                    assert!(
                        runtime_libraries(&rust, &sdk, game)
                            .unwrap_err()
                            .contains(name)
                    );
                    let library = bin.join(name);
                    fs::write(&library, b"").unwrap();
                    expected.push(library);
                }
            }
            assert_eq!(runtime_libraries(&rust, &sdk, game).unwrap(), expected);
        }
        let no_runtime: Identity = serde_json::from_value(published_identity()).unwrap();
        let rust = crate::rust::tests::unpacked(scratch.path(), crate::rust::tests::HOSTS[0]);
        assert!(runtime_libraries(&rust, &no_runtime, game).is_err());
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
            let mut identity = published_identity();
            identity["commit"] = commit.into();
            identity["rustc"] = rustc.into();
            identity["target"] = target.into();
            identity["side"] = side.into();
            fs::write(dir.join(IDENTITY), identity.to_string()).unwrap();
            check_identity(&dir, "abc123", "x86_64-pc-windows-gnu", Target::Client)
        };
        let rustc = format!("rustc x\nrelease: {}\nhost: y", crate::rust::VERSION);
        let host = "x86_64-pc-windows-gnu";
        assert!(check("abc123", &rustc, host, "client").is_ok());
        assert!(check("abc124", &rustc, host, "client").is_err());
        assert!(check("abc123", &rustc, "x86_64-unknown-linux-gnu", "client").is_err());
        assert!(check("abc123", &rustc, host, "server").is_err());
        let older = check("abc123", "rustc x\nrelease: 0.0.1", host, "client")
            .err()
            .unwrap();
        assert!(older.contains("the SDK is older"), "{older}");
        let newer = check("abc123", "rustc x\nrelease: 999.0.0", host, "client")
            .err()
            .unwrap();
        assert!(newer.contains("this tool is older"), "{newer}");
        let longer = format!("release: {}.1", crate::rust::VERSION);
        assert!(check("abc123", &longer, host, "client").is_err());
        fs::remove_file(dir.join(IDENTITY)).unwrap();
        assert!(check_identity(&dir, "abc123", host, Target::Client).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn kept_builds_of_two_tool_commits_never_share_a_folder() {
        let scratch = tempfile::tempdir().unwrap();
        let package = Package::at(scratch.path());
        let key = |commit| kept_build("x86_64-unknown-linux-gnu", commit, &package, "abc123");
        assert_eq!(key("one").unwrap(), key("one").unwrap());
        assert_ne!(key("one").unwrap(), key("two").unwrap());
    }

    #[test]
    fn native_build_refuses_an_internals_catalog_from_another_build() {
        let scratch = tempfile::tempdir().unwrap();
        let package = Package::at(scratch.path());
        let (commit, foreign) = (crate::tests::A, crate::tests::B);
        fs::create_dir_all(package.runtime.join("deps")).unwrap();
        fs::create_dir_all(&package.api).unwrap();
        let mut identity = published_identity();
        identity["commit"] = commit.into();
        fs::write(package.runtime.join(IDENTITY), identity.to_string()).unwrap();
        fs::write(
            package.api.join(internals::INTERNALS),
            serde_json::json!({"schema": 1, "build": {"commit": foreign}, "internals": []})
                .to_string(),
        )
        .unwrap();
        let rust = crate::rust::tests::unpacked(scratch.path().join("toolchain"), "test");
        for route in [Route::Api, Route::Engine, Route::Registry] {
            let result = compile(
                &rust,
                &package,
                &scratch.path().join("prebuilt-build"),
                scratch.path(),
                route,
                &[],
                &[],
                &dependencies::Resolution::default(),
                None,
                &|_, _, _| {},
                false,
            );
            let Err(error) = result else {
                panic!("build accepted another build's internals catalog");
            };
            assert!(error.contains("internals.json"), "{error}");
            assert!(
                error.contains(&format!("it describes game build {foreign}, not {commit}")),
                "{error}"
            );
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    #[ignore = "daily"]
    fn registry_refuses_linked_kept_and_target_folders_before_restaging() {
        let scratch = tempfile::tempdir().unwrap();
        let work = scratch.path();
        let package = Package::at(&work.join("sdk"));
        let commit = crate::tests::A;
        let mut identity = published_identity();
        identity["commit"] = commit.into();
        fs::create_dir_all(package.runtime.join("deps")).unwrap();
        fs::create_dir_all(&package.api).unwrap();
        fs::create_dir_all(&package.engine).unwrap();
        fs::write(package.runtime.join(IDENTITY), identity.to_string()).unwrap();
        fs::write(package.runtime.join("deps/fixture.rmeta"), "").unwrap();
        fs::write(
            package.api.join(internals::INTERNALS),
            serde_json::json!({"schema": 1, "build": {"commit": commit}, "internals": []})
                .to_string(),
        )
        .unwrap();
        fs::write(
            package.api.join(crate::api_route::TEMPLATE),
            "/* MOD_ENTRIES */\n/* BUILT_MODS */\n",
        )
        .unwrap();
        let mods = work.join("mods");
        let project = mods.join("example");
        crate::tests::write_bundle(&project, "example", "");
        fs::write(project.join("build.rs"), "fn main() {}").unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname='example'\nversion='0.1.0'\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n",
        )
        .unwrap();
        let bundle = crate::bundle_in(&project).unwrap().unwrap();
        let projects = [api::Project::read(&bundle).unwrap()];
        let declared = [Vec::new()];
        let resolution = dependencies::resolve(&[dependencies::Mod {
            id: &bundle.manifest.id,
            version: &bundle.manifest.version,
            dependencies: &declared[0],
            code: true,
        }])
        .unwrap();
        let route = Route::of(&projects);
        assert_eq!(route, Route::Registry);
        let rust = crate::rust::tests::unpacked(work.join("toolchain"), "test");
        let key = kept_build(
            rust.target(),
            crate::contract::TOOL_COMMIT,
            &package,
            commit,
        )
        .unwrap();
        let build = |builds: &Path| {
            compile(
                &rust,
                &package,
                builds,
                &mods,
                route,
                &projects,
                &declared,
                &resolution,
                None,
                &|_, _, _| {},
                false,
            )
            .err()
            .expect("the fixture has no SDK mod_api library to compile against")
        };
        let stage_name = format!("stage-{}", short_hash(&mods_key(&mods).unwrap()));
        let check_links = |name: &str, link: &dyn Fn(&Path, &Path) -> std::io::Result<()>| {
            let builds = work.join(name);
            let kept = builds.join(&key);
            let outside = work.join(format!("{name}-outside"));
            fs::create_dir(&builds).unwrap();
            fs::create_dir_all(outside.join(&stage_name)).unwrap();
            fs::write(outside.join(&stage_name).join("keep"), "outside").unwrap();
            // As in the direct route's fixture, only a Windows privilege refusal may skip a
            // symlink. The junction case below does not require that privilege.
            match link(&outside, &kept) {
                Ok(()) => (),
                Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => return,
                Err(error) => panic!("{error}"),
            }
            let error = build(&builds);
            assert!(
                error.contains(&format!("{}: not a folder of its own", kept.display())),
                "{error}"
            );
            assert_eq!(
                fs::read_to_string(outside.join(&stage_name).join("keep")).unwrap(),
                "outside"
            );
            #[cfg(unix)]
            fs::remove_file(&kept).unwrap();
            #[cfg(windows)]
            fs::remove_dir(&kept).unwrap();

            let stage = kept.join(&stage_name);
            fs::create_dir_all(&stage).unwrap();
            fs::write(stage.join("keep"), "stage").unwrap();
            let target = kept.join("target");
            link(&outside, &target).unwrap();
            let error = build(&builds);
            assert!(
                error.contains(&format!("{}: not a folder of its own", target.display())),
                "{error}"
            );
            assert_eq!(fs::read_to_string(stage.join("keep")).unwrap(), "stage");
        };
        #[cfg(unix)]
        check_links("symlink", &|to, at| std::os::unix::fs::symlink(to, at));
        #[cfg(windows)]
        {
            check_links("symlink", &|to, at| {
                std::os::windows::fs::symlink_dir(to, at)
            });
            check_links("junction", &|to, at| {
                let made = Command::new("cmd.exe")
                    .args(["/D", "/C", "mklink", "/J"])
                    .arg(at)
                    .arg(to)
                    .output()
                    .unwrap();
                assert!(made.status.success(), "mklink /J failed: {made:?}");
                Ok(())
            });
        }
        // Ordinary paths get past the guard and restage the mod before the intentionally
        // incomplete SDK is refused. No Cargo target is created on that early refusal.
        let builds = work.join("ordinary");
        let error = build(&builds);
        assert!(error.contains("no mod_api"), "{error}");
        let kept = builds.join(&key);
        assert!(
            kept.join(stage_name)
                .join("native-api/example/src/lib.rs")
                .is_file()
        );
        assert!(!kept.join("target").exists());
    }

    #[test]
    fn the_package_is_laid_out_as_its_slices_and_whole_again() {
        let dir = super::super::tests::dir("prebuilt-lay-out");
        let whole = dir.join("whole");
        let write = |slices: serde_json::Value| {
            let mut identity = published_identity();
            identity["slices"] = slices;
            fs::write(whole.join(IDENTITY), identity.to_string()).unwrap();
        };
        for (name, text) in [
            ("bin/sunrise-client", "exe"),
            ("deps/libmod_api-1.rmeta", "api"),
            ("deps/libsunrise_client-1.rlib", "engine"),
            ("internals.json", "{}"),
        ] {
            fs::create_dir_all(whole.join(name).parent().unwrap()).unwrap();
            fs::write(whole.join(name), text).unwrap();
        }
        let layout = |name: &str, link| lay_out(&whole, &dir.join(name), link, &identity(&whole)?);
        let runtime = serde_json::json!([IDENTITY, "bin/sunrise-client"]);
        write(
            serde_json::json!({"runtime": runtime, "api": ["deps/libmod_api-1.rmeta", "internals.json"]}),
        );
        let package = layout("linked", true).unwrap();
        assert_eq!(package, Package::at(&dir.join("linked")));
        let read = |path: PathBuf| fs::read_to_string(path).unwrap();
        assert_eq!(read(package.runtime.join("bin/sunrise-client")), "exe");
        assert_eq!(read(package.api.join("deps/libmod_api-1.rmeta")), "api");
        assert_eq!(
            read(package.engine.join("deps/libsunrise_client-1.rlib")),
            "engine"
        );
        assert!(!package.engine.join("internals.json").exists());
        // Linking leaves the developer's folder as it was.
        assert_eq!(read(whole.join("bin/sunrise-client")), "exe");
        let union = package.whole(&dir.join("union")).unwrap();
        for name in [
            "bin/sunrise-client",
            "deps/libmod_api-1.rmeta",
            "deps/libsunrise_client-1.rlib",
            IDENTITY,
        ] {
            assert!(union.join(name).is_file(), "{name}");
        }
        // A listed file that is absent, or a name out of the package, is refused.
        write(serde_json::json!({"runtime": runtime, "api": ["deps/gone.rmeta"]}));
        let error = layout("absent", true).unwrap_err();
        assert!(error.contains("deps/gone.rmeta"), "{error}");
        write(serde_json::json!({"runtime": runtime, "api": ["../internals.json"]}));
        // Even when that file exists, outside the package.
        fs::write(dir.join("internals.json"), "{}").unwrap();
        let error = layout("escape", true).unwrap_err();
        assert!(error.contains("not a plain path"), "{error}");
        write(serde_json::json!(null));
        assert!(layout("none", true).is_err());
        // Moving (an unpacked download) leaves nothing behind.
        write(
            serde_json::json!({"runtime": runtime, "api": ["deps/libmod_api-1.rmeta", "internals.json"]}),
        );
        layout("moved", false).unwrap();
        assert!(!whole.join("bin/sunrise-client").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mods_on_the_mod_api_alone_take_the_prebuilt_game() {
        assert_eq!(
            refusal_of(
                "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}"
            ),
            None
        );
        assert_eq!(
            refusal_of(
                "build=false\n[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}"
            ),
            None
        );
        assert_eq!(
            refusal_of(
                "[dependencies.api]\npackage='sunrise-mod-api'\npath='.sunrise/sdk/api/crates/sunrise-mod-api'"
            ),
            None
        );
    }

    #[test]
    fn registry_dependencies_do_not_rebuild_the_game() {
        for cargo in [
            "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\nserde='1'",
            "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n[target.'cfg(unix)'.dependencies]\nlibc='0.2'",
        ] {
            assert_eq!(refusal_of(cargo), None);
        }
    }

    #[test]
    fn anything_the_prebuilt_game_cant_build_names_why() {
        let id = || "example".to_owned();
        for (cargo, reason) in [
            (
                "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api',features=['testing']}",
                Refusal::Features { mod_id: id() },
            ),
            (
                "[dependencies]\nsunrise-mod-api={path='.sunrise/sdk/api/crates/sunrise-mod-api'}\n[features]\nextra=[]",
                Refusal::Features { mod_id: id() },
            ),
        ] {
            assert_eq!(refusal_of(cargo), Some(reason), "{cargo}");
        }
    }

    #[test]
    fn a_file_is_found_by_name_in_a_sums_listing() {
        let name = "horizon-mixins-x86_64-unknown-linux-gnu.tar.gz";
        let listing = format!("AB12  horizon-mixins-other.tar.gz\ncd34 *{name}\n");
        assert_eq!(expected_sum(&listing, name).as_deref(), Some("cd34"));
        assert_eq!(expected_sum(&listing, "horizon-mixins-x.tar.gz"), None);
    }

    #[test]
    fn an_unsupported_package_is_refused_before_its_body_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let refusal = |version| {
            fs::write(
                dir.path().join(IDENTITY),
                format!(r#"{{"package_format": {version}}}"#),
            )
            .unwrap();
            identity(dir.path()).err().unwrap()
        };
        let error = refusal(5);
        assert!(error.contains("SDK format 5 is too old"), "{error}");
        assert!(error.contains("the SDK is older"), "{error}");
        let error = refusal(7);
        assert!(error.contains("this tool is older"), "{error}");
        assert!(!error.contains("launcher"), "{error}");
    }

    #[test]
    fn format_six_requires_published_fields_but_accepts_pre_log_sdks() {
        let dir = tempfile::tempdir().unwrap();
        let read = |document: &serde_json::Value| {
            fs::write(dir.path().join(IDENTITY), document.to_string()).unwrap();
            identity(dir.path())
        };
        let mut current = published_identity();
        assert!(!read(&current).unwrap().native_log);
        current["native_log"] = true.into();
        assert!(read(&current).unwrap().native_log);
        for field in [
            "package_format",
            "registry_lock",
            "registry_patches",
            "native_links",
            "facade_crates",
            "game_features",
            "slices",
        ] {
            let mut incomplete = current.clone();
            incomplete.as_object_mut().unwrap().remove(field);
            // Even when the retired spelling advertises 6, the published name is required.
            incomplete["format_version"] = 6.into();
            let error = read(&incomplete).err().unwrap();
            assert!(error.contains("the SDK is older than this tool"), "{error}");
            assert!(
                error.contains(&format!("missing field `{field}`")),
                "{error}"
            );
        }
    }
}
