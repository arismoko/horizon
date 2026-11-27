//! The function index (`functions.json` in the SDK's build data): every function of the game's
//! source a `[[native.mixin]]` row may name, with where it is and whether function replacement can
//! take it. The staging pass writes it from the pristine source, with the same checks
//! [`crate::patch`] once ran on the player's machine on every build (a walk of the game's whole
//! source and a parse of every crate below it), so a mixin build reads only the files of the
//! modules its targets are in. Before a function is copied, its text is checked against the
//! index's hash: a source that is not the build's is refused by name, never walked.
use crate::patch::{Found, Source, TargetPath, replaceable_with_binding, target_path};
use crate::source::{SourceLayout, plain_path};
use mod_surface::build_data::FunctionDigests;
use quote::{ToTokens, quote};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use syn::spanned::Spanned;
use syn::{ImplItem, Item};

/// The index's file name in the build data.
pub const FILE: &str = mod_surface::build_data::FUNCTIONS;

/// The function index of one game build.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionIndex {
    /// The crates a function may be copied from, by the name a target starts with.
    pub crates: BTreeMap<String, CrateModules>,
    /// The functions function replacement can take, by [`key`].
    pub functions: BTreeMap<String, Function>,
    /// The functions it can't, by [`key`], with the refusal a row naming one gets.
    pub refused: BTreeMap<String, Refused>,
    /// Alternate spellings with the same resolved nominal owner as a published target.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, String>,
    /// Per-slot owner, trait, and installer evidence for every staged trait-method slot.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub trait_slots: BTreeMap<String, crate::replace::TraitSlotEvidence>,
}

/// One crate's modules, as a copy's mirror reads them. Every crate of the source is listed, so a
/// target naming none of them is told so; only the game crate and the crates with
/// `#[replaceable]` functions list their modules.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateModules {
    /// Its library's name (`[lib] name`), which the patch crate names it by.
    pub library: String,
    /// Each module's files.
    pub modules: BTreeMap<String, Vec<ModuleFile>>,
}

/// A file a copy may read: the patch step checks it is the build's before reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleFile {
    /// Relative to the source root, `/`-separated.
    pub path: String,
    /// Conditions on this contribution to the module. A pinned lower build selects its files
    /// before their declarations are re-exported into the patch crate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cfg: Vec<String>,
    /// The SHA-256 of the file's bytes, hex.
    pub sha256: String,
    /// The enclosing gates on a literal include, outermost first: the same conditions as `cfg`.
    /// An empty list means an unconditional include; a missing field is a regular module file
    /// (or an older index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_cfg: Option<Vec<String>>,
}

/// A function that can take a replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    /// The module its file is, as [`CrateModules::modules`] keys it (an inline module's function is
    /// in a module below it).
    pub module: String,
    /// Its file, relative to the source root, `/`-separated.
    pub file: String,
    /// Its bytes in the file, `[start, end)`: attributes, signature and body.
    pub span: [usize; 2],
    /// The hash of its signature, as the other build-data files hash theirs.
    pub signature_hash: String,
    /// The SHA-256 of the span's bytes, hex.
    pub sha256: String,
    /// The declaration and body fingerprints. Earlier build indexes have only the span hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digests: Option<FunctionDigests>,
    /// Resolved owner, trait, and installer module when this function is a trait-impl method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trait_binding: Option<crate::replace::TraitSlotEvidence>,
}

/// Why a function can't take a replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refused {
    pub code: Code,
    pub why: String,
}

/// A refusal's code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Code {
    #[serde(rename = "E_MIXIN_NOT_REPLACEABLE")]
    NotReplaceable,
    #[serde(rename = "E_MIXIN_TARGET_AMBIGUOUS")]
    Ambiguous,
}

impl Code {
    pub fn name(self) -> &'static str {
        match self {
            Code::NotReplaceable => "E_MIXIN_NOT_REPLACEABLE",
            Code::Ambiguous => "E_MIXIN_TARGET_AMBIGUOUS",
        }
    }
}

/// A target as the index keys it: no generic arguments or spaces, and a trait method's trait by
/// its last segment (`m::<GameLevel as LevelAccess>::block_state`), as the copy finds it.
pub fn key(target: &str) -> String {
    match target_path(target) {
        None => plain_path(target),
        Some(TargetPath {
            qualified: Some((module, ty, tr)),
            name,
            ..
        }) => format!("{}::<{ty} as {tr}>::{name}", plain_path(module)),
        Some(TargetPath { front, name, .. }) => format!("{}::{name}", plain_path(front)),
    }
}

/// `relative`, an index's path, under `root`: a path that would leave it is refused.
pub(crate) fn source_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(format!(
            "{FILE} names `{relative}`, which is not a path inside the game's source"
        ));
    }
    Ok(root.join(path))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl FunctionIndex {
    /// The published key, including a source-proven alternate spelling when present.
    pub fn target_key(&self, target: &str) -> String {
        let key = key(target);
        self.aliases.get(&key).cloned().unwrap_or(key)
    }

    /// Whether `target` can be copied out of the source at `root`: listed as replaceable, and its
    /// span there is the indexed text. Otherwise its refusal, `(code, why)`.
    ///
    /// Only the span is checked here; every other file the copy reads is checked against its
    /// [`ModuleFile::sha256`] as it is read.
    pub fn check(&self, root: &Path, target: &str) -> Result<(), (&'static str, String)> {
        self.check_with_layout(
            root,
            target,
            SourceLayout::read(root).map_err(|error| ("E_MIXIN_SOURCE", error))?,
        )
    }

    pub(crate) fn check_with_layout(
        &self,
        root: &Path,
        target: &str,
        layout: SourceLayout,
    ) -> Result<(), (&'static str, String)> {
        let key = self.target_key(target);
        let krate = key.split("::").next().unwrap_or_default();
        if !self.crates.contains_key(krate) {
            return Err((
                "E_MIXIN_TARGET_MISSING",
                format!("no crate `{krate}` in the game's source"),
            ));
        }
        if let Some(refused) = self.refused.get(&key) {
            return Err((refused.code.name(), refused.why.clone()));
        }
        let Some(function) = self.functions.get(&key) else {
            if let Some(named) = self.named_by_type(&key, layout) {
                return Err((
                    "E_MIXIN_TARGET_MISSING",
                    format!(
                        "`{key}` names the module of the file its `impl` is in; a method below the game crate is named by the module that declares its type: `{named}`"
                    ),
                ));
            }
            return Err(if layout.owns(&key) {
                (
                    "E_MIXIN_TARGET_MISSING",
                    format!("no function `{key}` in this game build's function index ({FILE})"),
                )
            } else {
                (
                    "E_MIXIN_NOT_REPLACEABLE",
                    crate::replace::UNMARKED.to_owned(),
                )
            });
        };
        let stale = |why: String| ("E_MIXIN_SOURCE", why);
        let path = source_path(root, &function.file).map_err(stale)?;
        let bytes = std::fs::read(&path).map_err(|e| stale(format!("{}: {e}", path.display())))?;
        let [start, end] = function.span;
        let text = bytes.get(start..end).ok_or_else(|| {
            stale(format!(
                "{} has no bytes {start}..{end}: it is not the source this game build's {FILE} describes",
                function.file
            ))
        })?;
        if sha256(text) != function.sha256 {
            return Err(stale(format!(
                "{} bytes {start}..{end} differ from this game build's {FILE}: the source is not the build's",
                function.file
            )));
        }
        Ok(())
    }

    /// The listed method a crate below the game crate names by its type's module that `key`
    /// names by the module of the file its `impl` is in, as targets were written before.
    fn named_by_type(&self, key: &str, layout: SourceLayout) -> Option<&str> {
        self.functions.iter().find_map(|(listed, function)| {
            if layout.owns(listed) {
                return None;
            }
            let by_file = match target_path(listed)? {
                TargetPath {
                    qualified: Some((module, ty, tr)),
                    name,
                    ..
                } => (plain_path(module) != function.module)
                    .then(|| format!("{}::<{ty} as {tr}>::{name}", function.module))?,
                TargetPath { front, name, .. } => {
                    let front = plain_path(front);
                    let (module, owner) = front.rsplit_once("::")?;
                    // A free function is in its file's module already.
                    if front == function.module || module == function.module {
                        return None;
                    }
                    format!("{}::{owner}::{name}", function.module)
                }
            };
            (by_file == key).then_some(listed.as_str())
        })
    }

    /// The index as its reader may trust it, or why not: every crate and library is a Rust
    /// name and every module a path of one, as the copy writes them into Rust code, and each
    /// function is in a module the index lists. A downloaded index is checked here once, so
    /// nothing in it reaches the copy unchecked.
    pub fn checked(self) -> Result<Self, String> {
        let name = |text: &str| syn::parse_str::<syn::Ident>(text).is_ok();
        let path = |text: &str| text.split("::").all(name);
        for (krate, facts) in &self.crates {
            if !name(krate) || !name(&facts.library) {
                return Err(format!(
                    "{FILE}: crate `{krate}` or its library `{}` is not a Rust name",
                    facts.library
                ));
            }
            if let Some(module) = facts
                .modules
                .keys()
                .find(|module| !path(module) || module.split("::").next() != Some(krate))
            {
                return Err(format!(
                    "{FILE}: `{module}` is not a module path of crate `{krate}`"
                ));
            }
        }
        if let Some((key, function)) = self.functions.iter().find(|(_, function)| {
            let krate = function.module.split("::").next().unwrap_or_default();
            !self
                .crates
                .get(krate)
                .is_some_and(|facts| facts.modules.contains_key(&function.module))
        }) {
            return Err(format!(
                "{FILE}: function `{key}` is in `{}`, which is not a module the index lists",
                function.module
            ));
        }
        for (alias, target) in &self.aliases {
            if self.functions.contains_key(alias)
                || self.refused.contains_key(alias)
                || !(self.functions.contains_key(target) || self.refused.contains_key(target))
            {
                return Err(format!("{FILE}: invalid alias `{alias}` for `{target}`"));
            }
        }
        Ok(self)
    }

    /// The function `target` names, read at its indexed span: the very item the index's verdict
    /// is about, never one looked up again by name. Call it once [`FunctionIndex::check`] passed.
    pub(crate) fn found(
        &self,
        source: &mut Source,
        target: &str,
    ) -> Result<Found, (&'static str, String)> {
        let key = self.target_key(target);
        let stale = |why: String| ("E_MIXIN_SOURCE", why);
        let function = self
            .functions
            .get(&key)
            .ok_or_else(|| stale(format!("no function `{key}` in {FILE}")))?;
        let path = source_path(source.root(), &function.file).map_err(stale)?;
        let text = source.read(&path).map_err(stale)?;
        let items = syn::parse_file(&text)
            .map_err(|e| stale(format!("{}: {e}", path.display())))?
            .items;
        let lines = line_starts(&text);
        let file = File {
            relative: &function.file,
            module: &function.module,
            text: &text,
            lines: &lines,
        };
        let krate = function.module.split("::").next().unwrap_or_default();
        let types = if krate == "sunrise_client" {
            None
        } else {
            Some(source.owner_types(krate).map_err(stale)?)
        };
        let mut candidates = BTreeMap::new();
        collect(&function.module, &items, &file, types, &mut candidates);
        let [start, end] = function.span;
        candidates
            .remove(&key)
            .into_iter()
            .flatten()
            .find(|candidate| candidate.function.span == function.span)
            .map(|candidate| candidate.found)
            .ok_or_else(|| {
                stale(format!(
                    "{} has no function `{key}` at bytes {start}..{end}, where this game build's {FILE} has it",
                    function.file
                ))
            })
    }

    /// Refuses each listed function, except a published loop's generic one (`generic`, keys),
    /// that the staging pass gave no replacement slot (`slots`, keys): it has no slot to take a
    /// copy.
    pub fn retain_slotted(&mut self, slots: &BTreeSet<String>, generic: &BTreeSet<String>) {
        let unslotted: Vec<String> = self
            .functions
            .keys()
            .filter(|key| !slots.contains(*key) && !generic.contains(*key))
            .cloned()
            .collect();
        for key in unslotted {
            self.functions.remove(&key);
            self.refused.insert(
                key,
                Refused {
                    code: Code::NotReplaceable,
                    why: "the game build gives it no replacement slot".into(),
                },
            );
        }
    }
    /// Count refused targets grouped by refusal reason, normalizing per-target names.
    pub fn refusal_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for refused in self.refused.values() {
            let key = refusal_reason(&refused.why);
            *counts.entry(key.to_owned()).or_default() += 1;
        }
        counts
    }
}

fn refusal_reason(why: &str) -> &str {
    if why.starts_with("its trait implementation has no replacement installer: owner type name `") {
        "its trait implementation has no replacement installer (ambiguous trait owner)"
    } else if why.starts_with("its body uses `")
        && why.ends_with("`, a macro defined outside its module's path")
    {
        "its body uses a macro defined outside its module's path"
    } else if why.starts_with("it names `") && why.contains("`, which crosses a non-public `use`") {
        "it names a path that crosses a non-public use"
    } else if why.starts_with("it names `")
        && why.contains("`, which reaches the non-public crate-root function `")
    {
        "it names a non-public crate-root function"
    } else {
        why
    }
}

/// The index of the pristine source at `root`, whose published loops' generic functions are
/// before the index; [`FunctionIndex::retain_slotted`] then drops what staging gave no slot.
pub fn index(root: &Path, generic: &BTreeSet<String>) -> Result<FunctionIndex, String> {
    index_of_with_context(root, SourceLayout::read(root)?, generic, None).map(|(index, _, _)| index)
}

pub fn index_with_layout(root: &Path, layout: SourceLayout) -> Result<FunctionIndex, String> {
    let generic = crate::loops::collect_with_layout(root, layout)?
        .into_iter()
        .map(|entry| key(&entry.generic_target()))
        .collect();
    index_with_context(root, layout, &generic).map(|(index, _, _)| index)
}

pub(crate) fn index_with_context(
    root: &Path,
    layout: SourceLayout,
    generic: &BTreeSet<String>,
) -> Result<
    (
        FunctionIndex,
        BTreeMap<String, crate::replace::OwnerTypes>,
        crate::replace::WorkspaceTraits,
    ),
    String,
> {
    index_of_with_context(root, layout, generic, None)
}

/// [`index`], listing only the functions whose keys are in `only`, when given.
#[cfg(test)]
pub(crate) fn index_of(
    root: &Path,
    generic: &BTreeSet<String>,
    only: Option<&BTreeSet<String>>,
) -> Result<FunctionIndex, String> {
    index_of_with_context(root, crate::LEGACY_LAYOUT, generic, only).map(|(index, _, _)| index)
}

fn index_of_with_context(
    root: &Path,
    layout: SourceLayout,
    generic: &BTreeSet<String>,
    only: Option<&BTreeSet<String>>,
) -> Result<
    (
        FunctionIndex,
        BTreeMap<String, crate::replace::OwnerTypes>,
        crate::replace::WorkspaceTraits,
    ),
    String,
> {
    let mut source = Source::new(root, layout)?;
    // Below the game crate, only the functions marked `#[replaceable]` take a replacement.
    let marked: BTreeSet<String> = crate::replace::marked(root, layout)?
        .into_iter()
        .map(|(_, path)| key(&path))
        .collect();
    let lower: BTreeSet<&str> = marked
        .iter()
        .filter_map(|path| path.split("::").next())
        .filter(|krate| layout.domain(krate).is_none())
        .collect();
    // A method below the game crate is named by its type's module.
    let mut owners = BTreeMap::new();
    for krate in lower {
        if !source.load(krate)? {
            return Err(format!(
                "{krate}: it has #[sunrise_mod_api_macros::replaceable] functions, but its sources can't be read"
            ));
        }
        owners.insert(krate, crate::replace::crate_owner_types(root, krate)?);
    }
    let mut index = FunctionIndex::default();
    for (module, files) in &source.files {
        let krate = module.split("::").next().unwrap_or_default();
        let facts = index
            .crates
            .entry(krate.to_owned())
            .or_insert_with(|| CrateModules {
                library: source
                    .libraries
                    .get(krate)
                    .cloned()
                    .unwrap_or_else(|| krate.to_owned()),
                modules: BTreeMap::new(),
            });
        let mut listed = Vec::new();
        for file in files {
            let path = &file.path;
            // A `#[path]` or include can lead through `..`: use the archive's file spelling.
            let inside = path.strip_prefix(root).ok().and_then(|inside| {
                let mut parts: Vec<String> = Vec::new();
                for component in inside.components() {
                    match component {
                        Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                        Component::ParentDir => drop(parts.pop()?),
                        _ => {}
                    }
                }
                Some(parts.join("/"))
            });
            let relative = inside.ok_or_else(|| {
                format!(
                    "{}: outside the source at {}",
                    path.display(),
                    root.display()
                )
            })?;
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            listed.push(ModuleFile {
                path: relative,
                cfg: file.cfg.clone(),
                sha256: sha256(&bytes),
                include_cfg: file.included.then(|| file.cfg.clone()),
            });
        }
        facts.modules.insert(module.clone(), listed);
    }
    // The other crates, by name only: a target in one is told it is not marked, not that the
    // crate does not exist.
    let crates = root.join("crates");
    for entry in std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))? {
        let package = entry
            .map_err(|e| format!("{}: {e}", crates.display()))?
            .path();
        let Some(folder) = package.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !package.join("Cargo.toml").is_file() {
            continue;
        }
        let krate = folder.replace('-', "_");
        if let std::collections::btree_map::Entry::Vacant(slot) = index.crates.entry(krate) {
            slot.insert(CrateModules {
                library: crate::replace::library_name(&package)?,
                modules: BTreeMap::new(),
            });
        }
    }
    // Every function a target can name, by key, with its file and span.
    let mut candidates: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
    let files: Vec<(String, String, PathBuf)> = index
        .crates
        .values()
        .flat_map(|facts| {
            facts.modules.iter().flat_map(|(module, files)| {
                files
                    .iter()
                    .map(|file| (module.clone(), file.path.clone(), root.join(&file.path)))
            })
        })
        .collect();
    for (module, relative, path) in files {
        let prefix = format!("{module}::");
        if only.is_some_and(|only| !only.iter().any(|key| key.starts_with(&prefix))) {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let items = syn::parse_file(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .items;
        let lines = line_starts(&text);
        let file = File {
            relative: &relative,
            module: &module,
            text: &text,
            lines: &lines,
        };
        let krate = module.split("::").next().unwrap_or_default();
        collect(&module, &items, &file, owners.get(krate), &mut candidates);
    }
    let mut aliases = Vec::new();
    for (key, mut found) in candidates {
        let krate = key.split("::").next().unwrap_or_default();
        if only.is_some_and(|only| !only.contains(&key))
            || (layout.domain(krate).is_none() && !marked.contains(&key))
        {
            continue;
        }
        if found.len() > 1 {
            index.refused.insert(
                key,
                Refused {
                    code: Code::Ambiguous,
                    why: "several functions have this path (a cfg-gated pair, for one)".into(),
                },
            );
            continue;
        }
        let mut candidate = found.pop().expect("one");
        match replaceable_with_binding(&candidate.found, &mut source, generic.contains(&key)) {
            Ok(trait_binding) => {
                if layout.is_split()
                    && let Some(owner) = candidate.found.owner.as_ref()
                    && let Some((alias, name)) = nominal_alias(&key, &candidate.found.module, owner)
                {
                    aliases.push((alias, key.clone(), candidate.found.module.clone(), name));
                }
                candidate.function.trait_binding = trait_binding;
                index.functions.insert(key, candidate.function);
            }
            Err(why) => {
                index.refused.insert(
                    key,
                    Refused {
                        code: Code::NotReplaceable,
                        why,
                    },
                );
            }
        }
    }
    let (owners, workspace) = source.into_resolve_context()?;
    for (alias, key, module, name) in aliases {
        let krate = module.split("::").next().unwrap_or_default();
        if !index.functions.contains_key(&alias)
            && !index.refused.contains_key(&alias)
            && owners.get(krate).is_some_and(|types| {
                crate::replace_resolve::is_game_nominal(&module, &name, types, &workspace)
            })
        {
            index.aliases.insert(alias, key);
        }
    }
    Ok((index, owners, workspace))
}

fn nominal_alias(key: &str, module: &str, owner: &syn::ItemImpl) -> Option<(String, String)> {
    let package = crate::GameplayPackage::from_library(module.split("::").next()?)?;
    let name = package.game_type();
    let bare = name;
    let qualified = format!("crate::game::{name}");
    let actual = plain_path(&quote::ToTokens::to_token_stream(&owner.self_ty).to_string());
    let alternate = if actual == bare {
        qualified.as_str()
    } else if actual == qualified {
        bare
    } else {
        return None;
    };
    let method = key.rsplit_once("::")?.1;
    let alias = match &owner.trait_ {
        Some((_, path, _)) => {
            let trait_name = &path.segments.last()?.ident;
            format!("{module}::<{alternate} as {trait_name}>::{method}")
        }
        None => format!("{module}::{alternate}::{method}"),
    };
    Some((alias, name.to_owned()))
}

/// A function found in a file, before its verdict.
struct Candidate {
    found: Found,
    function: Function,
}

/// Where each line of `text` starts.
fn line_starts(text: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// A file being indexed.
struct File<'a> {
    relative: &'a str,
    /// The module the file is.
    module: &'a str,
    text: &'a str,
    lines: &'a [usize],
}

impl File<'_> {
    fn candidate(&self, span: proc_macro2::Span, found: Found) -> Candidate {
        let start = crate::source::offset(self.text, self.lines, span.start());
        let end = crate::source::offset(self.text, self.lines, span.end());
        let syn::ImplItemFn {
            attrs,
            vis,
            defaultness,
            sig,
            block,
        } = &found.function;
        let owner = &found.owner;
        let declaration = quote!(#owner #(#attrs)* #vis #defaultness #sig);
        let function = Function {
            module: self.module.to_owned(),
            file: self.relative.to_owned(),
            span: [start, end],
            signature_hash: crate::internals::signature_hash(&sig.to_token_stream().to_string()),
            sha256: sha256(&self.text.as_bytes()[start..end]),
            digests: Some(FunctionDigests {
                declaration: sha256(declaration.to_string().as_bytes()),
                body: sha256(block.to_token_stream().to_string().as_bytes()),
            }),
            trait_binding: None,
        };
        Candidate { found, function }
    }
}

/// The functions of `items`, module `module`, by the key a target naming each has: free
/// functions and methods of every `impl`, inline modules included (test modules left out). Below
/// the game crate (`types`, its crate's), a method is keyed by its type's module.
fn collect(
    module: &str,
    items: &[Item],
    file: &File<'_>,
    types: Option<&crate::replace::OwnerTypes>,
    out: &mut BTreeMap<String, Vec<Candidate>>,
) {
    for item in items {
        match item {
            Item::Fn(function) => {
                out.entry(format!("{module}::{}", function.sig.ident))
                    .or_default()
                    .push(file.candidate(
                        function.span(),
                        Found {
                            module: module.to_owned(),
                            owner: None,
                            function: syn::ImplItemFn {
                                attrs: function.attrs.clone(),
                                vis: function.vis.clone(),
                                defaultness: None,
                                sig: function.sig.clone(),
                                block: (*function.block).clone(),
                            },
                        },
                    ));
            }
            Item::Impl(block) => {
                // The header and its associated types and constants: the verdict reads the
                // `impl`'s attributes, generics, trait and type, and a trait method's copy names
                // the associated items through the type; never the other methods.
                let header = syn::ItemImpl {
                    attrs: block.attrs.clone(),
                    defaultness: block.defaultness,
                    unsafety: block.unsafety,
                    impl_token: block.impl_token,
                    generics: block.generics.clone(),
                    trait_: block.trait_.clone(),
                    self_ty: block.self_ty.clone(),
                    brace_token: block.brace_token,
                    items: block
                        .items
                        .iter()
                        .filter(|item| !matches!(item, ImplItem::Fn(_)))
                        .cloned()
                        .collect(),
                };
                let owner = plain_path(&block.self_ty.to_token_stream().to_string());
                let named_in = types.map_or_else(
                    || module.to_owned(),
                    |types| crate::replace::owner_module(types, module, &block.self_ty),
                );
                let scope = match &block.trait_ {
                    Some((_, path, _)) => {
                        let last = path
                            .segments
                            .last()
                            .map(|s| s.ident.to_string())
                            .unwrap_or_default();
                        format!("{named_in}::<{owner} as {last}>")
                    }
                    None => format!("{named_in}::{owner}"),
                };
                for member in &block.items {
                    if let ImplItem::Fn(method) = member {
                        out.entry(format!("{scope}::{}", method.sig.ident))
                            .or_default()
                            .push(file.candidate(
                                method.span(),
                                Found {
                                    module: module.to_owned(),
                                    owner: Some(header.clone()),
                                    function: method.clone(),
                                },
                            ));
                    }
                }
            }
            Item::Mod(inline) if !mod_source::modules::test_only(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content {
                    collect(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        file,
                        types,
                        out,
                    );
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test]
fn game_nominal_alias_requires_the_real_owner() {
    let module = "sunrise_gameplay::game::fixture";
    let mut types = crate::replace::OwnerTypes::default();
    let workspace = crate::replace::WorkspaceTraits::default();
    let parse = |source| syn::parse_file(source).unwrap().items;
    types.declare(
        "sunrise_gameplay::game",
        &parse("pub struct ServerGame<'a>(&'a ());"),
    );
    types.declare(module, &parse("use crate::game::ServerGame;"));
    assert!(crate::replace_resolve::is_game_nominal(
        module,
        "ServerGame",
        &types,
        &workspace
    ));
    let source = "impl crate::game::ServerGame<'_> { fn move_to(&self) {} }";
    let [syn::Item::Impl(owner)] = &parse(source)[..] else {
        panic!("expected an impl")
    };
    let published = format!("{module}::crate::game::ServerGame::move_to");
    let (alias, _) = nominal_alias(&published, module, owner).unwrap();
    let mut index = FunctionIndex::default();
    index.aliases.insert(alias.clone(), published.clone());
    assert_eq!(index.target_key(&alias), published);
    assert_eq!(index.target_key(&published), published);
    let foreign = "sunrise_gameplay::game::unrelated";
    types.declare(foreign, &parse("pub struct ServerGame;"));
    assert!(!crate::replace_resolve::is_game_nominal(
        foreign,
        "ServerGame",
        &types,
        &workspace
    ));
}
