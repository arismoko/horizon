//! `horizon-mod.toml`, a mod's manifest, and the `.hmod` file around it.
//!
//! Raw injection schema adapted from horizon-mixins (Copyright (c) 2026 levzzz),
//! MIT licensed; see `HORIZON_MIXINS_LICENSE` and `THIRD_PARTY_NOTICES.md`.
pub mod inject;
mod jar;
mod wasm;
pub use jar::{
    JarConflict, JarInputsError, JarRef, JarRequirement, MissingJar, collect_jar_requirements,
    missing_jars,
};
pub use wasm::{MAX_WASM, Wasm, WasmError, export_prefix, sha512_of};

use serde::{Deserialize, Serialize};
use std::{
    fmt,
    io::Read,
    path::{Path, PathBuf},
};

/// The manifest's file name in a `.hmod` or a mod folder.
pub const MANIFEST: &str = "horizon-mod.toml";
/// Executable components are not content and are refused in a `.hmod`.
pub const WASM: &str = "mod.wasm";
/// The content manifest API this build loads.
pub const API: &str = "0.2";
/// Namespaces no mod may claim: vanilla's, the convention tags' (mods add
/// to those, docs/modding/src/how-to/content-mods.md) and the game's own.
const RESERVED_NAMESPACES: [&str; 3] = ["minecraft", "c", "horizon"];
/// A mod's saved settings, shared by native mods and modpack overrides.
pub const STORAGE_DIR: &str = ".horizon-mod-data";
/// The largest manifest this build reads.
const MAX_MANIFEST: u64 = 32 << 20;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// `[a-z0-9_]`, 1 to 64 characters: the mod's folder, log and settings key.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<String>,
    /// The content API line: `0.2`.
    pub api: String,
    #[serde(default)]
    pub side: Side,
    pub capabilities: Vec<Capability>,
    /// Namespaces besides `id` that the mod's `assets/` and `data/` use, for
    /// a content mod whose files sit under other names than its id (Towns
    /// and Towers ships `kaisyn` and `towns_and_towers`). No two content mods
    /// that are on may share one (`content::clash`).
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Other native mods whose hooks must be available before this one.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, Dependency>,
    /// Original Java jars, layered in list order below this mod's own files.
    #[serde(default)]
    pub jar: Vec<JarRef>,
    /// Exact build pins and optional raw injections for a code mod.
    pub native: Option<Native>,
    /// A wasm mod's `mod.wasm`, its toolchain and what it was linked against.
    pub wasm: Option<Wasm>,
    /// Unified listeners; routing and staging are separate from manifest parsing.
    #[serde(default)]
    pub inject: Vec<inject::Inject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Dependency {
    Version(semver::VersionReq),
    Detailed(DependencyDetails),
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DependencyDetails {
    pub version: semver::VersionReq,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Native {
    pub game: String,
    pub horizon: String,
    pub mixins: String,
    /// Game internals used by this mod, fingerprinted against its pinned build.
    #[serde(default)]
    pub internals: Vec<Internal>,
    #[serde(default)]
    pub injection: Vec<Injection>,
    #[serde(default)]
    pub module: Vec<Module>,
}

/// One declared game internal, as listed in the pinned game's `internals.json`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Internal {
    pub path: String,
    /// The catalog's member kind, required when several kinds share the path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub hash: String,
}

/// A support module merged into a game crate, forwarded as horizon-mixins'
/// `[[module]]`: self-contained Rust items, emitted into `file` as
/// `mod name`, for helpers that need more than one function.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Module {
    pub id: String,
    /// The game source file that receives the module, as the pin indexes it.
    pub file: String,
    /// The module's Rust name under the destination file's top level.
    pub name: String,
    /// The payload's file in this package, under `src/`.
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Injection {
    pub id: String,
    pub target: Option<String>,
    pub hook: Option<String>,
    pub hook_version: Option<u32>,
    pub kind: InjectionKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// A crate-root function of the target point's raw listener type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default)]
    pub cancellable: bool,
    pub argument: Option<usize>,
    #[serde(default)]
    pub bindings: Vec<Binding>,
    pub priority: Option<i32>,
    pub selector: Option<String>,
    pub ordinal: Option<usize>,
    pub require: Option<usize>,
    pub fingerprint: Option<String>,
    pub slice: Option<Slice>,
    pub variable: Option<Variable>,
    pub result: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InjectionKind {
    Head,
    BeforeCall,
    Overwrite,
    Return,
    ModifyArg,
    ModifyConstant,
    WrapOperation,
    /// A native raw site listener that skips a unit-returning call when it returns false.
    WrapWithCondition,
    ModifyArgs,
    ModifyVariable,
    ModifyExpressionValue,
    AfterCall,
    AfterAssign,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub name: String,
    pub ty: String,
    pub value: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Slice {
    pub from: Option<Anchor>,
    pub to: Option<Anchor>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub selector: String,
    pub ordinal: usize,
    pub require: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    pub ordinal: usize,
    pub at: Access,
    #[serde(default)]
    pub parameter: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Access {
    Store,
    Load,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// Local content, not compared with the server's mods when joining.
    #[default]
    Client,
    /// Both, and the server tells clients which version it expects.
    Both,
}

/// What a content mod adds; the player sees it before turning the mod on.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// New blocks, items and mob effects from the mod's `data/` (API 0.2).
    Registries,
}
impl Capability {
    /// What the player reads before turning the mod on: "May <this>".
    pub fn describe(self) -> &'static str {
        match self {
            Capability::Registries => {
                "add blocks, items, mob effects and entity types to your worlds"
            }
        }
    }
}
impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Registries => f.write_str("registries"),
        }
    }
}

/// Whether a source pin is an exact lowercase Git object id.
pub fn revision(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn valid_mod_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn rust_ident(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && !name.as_bytes()[0].is_ascii_digit()
}

/// One injection's own rules: a hook or a target, and a source helper or raw
/// function that listens at a point as only a point's listener can.
fn check_injection(injection: &Injection) -> Result<(), String> {
    // Modules share horizon-mixins' one id space with injections.
    if injection.hook.is_some() == injection.target.is_some() {
        return Err(format!(
            "{MANIFEST}: injection needs exactly one hook or target"
        ));
    }
    match (&injection.helper, &injection.raw) {
        (Some(helper), None) if plain_path(helper) && source_file(helper) => {}
        (None, Some(method)) if rust_ident(method) => {}
        _ => {
            return Err(format!(
                "{MANIFEST}: injection needs exactly one source helper or raw"
            ));
        }
    }
    let source_options = injection.cancellable
        || !injection.bindings.is_empty()
        || injection.argument.is_some()
        || injection.require.is_some()
        || injection.slice.is_some()
        || injection.variable.is_some()
        || injection.result.is_some();
    let site = injection.raw.is_some() && injection.selector.is_some();
    if site {
        if injection.target.is_none()
            || !matches!(
                injection.kind,
                InjectionKind::BeforeCall
                    | InjectionKind::AfterCall
                    | InjectionKind::ModifyArg
                    | InjectionKind::ModifyExpressionValue
                    | InjectionKind::WrapOperation
                    | InjectionKind::WrapWithCondition
            )
            || source_options
        {
            return Err(format!(
                "{MANIFEST}: injection {}: a raw call-site listener needs target, selector and a call-site kind; no non-local cancellation, bindings, argument, require, slice, variable or result",
                injection.id
            ));
        }
        return Ok(());
    }
    if matches!(injection.kind, InjectionKind::WrapWithCondition) {
        return Err(format!(
            "{MANIFEST}: WRAP_WITH_CONDITION needs a raw call-site listener"
        ));
    }
    // Point listeners run at the head or return with the point's own types, so nothing
    // else of a mixin applies.
    if injection.raw.is_some()
        && (injection.hook.is_some()
            || !matches!(injection.kind, InjectionKind::Head | InjectionKind::Return)
            || source_options
            || injection.selector.is_some()
            || injection.ordinal.is_some())
    {
        return Err(format!(
            "{MANIFEST}: injection {}: a raw listener listens at the HEAD or RETURN of the point its `target` names, and takes the point's record: no `hook` (a hook is its own Hooks method), bindings, cancellable, argument, selector, ordinal, require, slice, variable or result (cancel through the record's `cancel`); use a helper for the rest",
            injection.id
        ));
    }
    Ok(())
}

/// A wasm mod's declared internals and raw listeners; API listeners use `Manifest::inject`.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WasmNative {
    #[serde(default)]
    internals: Vec<Internal>,
}

impl Internal {
    fn check_all(internals: &[Self]) -> Result<(), String> {
        let mut paths = std::collections::HashSet::new();
        for internal in internals {
            if internal.path.is_empty()
                || internal.hash.is_empty()
                || internal.kind.as_ref().is_some_and(|kind| kind.is_empty())
                || !paths.insert((&internal.path, &internal.kind))
            {
                return Err(format!(
                    "{MANIFEST}: empty internal path, kind or hash, or duplicate internal path and kind"
                ));
            }
        }
        Ok(())
    }
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut table: toml::Table =
            toml::from_str(text).map_err(|e| format!("{MANIFEST}: {e}"))?;
        // Raw internals share native spelling, not native build pins or source modules.
        let declarations = if table.contains_key("wasm") {
            table
                .remove("native")
                .map(|native| native.try_into::<WasmNative>())
                .transpose()
                .map_err(|e| format!("{MANIFEST}: {e}"))?
                .unwrap_or_default()
        } else {
            WasmNative::default()
        };
        let mut m: Manifest = toml::Value::Table(table)
            .try_into()
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(wasm) = &mut m.wasm {
            Internal::check_all(&declarations.internals)?;
            if declarations.internals.is_empty()
                && m.inject
                    .iter()
                    .any(|i| i.access == inject::Access::Internals)
            {
                return Err(format!(
                    "{MANIFEST}: a wasm internals listener needs declared internals"
                ));
            }
            wasm.internals = declarations.internals;
        }
        if !valid_mod_id(&m.id) {
            return Err(format!(
                "{MANIFEST}: id {:?} is not [a-z0-9_]{{1,64}}",
                m.id
            ));
        }
        for id in m.dependencies.keys() {
            if !valid_mod_id(id) {
                return Err(format!("{MANIFEST}: invalid dependency id {id:?}"));
            }
        }
        if m.native.is_some() && m.wasm.is_some() {
            return Err(format!("{MANIFEST}: a mod is [native] or [wasm], not both"));
        }
        if m.wasm.is_some() && !wasm::exportable(&m.id) {
            return Err(format!(
                "{MANIFEST}: a wasm mod's id is words that each start with a letter (`{}`)",
                m.id
            ));
        }
        inject::Inject::check_all(&m.inject, &m.id, m.native.is_some() || m.wasm.is_some())
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if let Some(native) = &m.native {
            Internal::check_all(&native.internals)?;
            let mut ids: std::collections::HashSet<_> =
                m.inject.iter().map(|entry| &entry.id).collect();
            for injection in &native.injection {
                if injection.id.is_empty() || !ids.insert(&injection.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate injection id"));
                }
                check_injection(injection)?;
            }
            for module in &native.module {
                if module.id.is_empty() || !ids.insert(&module.id) {
                    return Err(format!("{MANIFEST}: empty or duplicate module id"));
                }
                if !plain_path(&module.file) {
                    return Err(format!("{MANIFEST}: module file must be a plain path"));
                }
                if !rust_ident(&module.name) {
                    return Err(format!("{MANIFEST}: module name must be an identifier"));
                }
                if !plain_path(&module.source) || !source_file(&module.source) {
                    return Err(format!(
                        "{MANIFEST}: module source must be a file under src/"
                    ));
                }
                if let Some(visibility) = &module.visibility
                    && !matches!(visibility.as_str(), "pub" | "pub(crate)")
                {
                    return Err(format!(
                        "{MANIFEST}: module visibility must be pub or pub(crate)"
                    ));
                }
            }
            if m.version.is_empty()
                || native.game.is_empty()
                || !revision(&native.horizon)
                || !revision(&native.mixins)
            {
                return Err(format!(
                    "{MANIFEST}: native requires game and exact revisions"
                ));
            }
        }
        if m.api != API {
            return Err(format!(
                "unsupported content API {}; this Horizon reads {API}",
                m.api
            ));
        }
        if m.capabilities != [Capability::Registries] {
            return Err(format!(
                "{MANIFEST}: a content mod has capabilities = [\"registries\"]"
            ));
        }
        for (i, ns) in m.namespaces.iter().enumerate() {
            // Vanilla's namespace characters, at most as long as an id.
            let chars_ok = (1..=64).contains(&ns.len())
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b));
            if !chars_ok {
                return Err(format!(
                    "{MANIFEST}: namespace {ns:?} is not [a-z0-9_.-]{{1,64}}"
                ));
            }
            if RESERVED_NAMESPACES.contains(&ns.as_str()) {
                return Err(format!(
                    "{MANIFEST}: namespace {ns} is not a mod's to claim (only to add to its tags)"
                ));
            }
            if *ns == m.id || m.namespaces[..i].contains(ns) {
                return Err(format!("{MANIFEST}: namespace {ns} is listed twice"));
            }
        }
        Ok(m)
    }
    /// The namespaces the mod's content owns: its id, then `namespaces`.
    pub fn owned_namespaces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.id.as_str()).chain(self.namespaces.iter().map(String::as_str))
    }
}

/// A source file that belongs to the flat Cargo project, not its resource packs.
pub fn source_file(name: &str) -> bool {
    if name
        .split('/')
        .any(|part| matches!(part, ".git" | "target"))
    {
        return false;
    }
    name == "Cargo.toml"
        || name == "build.rs"
        || (name.starts_with("src/") && (name.ends_with(".rs") || name.ends_with(".toml")))
        || (!name.contains('/')
            && ["README", "LICENSE", "LICENCE", "COPYING", "NOTICE"]
                .iter()
                .any(|prefix| name.starts_with(prefix)))
}

fn plain_path(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(['\\', ':'])
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
}

/// A mod as found on disk.
pub struct Package {
    pub manifest: Manifest,
    pub icon: Option<crate::icon::Icon>,
}
impl Package {
    /// The kind of this validated package, independent of its install folder.
    pub fn is_native(&self) -> bool {
        self.manifest.native.is_some()
    }

    /// Whether the package carries Rust source: a native mod, or a wasm mod built from it.
    fn has_source(&self) -> bool {
        self.manifest.native.is_some()
            || self
                .manifest
                .wasm
                .as_ref()
                .is_some_and(|w| w.built_with().is_none())
    }

    /// The package's `mod.wasm`, checked against its `[wasm]` section.
    pub fn read_wasm(path: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(path)?;
        let wasm = package
            .manifest
            .wasm
            .ok_or(WasmError::Undeclared.to_string())?;
        // Read at most one byte past the cap, so an oversized file is refused unread.
        let mut bytes = Vec::new();
        let read = if path.is_dir() {
            std::fs::File::open(path.join(WASM))
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
        } else {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            zip.by_name(WASM)
                .map_err(|_| WasmError::Missing.to_string())?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
        };
        read.map_err(|e| format!("{WASM}: {e}"))?;
        wasm.check(&bytes).map_err(|e| e.to_string())?;
        Ok(bytes)
    }

    /// Read a flat `.hmod` or project folder, without executing native code.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let has_wasm;
        let toml = if metadata.is_dir() {
            for old in [
                "horizon-native.json",
                "horizon-mixin.toml",
                "native",
                "rust",
            ] {
                if std::fs::symlink_metadata(path.join(old)).is_ok() {
                    return Err(format!("{old}: unsupported package path"));
                }
            }
            if std::fs::symlink_metadata(path.join("Cargo.toml")).is_ok() {
                check_folder_file(path, "Cargo.toml")?;
                has_cargo = true;
            }
            if let Ok(source) = std::fs::symlink_metadata(path.join("src")) {
                if !source.is_dir() {
                    return Err("src: unsupported package path".into());
                }
                has_source = true;
            }
            let icon_path = path.join("icon.png");
            match std::fs::symlink_metadata(&icon_path) {
                Ok(metadata) => {
                    check_folder_file(path, "icon.png")?;
                    if metadata.len() > crate::icon::MAX_ICON as u64 {
                        return Err("icon.png exceeds 256 KiB".into());
                    }
                    let mut bytes = Vec::new();
                    std::fs::File::open(&icon_path)
                        .map_err(|e| format!("icon.png: {e}"))?
                        .take(crate::icon::MAX_ICON as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| format!("icon.png: {e}"))?;
                    icon = Some(crate::icon::Icon::parse(bytes)?);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("icon.png: {e}")),
            }
            check_folder_file(path, MANIFEST)?;
            has_wasm = std::fs::symlink_metadata(path.join(WASM)).is_ok();
            for (name, _) in crate::content::folder_files(path)? {
                check_folder_file(path, &name)?;
            }
            let mut text = String::new();
            std::fs::File::open(path.join(MANIFEST))
                .map_err(|e| format!("{MANIFEST}: {e}"))?
                .take(MAX_MANIFEST + 1)
                .read_to_string(&mut text)
                .map_err(|e| format!("{MANIFEST}: {e}"))?;
            if text.len() as u64 > MAX_MANIFEST {
                return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
            }
            text
        } else {
            if !metadata.is_file() {
                return Err(format!(
                    "{}: unsupported file type in a content mod",
                    path.display()
                ));
            }
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
            return Self::read_zip(&mut zip);
        };
        let package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        if let Some(wasm) = &package.manifest.wasm {
            let bytes = std::fs::read(path.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        Ok(package)
    }

    /// Read and validate an archive, including all selected source and resource entries.
    pub fn read_zip<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Result<Self, String> {
        let mut icon = None;
        let mut has_cargo = false;
        let mut has_source = false;
        let mut has_wasm = false;
        let mut entries = Vec::new();
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().map_err(|e| e.to_string())?.into_owned();
            let mode = entry.unix_mode().unwrap_or_default();
            entries.push((name.clone(), entry.size(), entry.is_dir()));
            let first = name.split('/').next().unwrap_or("");
            if matches!(
                first,
                "horizon-native.json" | "horizon-mixin.toml" | "native" | "rust"
            ) {
                return Err(format!("{name}: unsupported package path"));
            }
            has_source |= first == "src";
            if entry.is_dir() && matches!(mode & 0o170000, 0 | 0o040000) {
                continue;
            }
            has_cargo |= name == "Cargo.toml";
            // The one executable a package may carry, checked against `[wasm]` below.
            if name == WASM && matches!(mode & 0o170000, 0 | 0o100000) {
                has_wasm = true;
                continue;
            }
            check_file(&name, mode, || Ok(&mut entry)).map_err(|e| e.to_string())?;
            if name == "icon.png" {
                if icon.is_some() {
                    return Err("duplicate icon.png".into());
                }
                if entry.size() > crate::icon::MAX_ICON as u64 {
                    return Err("icon.png exceeds 256 KiB".into());
                }
                let mut bytes = Vec::new();
                drop(entry);
                zip.by_index(i)
                    .map_err(|e| e.to_string())?
                    .take(crate::icon::MAX_ICON as u64 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| format!("icon.png: {e}"))?;
                icon = Some(crate::icon::Icon::parse(bytes)?);
            }
        }
        let mut toml = String::new();
        zip.by_name(MANIFEST)
            .map_err(|e| format!("{MANIFEST}: {e}"))?
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut toml)
            .map_err(|e| format!("{MANIFEST}: {e}"))?;
        if toml.len() as u64 > MAX_MANIFEST {
            return Err(format!("{MANIFEST}: over {} MiB", MAX_MANIFEST >> 20));
        }
        let package = Self::from_parts(&toml, icon, has_cargo, has_source, has_wasm)?;
        if let Some(wasm) = &package.manifest.wasm {
            let mut bytes = Vec::new();
            zip.by_name(WASM)
                .map_err(|e| format!("{WASM}: {e}"))?
                .take(MAX_WASM + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
        }
        if package.has_source() || package.manifest.wasm.is_some() {
            let mut names = std::collections::HashSet::new();
            let mut source_count = 0usize;
            let mut source_bytes = 0u64;
            let mut resource_count = 0usize;
            let mut resource_bytes = 0u64;
            for (name, size, directory) in entries {
                let path = if directory {
                    name.trim_end_matches('/')
                } else {
                    &name
                };
                if !plain_path(path) || !names.insert(name.clone()) {
                    return Err(format!("invalid or duplicate package path: {name}"));
                }
                if !directory {
                    if name != MANIFEST
                        && name != "icon.png"
                        && !(name == WASM && package.manifest.wasm.is_some())
                        && !name.starts_with("assets/")
                        && !name.starts_with("data/")
                        && !source_file(&name)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                    if source_file(&name) {
                        source_count += 1;
                        source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                    } else if name.starts_with("assets/") || name.starts_with("data/") {
                        crate::content::check_path(&name)?;
                        crate::content::check_namespace(&name, &package.manifest)?;
                        if size > crate::content::MAX_FILE {
                            return Err(format!("{name} exceeds the content file size limit"));
                        }
                        resource_count += 1;
                        resource_bytes = resource_bytes
                            .checked_add(size)
                            .ok_or("content too large")?;
                    }
                }
            }
            if source_count > 511 || source_bytes > 64 << 20 {
                return Err("native package exceeds its entry or size limit".into());
            }
            if resource_count > crate::content::MAX_FILES
                || resource_bytes > crate::content::MAX_TOTAL
            {
                return Err("content exceeds its entry or size limit".into());
            }
        }
        Ok(package)
    }

    fn from_parts(
        text: &str,
        icon: Option<crate::icon::Icon>,
        has_cargo: bool,
        has_source: bool,
        has_wasm: bool,
    ) -> Result<Self, String> {
        let manifest = Manifest::parse(text)?;
        let package = Package { manifest, icon };
        if has_cargo != package.has_source() || has_source != has_cargo {
            return Err("inconsistent code declaration ([native] or [wasm]) and source".into());
        }
        match (has_wasm, package.manifest.wasm.is_some()) {
            (true, false) => return Err(WasmError::Undeclared.to_string()),
            (false, true) => return Err(WasmError::Missing.to_string()),
            _ => {}
        }
        Ok(package)
    }
    /// Whether the mod at `path` (a `.hmod` or a folder) has a [`MANIFEST`].
    pub fn has_manifest(path: &Path) -> bool {
        if path.is_dir() {
            return path.join(MANIFEST).is_file();
        }
        std::fs::File::open(path)
            .ok()
            .and_then(|f| zip::ZipArchive::new(f).ok())
            .is_some_and(|mut zip| zip.by_name(MANIFEST).is_ok())
    }
    /// Select and validate a flat project's source and resource files deterministically.
    pub fn directory_files(
        &self,
        root: &Path,
    ) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
        let content = crate::Content::read(root, &self.manifest)?;
        let mut files = std::collections::BTreeMap::new();
        let mut reader = content.reader()?;
        for name in &content.files {
            files.insert(name.clone(), reader.read(name)?);
        }
        if let Some(icon) = &self.icon {
            files.insert("icon.png".to_string(), icon.bytes().to_vec());
        }
        if let Some(wasm) = &self.manifest.wasm {
            let bytes = std::fs::read(root.join(WASM)).map_err(|e| format!("{WASM}: {e}"))?;
            wasm.check(&bytes).map_err(|e| e.to_string())?;
            files.insert(WASM.to_owned(), bytes);
        }
        if self.has_source() {
            fn walk(
                root: &Path,
                dir: &Path,
                files: &mut std::collections::BTreeMap<String, Vec<u8>>,
                source_bytes: &mut u64,
                source_count: &mut usize,
            ) -> Result<(), String> {
                for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let path = entry.path();
                    let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
                    let name = relative
                        .to_str()
                        .ok_or("non-UTF-8 package path")?
                        .replace('\\', "/");
                    let kind = entry.file_type().map_err(|e| e.to_string())?;
                    if kind.is_dir() {
                        if matches!(entry.file_name().to_str(), Some(".git" | "target"))
                            || matches!(name.as_str(), "assets" | "data")
                        {
                            continue;
                        }
                        walk(root, &path, files, source_bytes, source_count)?;
                    } else if source_file(&name) {
                        check_folder_file(root, &name)?;
                        let size = entry.metadata().map_err(|e| e.to_string())?.len();
                        *source_bytes = source_bytes
                            .checked_add(size)
                            .ok_or("native package too large")?;
                        *source_count += 1;
                        if *source_bytes > 64 << 20 || *source_count > 511 {
                            return Err("native package exceeds its entry or size limit".into());
                        }
                        files.insert(name, std::fs::read(path).map_err(|e| e.to_string())?);
                    } else if name.ends_with(".rs") || (name.ends_with(".toml") && name != MANIFEST)
                    {
                        return Err(format!("unexpected package path: {name}"));
                    }
                }
                Ok(())
            }
            let (mut source_bytes, mut source_count) = (0, 0);
            walk(root, root, &mut files, &mut source_bytes, &mut source_count)?
        }
        files.insert(
            MANIFEST.to_owned(),
            std::fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?,
        );
        Ok(files)
    }

    /// Pack a flat project, selecting its source and resource files deterministically.
    pub fn pack_directory(root: &Path) -> Result<Vec<u8>, String> {
        let package = Self::read(root)?;
        let mut files = package.directory_files(root)?;
        let text = String::from_utf8(files.remove(MANIFEST).expect("selected manifest"))
            .map_err(|e| e.to_string())?;
        let files: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        Self::write_files(&text, &files)
    }

    /// The `.hmod` bytes for `manifest_text` and selected project files.
    pub fn write_files(manifest_text: &str, files: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
        use std::io::Write;
        let manifest = Manifest::parse(manifest_text)?;
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        let err = |e: zip::result::ZipError| e.to_string();
        let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
            zip.start_file(name, options).map_err(err)?;
            zip.write_all(bytes).map_err(|e| e.to_string())
        };
        put(MANIFEST, manifest_text.as_bytes())?;
        for (name, bytes) in files {
            let code = manifest.native.is_some() || manifest.wasm.is_some();
            let source = code && source_file(name) && plain_path(name);
            if *name == WASM && manifest.wasm.is_some() {
                put(name, bytes)?;
                continue;
            }
            if *name != "icon.png" && !source {
                crate::content::check_path(name)?;
            }
            check_file(name, 0, || Ok(*bytes)).map_err(|e| e.to_string())?;
            put(name, bytes)?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).map_err(err)?;
        Self::read_zip(&mut archive)?;
        Ok(bytes)
    }
}

/// A content entry that cannot be safely loaded as data.
#[derive(Debug)]
pub enum ContentFileError {
    UnsupportedType(String),
    Executable(String),
    Read {
        name: String,
        source: std::io::Error,
    },
}

impl std::fmt::Display for ContentFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedType(name) => {
                write!(f, "{name}: unsupported file type in a content mod")
            }
            Self::Executable(name) => {
                write!(f, "{name}: unsupported executable file in a content mod")
            }
            Self::Read { name, source } => write!(f, "{name}: {source}"),
        }
    }
}

impl std::error::Error for ContentFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The shared content-file boundary. Refuse type/mode before opening a file,
/// then inspect its header even when its name has no executable extension.
pub(crate) fn check_file<R: Read>(
    name: &str,
    mode: u32,
    open: impl FnOnce() -> std::io::Result<R>,
) -> Result<(), ContentFileError> {
    if !matches!(mode & 0o170000, 0 | 0o100000) {
        return Err(ContentFileError::UnsupportedType(name.into()));
    }
    let extension = name
        .rsplit(['/', '\\'])
        .next()
        .and_then(|n| n.rsplit_once('.'));
    let executable = extension.is_some_and(|(_, ext)| {
        [
            "wasm", "exe", "dll", "so", "dylib", "class", "jar", "com", "bat", "cmd", "ps1",
        ]
        .iter()
        .any(|kind| ext.eq_ignore_ascii_case(kind))
    });
    if executable || mode & 0o111 != 0 {
        return Err(ContentFileError::Executable(name.into()));
    }
    let error = |source| ContentFileError::Read {
        name: name.into(),
        source,
    };
    let mut file = open().map_err(error)?;
    let mut header = [0; 4];
    let mut read = 0;
    while read < header.len() {
        let size = file.read(&mut header[read..]).map_err(error)?;
        if size == 0 {
            break;
        }
        read += size;
    }
    let executable = matches!(
        &header,
        b"\0asm"
            | b"\x7fELF"
            | b"\xfe\xed\xfa\xce"
            | b"\xce\xfa\xed\xfe"
            | b"\xfe\xed\xfa\xcf"
            | b"\xcf\xfa\xed\xfe"
            | b"\xca\xfe\xba\xbe"
            | b"\xbe\xba\xfe\xca"
    ) || header.starts_with(b"MZ")
        || (header.starts_with(b"#!") && header[2] != b'[');
    if executable {
        return Err(ContentFileError::Executable(name.into()));
    }
    Ok(())
}

fn check_folder_file(root: &Path, name: &str) -> Result<(), String> {
    let path = root.join(name);
    let metadata = std::fs::symlink_metadata(&path).map_err(|e| format!("{name}: {e}"))?;
    let mode = if metadata.is_file() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.mode()
        }
        #[cfg(not(unix))]
        {
            0
        }
    } else {
        0o120000
    };
    check_file(name, mode, || std::fs::File::open(&path)).map_err(|e| e.to_string())
}

/// The `.hmod` files and mod folders in `dir`, by name.
pub fn find(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "hmod") || p.join(MANIFEST).is_file())
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
id = "ruby"
name = "Ruby"
version = "0.1.0"
api = "0.2"
capabilities = ["registries"]
"#;

    #[test]
    fn unified_and_native_raw_injections_share_the_id_namespace() {
        let pin = "a".repeat(40);
        let text = |raw_id| {
            format!(
                "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{pin}'\nmixins='{pin}'\n[[inject]]\nid='shared'\nat='game::tick'\nkind='head'\nwith='tick'\nhash='signature'\n[[native.injection]]\nid='{raw_id}'\ntarget='game::tick'\nkind='HEAD'\nraw='raw_tick'\n"
            )
        };
        assert!(Manifest::parse(&text("distinct")).is_ok());
        let error = Manifest::parse(&text("shared")).unwrap_err();
        assert!(error.contains("duplicate injection id"), "{error}");
    }

    #[test]
    fn refuses_bad_ids_and_retired_manifests() {
        let bad_id = EXAMPLE.replace("ruby", "Ruby!");
        assert!(Manifest::parse(&bad_id).unwrap_err().contains("is not"));
        for api in ["0.1", "0.3", "0.4"] {
            let text = EXAMPLE.replace("api = \"0.2\"", &format!("api = \"{api}\""));
            assert!(
                Manifest::parse(&text)
                    .unwrap_err()
                    .contains("unsupported content API")
            );
        }
        for caps in ["[]", "[\"hud\"]", "[\"registries\", \"registries\"]"] {
            let text = EXAMPLE.replace("[\"registries\"]", caps);
            assert!(Manifest::parse(&text).is_err());
        }
        for field in ["network = true", "experimental = []", "side = \"server\""] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{field}\n")).is_err());
        }
        assert_eq!(Manifest::parse(EXAMPLE).unwrap().side, Side::Client);
        assert_eq!(
            Manifest::parse(&format!("{EXAMPLE}side = \"both\"\n"))
                .unwrap()
                .side,
            Side::Both
        );
    }

    #[test]
    fn native_manifest_requires_exact_pins_and_known_fields() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        Manifest::parse(&format!("{EXAMPLE}{native}")).unwrap();
        assert!(
            Manifest::parse(&format!(
                "{EXAMPLE}{}",
                native.replace(&"a".repeat(40), "main")
            ))
            .is_err()
        );
        for unsupported in ["unexpected = true", "[[native.post_build]]\nbin = 'bake'"] {
            assert!(Manifest::parse(&format!("{EXAMPLE}{native}{unsupported}\n")).is_err());
        }
    }

    #[test]
    fn unknown_injection_fields_are_refused_by_the_schema() {
        for code in [
            format!(
                "[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
                "a".repeat(40),
                "b".repeat(40)
            ),
            format!("[wasm]\nsha512='{}'\ntoolchain='1.99.1'\n", "0".repeat(128)),
        ] {
            for kind in ["HEAD", "RETURN"] {
                let error = Manifest::parse(&format!("{EXAMPLE}{code}[[native.injection]]\nid='tick'\ntarget='game::tick'\nkind='{kind}'\ntrait_method='tick'\n")).unwrap_err();
                assert!(error.contains("unknown field"), "{error}");
            }
        }
    }

    #[test]
    fn internal_kinds_distinguish_members_and_round_trip() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let declaration = "internals=[{path='Round::listening',kind='field',hash='1234'}, {path='Round::listening',kind='method',hash='5678'}]\n";
        let manifest = Manifest::parse(&format!("{base}{declaration}")).unwrap();
        let native = manifest.native.unwrap();
        assert_eq!(native.internals[1].kind.as_deref(), Some("method"));
        let text = toml::to_string(&native).unwrap();
        assert_eq!(toml::from_str::<Native>(&text).unwrap(), native);
        assert!(
            Manifest::parse(&format!(
                "{base}{}",
                declaration.replace("kind='method'", "kind='field'")
            ))
            .is_err()
        );
        assert!(
            Manifest::parse(&format!(
                "{base}{}",
                declaration.replace("kind='method'", "kind=''")
            ))
            .is_err()
        );
        let untyped = Internal {
            path: "Game".into(),
            kind: None,
            hash: "1234".into(),
        };
        assert!(!toml::to_string(&untyped).unwrap().contains("kind"));
    }

    #[test]
    fn declared_internals_and_raw_injections_have_one_explicit_form() {
        let base = format!(
            "{EXAMPLE}[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let injection = "[[native.injection]]\nid='count'\ntarget=\"horizon_client::game::Game<'_>::tick\"\nkind='HEAD'\n";
        let text = format!(
            "{base}internals=[{{path='horizon_client::game::Game',hash='1234'}}]\n{injection}raw='count'\n"
        );
        let manifest = Manifest::parse(&text).unwrap();
        let native = manifest.native.unwrap();
        assert_eq!(native.internals[0].path, "horizon_client::game::Game");
        assert_eq!(native.injection[0].raw.as_deref(), Some("count"));
        for keys in [
            "",
            "raw='count'\nhelper='src/a.rs'",
            "raw=''",
            "raw='nested::count'",
            "raw='0count'",
        ] {
            let error = Manifest::parse(&format!("{base}{injection}{keys}\n")).unwrap_err();
            assert!(
                error.contains("exactly one source helper or raw"),
                "{error}"
            );
        }
        for bad in [
            text.replace("kind='HEAD'", "kind='BEFORE_CALL'"),
            text.replace("raw='count'", "raw='count'\ncancellable=true"),
            text.replace(
                "raw='count'",
                "raw='count'\nbindings=[{name='a',ty='u8',value='a'}]",
            ),
            text.replace("target=", "hook="),
            text.replace("hash='1234'", "hash=''"),
            text.replace("path='horizon_client::game::Game'", "path=''"),
            text.replace(
                "hash='1234'}",
                "hash='1234'},{path='horizon_client::game::Game',hash='5678'}",
            ),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "accepted {bad}");
        }
        Manifest::parse(&text.replace("kind='HEAD'", "kind='RETURN'")).unwrap();
    }

    #[test]
    fn native_modules_are_validated_like_injections() {
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let module = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        let manifest = Manifest::parse(&format!("{EXAMPLE}{native}{module}")).unwrap();
        let module = &manifest.native.as_ref().unwrap().module[0];
        assert_eq!(module.name, "runtime");
        assert_eq!(module.visibility, None);
        // Forwarded whole to horizon-mixins, like `injection`.
        let forwarded = toml::to_string(manifest.native.as_ref().unwrap()).unwrap();
        assert!(forwarded.contains("[[module]]"));
        assert!(forwarded.contains("id = \"runtime\""));
        let valid = "[[native.module]]\nid = 'runtime'\nfile = 'crates/horizon-client/src/game.rs'\nname = 'runtime'\nsource = 'src/runtime.rs'\n";
        for bad in [
            // escapes the package's src/
            valid.replace("source = 'src/runtime.rs'", "source = '../escape.rs'"),
            // unknown field
            format!("{valid}order = 1\n"),
            // duplicate id, also against an injection's
            format!("{valid}{valid}"),
            format!(
                "{valid}[[native.injection]]\nid = 'runtime'\ntarget = 'a'\nkind = 'HEAD'\nraw = 'b'\n"
            ),
            // not an identifier
            valid.replace("name = 'runtime'", "name = 'run time'"),
            // file must stay a plain path
            valid.replace(
                "file = 'crates/horizon-client/src/game.rs'",
                "file = '/abs.rs'",
            ),
            // horizon-mixins accepts exactly pub and pub(crate)
            valid.to_owned() + "visibility = 'public'\n",
        ] {
            assert!(
                Manifest::parse(&format!("{EXAMPLE}{native}{bad}")).is_err(),
                "accepted: {bad}"
            );
        }
        let public = valid.to_owned() + "visibility = 'pub(crate)'\n";
        assert!(Manifest::parse(&format!("{EXAMPLE}{native}{public}")).is_ok());
    }

    #[test]
    fn dependencies_are_typed_for_content_and_code_mods() {
        let deps =
            "[dependencies]\ncreate = '^6.0'\naddon = { version = '^1.2', optional = true }\n";
        let content = Manifest::parse(&format!("{EXAMPLE}{deps}")).unwrap();
        assert_eq!(content.dependencies.len(), 2);
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let text = format!("{EXAMPLE}{deps}{native}");
        let manifest = Manifest::parse(&text).unwrap();
        assert!(
            matches!(&manifest.dependencies["addon"], Dependency::Detailed(details) if details.optional)
        );
        assert!(matches!(
            &manifest.dependencies["create"],
            Dependency::Version(_)
        ));
        for bad in [
            text.replace("^6.0", "not semver"),
            text.replace("create =", "\"not-valid\" ="),
            text.replace("optional = true", "unexpected = true"),
        ] {
            assert!(Manifest::parse(&bad).is_err());
        }
    }

    #[test]
    fn flat_native_archive_round_trips_through_shared_writer() {
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        let files: &[(&str, &[u8])] = &[
            (
                "Cargo.toml",
                b"[package]\nname = 'other'\nversion = '2.0.0'\n",
            ),
            ("src/lib.rs", b"pub fn hooks() {}"),
        ];
        let bytes = Package::write_files(&manifest, files).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let package = Package::read_zip(&mut zip).unwrap();
        assert_eq!(package.manifest.id, "ruby");
        assert_eq!(package.manifest.version, "0.1.0");
        assert_eq!(package.manifest.native.unwrap().game, "26.3");
        assert!(
            Package::write_files(&manifest, &[("Cargo.toml", b""), ("src/../bad.rs", b"")])
                .is_err()
        );
        assert!(
            Package::write_files(
                &manifest,
                &[
                    ("Cargo.toml", b""),
                    ("src/lib.rs", b""),
                    ("src/lib.rs", b"")
                ]
            )
            .is_err()
        );
    }

    #[test]
    fn flat_project_packs_only_selected_safe_files() {
        let root = std::env::temp_dir().join(format!("horizon-flat-pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::create_dir_all(root.join("target/generated")).unwrap();
        let manifest = format!(
            "{EXAMPLE}[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), manifest).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'other'\nversion = '2.0.0'\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn hooks() {}\n").unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        std::fs::write(root.join("target/generated/bad.rs"), "#!/bin/sh\n").unwrap();
        let bytes = Package::pack_directory(&root).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(Package::read_zip(&mut zip).unwrap().manifest.id, "ruby");
        assert!(zip.by_name("Cargo.toml").is_ok());
        assert!(zip.by_name("src/lib.rs").is_ok());
        assert!(zip.by_name(".git/hooks/pre-commit").is_err());
        assert!(zip.by_name("target/generated/bad.rs").is_err());
        std::fs::write(root.join("src/lib.rs"), b"MZ executable").unwrap();
        assert!(
            Package::pack_directory(&root)
                .unwrap_err()
                .contains("unsupported executable")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_kind_requires_matching_source_and_native_declaration() {
        let root =
            std::env::temp_dir().join(format!("horizon-package-kind-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        assert!(Package::read(&root).is_ok());
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = 'unrelated'\nversion = '7.0.0'\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        assert!(Package::read(&root).is_err());
        let native = format!(
            "[native]\ngame = '26.3'\nhorizon = '{}'\nmixins = '{}'\n",
            "a".repeat(40),
            "b".repeat(40)
        );
        std::fs::write(root.join(MANIFEST), format!("{EXAMPLE}{native}")).unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::create_dir(root.join("native")).unwrap();
        assert!(Package::read(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checks_extra_namespaces() {
        let with = |list: &str| Manifest::parse(&format!("{EXAMPLE}namespaces = [{list}]\n"));
        let m = with("\"kaisyn\", \"towns_and_towers\"").unwrap();
        let owned: Vec<&str> = m.owned_namespaces().collect();
        assert_eq!(owned, ["ruby", "kaisyn", "towns_and_towers"]);
        for reserved in ["minecraft", "c", "horizon"] {
            let e = with(&format!("\"{reserved}\"")).unwrap_err();
            assert!(e.contains("not a mod's to claim"), "{e}");
        }
        assert!(with("\"ruby\"").unwrap_err().contains("twice"));
        assert!(with("\"a\", \"a\"").unwrap_err().contains("twice"));
        assert!(with("\"Kaisyn\"").unwrap_err().contains("is not"));
        assert!(with("\"\"").unwrap_err().contains("is not"));
    }

    #[test]
    fn a_wasm_mod_carries_exactly_its_declared_module() {
        let module = b"\0asm\x01\0\0\0".to_vec();
        let text = format!(
            "{}\n[wasm]\nsha512 = \"{}\"\ntoolchain = \"1.99.1\"\nbuilt_with = \"test\"\n",
            EXAMPLE.trim_end(),
            sha512_of(&module)
        );
        let bytes = Package::write_files(&text, &[(WASM, &module)]).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        assert!(Package::read_zip(&mut zip).unwrap().manifest.wasm.is_some());
        let error = Package::write_files(&text, &[(WASM, b"\0asm\x02\0\0\0")]).unwrap_err();
        assert_eq!(error, WasmError::Hash.to_string());
        assert_eq!(
            Package::write_files(&text, &[]).unwrap_err(),
            WasmError::Missing.to_string()
        );
        let both = format!(
            "{text}\n[native]\ngame = \"g\"\nhorizon = \"{0}\"\nmixins = \"{0}\"\n",
            "a".repeat(40)
        );
        assert!(
            Manifest::parse(&both)
                .unwrap_err()
                .contains("unknown field")
        );
    }

    #[test]
    fn wasm_non_raw_declarations_fail_schema_even_with_unified_injects() {
        use sha2::{Digest, Sha512};
        use std::io::Write;
        let module = b"\0asm\x01\0\0\0";
        let base = format!(
            "{EXAMPLE}[wasm]\nsha512='{:x}'\ntoolchain='1.99.1'\n",
            Sha512::digest(module)
        );
        let foreign = "[[native.injection]]\nid='non_raw'\ntarget='game::tick'\nkind='HEAD'\nhelper='src/count.rs'\n";
        let missing_raw = foreign.replace("helper='src/count.rs'\n", "");
        assert!(
            Manifest::parse(&format!("{base}{missing_raw}"))
                .unwrap_err()
                .contains("unknown field")
        );
        for unified in [
            "",
            "[[inject]]\nid='new'\nat='game::tick'\nkind='head'\nwith='tick'\nhash='signature'\n",
        ] {
            assert!(Manifest::parse(&format!("{base}{unified}")).is_ok());
            // Build an input archive directly: the public writer rightly refuses its manifest.
            let text = format!("{base}{foreign}{unified}");
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, bytes) in [(MANIFEST, text.as_bytes()), (WASM, module.as_slice())] {
                zip.start_file(name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            let mut archive = zip::ZipArchive::new(zip.finish().unwrap()).unwrap();
            let error = Package::read_zip(&mut archive)
                .err()
                .expect("non-raw wasm declaration refused");
            assert!(error.contains("unknown field"), "{error}");
        }
    }

    #[test]
    fn a_wasm_raw_listener_uses_the_native_internal_declaration_without_native_code() {
        let wasm = format!(
            "{}\n[wasm]\nsha512 = '{}'\ntoolchain = '1.99.1'\n",
            EXAMPLE.trim_end(),
            "0".repeat(128)
        );
        let native = "[native]\ninternals=[{path='Game::world',kind='field',hash='world-hash'}]\n";
        let injection = "[[inject]]\nid='fell'\nat='Game::break_block'\nkind='head'\nwith='fell'\naccess='internals'\nhash='h'\n";
        let parsed = Manifest::parse(&format!("{wasm}{native}{injection}")).unwrap();
        assert!(parsed.native.is_none());
        let declarations = parsed.wasm.unwrap();
        assert_eq!(declarations.internals()[0].hash, "world-hash");

        for wrong in [
            "internals=[{path='',hash='h'}]",
            "internals=[{path='Game',hash=''}]",
            "internals=[{path='Game',hash='h'}, {path='Game',hash='h'}]",
            "module=[]",
            "game='g'",
            "horizon='pin'",
            "mixins='pin'",
        ] {
            assert!(
                Manifest::parse(&format!("{wasm}[native]\n{wrong}\n{injection}")).is_err(),
                "{wrong}"
            );
        }
        assert!(
            Manifest::parse(&format!("{wasm}{injection}"))
                .unwrap_err()
                .contains("needs declared internals")
        );
    }

    #[test]
    fn content_round_trips_and_executable_files_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        let archive = root.join("ruby.hmod");
        std::fs::write(&archive, Package::write_files(EXAMPLE, &[]).unwrap()).unwrap();
        assert_eq!(Package::read(&archive).unwrap().manifest.id, "ruby");
        assert_eq!(Package::read(&dir).unwrap().manifest.id, "ruby");
        let cases: &[(&str, &[u8])] = &[
            ("mod.wasm", b""),
            ("./mod.wasm", b"\0asm"),
            ("MOD.WASM", b""),
            ("sub/mod.wasm", b""),
            ("assets/ruby/CODE.WaSm", b""),
            ("assets/ruby/model.bin", b"\0asm"),
            ("assets/ruby/native.bin", b"\x7fELF"),
            ("assets/ruby/windows.bin", b"MZ"),
            ("assets/ruby/mac.bin", b"\xcf\xfa\xed\xfe"),
            ("assets/ruby/script", b"#!/bin/sh"),
            ("assets/ruby/program.EXE", b""),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, bytes).unwrap();
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
            zip.finish().unwrap();
            let error = Package::read(&archive).err().unwrap();
            // The one place a module may sit is refused for want of `[wasm]`.
            let expected = if *name == WASM {
                WasmError::Undeclared.to_string()
            } else {
                "unsupported executable file".to_string()
            };
            assert!(error.contains(&expected), "{name}: {error}");
            if name.starts_with("assets/") {
                let error = Package::read(&dir).err().unwrap();
                assert!(
                    error.contains("unsupported executable file"),
                    "{name}: {error}"
                );
            } else if std::fs::symlink_metadata(dir.join(WASM)).is_ok() {
                // On disk `./mod.wasm` is the top-level module too, and so is `MOD.WASM` where the
                // filesystem ignores case (Windows, macOS).
                let undeclared = WasmError::Undeclared.to_string();
                assert_eq!(
                    Package::read(&dir).err(),
                    Some(undeclared),
                    "unpackaged {name}"
                );
            } else {
                assert!(Package::read(&dir).is_ok(), "unpackaged {name}");
            }
            std::fs::remove_file(file).unwrap();
        }
        assert!(Package::write_files(EXAMPLE, &[("assets/ruby/model.bin", b"\0asm")]).is_err());
        // Native bundle sources can start with Rust inner attributes, not a script shebang.
        assert!(
            Package::write_files(
                EXAMPLE,
                &[("assets/ruby/source.txt", b"#![allow(dead_code)]")]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_folder_loads_without_packaging_git_hooks() {
        let root = std::env::temp_dir().join(format!("horizon-mod-git-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(root.join(".git/hooks")).unwrap();
        std::fs::write(root.join(".git/hooks/pre-commit"), b"#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.join(".git/hooks/pre-commit"),
                std::fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        std::fs::create_dir_all(root.join("assets/ruby")).unwrap();
        std::fs::write(root.join("assets/ruby/icon.png"), b"image").unwrap();
        assert_eq!(Package::read(&root).unwrap().manifest.id, "ruby");
        std::fs::write(root.join("assets/ruby/icon.png"), b"\0asm").unwrap();
        assert!(
            Package::read(&root)
                .err()
                .unwrap()
                .contains("unsupported executable")
        );
        std::fs::remove_file(root.join("assets/ruby/icon.png")).unwrap();
        std::fs::create_dir_all(root.join("assets/minecraft/lang")).unwrap();
        std::fs::write(root.join("assets/minecraft/lang/en_us.json"), "{}").unwrap();
        let package = Package::read(&root).unwrap();
        assert!(crate::Content::read(&root, &package.manifest).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_links_and_executable_modes_are_refused() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!("horizon-mods-kinds-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("ruby.hmod");
        for link in [false, true] {
            let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            // Made by a Unix tool on every OS: the writer's default on Windows is DOS attributes,
            // which hold no mode to refuse.
            let options = zip::write::SimpleFileOptions::default().system(zip::System::Unix);
            zip.start_file(MANIFEST, options).unwrap();
            zip.write_all(EXAMPLE.as_bytes()).unwrap();
            if link {
                zip.add_symlink("assets/ruby/link", "missing", options)
                    .unwrap();
            } else {
                zip.start_file("assets/ruby/program", options.unix_permissions(0o755))
                    .unwrap();
                zip.write_all(b"ordinary bytes").unwrap();
            }
            zip.finish().unwrap();
            let error = Package::read(&path).err().unwrap();
            assert!(
                error.contains(if link {
                    "unsupported file type"
                } else {
                    "unsupported executable"
                }),
                "{error}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn folder_links_and_executable_modes_are_refused() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root =
            std::env::temp_dir().join(format!("horizon-mods-folder-kinds-{}", std::process::id()));
        let dir = root.join("folder");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), EXAMPLE).unwrap();
        std::fs::create_dir_all(dir.join("assets/ruby")).unwrap();
        let file = dir.join("assets/ruby/program");
        std::fs::write(&file, b"ordinary bytes").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = Package::read(&dir).err().unwrap();
        assert!(error.contains("unsupported executable"), "{error}");
        std::fs::remove_file(&file).unwrap();
        for name in ["assets/ruby/mod.wasm", "assets/ruby/nested/link"] {
            let link = dir.join(name);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            symlink("missing", &link).unwrap();
            let error = Package::read(&dir).err().unwrap();
            assert!(error.contains("unsupported file type"), "{error}");
            std::fs::remove_file(link).unwrap();
        }
        std::fs::remove_dir_all(dir.join("assets")).unwrap();
        symlink("missing", dir.join("assets")).unwrap();
        assert!(
            Package::read(&dir)
                .err()
                .unwrap()
                .contains("unsupported file type")
        );
        let link = root.join("linked-folder");
        symlink(&dir, &link).unwrap();
        let error = Package::read(&link).err().unwrap();
        assert!(error.contains("unsupported file type"), "{error}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
