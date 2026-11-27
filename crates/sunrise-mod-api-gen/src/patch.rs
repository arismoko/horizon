//! Function replacement for `[[native.mixin]]` (fn-replace): the patch crate a mixin build compiles
//! against the prebuilt game instead of rebuilding it. Each target function is copied, unchanged,
//! out of the pinned source into a module that mirrors its own (`patch::game::autosave` for
//! `sunrise_client::game::autosave`), where every name the body uses resolves as it did in the
//! game: the mirror re-exports its module's items from the prebuilt game and repeats the module's
//! own `use` items, so `self`, `super::` and `crate::` mean what they meant. Staging makes item
//! declarations public, not imports: restricted re-exports in modules outside the mirror stay
//! inaccessible. A method moves into a one-method trait implemented for its own type, so `self`,
//! `Self` and the helpers' bindings stay as written. horizon-mixins then patches the copies, and
//! the game installs each through its function's replacement slot (`__fnr_<name>`).
#[cfg(test)]
use crate::source::LEGACY_LAYOUT;
use crate::source::{ENTRY_DOMAIN, SourceLayout, unraw};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use syn::visit::Visit;
use syn::{ImplItem, Item};

mod reexports;
use reexports::Narrowing;

/// One target function, copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patched {
    /// The target as the `[[native.mixin]]` row writes it.
    pub target: String,
    /// The copy's id for horizon-mixins, in the patch crate.
    pub id: String,
    /// The expression that builds the copy's installation result, from the patch crate's root;
    /// none for a published loop's generic function, which has no cell: its entry's copy calls it.
    pub install: Option<String>,
}

/// The patch crate's library source and the copies in it.
#[derive(Debug)]
pub struct PatchCrate {
    /// One library per game crate with copies, `(crate name, source)`: a copy's `crate::` paths
    /// resolve against the mirror of its own crate.
    pub libs: Vec<(String, String)>,
    /// Source package key for each patch library.
    pub owners: BTreeMap<String, String>,
    pub patched: Vec<Patched>,
}

/// A `[[native.module]]`: Rust items emitted as a module of the mirror of the module `file`
/// holds, where the copies reach them by the paths they would have had in the game.
#[derive(Debug, Clone)]
pub struct SupportModule {
    pub id: String,
    /// The game source file, relative to the source root, as the row names it.
    pub file: String,
    pub name: String,
    /// The payload's text.
    pub text: String,
    pub public: bool,
}

/// A `[[native.bridge]]`: horizon-mixins' accessor or invoker, emitted as an extension trait the
/// mirror modules import, so the helpers' method calls resolve as they did on the game's type.
#[derive(Debug, Clone)]
pub struct Bridge {
    pub id: String,
    /// The generated method's name, `__horizon_mixins_<mod>_<id>`.
    pub method: String,
    /// `GET`, `GET_MUT`, `SET` or `INVOKE`.
    pub kind: String,
    pub target: String,
    pub member: Option<String>,
}

/// A published loop with mods' slots (`[[native.loop]]`): its entry's copy runs the loop with them.
#[derive(Debug, Clone)]
pub struct LoopSlots {
    pub published: crate::loops::PublishedLoop,
    /// The slots, in the order they are asked: `(priority, mod id)`.
    pub slots: Vec<LoopSlot>,
}

/// One mod's slot in a published loop: its helper's items, compiled in the loop's module like a
/// mixin helper, and the slot type they define.
#[derive(Debug, Clone)]
pub struct LoopSlot {
    /// The mod's `[mod] id`.
    pub mod_id: String,
    /// The row's id.
    pub id: String,
    /// The helper's text.
    pub helper: String,
    /// The slot type the helper defines.
    pub with: String,
}

/// What a mod set adds to the patch crate beside its copies.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    pub modules: Vec<SupportModule>,
    pub bridges: Vec<Bridge>,
    /// The loops whose entries are among the targets, with their slots.
    pub loops: Vec<LoopSlots>,
    /// The identifiers the rows' helpers and bindings use: a lower crate's mirror keeps a module's
    /// `use` only if a copied body or one of these names it.
    pub idents: BTreeSet<String>,
    /// The crates the patch crates link (`--extern`) besides std: an import from one of them
    /// resolves in a mirror as it did in the game.
    pub linked: BTreeSet<String>,
    /// The pinned cfg atoms of each lower library, by package folder with underscores. `None`
    /// is for source-only inspection, which leaves inferred conditions in the generated Rust.
    pub lower_crate_cfg: Option<BTreeMap<String, Vec<String>>>,
}

/// Why a target can't be copied out: a named refusal, never a game rebuild.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub target: String,
    pub code: &'static str,
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}: {}", self.code, self.target, self.why)
    }
}

/// Macros whose expansion depends on the file or crate they are written in.
const PLACE_MACROS: &[&str] = &[
    "file",
    "line",
    "column",
    "module_path",
    "include",
    "include_str",
    "include_bytes",
    "env",
    "option_env",
];

/// One source module: items from its file or inline block, and any indexed fragments.
pub(crate) struct Module {
    pub(crate) items: Vec<Item>,
}

/// A module's contribution from a physical source file, and the conditions of the declarations
/// that reach it.
#[derive(Clone)]
pub(crate) struct SourceFile {
    pub(crate) path: PathBuf,
    pub(crate) cfg: Vec<String>,
    /// Present for a literal include, even if unconditional. The other files are `mod` bodies.
    pub(crate) included: bool,
}

impl From<mod_source::modules::ModuleFile> for SourceFile {
    fn from(file: mod_source::modules::ModuleFile) -> Self {
        Self {
            path: file.path,
            cfg: file.cfg,
            included: file.included,
        }
    }
}

/// The game crate's modules by path, parsed lazily from the pinned source.
pub(crate) struct Source {
    layout: SourceLayout,
    root: PathBuf,
    /// The crates whose modules are in `files`.
    loaded: BTreeSet<String>,
    pub(crate) files: BTreeMap<String, Vec<SourceFile>>,
    parsed: BTreeMap<String, Module>,
    lower_crate_cfg: BTreeMap<String, BTreeSet<String>>,
    /// `macro_rules!` names the crate defines without `#[macro_export]`: a body using one can't
    /// leave the crate.
    local_macros: BTreeSet<String>,
    /// Each loaded lower crate's library name (`[lib] name`), by its folder's crate name.
    pub(crate) libraries: BTreeMap<String, String>,
    /// Each crate's declared types, read when a row first targets a trait method of the crate.
    owners: BTreeMap<String, crate::replace::OwnerTypes>,
    /// What [`replaceable`] reads once for every function it is asked about.
    cache: Cache,
    /// Each file's SHA-256 in the function index: a file that differs is refused as it is read.
    hashes: BTreeMap<PathBuf, String>,
    /// The first unreadable, invalid or mismatched indexed source file. Refuse the build even
    /// when a speculative module lookup treated the failure as "no such module".
    stale: Option<String>,
}

/// The walks [`replaceable`] would otherwise repeat per function: the index asks it about every
/// function of the game.
#[derive(Default)]
struct Cache {
    /// Workspace-wide trait resolver for cross-crate trait declarations.
    traits: Option<crate::replace::WorkspaceTraits>,
    /// The macro names the modules on each path define.
    on_path: BTreeMap<String, BTreeSet<String>>,
    scopes: reexports::Scopes,
}

impl Source {
    /// The modules the function index lists, with nothing walked: each file is read when a
    /// target's module is.
    fn indexed(
        root: &Path,
        layout: SourceLayout,
        index: &crate::functions::FunctionIndex,
        cfgs: Option<&BTreeMap<String, Vec<String>>>,
    ) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        let mut hashes = BTreeMap::new();
        let mut lower_crate_cfg = BTreeMap::new();
        for (krate, facts) in &index.crates {
            let cfg = cfgs
                .and_then(|cfgs| cfgs.get(krate))
                .map(|cfg| {
                    cfg.iter()
                        .map(|atom| {
                            mod_source::modules::cfg_atom(atom).map_err(|error| {
                                format!("invalid SDK cfg for {krate} ({atom}): {error}")
                            })
                        })
                        .collect::<Result<BTreeSet<_>, _>>()
                })
                .transpose()?;
            for (module, listed) in &facts.modules {
                let mut joined = Vec::new();
                'file: for file in listed {
                    let path = crate::functions::source_path(root, &file.path)?;
                    hashes.insert(path.clone(), file.sha256.clone());
                    // An older index carries an include's gates only in `include_cfg`.
                    let gates = file.include_cfg.clone().unwrap_or_else(|| file.cfg.clone());
                    if let Some(cfg) = &cfg {
                        for condition in &gates {
                            match mod_source::modules::evaluate_cfg(condition, |atom| {
                                Some(cfg.contains(atom))
                            }) {
                                Some(true) => {}
                                Some(false) => continue 'file,
                                None => {
                                    return Err(format!(
                                        "invalid module cfg for {}: {condition}",
                                        file.path
                                    ));
                                }
                            }
                        }
                    }
                    joined.push(SourceFile {
                        path,
                        cfg: gates,
                        included: file.include_cfg.is_some(),
                    });
                }
                if !joined.is_empty() {
                    files.insert(module.clone(), joined);
                }
            }
            libraries.insert(krate.clone(), facts.library.clone());
            if let Some(cfg) = cfg {
                lower_crate_cfg.insert(krate.clone(), cfg);
            }
        }
        Ok(Self {
            layout,
            root: root.to_owned(),
            loaded: index.crates.keys().cloned().collect(),
            files,
            parsed: BTreeMap::new(),
            lower_crate_cfg,
            local_macros: BTreeSet::new(),
            libraries,
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes,
            stale: None,
        })
    }

    fn extras_owner<'a>(&self, path: &'a str) -> &'a str {
        let owner = path.split("::").next().expect("a crate");
        self.layout
            .domain(owner)
            .map_or(self.layout.runtime.key, |domain| domain.key)
    }

    /// The source root.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The text of `path`, refused when the function index has another hash for it.
    pub(crate) fn read(&mut self, path: &Path) -> Result<String, String> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            let why = format!("{}: {e}", path.display());
            if self.hashes.contains_key(path) {
                self.stale.get_or_insert_with(|| why.clone());
            }
            why
        })?;
        if let Some(expected) = self.hashes.get(path)
            && crate::functions::sha256(text.as_bytes()) != *expected
        {
            let why = format!(
                "{} differs from this game build's {}: the source is not the build's",
                path.display(),
                crate::functions::FILE
            );
            self.stale.get_or_insert_with(|| why.clone());
            return Err(why);
        }
        Ok(text)
    }

    pub(crate) fn new(root: &Path, layout: SourceLayout) -> Result<Self, String> {
        let mut files = BTreeMap::new();
        let mut libraries = BTreeMap::new();
        let mut loaded = BTreeSet::new();
        let mut local_macros = BTreeSet::new();
        for domain in layout.domains {
            let lib = domain.library(root)?;
            let package = domain.package_path(root);
            if domain.key != ENTRY_DOMAIN.key && package.join("Cargo.toml").is_file() {
                libraries.insert(
                    domain.key.to_owned(),
                    crate::replace::library_name(&package)?,
                );
            }
            loaded.insert(domain.key.to_owned());
            let mut local = BTreeSet::new();
            let mut exported = BTreeSet::new();
            for path in walk(lib.parent().expect("a source file has a folder")) {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                macro_names(&text, &mut local, &mut exported);
            }
            local_macros.extend(local.into_iter().filter(|name| !exported.contains(name)));
            files.insert(
                domain.key.to_owned(),
                vec![SourceFile {
                    path: lib,
                    cfg: Vec::new(),
                    included: false,
                }],
            );
        }
        for file in layout.files(root, false)? {
            files
                .entry(file.module.clone())
                .or_insert_with(Vec::new)
                .push(file.into());
        }
        Ok(Self {
            layout,
            root: root.to_owned(),
            loaded,
            files,
            parsed: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            local_macros,
            libraries,
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        })
    }

    /// Adds a lower crate's modules, once: `Ok(false)` when the source has no such crate.
    pub(crate) fn load(&mut self, krate: &str) -> Result<bool, String> {
        if self.loaded.contains(krate) {
            return Ok(true);
        }
        let Ok((package, lib)) = crate::source::crate_root(&self.root, krate) else {
            return Ok(false);
        };
        let library = crate::replace::library_name(&package)?;
        let dir = lib.parent().expect("a source file has a folder").to_owned();
        let files = crate::source::module_files(&self.root, krate.to_owned(), lib, dir, false)?;
        let mut exported = BTreeSet::new();
        let mut local = BTreeSet::new();
        for file in files {
            let text = self.read(&file.path)?;
            macro_names(&text, &mut local, &mut exported);
            self.files
                .entry(file.module.clone())
                .or_default()
                .push(file.into());
        }
        self.local_macros
            .extend(local.into_iter().filter(|name| !exported.contains(name)));
        self.libraries.insert(krate.to_owned(), library);
        self.loaded.insert(krate.to_owned());
        Ok(true)
    }

    /// The types `krate` declares, from the files the staging pass reads: the installer a
    /// trait method has in the prebuilt game was decided on the same set.
    pub(crate) fn owner_types(
        &mut self,
        krate: &str,
    ) -> Result<&crate::replace::OwnerTypes, String> {
        if !self.owners.contains_key(krate) {
            let mut types = crate::replace::OwnerTypes::default();
            if let Some(domain) = self.layout.domain(krate) {
                let lib_path = domain.library(&self.root)?;
                if lib_path.is_file() {
                    let text = std::fs::read_to_string(&lib_path)
                        .map_err(|e| format!("{}: {e}", lib_path.display()))?;
                    let parsed = syn::parse_file(&text)
                        .map_err(|e| format!("{}: {e}", lib_path.display()))?;
                    types.declare(krate, &parsed.items);
                }
                for file in domain.files(&self.root, true)? {
                    let text = std::fs::read_to_string(&file.path)
                        .map_err(|e| format!("{}: {e}", file.path.display()))?;
                    let parsed = syn::parse_file(&text)
                        .map_err(|e| format!("{}: {e}", file.path.display()))?;
                    types.declare(&file.module, &parsed.items);
                }
            } else {
                for file in crate::source::crate_files(&self.root, krate)? {
                    let text = std::fs::read_to_string(&file.path)
                        .map_err(|e| format!("{}: {e}", file.path.display()))?;
                    let parsed = syn::parse_file(&text)
                        .map_err(|e| format!("{}: {e}", file.path.display()))?;
                    types.declare(&file.module, &parsed.items);
                }
            }
            self.owners.insert(krate.to_owned(), types);
        }
        Ok(&self.owners[krate])
    }

    pub(crate) fn into_resolve_context(
        mut self,
    ) -> Result<
        (
            BTreeMap<String, crate::replace::OwnerTypes>,
            crate::replace::WorkspaceTraits,
        ),
        String,
    > {
        let workspace = match self.cache.traits.take() {
            Some(workspace) => workspace,
            None => crate::replace::WorkspaceTraits::load(&self.root)?,
        };
        Ok((self.owners, workspace))
    }

    fn cfg(&self, module: &str) -> Option<&BTreeSet<String>> {
        self.lower_crate_cfg.get(module.split("::").next()?)
    }

    /// Module `path`'s items with the gates on each included fragment. When reading an SDK, only
    /// files named by its index are read, never paths taken from the `include!` invocations.
    /// Under a pinned lower crate's cfg the scope is then configured: an item that build leaves
    /// out is dropped, and a gate it decided is gone.
    pub(crate) fn module(&mut self, path: &str) -> Result<&Module, String> {
        if !self.parsed.contains_key(path) {
            let files = self.files.get(path).cloned().unwrap_or_default();
            // The fragments the index followed; only their invocations are dropped below.
            let fragments: BTreeSet<PathBuf> = files
                .iter()
                .filter(|file| file.included)
                .map(|file| file.path.clone())
                .collect();
            let followed = |item: &Item, from: &Path| {
                mod_source::modules::literal_include(item, false).is_some_and(|(relative, _)| {
                    fragments.contains(&from.parent().unwrap_or(from).join(relative.value()))
                })
            };
            // An inline module can contribute its own items even when the index also lists
            // separate fragments for it. These files are not alternative `mod` declarations.
            let mut items = if files.iter().all(|file| file.included) {
                let (parent, name) = path
                    .rsplit_once("::")
                    .ok_or_else(|| format!("no module {path}"))?;
                let parent = self.module(parent)?;
                let mut declarations = crate::source::inline_module_items(&parent.items, name);
                let items = declarations
                    .next()
                    .ok_or_else(|| format!("no module {path}"))?;
                if declarations.next().is_some() {
                    return Err(format!("multiple inline declarations for module {path}"));
                }
                items.to_vec()
            } else {
                Vec::new()
            };
            let inline_from: Vec<PathBuf> = match path.rsplit_once("::") {
                Some((parent, _)) if files.iter().all(|file| file.included) => self
                    .files
                    .get(parent)
                    .map(|parents| parents.iter().map(|file| file.path.clone()).collect())
                    .unwrap_or_default(),
                _ => Vec::new(),
            };
            items.retain(|item| !inline_from.iter().any(|from| followed(item, from)));
            for file in files {
                let text = self.read(&file.path)?;
                let mut parsed = syn::parse_file(&text).map_err(|e| {
                    let why = format!("{}: {e}", file.path.display());
                    if self.hashes.contains_key(&file.path) {
                        self.stale.get_or_insert_with(|| why.clone());
                    }
                    why
                })?;
                parsed.items.retain(|item| !followed(item, &file.path));
                if file.included && !file.cfg.is_empty() {
                    let attrs = file
                        .cfg
                        .iter()
                        .map(|condition| {
                            let predicate: proc_macro2::TokenStream =
                                condition.parse().map_err(|e| {
                                    format!(
                                        "{}: invalid source fragment cfg `{condition}`: {e}",
                                        file.path.display()
                                    )
                                })?;
                            Ok(syn::parse_quote!(#[cfg(#predicate)]))
                        })
                        .collect::<Result<Vec<syn::Attribute>, String>>()?;
                    if !attrs.is_empty() {
                        for item in &mut parsed.items {
                            gate_fragment_item(item, &attrs);
                        }
                    }
                }
                items.extend(parsed.items);
            }
            // The followed invocations' declarations are already in `items`; any other macro,
            // a shadowing `include!` included, stays for the opaque-macro handling.

            if let Some(cfg) = self.cfg(path) {
                // Configure the original scope before mirroring it. Copying the attributes as
                // written would let the game crate's features change its lower crates' imports.
                items.retain_mut(|item| {
                    let attrs = match item {
                        Item::Const(item) => &mut item.attrs,
                        Item::Enum(item) => &mut item.attrs,
                        Item::ExternCrate(item) => &mut item.attrs,
                        Item::Fn(item) => &mut item.attrs,
                        Item::Impl(item) => &mut item.attrs,
                        Item::Macro(item) => &mut item.attrs,
                        Item::Mod(item) => &mut item.attrs,
                        Item::Static(item) => &mut item.attrs,
                        Item::Struct(item) => &mut item.attrs,
                        Item::Trait(item) => &mut item.attrs,
                        Item::Type(item) => &mut item.attrs,
                        Item::Union(item) => &mut item.attrs,
                        Item::Use(item) => &mut item.attrs,
                        _ => return true,
                    };
                    let Some(active) =
                        mod_source::modules::configured_attrs(attrs, |atom| cfg.contains(atom))
                    else {
                        return false;
                    };
                    *attrs = active;
                    true
                });
            }
            self.parsed.insert(path.to_owned(), Module { items });
        }
        Ok(&self.parsed[path])
    }

    /// The `macro_rules!` items, not exported, that the modules on `path` define, outermost
    /// first: the macros a function in `path` may use by their bare names.
    fn path_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let segments: Vec<&str> = path.split("::").collect();
        let mut out = Vec::new();
        for end in 1..=segments.len() {
            out.extend(self.scope_macros(&segments[..end].join("::")));
        }
        out
    }

    /// The macros module `path` itself defines and those its `#[macro_use] mod` children do,
    /// which textual scope carries on to the rest of `path`.
    fn scope_macros(&mut self, path: &str) -> Vec<syn::ItemMacro> {
        let Ok(module) = self.module(path) else {
            return Vec::new();
        };
        let items = module.items.clone();
        let mut out = module_macros(&items);
        for item in &items {
            if let Item::Mod(child) = item
                && child.attrs.iter().any(|a| a.path().is_ident("macro_use"))
            {
                out.extend(self.scope_macros(&format!("{path}::{}", child.ident)));
            }
        }
        out
    }

    /// Whether `path` names a module of a loaded crate.
    fn is_module(&mut self, path: &str) -> bool {
        self.files.contains_key(path) || self.module(path).is_ok()
    }
}

/// Attach the enclosing include gates where the mirror reads item attributes. Inline
/// children can also be mirrored directly, without emitting the surrounding `mod` declaration.
fn gate_fragment_item(item: &mut Item, gates: &[syn::Attribute]) {
    let attrs = match item {
        Item::Const(item) => &mut item.attrs,
        Item::Enum(item) => &mut item.attrs,
        Item::ExternCrate(item) => &mut item.attrs,
        Item::Fn(item) => &mut item.attrs,
        Item::ForeignMod(item) => &mut item.attrs,
        Item::Impl(item) => &mut item.attrs,
        Item::Macro(item) => &mut item.attrs,
        Item::Mod(item) => {
            if let Some((_, inner)) = &mut item.content {
                for item in inner {
                    gate_fragment_item(item, gates);
                }
            }
            &mut item.attrs
        }
        Item::Static(item) => &mut item.attrs,
        Item::Struct(item) => &mut item.attrs,
        Item::Trait(item) => &mut item.attrs,
        Item::TraitAlias(item) => &mut item.attrs,
        Item::Type(item) => &mut item.attrs,
        Item::Union(item) => &mut item.attrs,
        Item::Use(item) => &mut item.attrs,
        _ => return,
    };
    attrs.extend_from_slice(gates);
}

/// The names `text` defines with `macro_rules!`, into `local` or (with `#[macro_export]`)
/// `exported`.
fn macro_names(text: &str, local: &mut BTreeSet<String>, exported: &mut BTreeSet<String>) {
    for (at, _) in text.match_indices("macro_rules!") {
        let name: String = text[at + "macro_rules!".len()..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if text[..at].trim_end().ends_with("#[macro_export]") {
            exported.insert(name);
        } else {
            local.insert(name);
        }
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    out
}

/// What a copied body may not contain.
#[derive(Default)]
struct BodyCheck<'a> {
    local_macros: Option<&'a BTreeSet<String>>,
    refusal: Option<String>,
}

impl<'ast> Visit<'ast> for BodyCheck<'_> {
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(last) = mac.path.segments.last() {
            let name = last.ident.to_string();
            if mac.path.segments.len() == 1 && PLACE_MACROS.contains(&name.as_str()) {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, which means something else outside the game's source"
                ));
            } else if mac.path.segments.len() == 1 && name == "thread_local" {
                self.refusal.get_or_insert(
                    "its body declares a thread-local: a copy would have a second one".into(),
                );
            } else if mac.path.segments.len() == 1
                && self.local_macros.is_some_and(|local| local.contains(&name))
            {
                self.refusal.get_or_insert(format!(
                    "its body uses `{name}!`, a macro defined outside its module's path"
                ));
            }
        }
        syn::visit::visit_macro(self, mac);
    }
    fn visit_item(&mut self, item: &'ast Item) {
        match item {
            Item::Static(_) => {
                self.refusal.get_or_insert(
                    "its body declares a `static`: a copy would have a second one".into(),
                );
            }
            Item::Impl(_) => {
                self.refusal.get_or_insert(
                    "its body implements a trait or type: a copy would implement it twice".into(),
                );
            }
            Item::Macro(mac) if mac.ident.is_some() => {
                self.refusal.get_or_insert(
                    "its body defines a `macro_rules!`, which moves with different hygiene".into(),
                );
            }
            _ => {}
        }
        syn::visit::visit_item(self, item);
    }
}

/// The function a target names, found in its module.
pub(crate) struct Found {
    pub(crate) module: String,
    /// The `impl` it is in: none for a free function. Only its header is read.
    pub(crate) owner: Option<syn::ItemImpl>,
    pub(crate) function: syn::ImplItemFn,
}

/// A target's path as `[[native.mixin]]` rows and bridges write it: what comes before its last
/// segment, its name, and for a trait method (`module::<Type as Trait>::name`) the module, the
/// type without generic arguments or spaces, and the trait's last segment. The one parse of a
/// target: [`find`] and the function index's keys both read it.
pub(crate) struct TargetPath<'a> {
    pub(crate) front: &'a str,
    pub(crate) name: &'a str,
    pub(crate) qualified: Option<(&'a str, String, String)>,
}

pub(crate) fn target_path(target: &str) -> Option<TargetPath<'_>> {
    let (front, name) = target.rsplit_once("::")?;
    let qualified = front.split_once("::<").and_then(|(module, rest)| {
        let (ty, tr) = rest.strip_suffix('>')?.split_once(" as ")?;
        let tr = crate::source::plain_path(tr);
        let last = tr.rsplit("::").next().unwrap_or_default().to_owned();
        Some((module, crate::source::plain_path(ty), last))
    });
    Some(TargetPath {
        front,
        name,
        qualified,
    })
}

/// The function a bridge's target names, looked up by name in its module. A mixin's target is
/// read at its indexed span instead ([`crate::functions::FunctionIndex::found`]).
pub(crate) fn find(source: &mut Source, target: &str) -> Result<Found, (&'static str, String)> {
    let TargetPath {
        front,
        name,
        qualified,
    } = target_path(target).ok_or(("E_MIXIN_TARGET_MISSING", "not a function path".to_owned()))?;
    let krate = front.split("::").next().unwrap_or_default();
    if !source.load(krate).map_err(|why| ("E_MIXIN_SOURCE", why))? {
        return Err((
            "E_MIXIN_TARGET_MISSING",
            format!("no crate `{krate}` in the game's source"),
        ));
    }
    // A trait method is written `module::<Type as Trait>::name`.
    let (module, owner, trait_name) = match qualified {
        Some((module, ty, last)) => (module.to_owned(), ty, Some(last)),
        None => {
            // The longest prefix that is a module; what follows it is the `impl`'s type.
            let segments: Vec<&str> = front.split("::").collect();
            let mut cut = segments.len();
            while cut > 0 && !source.is_module(&segments[..cut].join("::")) {
                cut -= 1;
            }
            (
                segments[..cut].join("::"),
                crate::source::plain_path(&segments[cut..].join("::")),
                None,
            )
        }
    };
    // Below the game crate, a method is named by the module that declares its type, so its
    // `impl` may be in any module of the crate.
    let by_type = source.layout.domain(krate).is_none() && !owner.is_empty();
    let modules: Vec<String> = if by_type {
        source
            .files
            .keys()
            .filter(|listed| {
                listed.as_str() == krate
                    || listed
                        .strip_prefix(krate)
                        .is_some_and(|rest| rest.starts_with("::"))
            })
            .cloned()
            .collect()
    } else {
        vec![module.clone()]
    };
    let mut found = Vec::new();
    for at in &modules {
        let items = source
            .module(at)
            .map_err(|why| ("E_MIXIN_TARGET_MISSING", why))?
            .items
            .clone();
        found.extend(named(&items, at, &owner, trait_name.as_deref(), name));
    }
    // Where the type is declared: the module a target names a method below the game crate by.
    let mut misnamed = None;
    if by_type {
        let types = source
            .owner_types(krate)
            .map_err(|why| ("E_MIXIN_SOURCE", why))?;
        found.retain(|candidate| {
            let block = candidate.owner.as_ref().expect("a method has its impl");
            let declaring = crate::replace::owner_module(types, &candidate.module, &block.self_ty);
            if declaring == module {
                return true;
            }
            if candidate.module == module {
                misnamed = Some(match &trait_name {
                    Some(trait_name) => format!("{declaring}::<{owner} as {trait_name}>::{name}"),
                    None => format!("{declaring}::{owner}::{name}"),
                });
            }
            false
        });
    }
    match found.len() {
        0 if misnamed.is_some() => Err((
            "E_MIXIN_TARGET_MISSING",
            format!(
                "`{target}` names the module of the file its `impl` is in; a method below the game crate is named by the module that declares its type: `{}`",
                misnamed.expect("named")
            ),
        )),
        0 => Err((
            "E_MIXIN_TARGET_MISSING",
            format!(
                "no function `{name}` in {module}{}",
                if owner.is_empty() {
                    String::new()
                } else {
                    format!(" on {owner}")
                }
            ),
        )),
        1 => Ok(found.pop().expect("one")),
        _ => Err((
            "E_MIXIN_TARGET_AMBIGUOUS",
            "several functions have this path (a cfg-gated pair, for one)".into(),
        )),
    }
}

/// The functions among `items`, module `module`'s, that a target naming `name` on `owner` (a
/// free function when empty) through `trait_name` finds.
fn named(
    items: &[Item],
    module: &str,
    owner: &str,
    trait_name: Option<&str>,
    name: &str,
) -> Vec<Found> {
    let mut found = Vec::new();
    for item in items {
        match item {
            Item::Fn(function) if owner.is_empty() && function.sig.ident == name => {
                found.push(Found {
                    module: module.to_owned(),
                    owner: None,
                    function: syn::ImplItemFn {
                        attrs: function.attrs.clone(),
                        vis: function.vis.clone(),
                        defaultness: None,
                        sig: function.sig.clone(),
                        block: (*function.block).clone(),
                    },
                })
            }
            Item::Impl(block)
                if !owner.is_empty()
                    && block
                        .trait_
                        .as_ref()
                        .map(|(_, path, _)| {
                            path.segments
                                .last()
                                .map(|s| s.ident.to_string())
                                .unwrap_or_default()
                        })
                        .as_deref()
                        == trait_name
                    && crate::source::plain_path(&block.self_ty.to_token_stream().to_string())
                        == owner =>
            {
                for member in &block.items {
                    if let ImplItem::Fn(function) = member
                        && function.sig.ident == name
                    {
                        found.push(Found {
                            module: module.to_owned(),
                            owner: Some(block.clone()),
                            function: function.clone(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    found
}

/// Whether `function` can be copied and installed through its function's replacement slot: the
/// verdict the function index records for it.
#[cfg(test)]
fn replaceable(found: &Found, source: &mut Source, loop_generic: bool) -> Result<(), String> {
    replaceable_with_binding(found, source, loop_generic).map(|_| ())
}

/// Whether `function` can be copied and installed through its function's replacement slot,
/// returning resolved owner/trait evidence for trait-impl methods.
pub(crate) fn replaceable_with_binding(
    found: &Found,
    source: &mut Source,
    loop_generic: bool,
) -> Result<Option<crate::replace::TraitSlotEvidence>, String> {
    // A crate-local macro defined in a module on the function's path moves with the copy (the
    // mirror repeats it); one defined elsewhere would not resolve.
    if !source.cache.on_path.contains_key(&found.module) {
        let names = source
            .path_macros(&found.module)
            .iter()
            .filter_map(|mac| mac.ident.as_ref().map(ToString::to_string))
            .collect();
        source.cache.on_path.insert(found.module.clone(), names);
    }
    let local_macros: BTreeSet<String> = source
        .local_macros
        .difference(&source.cache.on_path[&found.module])
        .cloned()
        .collect();
    let local_macros = &local_macros;
    let sig = &found.function.sig;
    // A published loop's function is generic over its slot alone: each mod build copies it once,
    // for the one slot type its entry's copy calls it with. Its other rules are a function's.
    use crate::replace::CandidateExclusion;
    match crate::replace::candidate_exclusion(
        &found.function.attrs,
        sig,
        found.owner.as_ref(),
        loop_generic,
    ) {
        Some(CandidateExclusion::Signature(_)) => {
            return Err("it is not replaceable: `const`, `async`, `unsafe`, \
                 `self` by value, `impl Trait` or `!` in its signature, `#[inline(always)]`, or \
                 compiled only under a `cfg`"
                .into());
        }
        Some(CandidateExclusion::GenericImpl) => {
            return Err("it is generic over types or takes `impl Trait`: one copy can't stand for every instance".into());
        }
        None => {}
    }
    let mut trait_binding = None;
    if let Some(block) = &found.owner {
        let krate = found
            .module
            .split("::")
            .next()
            .unwrap_or_default()
            .to_owned();
        if let Some((_, path, _)) = &block.trait_ {
            struct Associated(bool);
            impl<'ast> Visit<'ast> for Associated {
                fn visit_path(&mut self, path: &'ast syn::Path) {
                    self.0 |= path.segments.len() > 1 && path.segments[0].ident == "Self";
                    syn::visit::visit_path(self, path);
                }
            }
            let mut associated = Associated(false);
            associated.visit_signature(&crate::replace::qualified(sig, path));
            if associated.0 {
                return Err("its signature names a trait-associated constant through Self".into());
            }
            if source.cache.traits.is_none() {
                source.cache.traits = Some(crate::replace::WorkspaceTraits::load(&source.root)?);
            }
            source.owner_types(&krate)?;
            let workspace = source.cache.traits.as_ref().expect("read above");
            trait_binding = Some(crate::replace::resolve_trait_binding(
                &found.module,
                block,
                &sig.ident,
                &source.owners[&krate],
                workspace,
            )?);
        } else {
            if source.cache.traits.is_none() {
                source.cache.traits = Some(crate::replace::WorkspaceTraits::load(&source.root)?);
            }
            source.owner_types(&krate)?;
            let workspace = source.cache.traits.as_ref().expect("read above");
            if let Some(reason) = crate::replace::module_widened_glob_shadow_refusal(
                &found.module,
                &source.owners[&krate],
                workspace,
            ) {
                return Err(format!(
                    "its `impl` has no replacement installer ({reason})"
                ));
            }
        }
    }
    if let Some(block) = &found.owner
        && block.attrs.iter().any(|attr| attr.path().is_ident("cfg"))
    {
        return Err("its `impl` is compiled only under a `cfg`".into());
    }
    let mut check = BodyCheck {
        local_macros: Some(local_macros),
        refusal: None,
    };
    check.visit_block(&found.function.block);
    if let Some(why) = check.refusal {
        return Err(why);
    }
    reexports::check_paths(source, found)?;
    Ok(trait_binding)
}

/// An item's own definition, before filtering for re-export.
fn definition(item: &Item) -> Option<(&syn::Ident, &[syn::Attribute])> {
    let (ident, attrs) = match item {
        Item::Fn(item) => (&item.sig.ident, &item.attrs),
        Item::Struct(item) => (&item.ident, &item.attrs),
        Item::Enum(item) => (&item.ident, &item.attrs),
        Item::Union(item) => (&item.ident, &item.attrs),
        Item::Const(item) => (&item.ident, &item.attrs),
        Item::Static(item) => (&item.ident, &item.attrs),
        Item::Type(item) => (&item.ident, &item.attrs),
        Item::Trait(item) => (&item.ident, &item.attrs),
        Item::Mod(item) => (&item.ident, &item.attrs),
        _ => return None,
    };
    Some((ident, attrs))
}

/// A module's own `macro_rules!` items that it does not export.
fn module_macros(items: &[Item]) -> Vec<syn::ItemMacro> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Macro(mac)
                if mac.ident.is_some()
                    && mac.mac.path.is_ident("macro_rules")
                    && !mac.attrs.iter().any(|a| a.path().is_ident("macro_export")) =>
            {
                Some(mac.clone())
            }
            _ => None,
        })
        .collect()
}

/// The module's `use` items, as written.
fn uses(items: &[Item]) -> Vec<&syn::ItemUse> {
    items
        .iter()
        .filter_map(|item| match item {
            Item::Use(item) => Some(item),
            _ => None,
        })
        .collect()
}

/// A copied function's attributes: its docs, lint levels and cfg, nothing that changes codegen
/// or the point machinery.
fn kept(attrs: &[syn::Attribute]) -> Vec<syn::Attribute> {
    attrs
        .iter()
        .filter(|attr| {
            ["doc", "allow", "cfg", "expect", "warn"]
                .iter()
                .any(|name| attr.path().is_ident(name))
        })
        .cloned()
        .collect()
}

/// A tree of mirror modules, keyed by segment.
#[derive(Default)]
struct Tree {
    children: BTreeMap<String, Tree>,
    copies: Vec<proc_macro2::TokenStream>,
    /// Names the copies define, which the mirror must not also re-export.
    own: BTreeSet<String>,
}

/// The patch crate for `targets` (as `[[native.mixin]]` rows write them), from the pinned game source
/// at `root`, which `index` (the build's `functions.json`) describes. `krate` is the patch crate's
/// name. A target's file and verdict come from the index, and its text is checked against it
/// before it is copied: the source is never walked. Every target that can't be copied is refused
/// by name; the crate is returned only when none is.
pub fn generate(
    root: &Path,
    krate: &str,
    targets: &[String],
    extras: &Extras,
    index: &crate::functions::FunctionIndex,
) -> Result<PatchCrate, Vec<Refusal>> {
    let layout = SourceLayout::read(root).map_err(|why| {
        vec![Refusal {
            target: root.display().to_string(),
            code: "E_MIXIN_SOURCE",
            why,
        }]
    })?;
    generate_with_layout(root, layout, krate, targets, extras, index)
}

/// Copy patches from a main-layout archive selected by its verified SDK identity.
pub fn generate_for_main_sdk(
    root: &Path,
    krate: &str,
    targets: &[String],
    extras: &Extras,
    index: &crate::functions::FunctionIndex,
) -> Result<PatchCrate, Vec<Refusal>> {
    generate_with_layout(
        root,
        crate::source::LEGACY_LAYOUT,
        krate,
        targets,
        extras,
        index,
    )
}

pub(crate) fn generate_with_layout(
    root: &Path,
    layout: SourceLayout,
    krate: &str,
    targets: &[String],
    extras: &Extras,
    index: &crate::functions::FunctionIndex,
) -> Result<PatchCrate, Vec<Refusal>> {
    let refuse = |target: &str, code, why: String| Refusal {
        target: target.to_owned(),
        code,
        why,
    };
    let mut source = match Source::indexed(root, layout, index, extras.lower_crate_cfg.as_ref()) {
        Ok(source) => source,
        Err(why) => {
            return Err(targets
                .iter()
                .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                .collect());
        }
    };
    let mut refusals = Vec::new();
    let mut trees: BTreeMap<String, Tree> = BTreeMap::new();
    let mut patched = Vec::new();
    let mut chosen = BTreeMap::new();
    for target in targets {
        chosen.entry(index.target_key(target)).or_insert(target);
    }
    let unique: BTreeSet<&String> = chosen.into_values().collect();
    // The loops whose generic function a row targets: their entries' copies call the copy.
    let generic_copied: BTreeSet<&str> = extras
        .loops
        .iter()
        .filter(|l| {
            unique.iter().any(|target| {
                index.target_key(target) == index.target_key(&l.published.generic_target())
            })
        })
        .map(|l| l.published.name.as_str())
        .collect();
    for target in unique {
        let loop_generic = extras
            .loops
            .iter()
            .any(|l| index.target_key(&l.published.generic_target()) == index.target_key(target));
        if let Err((code, why)) = index.check_with_layout(root, target, layout) {
            refusals.push(refuse(target, code, why));
            continue;
        }
        let found = match index.found(&mut source, target) {
            Ok(found) => found,
            Err((code, why)) => {
                refusals.push(refuse(target, code, why));
                continue;
            }
        };
        let name = found.function.sig.ident.clone();
        let source_crate = found.module.split("::").next().expect("a crate").to_owned();
        if layout.domain(&source_crate).is_none()
            && extras.lower_crate_cfg.is_some()
            && source.cfg(&source_crate).is_none()
        {
            refusals.push(refuse(
                target,
                "E_MIXIN_SOURCE",
                format!("the SDK has no build cfg for {source_crate}"),
            ));
            continue;
        }
        let patch_name = patch_name(krate, &source_crate, layout);
        let mut node = trees.entry(source_crate).or_default();
        for segment in found.module.split("::").skip(1) {
            node = node.children.entry(segment.to_owned()).or_default();
        }
        let attrs = kept(&found.function.attrs);
        let mut sig = found.function.sig.clone();
        // A loop's function is copied for the one slot type its entry's copy runs it with:
        // horizon-mixins patches only non-generic functions.
        let filled_block = extras
            .loops
            .iter()
            .find(|l| {
                loop_generic
                    && index.target_key(&l.published.generic_target()) == index.target_key(target)
            })
            .map(|slots| fill_slot(&mut sig, &found.function.block, slots));
        let entry_of = extras
            .loops
            .iter()
            .find(|l| index.target_key(&l.published.entry) == index.target_key(target));
        let loop_block = match entry_of
            .map(|slots| {
                loop_entry(
                    slots,
                    source
                        .libraries
                        .get(layout.runtime.key)
                        .map_or(layout.runtime.key, String::as_str),
                    layout.is_split(),
                    found
                        .owner
                        .as_ref()
                        .filter(|_| generic_copied.contains(slots.published.name.as_str())),
                )
            })
            .transpose()
        {
            Ok(entry) => entry.map(|(items, block)| {
                node.copies.push(items);
                block
            }),
            Err(refusal) => {
                refusals.push(refuse(target, "E_LOOP_HELPER", refusal));
                continue;
            }
        };
        let block = loop_block
            .as_ref()
            .or(filled_block.as_ref())
            .unwrap_or(&found.function.block);
        let install = format_ident!("__fnr_install_{}", name);
        let constructor = format_ident!("__fnr_{}", name);
        let module_path = found.module.split("::").skip(1).collect::<Vec<_>>();
        let (copy, id, install) = match &found.owner {
            None => {
                node.own.insert(name.to_string());
                node.own.insert(install.to_string());
                // The extern a lower crate is named by is its library's name (`mod_render`), not
                // its folder's (`sunrise_mod_render`).
                let original: syn::Path =
                    syn::parse_str(&original_path(&source.libraries, &found.module))
                        .expect("a module path");
                (
                    quote! {
                        #(#attrs)*
                        pub #sig #block

                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            #original::#constructor(#name)
                        }
                    },
                    format!("{patch_name}::{}::{name}", module_path.join("::")),
                    install,
                )
            }
            Some(owner) => {
                let self_ty = &owner.self_ty;
                let owner_name = patch_owner_name(self_ty);
                // A trait method's copy is renamed in its helper trait: with the game's trait in
                // scope too, `x.name()` would be ambiguous.
                let source_trait = owner.trait_.as_ref().map(|(_, path, _)| path.clone());
                let source_trait_name = source_trait
                    .as_ref()
                    .and_then(|path| path.segments.last())
                    .map(|segment| segment.ident.to_string());
                let (trait_name, install, method, constructor) = match &source_trait_name {
                    Some(tr) => (
                        format_ident!("__Fnr_{}_as_{}_{}", owner_name, tr, name),
                        format_ident!("__fnr_install_{}_as_{}_{}", owner_name, tr, name),
                        format_ident!("__fnr_{}", name),
                        format_ident!("__fnr_as_{}_{}", tr, name),
                    ),
                    None => (
                        format_ident!("__Fnr_{}_{}", owner_name, name),
                        format_ident!("__fnr_install_{}_{}", owner_name, name),
                        name.clone(),
                        constructor.clone(),
                    ),
                };
                sig.ident = method.clone();
                let mut block = block.clone();
                if let Some(path) = &source_trait {
                    qualify_self(&mut block, owner, self_ty, path);
                    qualify_self_sig(&mut sig, owner, self_ty, path);
                }
                node.own.insert(trait_name.to_string());
                node.own.insert(install.to_string());
                let generics = &owner.generics;
                let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
                // A trait declaration takes no patterns: its parameters are renamed.
                let mut declared = sig.clone();
                for (index, input) in declared.inputs.iter_mut().enumerate() {
                    if let syn::FnArg::Typed(typed) = input {
                        let fresh = format_ident!("__a{index}");
                        *typed.pat = syn::parse_quote!(#fresh);
                    }
                }
                sig.constness = None;
                // `Game<'_>` and `Game<'g>` alike are named with elided lifetimes in expressions.
                let mut elided = (**self_ty).clone();
                struct Elide;
                impl syn::visit_mut::VisitMut for Elide {
                    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
                        if lifetime.ident != "static" {
                            *lifetime = syn::parse_quote!('_);
                        }
                    }
                }
                syn::visit_mut::VisitMut::visit_type_mut(&mut Elide, &mut elided);
                let trait_args: Vec<_> = generics.lifetimes().map(|_| quote!('_)).collect();
                let installer = (!loop_generic).then(|| {
                    let trait_ref = if trait_args.is_empty() {
                        quote!(#trait_name)
                    } else {
                        quote!(#trait_name<#(#trait_args),*>)
                    };
                    quote! {
                        #[doc(hidden)]
                        pub fn #install() -> bool {
                            <#elided>::#constructor(<#elided as #trait_ref>::#method)
                        }
                    }
                });
                let id_type = plain_spaces(&elided.to_token_stream().to_string());
                let id_trait = plain_spaces(&{
                    let generics_args: Vec<_> =
                        generics.lifetimes().map(|l| l.lifetime.clone()).collect();
                    if generics_args.is_empty() {
                        quote!(#trait_name).to_string()
                    } else {
                        quote!(#trait_name<#(#generics_args),*>).to_string()
                    }
                });
                (
                    quote! {
                        #[doc(hidden)]
                        #[allow(non_camel_case_types)]
                        pub trait #trait_name #impl_generics #where_clause {
                            #declared;
                        }
                        impl #impl_generics #trait_name #type_generics for #self_ty #where_clause {
                            #(#attrs)*
                            #sig #block
                        }

                        #installer
                    },
                    format!(
                        "{patch_name}::{}::<{id_type} as {id_trait}>::{method}",
                        module_path.join("::")
                    ),
                    install,
                )
            }
        };
        node.copies.push(copy);
        patched.push(Patched {
            target: target.clone(),
            id,
            install: (!loop_generic)
                .then(|| format!("{patch_name}::{}::{install}()", module_path.join("::"))),
        });
    }
    for module in &extras.modules {
        match support_module(&source, module) {
            Ok((path, tokens)) => {
                let mut node = trees
                    .entry(source.extras_owner(&path).to_owned())
                    .or_default();
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(module.name.clone());
                node.copies.push(tokens);
            }
            Err((code, why)) => refusals.push(refuse(&module.id, code, why)),
        }
    }
    #[derive(Default)]
    struct Bridges {
        traits: Vec<proc_macro2::TokenStream>,
        names: Vec<syn::Ident>,
    }
    let mut bridge_imports = BTreeMap::<String, Bridges>::new();
    for bridge in &extras.bridges {
        match bridge_trait(&mut source, bridge) {
            Ok((path, trait_name, tokens)) => {
                let (declaring, module) = path.split_once("::").unwrap_or((&path, ""));
                let owner = source.extras_owner(&path);
                let mut node = trees.entry(owner.to_owned()).or_default();
                for segment in path.split("::").skip(1) {
                    node = node.children.entry(segment.to_owned()).or_default();
                }
                node.own.insert(trait_name.to_string());
                node.copies.push(tokens);
                let mirror: syn::Path = if owner != declaring {
                    syn::parse_str(&path).expect("a module path")
                } else if module.is_empty() {
                    syn::parse_quote!(crate)
                } else {
                    syn::parse_str(&format!("crate::{module}")).expect("a module path")
                };
                let imports = bridge_imports.entry(owner.to_owned()).or_default();
                imports.traits.push(quote!(pub use #mirror::#trait_name;));
                imports.names.push(trait_name);
            }
            Err((code, why)) => refusals.push(refuse(&bridge.id, code, why)),
        }
    }
    if let Some(why) = source.stale.take() {
        return Err(targets
            .iter()
            .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
            .collect());
    }
    if !refusals.is_empty() {
        return Err(refusals);
    }
    let mut libs = Vec::new();
    let mut owners = BTreeMap::new();
    for (source_crate, tree) in &trees {
        let imports = bridge_imports.remove(source_crate).unwrap_or_default();
        let bridges = imports.names.as_slice();
        let bridge_traits = &imports.traits;
        let used = layout.domain(source_crate).is_none().then(|| {
            let mut used = extras.idents.clone();
            idents_in(&tree_tokens(tree), &mut used);
            used
        });
        let narrowing = used
            .as_ref()
            .map(|used| Narrowing::new(used, &extras.linked));
        let body = match mirror(&mut source, source_crate, tree, bridges, narrowing.as_ref()) {
            Ok(body) => body,
            Err(why) => {
                return Err(targets
                    .iter()
                    .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
                    .collect());
            }
        };
        let bridge_module = (!bridges.is_empty()).then(|| {
            quote! {
                /// The bridges' traits, which every mirror module imports.
                #[doc(hidden)]
                pub mod __fnr_bridges {
                    #(#bridge_traits)*
                }
            }
        });
        let lib = quote! {
            //! Generated by the launcher: game functions copied out of the pinned source for the
            //! installed mods' `[[native.mixin]]` rows, patched by horizon-mixins. Not for editing.
            #![allow(unused_imports, unused_macros, unused_mut, unused_variables, dead_code, unused_braces, unused_parens)]
            #![allow(clippy::all)]
            #body
            #bridge_module
        };
        let name = patch_name(krate, source_crate, layout);
        owners.insert(name.clone(), source_crate.clone());
        libs.push((name, prettyplease_or_tokens(lib)));
    }
    // A mirror's module read from a file that is not the build's.
    if let Some(why) = source.stale.take() {
        return Err(targets
            .iter()
            .map(|t| refuse(t, "E_MIXIN_SOURCE", why.clone()))
            .collect());
    }
    Ok(PatchCrate {
        libs,
        owners,
        patched,
    })
}

/// The loop function's signature `sig` with its slot type parameter replaced by the slots `slots`
/// fills it with (`()` with none), and its body `block` with the same replacement: the type in
/// type position, and `<Slots>::NAME` for a path through it (`S::RECHECK`).
fn fill_slot(sig: &mut syn::Signature, block: &syn::Block, slots: &LoopSlots) -> syn::Block {
    let mut block = block.clone();
    let Some(param) = sig.generics.type_params().next().map(|p| p.ident.clone()) else {
        return block;
    };
    sig.generics.params = sig
        .generics
        .params
        .iter()
        .filter(|p| !matches!(p, syn::GenericParam::Type(_)))
        .cloned()
        .collect();
    let filled: syn::Type = if slots.slots.is_empty() {
        syn::parse_quote!(())
    } else {
        let ty = format_ident!("__FnrLoopSlots_{}", loop_tag(&slots.published.name));
        syn::parse_quote!(#ty)
    };
    struct Fill(syn::Ident, syn::Type);
    impl Fill {
        /// `S::rest` as `<Slots>::rest`; `None` for any other path.
        fn through(&self, qself: &Option<syn::QSelf>, path: &syn::Path) -> Option<syn::TypePath> {
            let first = path.segments.first()?;
            if qself.is_some()
                || path.leading_colon.is_some()
                || path.segments.len() < 2
                || first.ident != self.0
                || !first.arguments.is_none()
            {
                return None;
            }
            let ty = &self.1;
            let rest = path.segments.iter().skip(1);
            Some(syn::parse_quote!(<#ty>::#(#rest)::*))
        }
    }
    impl syn::visit_mut::VisitMut for Fill {
        fn visit_type_mut(&mut self, ty: &mut syn::Type) {
            if matches!(ty, syn::Type::Path(p) if p.qself.is_none() && p.path.is_ident(&self.0)) {
                *ty = self.1.clone();
                return;
            }
            if let syn::Type::Path(p) = ty
                && let Some(filled) = self.through(&p.qself, &p.path)
            {
                *p = filled;
            }
            syn::visit_mut::visit_type_mut(self, ty);
        }
        fn visit_expr_path_mut(&mut self, expr: &mut syn::ExprPath) {
            if let Some(filled) = self.through(&expr.qself, &expr.path) {
                expr.qself = filled.qself;
                expr.path = filled.path;
            }
            syn::visit_mut::visit_expr_path_mut(self, expr);
        }
    }
    let mut fill = Fill(param, filled);
    syn::visit_mut::VisitMut::visit_signature_mut(&mut fill, sig);
    syn::visit_mut::VisitMut::visit_block_mut(&mut fill, &mut block);
    block
}

/// A published loop's name as an identifier part (`block_entities_tick`).
fn loop_tag(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn patch_owner_name(self_ty: &syn::Type) -> String {
    crate::source::plain_path(&self_ty.to_token_stream().to_string()).replace("::", "_")
}

/// A loop entry's copy: each slot's helper as a module beside it, marked with its mod for the crash
/// report, and its body, which runs the loop with the slots in this game's state for it
/// (`Game::run_published_loop`). Refused when a helper is not Rust items.
fn loop_entry(
    slots: &LoopSlots,
    runtime: &str,
    split: bool,
    copied_owner: Option<&syn::ItemImpl>,
) -> Result<(proc_macro2::TokenStream, syn::Block), String> {
    let published = &slots.published;
    let tag = loop_tag(&published.name);
    let hook = format!("loop {}", published.name);
    let runtime = format_ident!("{runtime}");
    let loops = if split {
        quote!(::sunrise_gameplay_common::game::published_loops)
    } else {
        quote!(::#runtime::game::published_loops)
    };
    let mut items = Vec::new();
    let mut nested = quote!(());
    for (index, slot) in slots.slots.iter().enumerate().rev() {
        let helper = syn::parse_file(&slot.helper)
            .map_err(|e| format!("{}:{}: its helper: {e}", slot.mod_id, slot.id))?
            .items;
        let module = format_ident!("__fnr_loop_{}_{}", index, tag);
        let marker = format_ident!("__FnrLoopMod_{}_{}", index, tag);
        let with = syn::parse_str::<syn::Ident>(&slot.with)
            .map_err(|e| format!("{}:{}: with: {e}", slot.mod_id, slot.id))?;
        let id = &slot.mod_id;
        let part = format!("{}::{}", slot.mod_id, slot.with);
        items.push(quote! {
            /// A `[[native.loop]]` helper, compiled where the loop's code is.
            #[doc(hidden)]
            #[allow(non_snake_case)]
            pub mod #module {
                use super::*;
                #(#helper)*
            }
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub struct #marker;
            impl #loops::LoopMod for #marker {
                const CALL: ::mod_api::content::crash::ModCall = ::mod_api::content::crash::ModCall {
                    id: #id,
                    hook: #hook,
                    part: Some(#part),
                };
            }
        });
        nested = quote!((#loops::Attributed<#marker, #module::#with>, #nested));
    }
    let slots_ty = format_ident!("__FnrLoopSlots_{}", tag);
    items.push(quote! {
        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        pub type #slots_ty = #nested;
    });
    let name = &published.name;
    let generic = format_ident!("{}", published.generic);
    // With rows on the loop's function, its patched copy (a method of its helper trait, beside this
    // one, for these slots) runs the loop; the game's own otherwise, and always for the run without
    // the slots (nested, or after a panic), which the copy's slot type can't take.
    let call = |game: proc_macro2::TokenStream, slot: proc_macro2::TokenStream| {
        if let Some(owner) = copied_owner {
            let owner_name = patch_owner_name(&owner.self_ty);
            let helper = format_ident!("__Fnr_{}_{}", owner_name, published.generic);
            quote!(#helper::#generic(#game, #slot))
        } else {
            quote!(#game.#generic(#slot))
        }
    };
    let block = if slots.slots.is_empty() {
        let run = call(quote!(self), quote!(&mut ()));
        syn::parse_quote!({ #run })
    } else {
        let filled = call(quote!(game), quote!(slots));
        let vanilla = quote!(game.#generic(&mut ()));
        let runner = if split {
            quote!(#loops::run_published_loop)
        } else {
            quote!(self.run_published_loop)
        };
        let receiver = split.then(|| quote!(self,));
        syn::parse_quote!({
            #runner(
                #receiver
                #name,
                |game, slots: &mut #slots_ty| #filled,
                |game| #vanilla,
            )
        })
    };
    Ok((quote!(#(#items)*), block))
}

/// Every copy's tokens in `tree` and its children.
fn tree_tokens(tree: &Tree) -> proc_macro2::TokenStream {
    let copies = &tree.copies;
    let children = tree.children.values().map(tree_tokens);
    quote!(#(#copies)* #(#children)*)
}

/// The identifiers in `tokens`, into `out`.
pub fn idents_in(tokens: &proc_macro2::TokenStream, out: &mut BTreeSet<String>) {
    for tree in tokens.clone() {
        match tree {
            proc_macro2::TokenTree::Ident(ident) => {
                out.insert(ident.to_string());
            }
            proc_macro2::TokenTree::Group(group) => idents_in(&group.stream(), out),
            _ => {}
        }
    }
}

/// The patch crate that mirrors `source_crate`: `krate` itself for the game crate, and
/// `krate_<crate>` for a lower one (`sunrise_patch_client_lib` for `sunrise_client_lib`).
fn patch_name(krate: &str, source_crate: &str, layout: SourceLayout) -> String {
    match source_crate {
        primary if primary == layout.runtime.key => krate.to_owned(),
        other => format!("{krate}_{}", other.trim_start_matches("sunrise_")),
    }
}

/// `Self::Item` for an associated type or constant of the trait `path` that `owner` implements,
/// rewritten `<Type as Trait>::Item`: in the helper trait `Self` has no such item.
struct QualifySelf {
    items: BTreeSet<String>,
    qualified: proc_macro2::TokenStream,
}

impl QualifySelf {
    fn new(owner: &syn::ItemImpl, self_ty: &syn::Type, path: &syn::Path) -> Self {
        let items = owner
            .items
            .iter()
            .filter_map(|item| match item {
                ImplItem::Type(item) => Some(item.ident.to_string()),
                ImplItem::Const(item) => Some(item.ident.to_string()),
                _ => None,
            })
            .collect();
        Self {
            items,
            qualified: quote!(<#self_ty as #path>),
        }
    }
    fn rewrite(&self, qself: &mut Option<syn::QSelf>, path: &mut syn::Path) {
        if qself.is_none()
            && path.segments.len() == 2
            && path.segments[0].ident == "Self"
            && self.items.contains(&path.segments[1].ident.to_string())
        {
            let item = &path.segments[1];
            let qualified = &self.qualified;
            let rewritten: syn::ExprPath =
                syn::parse2(quote!(#qualified::#item)).expect("a qualified path");
            *qself = rewritten.qself;
            *path = rewritten.path;
        }
    }
}

impl syn::visit_mut::VisitMut for QualifySelf {
    fn visit_expr_path_mut(&mut self, expr: &mut syn::ExprPath) {
        self.rewrite(&mut expr.qself, &mut expr.path);
        syn::visit_mut::visit_expr_path_mut(self, expr);
    }
    fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
        self.rewrite(&mut ty.qself, &mut ty.path);
        syn::visit_mut::visit_type_path_mut(self, ty);
    }
}

fn qualify_self(
    block: &mut syn::Block,
    owner: &syn::ItemImpl,
    self_ty: &syn::Type,
    path: &syn::Path,
) {
    syn::visit_mut::VisitMut::visit_block_mut(&mut QualifySelf::new(owner, self_ty, path), block);
}

fn qualify_self_sig(
    sig: &mut syn::Signature,
    owner: &syn::ItemImpl,
    self_ty: &syn::Type,
    path: &syn::Path,
) {
    syn::visit_mut::VisitMut::visit_signature_mut(&mut QualifySelf::new(owner, self_ty, path), sig);
}

/// An invoker's signature moves into a trait of its own, where `Self::Item` names nothing: an
/// item of the trait `owner` implements is written `<Type as Trait>::Item`, and any other is
/// refused, naming it.
fn invoker_self_items(sig: &mut syn::Signature, owner: &syn::ItemImpl) -> Result<(), String> {
    if let Some((_, path, _)) = &owner.trait_ {
        qualify_self_sig(sig, owner, &owner.self_ty, path);
    }
    struct Left(Option<String>);
    impl<'ast> syn::visit::Visit<'ast> for Left {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if self.0.is_none() && path.segments.len() > 1 && path.segments[0].ident == "Self" {
                self.0 = Some(path.to_token_stream().to_string().replace(' ', ""));
            }
            syn::visit::visit_path(self, path);
        }
    }
    let mut left = Left(None);
    syn::visit::Visit::visit_signature(&mut left, sig);
    match left.0 {
        Some(item) => Err(format!(
            "an invoker's signature names `{item}`, which a bridge cannot name; bridge a function whose signature spells the type out"
        )),
        None => Ok(()),
    }
}

/// A path's tokens without spaces, as horizon-mixins writes an id.
fn plain_spaces(text: &str) -> String {
    text.replace(' ', "")
}

fn prettyplease_or_tokens(tokens: proc_macro2::TokenStream) -> String {
    match syn::parse2::<syn::File>(tokens.clone()) {
        Ok(file) => prettyplease::unparse(&file),
        Err(_) => tokens.to_string(),
    }
}

/// The module `path` of the game's source as the patch crate names it: a lower crate's folder
/// name gives the path's first segment, and its library's name (`[lib] name`) the extern.
fn original_path(libraries: &BTreeMap<String, String>, path: &str) -> String {
    let (krate, rest) = path
        .split_once("::")
        .map_or((path, None), |(k, r)| (k, Some(r)));
    let library = libraries.get(krate).map_or(krate, String::as_str);
    match rest {
        Some(rest) => format!("::{library}::{rest}"),
        None => format!("::{library}"),
    }
}

/// The statics std's `thread_local!` declares in `mac`, with their attributes and visibility;
/// `None` for any other macro, for one that may not be std's (a bare `thread_local!` unless
/// `bare_is_std`, `std::thread_local!` unless `std_is_std`), or when the body doesn't read as
/// statics.
fn thread_local_statics(
    mac: &syn::ItemMacro,
    bare_is_std: bool,
    std_is_std: bool,
) -> Option<Vec<syn::ItemStatic>> {
    let segments: Vec<String> = mac
        .mac
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect();
    let absolute = mac.mac.path.leading_colon.is_some();
    let standard = match segments.as_slice() {
        [name] => name == "thread_local" && !absolute && bare_is_std,
        [std, name] => std == "std" && name == "thread_local" && (absolute || std_is_std),
        _ => false,
    };
    if !standard {
        return None;
    }
    syn::parse2::<syn::File>(mac.mac.tokens.clone())
        .ok()?
        .items
        .into_iter()
        .map(|item| match item {
            Item::Static(item) => Some(item),
            _ => None,
        })
        .collect()
}

/// The mirror of module `path` and its children in `tree`.
fn mirror(
    source: &mut Source,
    path: &str,
    tree: &Tree,
    bridges: &[syn::Ident],
    narrowing: Option<&Narrowing<'_>>,
) -> Result<proc_macro2::TokenStream, String> {
    use mod_source::modules::{Cfg, cfg_predicates};

    let items = source.module(path)?.items.clone();
    let original: syn::Path =
        syn::parse_str(&original_path(&source.libraries, path)).expect("a module path");
    let reserved: BTreeSet<String> = tree
        .children
        .keys()
        .cloned()
        .chain(tree.own.iter().cloned())
        .collect();
    // The crate root is not widened: only its public items are there to re-export.
    let mut public: BTreeSet<String> = items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.sig.ident.to_string())
            }
            Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Mod(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            Item::Union(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
                Some(item.ident.to_string())
            }
            _ => None,
        })
        .collect();
    let root = !path.contains("::");
    let lower = reexports::preserve_owner_features(source.layout, path);
    let only_public = root || (lower && tree.copies.is_empty());
    // A `thread_local!` names the statics it declares. Only a public one can be re-exported:
    // nothing widens a private one, which a glob would not have brought either. Other
    // macro invocations may define items syn can't see, which only a glob re-exports.
    // Which `thread_local!` is std's: a bare one unless another macro of that name may be in
    // scope, and `std::thread_local!` unless the module may bind `std` to something else.
    // `::std::thread_local!` always is. Unresolved, the module keeps the glob.
    let defined = items.iter().filter_map(definition);
    let bare_is_std = !source.local_macros.contains("thread_local")
        && !macro_may_be_in_scope(source, path, "thread_local")
        && !uses(&items)
            .iter()
            .any(|item| may_import(&item.tree, None, "thread_local"));
    let std_is_std = !uses(&items)
        .iter()
        .any(|item| may_import(&item.tree, None, "std"))
        && !defined
            .clone()
            .any(|(ident, attrs)| unraw(ident) == "std" && !mod_source::modules::test_only(attrs))
        && !items.iter().any(|item| {
            matches!(item, Item::ExternCrate(krate)
                if krate.rename.as_ref().is_some_and(|(_, name)| unraw(name) == "std"))
        });
    let build_cfg = source.cfg(path);
    let mut glob = false;
    let mut thread_locals = Vec::new();
    for item in &items {
        if let Item::Macro(mac) = item
            && mac.ident.is_none()
        {
            let invocation_cfg = cfg_predicates(&mac.attrs);
            if invocation_cfg.iter().any(Cfg::disabled_without_tests) {
                continue;
            }
            match thread_local_statics(mac, bare_is_std, std_is_std) {
                // Each static exists under the invocation's conditions and its own.
                Some(statics) => thread_locals.extend(statics.into_iter().filter_map(|item| {
                    if !matches!(item.vis, syn::Visibility::Public(_)) {
                        return None;
                    }
                    let mut cfg = invocation_cfg.clone();
                    let predicates = cfg_predicates(&item.attrs);
                    if let Some(build_cfg) = build_cfg {
                        if predicates.iter().any(|predicate| {
                            predicate.evaluate(|atom| Some(build_cfg.contains(atom))) == Some(false)
                        }) {
                            return None;
                        }
                    } else {
                        cfg.extend(predicates);
                    }
                    Some((item.ident, cfg))
                })),
                None => glob = true,
            }
        }
    }
    public.extend(thread_locals.iter().map(|(ident, _)| ident.to_string()));
    // One `pub use` brings every namespace a name has: `pub mod render;` beside
    // `pub fn render` is one re-export, under the cfg either item is compiled under.
    let mut names: Vec<(syn::Ident, Vec<Vec<Cfg>>)> = Vec::new();
    let re_exported = defined.filter_map(|(ident, attrs)| {
        let cfg = cfg_predicates(attrs);
        (ident != "tests" && !cfg.iter().any(Cfg::disabled_without_tests))
            .then(|| (ident.clone(), cfg))
    });
    for (ident, cfg) in re_exported
        .chain(thread_locals)
        .filter(|(ident, _)| !reserved.contains(&ident.to_string()))
        .filter(|(ident, _)| !only_public || public.contains(&ident.to_string()))
    {
        match names.iter_mut().find(|(name, _)| *name == ident) {
            Some((_, cfgs)) => cfgs.push(cfg),
            None => names.push((ident, vec![cfg])),
        }
    }
    let explicit = names.into_iter().map(|(ident, cfgs)| {
        let cfg = match cfgs.as_slice() {
            [cfg] => quote!(#(#[cfg(#cfg)])*),
            _ if cfgs.iter().any(Vec::is_empty) => quote!(),
            _ => {
                let each = cfgs.iter().map(|cfg| quote!(all(#(#cfg),*)));
                quote!(#[cfg(any(#(#each),*))])
            }
        };
        quote!(#cfg pub use #original::#ident;)
    });
    // A lower crate's import under one of its features binds whatever that crate's build chose;
    // staging made it public, so the glob re-exports that binding instead of a copy evaluated
    // under the game's features.
    // A lower crate is compiled with its own features; the patch crate with the game's.
    let lower = !source.layout.owns(path);
    let feature_import = |item: &syn::ItemUse| {
        lower
            && cfg_predicates(&item.attrs)
                .iter()
                .any(|cfg| names_feature_text(&cfg.to_token_stream().to_string()))
    };
    if uses(&items).into_iter().any(feature_import) {
        glob = true;
    }
    let uses = uses(&items)
        .into_iter()
        .filter(|item| !feature_import(item))
        .filter_map(|item| {
            let mut item = item.clone();
            item.tree = filter_use_tree(item.tree, None, None, &|name, root| {
                // A `use` naming a child mirror or a copy would collide with it. In a lower
                // crate, only the listed functions' modules are widened: other private imports
                // wouldn't resolve, so keep only names the copies and helpers refer to, and in a
                // copy's module every import from std and the linked crates ([`Narrowing`]).
                !reserved.contains(name)
                    && narrowing.is_none_or(|narrowing| {
                        narrowing.used.contains(name)
                            || (!tree.copies.is_empty() && narrowing.resolves(root))
                    })
            })?;
            Some(item)
        });
    let mut children = Vec::new();
    for (name, child) in &tree.children {
        let ident = format_ident!("{name}");
        let inner = mirror(
            source,
            &format!("{path}::{name}"),
            child,
            bridges,
            narrowing,
        )?;
        children.push(quote!(pub mod #ident { #inner }));
    }
    // Repeated before the children, which see them by textual scope as the game's did.
    let macros = source.scope_macros(path);
    let copies = &tree.copies;
    // Each trait by `as _`, never a glob: a glob would make horizon-mixins take `format!`/`vec!`/
    // `matches!` in a function copied here as possibly shadowed, and an unnamed import cannot
    // collide with the module's own names.
    let bridge_import = bridges.iter().map(|name| {
        quote!(
            use crate::__fnr_bridges::#name as _;
        )
    });
    // The explicit re-exports name every item the module defines; the glob is there for items a
    // macro invocation defines, which syn can't see. Known definitions must stay named:
    // a glob would change their precedence over the module's own glob imports.
    let glob = glob.then(|| quote!(pub use #original::*;));
    Ok(quote! {
        #glob
        #(#bridge_import)*
        #(#macros)*
        #(#explicit)*
        #(#uses)*
        #(#children)*
        #(#copies)*
    })
}

/// The module path whose file is `file`, and the support module's tokens.
fn support_module(
    source: &Source,
    module: &SupportModule,
) -> Result<(String, proc_macro2::TokenStream), (&'static str, String)> {
    let path = source
        .files
        .iter()
        .find(|(_, files)| files.iter().any(|file| file.path.ends_with(&module.file)))
        .map(|(name, _)| name.clone())
        .ok_or((
            "E_MIXIN_MODULE_DESTINATION",
            format!(
                "{} is not a file in the indexed gameplay sources",
                module.file
            ),
        ))?;
    let items = syn::parse_file(&module.text)
        .map_err(|e| ("E_MIXIN_MODULE_SOURCE", e.to_string()))?
        .items;
    let name = format_ident!("{}", module.name);
    let vis = if module.public {
        quote!(pub)
    } else {
        quote!(pub(crate))
    };
    Ok((path, quote!(#vis mod #name { #(#items)* })))
}

/// The bridge's owner module, trait name and the trait with its impl, emitted in the mirror of
/// the owner's module so its types are named as the game names them.
fn bridge_trait(
    source: &mut Source,
    bridge: &Bridge,
) -> Result<(String, syn::Ident, proc_macro2::TokenStream), (&'static str, String)> {
    let method = format_ident!("{}", bridge.method);
    let trait_name = format_ident!("__Fnr_Bridge_{}", bridge.method);
    if bridge.kind == "INVOKE" {
        let found = find(source, &bridge.target)?;
        let owner = found.owner.as_ref().ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            "an invoker's target is a method or associated function of a type".to_owned(),
        ))?;
        if !owner.generics.params.is_empty() || !found.function.sig.generics.params.is_empty() {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                "an invoker's type and method are not generic".into(),
            ));
        }
        let self_ty = &owner.self_ty;
        let original = &found.function.sig.ident;
        let mut args = Vec::new();
        let mut sig = found.function.sig.clone();
        for (index, input) in sig.inputs.iter_mut().enumerate() {
            match input {
                syn::FnArg::Receiver(_) => args.push(quote!(self)),
                syn::FnArg::Typed(typed) => {
                    let name = format_ident!("__a{index}");
                    *typed.pat = syn::parse_quote!(#name);
                    args.push(quote!(#name));
                }
            }
        }
        sig.ident = method.clone();
        invoker_self_items(&mut sig, owner).map_err(|why| ("E_MIXIN_BRIDGE_TARGET", why))?;
        let call = match &owner.trait_ {
            Some((_, path, _)) => quote!(<#self_ty as #path>::#original(#(#args),*)),
            None => quote!(<#self_ty>::#original(#(#args),*)),
        };
        let path = found.module.clone();
        return Ok((
            path,
            trait_name.clone(),
            quote! {
                #[doc(hidden)]
                #[allow(non_camel_case_types)]
                pub trait #trait_name { #sig; }
                impl #trait_name for #self_ty { #sig { #call } }
            },
        ));
    }
    let (front, owner) = bridge.target.rsplit_once("::").ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge's target is a struct path".to_owned(),
    ))?;
    if !source.is_module(front) {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no indexed gameplay module {front}"),
        ));
    }
    let items = source
        .module(front)
        .map_err(|why| ("E_MIXIN_BRIDGE_TARGET", why))?
        .items
        .clone();
    let item = items
        .iter()
        .find_map(|item| match item {
            Item::Struct(item) if item.ident == owner => Some(item.clone()),
            _ => None,
        })
        .ok_or((
            "E_MIXIN_BRIDGE_TARGET",
            format!("no struct {owner} in {front}"),
        ))?;
    if item.generics.type_params().next().is_some() || item.generics.const_params().next().is_some()
    {
        return Err((
            "E_MIXIN_BRIDGE_TARGET",
            format!("{owner} is generic over types"),
        ));
    }
    let member_name = bridge.member.as_deref().ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        "a field bridge names its `member`".to_owned(),
    ))?;
    let field = match &item.fields {
        syn::Fields::Named(fields) => fields
            .named
            .iter()
            .find(|f| f.ident.as_ref().is_some_and(|i| i == member_name)),
        syn::Fields::Unnamed(fields) => member_name
            .parse::<usize>()
            .ok()
            .and_then(|i| fields.unnamed.get(i)),
        syn::Fields::Unit => None,
    }
    .ok_or((
        "E_MIXIN_BRIDGE_TARGET",
        format!("{owner} has no field {member_name}"),
    ))?;
    let ty = &field.ty;
    let member: syn::Member =
        syn::parse_str(member_name).map_err(|e| ("E_MIXIN_BRIDGE_TARGET", e.to_string()))?;
    let owner = &item.ident;
    // A type generic over lifetimes gets a trait over the same lifetimes.
    let (generics, args, _) = item.generics.split_for_impl();
    let (declared, defined) = match bridge.kind.as_str() {
        "GET" => (quote!(fn #method(&self) -> &#ty), quote!({ &self.#member })),
        "GET_MUT" => (
            quote!(fn #method(&mut self) -> &mut #ty),
            quote!({ &mut self.#member }),
        ),
        "SET" => (
            quote!(fn #method(&mut self, value: #ty)),
            quote!({ self.#member = value; }),
        ),
        other => {
            return Err((
                "E_MIXIN_BRIDGE_TARGET",
                format!("unknown bridge kind {other}"),
            ));
        }
    };
    Ok((
        front.to_owned(),
        trait_name.clone(),
        quote! {
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            pub trait #trait_name #generics { #declared; }
            impl #generics #trait_name #args for #owner #args { #declared #defined }
        },
    ))
}

/// Whether a `macro_rules!` named `name`, exported or not, may be in textual scope in module
/// `path`: one a module on the path defines, or a `#[macro_use]` child of one, or one a
/// `#[macro_use] extern crate` brings.
fn macro_may_be_in_scope(source: &mut Source, path: &str, name: &str) -> bool {
    fn within(source: &mut Source, module: &str, name: &str) -> bool {
        let Ok(found) = source.module(module) else {
            return false;
        };
        let items = found.items.clone();
        items.iter().any(|item| match item {
            Item::Macro(mac) => mac.ident.as_ref().is_some_and(|ident| ident == name),
            Item::ExternCrate(krate) => krate.attrs.iter().any(|a| a.path().is_ident("macro_use")),
            Item::Mod(child) if child.attrs.iter().any(|a| a.path().is_ident("macro_use")) => {
                within(source, &format!("{module}::{}", child.ident), name)
            }
            _ => false,
        })
    }
    let segments: Vec<&str> = path.split("::").collect();
    (1..=segments.len()).any(|end| within(source, &segments[..end].join("::"), name))
}

/// Whether a written cfg predicate reads a Cargo feature.
pub(crate) fn names_feature_text(predicate: &str) -> bool {
    predicate
        .parse::<proc_macro2::TokenStream>()
        .is_ok_and(|predicate| names_feature(&predicate))
}

/// Whether a cfg predicate reads a Cargo feature.
fn names_feature(predicate: &proc_macro2::TokenStream) -> bool {
    predicate.clone().into_iter().any(|tree| match tree {
        proc_macro2::TokenTree::Ident(ident) => ident == "feature",
        proc_macro2::TokenTree::Group(group) => names_feature(&group.stream()),
        _ => false,
    })
}

/// The binding of an unaliased import: `use x::name::{self}` binds `name`.
fn use_binding<'a>(ident: &'a syn::Ident, prefix: Option<&'a syn::Ident>) -> &'a syn::Ident {
    if ident == "self" {
        prefix.unwrap_or(ident)
    } else {
        ident
    }
}

/// Whether `tree` may bring `name` into scope: it binds it, or it has a glob.
fn may_import(tree: &syn::UseTree, prefix: Option<&syn::Ident>, name: &str) -> bool {
    match tree {
        syn::UseTree::Path(path) => may_import(&path.tree, Some(&path.ident), name),
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .any(|item| may_import(item, prefix, name)),
        syn::UseTree::Name(binding) => unraw(use_binding(&binding.ident, prefix)) == name,
        syn::UseTree::Rename(rename) => unraw(&rename.rename) == name,
        syn::UseTree::Glob(_) => true,
    }
}

/// Filter individual bindings, preserving paths and unnameable imports that can affect resolution.
/// `keep` is asked about each binding with the first segment of its path (`sunrise_core` for
/// `sunrise_core::trig::Mth`, in a group or not): `root` is that segment once a branch has one.
fn filter_use_tree(
    tree: syn::UseTree,
    prefix: Option<&syn::Ident>,
    root: Option<&syn::Ident>,
    keep: &impl Fn(&str, &syn::Ident) -> bool,
) -> Option<syn::UseTree> {
    use syn::UseTree::*;
    match tree {
        Path(mut path) => {
            let root = root.unwrap_or(&path.ident).clone();
            path.tree = Box::new(filter_use_tree(
                *path.tree,
                Some(&path.ident),
                Some(&root),
                keep,
            )?);
            Some(Path(path))
        }
        Group(mut group) => {
            group.items = group
                .items
                .into_iter()
                .filter_map(|item| filter_use_tree(item, prefix, root, keep))
                .collect();
            (!group.items.is_empty()).then_some(Group(group))
        }
        Name(ref name) => keep(
            &use_binding(&name.ident, prefix).to_string(),
            root.unwrap_or(&name.ident),
        )
        .then_some(tree),
        Rename(ref rename) => (rename.rename == "_"
            || keep(&rename.rename.to_string(), root.unwrap_or(&rename.ident)))
        .then_some(tree),
        Glob(_) => Some(tree),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lower_crate_is_mirrored_through_its_library_name() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("crates/sunrise-mod-render");
        std::fs::create_dir_all(package.join("src")).unwrap();
        std::fs::write(
            package.join("Cargo.toml"),
            "[package]\nname = 'sunrise-mod-render'\n[lib]\nname = 'mod_render'\n",
        )
        .unwrap();
        std::fs::write(package.join("src/lib.rs"), "pub mod render {}\n").unwrap();
        let mut source = Source {
            layout: LEGACY_LAYOUT,
            root: root.path().to_owned(),
            loaded: BTreeSet::new(),
            files: BTreeMap::new(),
            parsed: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        assert!(source.load("sunrise_mod_render").unwrap());
        assert_eq!(
            original_path(&source.libraries, "sunrise_mod_render::render"),
            "::mod_render::render"
        );
        assert_eq!(
            original_path(
                &source.libraries,
                "sunrise_client::game::server_side::autosave_server"
            ),
            "::sunrise_client::game::server_side::autosave_server"
        );
    }

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    const AUTOSAVE_INTERVAL: &str = "sunrise_gameplay::game::server_side::autosave_server::crate::game::ServerGame<'_>::next_autosave_interval";

    /// [`generate`] on this checkout, with its function index for `targets`.
    fn generate_here(targets: &[String], extras: &Extras) -> Result<PatchCrate, Vec<Refusal>> {
        static INDEX: std::sync::OnceLock<crate::functions::FunctionIndex> =
            std::sync::OnceLock::new();
        let root = root();
        let index = INDEX.get_or_init(|| {
            crate::functions::index_with_layout(&root, SourceLayout::read(&root).unwrap()).unwrap()
        });
        generate(&root, "sunrise_patch", targets, extras, index)
    }

    #[test]
    #[ignore = "daily"]
    fn a_lower_free_function_installs_through_its_library_name() {
        // `sunrise-mod-render`'s library is `mod_render`: the installer must call the slot there.
        let crate_ = generate_here(
            &["sunrise_mod_render::render::world_drawn".into()],
            &Extras::default(),
        )
        .unwrap();
        let lib: String = crate_.libs[0].1.split_whitespace().collect();
        assert!(
            lib.contains("::mod_render::render::__fnr_world_drawn(world_drawn)"),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            !lib.contains("sunrise_mod_render::render::__fnr_"),
            "{}",
            crate_.libs[0].1
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_method_moves_into_a_trait_of_its_own_type() {
        let crate_ = generate_here(&[AUTOSAVE_INTERVAL.into()], &Extras::default()).unwrap();
        assert_eq!(
            crate_.patched[0].id,
            "sunrise_patch_gameplay::game::server_side::autosave_server::<crate::game::ServerGame<'_> as __Fnr_crate_game_ServerGame_next_autosave_interval>::next_autosave_interval"
        );
        let lib = &crate_.libs[0].1;
        assert!(lib.contains("pub mod server_side {"));
        assert!(lib.contains("pub mod autosave_server {"));
        assert!(lib.replace(char::is_whitespace, "").contains(
            "impl__Fnr_crate_game_ServerGame_next_autosave_intervalforcrate::game::ServerGame<'_>"
        ));
        assert!(lib.contains("self.common.clock.sprinting()"));
    }

    #[test]
    #[ignore = "daily"]
    fn refusals_are_named() {
        let error = generate_here(
            &[
                "sunrise_gameplay::game::server_side::autosave_server::crate::game::ServerGame<'_>::no_such_function"
                    .into(),
                "sunrise_no_such_crate::f".into(),
            ],
            &Extras::default(),
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(codes, ["E_MIXIN_TARGET_MISSING", "E_MIXIN_TARGET_MISSING"]);
    }

    #[test]
    #[ignore = "daily"]
    fn a_lower_crate_function_gets_a_patch_crate_of_its_own() {
        let crate_ = generate_here(
            &[
                AUTOSAVE_INTERVAL.into(),
                "sunrise_sim::clock::FixedStep::advance".into(),
            ],
            &Extras::default(),
        )
        .unwrap();
        let names: Vec<_> = crate_.libs.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(names, ["sunrise_patch_gameplay", "sunrise_patch_sim"]);
        let lower = &crate_.libs[1].1;
        assert!(lower.contains("pub mod clock"), "{lower}");
        assert!(
            lower.contains("pub use ::sunrise_sim::clock::FixedStep;"),
            "{lower}"
        );
        assert!(
            crate_
                .patched
                .iter()
                .any(|p| { p.id.starts_with("sunrise_patch_sim::clock::<FixedStep as") })
        );
    }

    #[test]
    fn lower_ancestors_do_not_export_unwidened_private_items() {
        let mut source = Source {
            layout: LEGACY_LAYOUT,
            root: PathBuf::new(),
            loaded: BTreeSet::new(),
            files: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            parsed: BTreeMap::from([(
                "lower::ancestor".to_owned(),
                Module {
                    items: syn::parse_file("fn private_helper() {} pub fn public_helper() {}")
                        .unwrap()
                        .items,
                },
            )]),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let ancestor = mirror(&mut source, "lower::ancestor", &Tree::default(), &[], None)
            .unwrap()
            .to_string();
        assert!(ancestor.contains("public_helper"));
        assert!(!ancestor.contains("private_helper"));
        let mut target = Tree::default();
        target.copies.push(quote!(
            pub fn copy() {}
        ));
        let target = mirror(&mut source, "lower::ancestor", &target, &[], None)
            .unwrap()
            .to_string();
        assert!(target.contains("private_helper"));
        source.parsed.insert(
            "sunrise_client::game".to_owned(),
            Module {
                items: syn::parse_file("fn game_helper() {}").unwrap().items,
            },
        );
        assert!(
            mirror(
                &mut source,
                "sunrise_client::game",
                &Tree::default(),
                &[],
                None
            )
            .unwrap()
            .to_string()
            .contains("game_helper")
        );
    }

    #[test]
    fn a_lower_crate_mirror_keeps_only_the_imports_its_copies_and_helpers_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lib.rs");
        std::fs::write(
            &file,
            "use glam::{Mat4, Vec3};
             use child::{self, Private, Reserved, Visible as Alias, tools::{Lookup as _, Gone}, missing::{Private}};
             use std::rc::Rc;
             use std::collections::*;",
        )
        .unwrap();
        let mut source = Source {
            layout: LEGACY_LAYOUT,
            root: dir.path().to_owned(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([(
                "fixture".to_owned(),
                vec![SourceFile {
                    path: file,
                    cfg: Vec::new(),
                    included: false,
                }],
            )]),
            parsed: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let used = BTreeSet::from_iter(["Mat4", "child", "Alias", "Reserved"].map(str::to_owned));
        let tree = Tree {
            own: BTreeSet::from(["Reserved".to_owned()]),
            ..Tree::default()
        };
        let linked = BTreeSet::new();
        let narrowing = Narrowing::new(&used, &linked);
        let out = mirror(&mut source, "fixture", &tree, &[], Some(&narrowing)).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let imports: Vec<String> = uses(&parsed.items)
            .iter()
            .map(|item| item.tree.to_token_stream().to_string())
            .collect();
        assert_eq!(
            imports,
            [
                "glam :: { Mat4 }",
                "child :: { self , Visible as Alias , tools :: { Lookup as _ } }",
                "std :: collections :: *",
            ]
        );
    }

    #[test]
    fn included_files_keep_both_sets_of_module_imports() {
        let dir = std::env::temp_dir().join(format!("fnr-included-imports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let outer = dir.join("mod.rs");
        let included = dir.join("shared.rs");
        std::fs::write(&outer, "use std::sync::Arc;").unwrap();
        std::fs::write(&included, "use std::rc::Rc;").unwrap();
        let mut source = Source {
            layout: LEGACY_LAYOUT,
            root: dir.clone(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([(
                "fixture".to_owned(),
                [outer, included]
                    .into_iter()
                    .map(|path| SourceFile {
                        path,
                        cfg: Vec::new(),
                        included: false,
                    })
                    .collect(),
            )]),
            parsed: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let out = mirror(&mut source, "fixture", &Tree::default(), &[], None).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let imports: Vec<String> = uses(&parsed.items)
            .iter()
            .map(|item| item.tree.to_token_stream().to_string())
            .collect();
        assert_eq!(imports, ["std :: sync :: Arc", "std :: rc :: Rc"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_mirror_imports_the_bridge_traits_by_name_never_by_glob() {
        let dir = std::env::temp_dir().join(format!("fnr-bridge-imports-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("mod.rs");
        std::fs::write(&file, "pub fn f() {}").unwrap();
        let mut source = Source {
            layout: LEGACY_LAYOUT,
            root: dir.clone(),
            loaded: BTreeSet::new(),
            files: BTreeMap::from([(
                "fixture".to_owned(),
                vec![SourceFile {
                    path: file,
                    cfg: Vec::new(),
                    included: false,
                }],
            )]),
            parsed: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        };
        let traits = [format_ident!("BridgeA"), format_ident!("BridgeB")];
        let out = mirror(&mut source, "fixture", &Tree::default(), &traits, None).unwrap();
        // A glob would make horizon-mixins refuse a bare `matches!` in a function copied here.
        let text = out.to_string().replace(' ', "");
        assert!(
            text.contains("usecrate::__fnr_bridges::BridgeAas_;"),
            "{text}"
        );
        assert!(
            text.contains("usecrate::__fnr_bridges::BridgeBas_;"),
            "{text}"
        );
        assert!(!text.contains("::*"), "{text}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A source whose module `path` is `text`, read from no file.
    fn source_of(path: &str, text: &str) -> Source {
        Source {
            layout: LEGACY_LAYOUT,
            root: PathBuf::new(),
            loaded: BTreeSet::new(),
            files: BTreeMap::new(),
            lower_crate_cfg: BTreeMap::new(),
            parsed: BTreeMap::from([(
                path.to_owned(),
                Module {
                    items: syn::parse_file(text).unwrap().items,
                },
            )]),
            local_macros: BTreeSet::new(),
            libraries: BTreeMap::new(),
            owners: BTreeMap::new(),
            cache: Cache::default(),
            hashes: BTreeMap::new(),
            stale: None,
        }
    }

    /// A mirror tree holding one copied function.
    fn with_a_copy() -> Tree {
        let mut tree = Tree::default();
        tree.copies.push(quote!(
            pub fn copy() {}
        ));
        tree
    }

    #[test]
    fn a_thread_local_re_exports_its_public_statics_by_name_not_by_glob() {
        let mut source = source_of(
            "lower::anim",
            "thread_local! {
                 static POOL: Cell<u8> = const { Cell::new(0) };
                 pub static SHARED: Cell<u8> = Cell::new(0);
             }
             pub fn setup() -> bool { matches!(1, 1) }",
        );
        let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
            .unwrap()
            .to_string()
            .replace(' ', "");
        // A glob would make horizon-mixins refuse the bare `matches!` in `setup`.
        assert!(!out.contains("::*"), "{out}");
        assert!(out.contains("pubuse::lower::anim::SHARED;"), "{out}");
        assert!(!out.contains("POOL"), "{out}");
        // A module above a copy re-exports only public names: the public static is one.
        let out = mirror(&mut source, "lower::anim", &Tree::default(), &[], None)
            .unwrap()
            .to_string()
            .replace(' ', "");
        assert!(out.contains("pubuse::lower::anim::SHARED;"), "{out}");
        // A macro whose items syn can't see still gets the glob.
        let mut source = source_of("lower::anim", "declare_things! {}");
        let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
            .unwrap()
            .to_string()
            .replace(' ', "");
        assert!(out.contains("pubuse::lower::anim::*;"), "{out}");
    }

    #[test]
    fn a_thread_local_static_is_re_exported_under_its_invocations_conditions_and_its_own() {
        let mut source = source_of(
            "lower::anim",
            "#[cfg(unix)]
             thread_local! {
                 pub static SHARED: Cell<u8> = Cell::new(0);
                 #[cfg(not(test))]
                 /// Documented.
                 pub static RELEASED: Cell<u8> = Cell::new(0);
             }",
        );
        let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
            .unwrap()
            .to_string()
            .replace(' ', "");
        assert!(
            out.contains("#[cfg(unix)]pubuse::lower::anim::SHARED;"),
            "{out}"
        );
        assert!(
            out.contains("#[cfg(unix)]#[cfg(not(test))]pubuse::lower::anim::RELEASED;"),
            "{out}"
        );
    }

    #[test]
    fn re_exports_keep_conditions_added_by_cfg_attr() {
        let mut source = source_of(
            "lower::anim",
            r#"
            #[cfg_attr(true, cfg(unix,), cfg(target_pointer_width = "64"))]
            thread_local! {
                #[cfg_attr(windows, cfg_attr(target_env = "msvc", cfg(target_arch = "x86_64")))]
                pub static SHARED: u8 = 0;
            }
            #[cfg_attr(windows, cfg(target_arch = "aarch64"), inline)]
            pub fn ordinary() {}
            #[cfg(unix)]
            pub mod ordinary {}
            "#,
        );
        let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let imports = uses(&parsed.items);
        let names: Vec<_> = imports
            .iter()
            .map(|item| item.tree.to_token_stream().to_string())
            .collect();
        assert_eq!(
            names,
            ["lower :: anim :: ordinary", "lower :: anim :: SHARED"]
        );
        let enabled = |item: &syn::ItemUse, atoms: &[&str]| {
            mod_source::modules::cfg_predicates(&item.attrs)
                .iter()
                .all(|cfg| cfg.evaluate(|atom| Some(atoms.contains(&atom))) == Some(true))
        };
        assert!(enabled(imports[0], &["unix"]));
        assert!(!enabled(imports[1], &["unix"]));
        assert!(enabled(
            imports[1],
            &["unix", "target_pointer_width = \"64\""]
        ));
        assert!(!enabled(imports[0], &["windows"]));
        assert!(enabled(
            imports[0],
            &["windows", "target_arch = \"aarch64\""]
        ));
        // Exercise the nested cfg_attr guard independently of the other cfg restrictions.
        let attrs = &imports[1].attrs;
        let nested = mod_source::modules::cfg_predicates(attrs).pop().unwrap();
        assert_eq!(nested.evaluate(|atom| Some(atom == "windows")), Some(true));
        assert_eq!(
            nested.evaluate(|atom| Some(["windows", "target_env = \"msvc\""].contains(&atom))),
            Some(false)
        );
    }

    #[test]
    fn a_thread_local_under_a_gated_std_module_keeps_the_glob() {
        let thread_local = "std::thread_local! { pub static SHARED: u8 = 0; }";
        for gated in [
            "#[cfg(not(test))] mod std {}",
            "#[cfg(not(test))] mod r#std {}",
        ] {
            let mut source = source_of("lower::anim", &format!("{gated} {thread_local}"));
            let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
                .unwrap()
                .to_string()
                .replace(' ', "");
            assert!(out.contains("pubuse::lower::anim::*;"), "{out}");
            assert!(!out.contains("pubuse::lower::anim::SHARED;"), "{out}");
        }
        // A test-only `std` doesn't exist in the build, and a release item keeps its cfg guard.
        let mut source = source_of(
            "lower::anim",
            &format!(
                "#[cfg(test)] mod std {{}} #[cfg(not(test))] pub fn release() {{}} {thread_local}"
            ),
        );
        let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None).unwrap();
        let parsed: syn::File = syn::parse2(out).unwrap();
        let re_exports: Vec<(Vec<String>, String)> = uses(&parsed.items)
            .into_iter()
            .map(|item| {
                assert!(matches!(item.vis, syn::Visibility::Public(_)));
                assert!(item.leading_colon.is_some());
                let cfgs = item
                    .attrs
                    .iter()
                    .map(|attr| attr.to_token_stream().to_string().replace(' ', ""))
                    .collect();
                let tree = item.tree.to_token_stream().to_string().replace(' ', "");
                (cfgs, tree)
            })
            .collect();
        assert_eq!(
            re_exports,
            [
                (
                    vec!["#[cfg(not(test))]".to_owned()],
                    "lower::anim::release".to_owned(),
                ),
                (vec![], "lower::anim::SHARED".to_owned()),
            ]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_lower_crates_feature_gated_definition_takes_precedence_over_a_glob() {
        use crate::functions::{FILE, FunctionIndex};
        use crate::staging::{self, Build, Mode};
        use std::{fs, process::Command};

        let pristine = tempfile::tempdir().unwrap();
        let staged = tempfile::tempdir().unwrap();
        let lower = "crates/lower/src";
        let anim = format!("{lower}/anim.rs");
        let marker = "#[sunrise_mod_api_macros::replaceable]";
        let sources = [
            (
                "crates/sunrise-client/Cargo.toml",
                "[package]\nname = 'sunrise-client'\n",
            ),
            ("crates/sunrise-client/src/lib.rs", "pub mod game;\n"),
            ("crates/sunrise-client/src/game.rs", ""),
            ("crates/sunrise-client/src/entry.rs", ""),
            ("crates/sunrise-mod-api-gen/internals.txt", ""),
            (crate::crossing::CROSSING_FOREIGN_FILE, "[]"),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            ),
            ("crates/lower/Cargo.toml", "[package]\nname = 'lower'\n"),
            (
                "crates/lower/src/lib.rs",
                "pub mod other { pub fn helper() -> u8 { 3 } }\npub mod anim;\n",
            ),
            (
                &anim,
                r#"
                use super::other::*;
                #[cfg(feature = "local-helper")]
                include!("with_helper.rs");
                #[cfg(not(feature = "local-helper"))]
                include!("without_helper.rs");
                #[sunrise_mod_api_macros::replaceable]
                pub fn copy() -> u8 { helper() }
                "#,
            ),
            (
                "crates/lower/src/with_helper.rs",
                "pub fn helper() -> u8 { 7 }\n",
            ),
            ("crates/lower/src/without_helper.rs", ""),
        ];
        for root in [pristine.path(), staged.path()] {
            for (path, text) in sources {
                let path = root.join(path);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, text).unwrap();
            }
        }
        let root = staged.path();
        let build = Build {
            commit: "c".repeat(40),
        };
        staging::stage(root, &root.join("internals.json"), Mode::Build, &build).unwrap();
        let (pinned, index): (Build, FunctionIndex) =
            mod_surface::build_data::read(FILE, &fs::read_to_string(root.join(FILE)).unwrap())
                .unwrap();
        assert_eq!(pinned, build);
        let index = index.checked().unwrap();
        // The staged installer is real; the marker is only a proc-macro input to the unchanged
        // game source and needn't be linked into this direct rustc fixture.
        let staged_anim = root.join(anim);
        let text = fs::read_to_string(&staged_anim).unwrap();
        fs::write(&staged_anim, text.replace(marker, "")).unwrap();
        let lib = root.join(lower).join("lib.rs");
        let feature = r#"feature="local-helper""#;
        let targets = ["lower::anim::copy".into()];
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let rlib = root.join("liblower.rlib");
        for (lower_has_feature, expected) in [(true, 7), (false, 3)] {
            let flags = lower_has_feature
                .then_some(["--cfg", feature])
                .into_iter()
                .flatten();
            let compiled = Command::new(&rustc)
                .args(["--edition=2024", "--crate-type=rlib", "--crate-name=lower"])
                .args(flags)
                .arg(&lib)
                .arg("-o")
                .arg(&rlib)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );

            let extras = Extras {
                lower_crate_cfg: Some(BTreeMap::from([(
                    "lower".to_owned(),
                    lower_has_feature
                        .then_some(feature.to_owned())
                        .into_iter()
                        .collect(),
                )])),
                ..Extras::default()
            };
            let generated = generate(pristine.path(), "patch", &targets, &extras, &index).unwrap();
            let [(_, body)] = generated.libs.as_slice() else {
                panic!("expected one lower-crate patch library");
            };
            let program = root.join("patch.rs");
            fs::write(
                &program,
                format!("{body}\nfn main() {{ assert_eq!(anim::copy(), {expected}); }}"),
            )
            .unwrap();
            let exe = root.join(format!("patch{}", std::env::consts::EXE_SUFFIX));
            // The patch crate is compiled with the opposite game feature, just like a lower
            // library can have a different feature set from sunrise-client.
            let flags = (!lower_has_feature)
                .then_some(["--cfg", feature])
                .into_iter()
                .flatten();
            let compiled = Command::new(&rustc)
                .arg("--edition=2024")
                .args(flags)
                .arg("--extern")
                .arg(format!("lower={}", rlib.display()))
                .arg(&program)
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let ran = Command::new(&exe).output().unwrap();
            assert!(
                ran.status.success(),
                "{}",
                String::from_utf8_lossy(&ran.stderr)
            );
        }
    }

    #[test]
    fn a_thread_local_with_a_grouped_std_self_import_keeps_the_glob() {
        for import in ["use other::std::{self};", "use other::r#std::{self};"] {
            let mut source = source_of(
                "lower::anim",
                &format!("{import} std::thread_local! {{ pub static SHARED: u8 = 0; }}"),
            );
            let out = mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
                .unwrap()
                .to_string()
                .replace(' ', "");
            assert!(out.contains("pubuse::lower::anim::*;"), "{out}");
            assert!(!out.contains("pubuse::lower::anim::SHARED;"), "{out}");
        }
    }

    #[test]
    fn a_thread_local_that_may_not_be_stds_keeps_the_glob() {
        let glob = |text: &str| {
            let mut source = source_of("lower::anim", text);
            mirror(&mut source, "lower::anim", &with_a_copy(), &[], None)
                .unwrap()
                .to_string()
                .replace(' ', "")
                .contains("pubuse::lower::anim::*;")
        };
        let invocation = "thread_local! { pub static SHARED: u8 = 0; }";
        assert!(!glob(invocation));
        // A `macro_rules!` of that name may expand to anything.
        assert!(glob(&format!(
            "macro_rules! thread_local {{ ($($t:tt)*) => {{ pub fn made() {{}} }}; }} {invocation}"
        )));
        // So may an imported one, or one a glob import brings.
        assert!(glob(&format!("use other::thread_local; {invocation}")));
        assert!(glob(&format!("use other::*; {invocation}")));
        // An exported one is in textual scope too.
        assert!(glob(&format!(
            "#[macro_export] macro_rules! thread_local {{ ($($t:tt)*) => {{}}; }} {invocation}"
        )));
        // Spelled through std, it is std's whatever else is in scope.
        assert!(!glob(
            "use other::thread_local; std::thread_local! { pub static SHARED: u8 = 0; }"
        ));
        // Unless `std` may be something else there; `::std` is always std.
        assert!(glob(
            "use other as std; std::thread_local! { pub static SHARED: u8 = 0; }"
        ));
        assert!(!glob(
            "use other as std; ::std::thread_local! { pub static SHARED: u8 = 0; }"
        ));
    }

    #[test]
    fn a_name_in_two_namespaces_is_re_exported_once() {
        let mut source = source_of(
            "sunrise_client::mobs",
            "pub mod render {}
             pub fn render() {}
             #[cfg(unix)] pub fn os() {}
             #[cfg(windows)] pub mod os {}
             #[cfg(unix)] pub fn local() {}
             pub mod local {}",
        );
        let out = mirror(
            &mut source,
            "sunrise_client::mobs",
            &Tree::default(),
            &[],
            None,
        )
        .unwrap()
        .to_string()
        .replace(' ', "");
        assert_eq!(
            out.matches("pubuse::sunrise_client::mobs::render;").count(),
            1,
            "{out}"
        );
        assert!(
            out.contains("#[cfg(any(all(unix),all(windows)))]pubuse::sunrise_client::mobs::os;"),
            "{out}"
        );
        // One of them is compiled always, so the name is.
        assert!(
            !out.contains("]pubuse::sunrise_client::mobs::local;"),
            "{out}"
        );
        assert_eq!(
            out.matches("pubuse::sunrise_client::mobs::local;").count(),
            1,
            "{out}"
        );
    }

    #[test]
    fn a_copys_module_keeps_its_imports_from_std_and_the_linked_crates() {
        let text = "use sunrise_core::trig::Mth;
                    use std::fmt::Write;
                    use {sunrise_core::lerp::Lerp, super::Hidden};
                    use alloc::vec::Vec;
                    use unlinked::Thing;
                    use super::Private;
                    use crate::helpers::Helper;";
        let used = BTreeSet::new();
        let linked = BTreeSet::from(["sunrise_core".to_owned()]);
        let narrowing = Narrowing::new(&used, &linked);
        let imports = |tree: &Tree| {
            let mut source = source_of("lower::render", text);
            let out = mirror(&mut source, "lower::render", tree, &[], Some(&narrowing)).unwrap();
            let parsed: syn::File = syn::parse2(out).unwrap();
            uses(&parsed.items)
                .iter()
                .map(|item| item.tree.to_token_stream().to_string())
                .collect::<Vec<_>>()
        };
        // `angle.mcos()` in a copy needs `Mth` in scope without naming it.
        // In a group too, without the group's imports through the crate's own modules. `alloc`
        // is not a root the patch crate has.
        assert_eq!(
            imports(&with_a_copy()),
            [
                "sunrise_core :: trig :: Mth",
                "std :: fmt :: Write",
                "{ sunrise_core :: lerp :: Lerp }"
            ]
        );
        // A module with no copy runs no method call: it keeps only what is named.
        assert!(imports(&Tree::default()).is_empty());
    }

    #[test]
    #[ignore = "daily"]
    fn support_modules_and_bridges_land_in_the_mirror_their_rows_name() {
        let extras = Extras {
            modules: vec![SupportModule {
                id: "support".into(),
                file: "crates/sunrise-gameplay/src/game/world/autosave_server.rs".into(),
                name: "fnr_support".into(),
                text: "pub fn twice(n: i64) -> i64 { n * 2 }".into(),
                public: false,
            }],
            bridges: vec![Bridge {
                id: "ticks".into(),
                method: "__horizon_mixins_m_ticks".into(),
                kind: "GET_MUT".into(),
                target: "sunrise_gameplay::game::server_side::ServerState".into(),
                member: Some("ticks_until_autosave".into()),
            }],
            ..Extras::default()
        };
        let crate_ = generate_here(&[AUTOSAVE_INTERVAL.into()], &extras).unwrap();
        let lib = crate_.libs[0].1.replace(char::is_whitespace, "");
        assert!(
            lib.contains("pub(crate)modfnr_support{pubfntwice"),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("fn__horizon_mixins_m_ticks(&mutself)->&mut"),
            "{}",
            crate_.libs[0].1
        );
        assert!(lib.contains("{&mutself.ticks_until_autosave}"));
        assert!(lib.contains(
            "pubmod__fnr_bridges{pubusecrate::game::server_side::__Fnr_Bridge___horizon_mixins_m_ticks;}"
        ));
        assert!(lib.contains("usecrate::__fnr_bridges::__Fnr_Bridge___horizon_mixins_m_ticksas_;"));
        assert!(!lib.contains("__fnr_bridges::*"));
        let error = generate_here(
            &[],
            &Extras {
                modules: vec![SupportModule {
                    file: "crates/sunrise-client-lib/src/clock.rs".into(),
                    ..extras.modules[0].clone()
                }],
                bridges: vec![Bridge {
                    member: Some("no_such_field".into()),
                    ..extras.bridges[0].clone()
                }],
                ..Extras::default()
            },
        )
        .unwrap_err();
        let codes: Vec<_> = error.iter().map(|r| r.code).collect();
        assert_eq!(
            codes,
            ["E_MIXIN_MODULE_DESTINATION", "E_MIXIN_BRIDGE_TARGET"]
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_filled_loops_entry_runs_the_loop_with_the_mods_slots_in_order() {
        let published = crate::loops::collect(&root())
            .unwrap()
            .into_iter()
            .find(|l| l.name == "block_entities.tick")
            .expect("the game publishes block_entities.tick");
        let slot = |mod_id: &str, with: &str| LoopSlot {
            mod_id: mod_id.into(),
            id: "row".into(),
            helper: format!(
                "#[derive(Default)] pub struct {with}; impl BlockEntityTickSlot for {with} {{}}"
            ),
            with: with.into(),
        };
        let extras = Extras {
            loops: vec![LoopSlots {
                published: published.clone(),
                slots: vec![slot("first", "Count"), slot("second", "Skip")],
            }],
            ..Extras::default()
        };
        let crate_ = generate_here(std::slice::from_ref(&published.entry), &extras).unwrap();
        assert_eq!(crate_.patched.len(), 1);
        let (items, block) = loop_entry(&extras.loops[0], "sunrise_client", true, None).unwrap();
        let fixture = quote::quote! {
            extern crate self as sunrise_gameplay_common;
            extern crate self as mod_api;
            pub mod content {
                pub mod crash {
                    pub struct ModCall {
                        pub id: &'static str,
                        pub hook: &'static str,
                        pub part: Option<&'static str>,
                    }
                }
            }
            pub mod game {
                pub mod published_loops {
                    pub struct Attributed<M, T>(pub M, pub T);
                    pub trait LoopMod {
                        const CALL: crate::content::crash::ModCall;
                    }
                    pub fn run_published_loop<G, T>(
                        game: &mut G,
                        _: &'static str,
                        run: impl FnOnce(&mut G, &mut T),
                        vanilla: impl FnOnce(&mut G),
                    ) {
                        let _ = (game, run, vanilla);
                    }
                }
            }
            pub trait BlockEntityTickSlot {}
            struct ServerGame;
            impl ServerGame {
                fn tick_block_entities_with<T>(&mut self, _: &mut T) {}
                fn tick_block_entities(&mut self) #block
            }
            #items
            fn main() { ServerGame.tick_block_entities(); }
        };
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("main.rs");
        std::fs::write(&source, fixture.to_string()).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["--edition", "2024", "--emit", "metadata"])
            .arg(source)
            .arg("-o")
            .arg(dir.path().join("fixture.rmeta"))
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let lib = crate_.libs[0].1.replace(char::is_whitespace, "");
        // First by priority outside, then the next, then the empty slot.
        assert!(
            lib.contains("pubtype__FnrLoopSlots_block_entities_tick=(::sunrise_gameplay_common::game::published_loops::Attributed<__FnrLoopMod_0_block_entities_tick,__fnr_loop_0_block_entities_tick::Count,>,(::sunrise_gameplay_common::game::published_loops::Attributed<__FnrLoopMod_1_block_entities_tick,__fnr_loop_1_block_entities_tick::Skip,>,(),),);"),
            "{}",
            crate_.libs[0].1
        );
        assert!(lib.contains("pubmod__fnr_loop_1_block_entities_tick{usesuper::*;#[derive(Default)]pubstructSkip;implBlockEntityTickSlotforSkip{}}"), "{}", crate_.libs[0].1);
        assert!(
            lib.contains(
                "id:\"second\",hook:\"loopblock_entities.tick\",part:Some(\"second::Skip\")"
            ),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("::sunrise_gameplay_common::game::published_loops::run_published_loop(self,\"block_entities.tick\","),
            "{}",
            crate_.libs[0].1
        );
        assert!(
            lib.contains("|game|game.tick_block_entities_with(&mut())"),
            "{}",
            crate_.libs[0].1
        );
        // A row on the loop's function: it is copied once, with no cell of its own, and the entry's
        // copy calls the copy.
        let generic = published.generic_target();
        let both = generate_here(&[published.entry.clone(), generic.clone()], &extras).unwrap();
        let installs: Vec<_> = both
            .patched
            .iter()
            .map(|p| (p.target.as_str(), p.install.is_some()))
            .collect();
        assert_eq!(
            installs,
            [(published.entry.as_str(), true), (generic.as_str(), false)]
        );
        let lib = both.libs[0].1.replace(char::is_whitespace, "");
        assert!(lib.contains("pubtrait__Fnr_ServerGame_tick_block_entities_with{fntick_block_entities_with(&mutself,__a1:&mut__FnrLoopSlots_block_entities_tick,)->Result<()>;"), "{}", both.libs[0].1);
        assert!(lib.contains("|game,slots:&mut__FnrLoopSlots_block_entities_tick|{__Fnr_ServerGame_tick_block_entities_with::tick_block_entities_with(game,slots,)}") || lib.contains("|game,slots:&mut__FnrLoopSlots_block_entities_tick|__Fnr_ServerGame_tick_block_entities_with::tick_block_entities_with(game,slots,),"), "{}", both.libs[0].1);
        assert!(
            lib.contains("|game|game.tick_block_entities_with(&mut()),"),
            "{}",
            both.libs[0].1
        );
        assert!(
            !lib.contains("__fnr_install_ServerGame_tick_block_entities_with"),
            "{}",
            both.libs[0].1
        );
        // The copy is for the one slot type: a path through the slot parameter names that type,
        // since the copy declares no `S` (`S::RECHECK` failed to build).
        assert!(
            lib.contains("if<__FnrLoopSlots_block_entities_tick>::RECHECK"),
            "{}",
            both.libs[0].1
        );
        assert!(!lib.contains("ifS::RECHECK"), "{}", both.libs[0].1);
        // The copy holds the loop itself, not a call to it: a call-site row inside the loop (the
        // check mod's selector) finds its one call there.
        assert_eq!(
            lib.matches("ticking(pos.0,pos.2)").count(),
            1,
            "{}",
            both.libs[0].1
        );
        // A helper that is not Rust is refused by name.
        let mut broken = extras.clone();
        broken.loops[0].slots[1].helper = "fn (".into();
        let refused = generate_here(std::slice::from_ref(&published.entry), &broken).unwrap_err();
        assert_eq!(refused[0].code, "E_LOOP_HELPER");
        assert!(
            refused[0].why.starts_with("second:row: its helper:"),
            "{}",
            refused[0].why
        );
    }
}

#[cfg(test)]
mod spike {
    /// Writes the patch crate for `FNR_TARGETS` (comma-separated) into `FNR_OUT`, for trying
    /// horizon-mixins on it by hand.
    #[test]
    #[ignore = "manual: writes a patch crate"]
    fn write_patch_crate() {
        let out = std::path::PathBuf::from(std::env::var("FNR_OUT").unwrap());
        let targets: Vec<String> = std::env::var("FNR_TARGETS")
            .unwrap()
            .split(',')
            .map(str::to_owned)
            .collect();
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let only = targets.iter().map(|t| crate::functions::key(t)).collect();
        let index = crate::functions::index_of(&root, &Default::default(), Some(&only)).unwrap();
        match super::generate(
            &root,
            "sunrise_patch",
            &targets,
            &super::Extras::default(),
            &index,
        ) {
            Ok(crate_) => {
                for (name, lib) in &crate_.libs {
                    std::fs::create_dir_all(out.join(name).join("src")).unwrap();
                    std::fs::write(out.join(name).join("src/lib.rs"), lib).unwrap();
                }
                for patched in crate_.patched {
                    println!(
                        "{} => {} | {:?}",
                        patched.target, patched.id, patched.install
                    );
                }
            }
            Err(refusals) => refusals.iter().for_each(|r| println!("REFUSED {r}")),
        }
    }
}

#[cfg(test)]
mod census {
    use super::*;

    /// Every function of the game module, by whether the copy route takes it and, if not, the
    /// refusal it gets. `cargo test -p sunrise-mod-api-gen --lib census -- --ignored --nocapture`
    #[test]
    #[ignore = "measurement: prints the census"]
    fn refusal_census() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut source = Source::new(&root, LEGACY_LAYOUT).unwrap();
        let modules: Vec<String> = source.files.keys().cloned().collect();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut total = 0;
        // Call sites in copyable bodies: all, beneath a borrow, borrowing a call themselves.
        let mut sites = [0usize; 3];
        let mut copyable: Vec<String> = Vec::new();
        let mut note = |why: String| *counts.entry(why).or_default() += 1;
        for module in modules
            .iter()
            .filter(|m| m.starts_with("sunrise_client::game"))
        {
            let items = source.module(module).unwrap().items.clone();
            let mut functions: Vec<Found> = Vec::new();
            fn walk(items: &[Item], module: &str, out: &mut Vec<Found>, traits: &mut usize) {
                for item in items {
                    match item {
                        Item::Fn(f) => out.push(Found {
                            module: module.into(),
                            owner: None,
                            function: syn::ImplItemFn {
                                attrs: f.attrs.clone(),
                                vis: f.vis.clone(),
                                defaultness: None,
                                sig: f.sig.clone(),
                                block: (*f.block).clone(),
                            },
                        }),
                        Item::Impl(block) if block.trait_.is_some() => {
                            *traits += block
                                .items
                                .iter()
                                .filter(|i| matches!(i, ImplItem::Fn(_)))
                                .count();
                        }
                        Item::Impl(block) => {
                            for member in &block.items {
                                if let ImplItem::Fn(f) = member {
                                    out.push(Found {
                                        module: module.into(),
                                        owner: Some(block.clone()),
                                        function: f.clone(),
                                    });
                                }
                            }
                        }
                        Item::Mod(syn::ItemMod {
                            attrs,
                            ident,
                            content: Some((_, body)),
                            ..
                        }) if !mod_source::modules::test_only(attrs) => {
                            walk(body, &format!("{module}::{ident}"), out, traits);
                        }
                        _ => {}
                    }
                }
            }
            let mut traits = 0;
            walk(&items, module, &mut functions, &mut traits);
            total += traits;
            for _ in 0..traits {
                note("refused: trait method (not enumerated)".into());
            }
            for found in functions {
                let attrs = &found.function.attrs;
                if mod_source::modules::test_only(attrs) {
                    continue;
                }
                total += 1;
                let sig = &found.function.sig;
                let why = if attrs
                    .iter()
                    .any(|a| a.path().is_ident("cfg") || a.path().is_ident("cfg_attr"))
                    || found
                        .owner
                        .as_ref()
                        .is_some_and(|b| b.attrs.iter().any(|a| a.path().is_ident("cfg")))
                {
                    "refused: compiled only under a cfg".to_owned()
                } else if !crate::replace::eligible(attrs, sig) {
                    "refused: const/async/unsafe/self by value/impl Trait or ! /inline(always)"
                        .to_owned()
                } else if !(crate::replace::lifetimes_only(&sig.generics)
                    && found
                        .owner
                        .as_ref()
                        .is_none_or(|b| crate::replace::lifetimes_only(&b.generics)))
                {
                    "refused: generic over types".to_owned()
                } else {
                    if replaceable(&found, &mut source, false).is_ok() {
                        let owner = found.owner.as_ref().map(|b| {
                            format!(
                                "{}::",
                                crate::source::plain_path(&b.self_ty.to_token_stream().to_string())
                            )
                        });
                        copyable.push(format!(
                            "{}::{}{}",
                            found.module,
                            owner.unwrap_or_default(),
                            found.function.sig.ident
                        ));
                        let mut s = CallSites::default();
                        s.visit_block(&found.function.block);
                        sites[0] += s.all;
                        sites[1] += s.beneath;
                        sites[2] += s.borrowing;
                    }
                    match replaceable(&found, &mut source, false) {
                        Ok(()) if referenced_call(&found.function.block) => {
                            "copied, but upstream refuses call-site kinds: `&` over a call"
                                .to_owned()
                        }
                        Ok(()) => "copied".to_owned(),
                        Err(why) => format!("refused: {}", why.split(':').next().unwrap_or(&why)),
                    }
                };
                note(why);
            }
        }
        println!("game-module functions: {total}");
        if let Ok(list) = std::env::var("FNR_CENSUS_LIST") {
            std::fs::write(list, copyable.join("\n")).unwrap();
        }
        println!(
            "call sites in copyable functions: {}; beneath `&` (value-wrapping kinds only with the \
             carried narrowing): {}; borrowing a call in their own expression (refused): {}",
            sites[0], sites[1], sites[2]
        );
        #[derive(Default)]
        struct CallSites {
            all: usize,
            beneath: usize,
            borrowing: usize,
            references: usize,
        }
        impl<'ast> Visit<'ast> for CallSites {
            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                    self.all += 1;
                    self.beneath += usize::from(self.references > 0);
                    self.borrowing += usize::from(referenced_call_in(e));
                }
                let reference = matches!(e, syn::Expr::Reference(_));
                self.references += usize::from(reference);
                syn::visit::visit_expr(self, e);
                self.references -= usize::from(reference);
            }
            fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
        }
        fn referenced_call_in(e: &syn::Expr) -> bool {
            let block = syn::Block {
                brace_token: Default::default(),
                stmts: vec![syn::Stmt::Expr(e.clone(), None)],
            };
            referenced_call(&block)
        }
        fn referenced_call(block: &syn::Block) -> bool {
            // horizon-mixins' rule: a call beneath `&` outside macro arguments refuses every
            // call-site kind on the function.
            struct Find(bool);
            impl<'ast> Visit<'ast> for Find {
                fn visit_expr(&mut self, e: &'ast syn::Expr) {
                    if let syn::Expr::Reference(r) = e {
                        struct Call(bool);
                        impl<'ast> Visit<'ast> for Call {
                            fn visit_expr(&mut self, e: &'ast syn::Expr) {
                                if matches!(e, syn::Expr::Call(_) | syn::Expr::MethodCall(_)) {
                                    self.0 = true;
                                }
                                syn::visit::visit_expr(self, e);
                            }
                            fn visit_macro(&mut self, _: &'ast syn::Macro) {}
                        }
                        let mut c = Call(false);
                        c.visit_expr(&r.expr);
                        self.0 |= c.0;
                    }
                    syn::visit::visit_expr(self, e);
                }
                fn visit_macro(&mut self, _: &'ast syn::Macro) {}
            }
            let mut f = Find(false);
            f.visit_block(block);
            f.0
        }
        for (why, n) in counts {
            println!("  {why}: {n} ({:.1}%)", 100.0 * n as f64 / total as f64);
        }
    }

    #[test]
    fn an_invokers_self_items_are_qualified_or_refused_by_name() {
        let owner: syn::ItemImpl = syn::parse_quote! {
            impl Convert for Bar {
                type Error = Failure;
                fn convert(&self) -> Result<u8, Self::Error> { Ok(0) }
            }
        };
        let mut sig: syn::Signature =
            syn::parse_quote!(fn convert(&self) -> Result<u8, Self::Error>);
        invoker_self_items(&mut sig, &owner).unwrap();
        assert_eq!(
            sig.to_token_stream().to_string().replace(' ', ""),
            "fnconvert(&self)->Result<u8,<BarasConvert>::Error>"
        );
        // An item the `impl` does not define has no spelling the bridge's trait could use.
        let mut sig: syn::Signature = syn::parse_quote!(fn convert(&self) -> Self::Other);
        assert_eq!(
            invoker_self_items(&mut sig, &owner).unwrap_err(),
            "an invoker's signature names `Self::Other`, which a bridge cannot name; bridge a function whose signature spells the type out"
        );
    }
}
