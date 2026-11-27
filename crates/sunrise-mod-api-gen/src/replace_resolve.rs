//! Narrow owner and trait resolution for function-replacement staging and indexing.
use crate::source;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use syn::{Item, Visibility};

/// Canonical module-scope declaration of a resolved owner type or trait.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ResolvedItem {
    pub module: String,
    pub item: String,
}

/// Resolved owner, trait, installer, and reader/cell identity for one trait-method slot.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TraitSlotEvidence {
    pub owner: ResolvedItem,
    pub trait_decl: ResolvedItem,
    pub installer_module: String,
    pub installer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reader: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<usize>,
}

impl TraitSlotEvidence {
    /// Whether two evidence records describe the same owner, trait, and installer binding.
    pub fn same_binding(&self, other: &Self) -> bool {
        self.owner == other.owner
            && self.trait_decl == other.trait_decl
            && self.installer_module == other.installer_module
            && self.installer == other.installer
    }
}

/// Resolved owner and trait declarations for a replaceable trait implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTraitImpl {
    pub(crate) owner: ResolvedItem,
    pub(crate) trait_decl: ResolvedItem,
}

pub(crate) const AMBIGUOUS_OWNER: &str = "ambiguous trait owner";
pub(crate) const WIDENED_GLOB_SHADOW: &str = "widened glob shadows prelude or external name";

#[derive(Clone, PartialEq, Eq, Debug)]
enum Vis {
    Public,
    Restricted(String),
    Invalid,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Viewer<'a> {
    Inside(&'a str),
    External,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CfgState {
    Disabled,
    Always,
    Conditional,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ResolveMode {
    Owner,
    Trait,
}

#[derive(Clone, PartialEq, Eq)]
enum Anchor {
    Module(String),
    Bare,
    External,
    Invalid,
}

#[derive(Clone, PartialEq, Eq)]
struct NamedImport {
    anchor: Anchor,
    path: Vec<String>,
    original: Option<String>,
    limit: Vis,
    gated: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct Glob {
    anchor: Anchor,
    path: Vec<String>,
    limit: Vis,
    gated: bool,
    is_direct_super: bool,
}

#[derive(Clone, PartialEq, Eq)]
enum SubmoduleDecl {
    Gated,
    Unconditional { limit: Vis },
}

#[derive(Clone, PartialEq, Eq)]
enum TypeItemKind {
    Nominal,
    Trait,
    TypeAlias,
    TraitAlias,
    ExternCrate { target: String },
}

#[derive(Clone, PartialEq, Eq)]
enum TypeItemDecl {
    Gated,
    Unconditional {
        kind: TypeItemKind,
        limit: Vis,
        enum_variants: Option<(BTreeSet<String>, bool)>,
        also_value: bool,
    },
}

#[derive(Clone, PartialEq, Eq)]
enum ValueItemDecl {
    Gated,
    Unconditional { limit: Vis },
}

#[derive(Clone)]
enum ChildModSource {
    File { path: PathBuf, dir: PathBuf },
    Inline { items: Vec<Item>, dir: PathBuf },
}

#[derive(Default)]
struct ModuleScope {
    submodules: BTreeMap<String, SubmoduleDecl>,
    type_items: BTreeMap<String, TypeItemDecl>,
    value_items: BTreeMap<String, ValueItemDecl>,
    named: BTreeMap<String, Vec<NamedImport>>,
    globs: Vec<Glob>,
    child_sources: BTreeMap<String, ChildModSource>,
}

/// One crate's module-scope declarations and imports for owner and trait resolution.
#[derive(Default)]
pub struct OwnerTypes {
    modules: BTreeMap<String, ModuleScope>,
    self_aliases: BTreeMap<String, bool>,
    widened_glob_shadow: RefCell<BTreeMap<(String, u64), bool>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TypeNsItem {
    Nominal(ResolvedItem),
    Trait(ResolvedItem),
    Module(String),
    TypeAlias(ResolvedItem),
    TraitAlias(ResolvedItem),
    EnumVariant(ResolvedItem),
    ExternCrate { target: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailureReason {
    Conditional,
    GlobInPath,
    Foreign,
    Unresolved,
}

enum ModuleBinding {
    Local(String),
    NonModule(ResolvedItem),
    Foreign,
    NotVisible,
    Ambiguous,
    NotFound,
    Failed(FailureReason),
}

enum TypeNsResult {
    Resolved(TypeNsItem),
    Foreign,
    NotVisible,
    Ambiguous,
    NotFound,
    Cycle,
    Failed(FailureReason),
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GlobCandidate {
    Local(TypeNsItem),
    Foreign,
}

struct WorkspaceCrateInfo {
    lib_path: PathBuf,
    deps: BTreeMap<String, String>,
    extern_crates: BTreeSet<String>,
}

static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(1);

/// Lazy workspace-wide resolver for cross-crate trait declarations.
pub struct WorkspaceTraits {
    id: u64,
    root: Option<PathBuf>,
    crates_by_id: BTreeMap<String, WorkspaceCrateInfo>,
    lib_name_to_crates: BTreeMap<String, Vec<String>>,
    loaded: RefCell<BTreeMap<String, OwnerTypes>>,
    vocab_generated: RefCell<Option<BTreeMap<String, String>>>,
}

impl Default for WorkspaceTraits {
    fn default() -> Self {
        Self::empty()
    }
}

fn parent_module(module: &str) -> Option<String> {
    module
        .rsplit_once("::")
        .map(|(parent, _)| parent.to_owned())
}

fn crate_of(module: &str) -> &str {
    module.split("::").next().unwrap_or(module)
}

fn within(viewer: &str, ancestor: &str) -> bool {
    viewer == ancestor
        || viewer
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with("::"))
}

fn visible_to(viewer: Viewer<'_>, vis: &Vis) -> bool {
    match vis {
        Vis::Public => true,
        Vis::Invalid => false,
        Vis::Restricted(ancestor) => match viewer {
            Viewer::Inside(module) => within(module, ancestor),
            Viewer::External => false,
        },
    }
}

fn common_ancestor(a: &str, b: &str) -> String {
    a.split("::")
        .zip(b.split("::"))
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x)
        .collect::<Vec<_>>()
        .join("::")
}

fn narrow_viewer<'a>(in_module: &str, viewer: Viewer<'a>, storage: &'a mut String) -> Viewer<'a> {
    match viewer {
        Viewer::External => Viewer::External,
        Viewer::Inside(v) => {
            *storage = common_ancestor(in_module, v);
            if storage.is_empty() {
                Viewer::External
            } else {
                Viewer::Inside(storage.as_str())
            }
        }
    }
}

fn cfg_state(attrs: &[syn::Attribute]) -> CfgState {
    if mod_source::modules::test_only(attrs) {
        return CfgState::Disabled;
    }
    let conditions = mod_source::modules::cfg_conditions(attrs);
    if conditions.is_empty() {
        return CfgState::Always;
    }
    let mut all_true = true;
    for cond in &conditions {
        match mod_source::modules::evaluate_cfg(cond, |atom| (atom == "test").then_some(false)) {
            Some(false) => return CfgState::Disabled,
            Some(true) => {}
            None => all_true = false,
        }
    }
    if all_true {
        CfgState::Always
    } else {
        CfgState::Conditional
    }
}

fn has_conditional_path_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let syn::Meta::List(list) = &attr.meta else {
            return false;
        };
        list.path.is_ident("cfg_attr") && list.tokens.to_string().contains("path")
    })
}

fn explicit_path_attr(attrs: &[syn::Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        let syn::Meta::NameValue(nv) = &attr.meta else {
            return None;
        };
        if !nv.path.is_ident("path") {
            return None;
        }
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(lit),
            ..
        }) = &nv.value
        else {
            return None;
        };
        Some(lit.value())
    })
}

fn vis_limit(module: &str, vis: &Visibility) -> Vis {
    match vis {
        Visibility::Public(_) => Vis::Public,
        Visibility::Inherited => Vis::Restricted(module.to_owned()),
        Visibility::Restricted(restricted) => {
            let segments: Vec<String> = restricted
                .path
                .segments
                .iter()
                .map(|s| source::unraw(&s.ident))
                .collect();
            let Some((first, rest)) = segments.split_first() else {
                return Vis::Invalid;
            };
            if restricted.in_token.is_none()
                && !matches!(first.as_str(), "crate" | "self" | "super")
            {
                return Vis::Invalid;
            }
            let mut base = match first.as_str() {
                "crate" => crate_of(module).to_owned(),
                "self" => module.to_owned(),
                "super" => {
                    let Some(parent) = parent_module(module) else {
                        return Vis::Invalid;
                    };
                    parent
                }
                _ => return Vis::Invalid,
            };
            let mut iter = rest.iter().map(String::as_str).peekable();
            if first == "super" {
                while iter.peek().copied() == Some("super") {
                    iter.next();
                    let Some(parent) = parent_module(&base) else {
                        return Vis::Invalid;
                    };
                    base = parent;
                }
            }
            for seg in iter {
                base.push_str("::");
                base.push_str(seg);
            }
            if !within(module, &base) {
                return Vis::Invalid;
            }
            Vis::Restricted(base)
        }
    }
}

fn merge_type_item(scope: &mut ModuleScope, name: String, entry: TypeItemDecl) {
    scope
        .type_items
        .entry(name)
        .and_modify(|slot| {
            if *slot != entry {
                *slot = TypeItemDecl::Gated;
            }
        })
        .or_insert(entry);
}

fn merge_value_item(scope: &mut ModuleScope, name: String, entry: ValueItemDecl) {
    scope
        .value_items
        .entry(name)
        .and_modify(|slot| {
            if *slot != entry {
                *slot = ValueItemDecl::Gated;
            }
        })
        .or_insert(entry);
}

fn merge_submodule(scope: &mut ModuleScope, name: String, entry: SubmoduleDecl) {
    scope
        .submodules
        .entry(name)
        .and_modify(|slot| {
            if *slot != entry {
                *slot = SubmoduleDecl::Gated;
            }
        })
        .or_insert(entry);
}

impl OwnerTypes {
    /// Adds what `items`, a file of `module`, declare and import, inline modules included.
    pub fn declare(&mut self, module: &str, items: &[Item]) {
        self.declare_with_dir(module, items, None, None);
    }

    /// Adds any modules from `items` not yet present (for materialized generated inline modules).
    pub(crate) fn declare_missing_inline(&mut self, module: &str, items: &[Item]) {
        self.widened_glob_shadow.borrow_mut().clear();
        for item in items {
            if let Item::Mod(inline) = item
                && cfg_state(&inline.attrs) == CfgState::Always
                && let Some((_, inner)) = &inline.content
            {
                let name = source::unraw(&inline.ident);
                let child = format!("{module}::{name}");
                if !self.modules.contains_key(&child) {
                    if let Some(parent) = self.modules.get_mut(module) {
                        merge_submodule(
                            parent,
                            name,
                            SubmoduleDecl::Unconditional {
                                limit: vis_limit(module, &inline.vis),
                            },
                        );
                    }
                    self.declare(&child, inner);
                } else {
                    self.declare_missing_inline(&child, inner);
                }
            }
        }
    }

    fn declare_with_dir(
        &mut self,
        module: &str,
        items: &[Item],
        here: Option<&Path>,
        dir: Option<&Path>,
    ) {
        #[derive(Clone)]
        enum Prefix {
            Root,
            External(Vec<String>),
            Bare(Vec<String>),
            Module {
                base: String,
                path: Vec<String>,
                allow_super: bool,
                direct_super: bool,
            },
            Invalid,
        }
        impl Prefix {
            fn into_anchor(self) -> (Anchor, Vec<String>, bool) {
                match self {
                    Prefix::Root => (Anchor::Bare, Vec::new(), false),
                    Prefix::External(path) => (Anchor::External, path, false),
                    Prefix::Bare(path) => (Anchor::Bare, path, false),
                    Prefix::Module {
                        base,
                        path,
                        direct_super,
                        ..
                    } => (Anchor::Module(base), path, direct_super),
                    Prefix::Invalid => (Anchor::Invalid, Vec::new(), false),
                }
            }
        }
        fn walk_use(
            tree: &syn::UseTree,
            module: &str,
            prefix: &Prefix,
            limit: &Vis,
            gated: bool,
            scope: &mut ModuleScope,
        ) {
            match tree {
                syn::UseTree::Path(next) => {
                    let seg = source::unraw(&next.ident);
                    let next_prefix = match prefix {
                        Prefix::Root if seg == "crate" => Prefix::Module {
                            base: crate_of(module).to_owned(),
                            path: Vec::new(),
                            allow_super: false,
                            direct_super: false,
                        },
                        Prefix::Root if seg == "self" => Prefix::Module {
                            base: module.to_owned(),
                            path: Vec::new(),
                            allow_super: false,
                            direct_super: false,
                        },
                        Prefix::Root if seg == "super" => match parent_module(module) {
                            Some(parent) => Prefix::Module {
                                base: parent,
                                path: Vec::new(),
                                allow_super: true,
                                direct_super: true,
                            },
                            None => Prefix::Invalid,
                        },
                        Prefix::Root => Prefix::Bare(vec![seg]),
                        Prefix::Module {
                            base,
                            path,
                            allow_super: true,
                            ..
                        } if path.is_empty() && seg == "super" => match parent_module(base) {
                            Some(parent) => Prefix::Module {
                                base: parent,
                                path: Vec::new(),
                                allow_super: true,
                                direct_super: false,
                            },
                            None => Prefix::Invalid,
                        },
                        Prefix::Module { base, path, .. } => {
                            let mut next_path = path.clone();
                            next_path.push(seg);
                            Prefix::Module {
                                base: base.clone(),
                                path: next_path,
                                allow_super: false,
                                direct_super: false,
                            }
                        }
                        Prefix::Bare(path) => {
                            let mut next_path = path.clone();
                            next_path.push(seg);
                            Prefix::Bare(next_path)
                        }
                        Prefix::External(path) => {
                            let mut next_path = path.clone();
                            next_path.push(seg);
                            Prefix::External(next_path)
                        }
                        Prefix::Invalid => Prefix::Invalid,
                    };
                    walk_use(&next.tree, module, &next_prefix, limit, gated, scope);
                }
                syn::UseTree::Name(name) => {
                    let ident = source::unraw(&name.ident);
                    let (anchor, path, _) = prefix.clone().into_anchor();
                    if ident == "self" {
                        if let Some(key) = path.last().cloned() {
                            if anchor == Anchor::Bare && path.len() == 1 {
                                scope
                                    .named
                                    .entry(key.clone())
                                    .or_default()
                                    .push(NamedImport {
                                        anchor: Anchor::Bare,
                                        path: Vec::new(),
                                        original: Some(key),
                                        limit: limit.clone(),
                                        gated,
                                    });
                            } else {
                                scope.named.entry(key).or_default().push(NamedImport {
                                    anchor,
                                    path,
                                    original: None,
                                    limit: limit.clone(),
                                    gated,
                                });
                            }
                        }
                    } else {
                        scope
                            .named
                            .entry(ident.clone())
                            .or_default()
                            .push(NamedImport {
                                anchor,
                                path,
                                original: Some(ident),
                                limit: limit.clone(),
                                gated,
                            });
                    }
                }
                syn::UseTree::Rename(rename) => {
                    let alias = source::unraw(&rename.rename);
                    if alias == "_" {
                        return;
                    }
                    let ident = source::unraw(&rename.ident);
                    let (anchor, path, _) = prefix.clone().into_anchor();
                    let original = (ident != "self").then_some(ident);
                    scope.named.entry(alias).or_default().push(NamedImport {
                        anchor,
                        path,
                        original,
                        limit: limit.clone(),
                        gated,
                    });
                }
                syn::UseTree::Glob(_) => {
                    let (anchor, path, direct_super) = prefix.clone().into_anchor();
                    let is_direct_super = direct_super && path.is_empty();
                    scope.globs.push(Glob {
                        anchor,
                        path,
                        limit: limit.clone(),
                        gated,
                        is_direct_super,
                    });
                }
                syn::UseTree::Group(group) => {
                    for item in &group.items {
                        walk_use(item, module, prefix, limit, gated, scope);
                    }
                }
            }
        }

        self.widened_glob_shadow.borrow_mut().clear();
        let is_root = !module.contains("::");
        let mut scope = self.modules.remove(module).unwrap_or_default();
        for item in items {
            match item {
                Item::Struct(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::Nominal,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: matches!(
                                    item.fields,
                                    syn::Fields::Unit | syn::Fields::Unnamed(_)
                                ),
                            },
                        );
                    }
                },
                Item::Union(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::Nominal,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: false,
                            },
                        );
                    }
                },
                Item::Enum(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        let mut variants = BTreeSet::new();
                        let mut has_gated = false;
                        for variant in &item.variants {
                            match cfg_state(&variant.attrs) {
                                CfgState::Disabled => {}
                                CfgState::Conditional => has_gated = true,
                                CfgState::Always => {
                                    variants.insert(source::unraw(&variant.ident));
                                }
                            }
                        }
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::Nominal,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: Some((variants, has_gated)),
                                also_value: false,
                            },
                        );
                    }
                },
                Item::Trait(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::Trait,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: false,
                            },
                        );
                    }
                },
                Item::Type(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::TypeAlias,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: false,
                            },
                        );
                    }
                },
                Item::TraitAlias(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_type_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::TraitAlias,
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: false,
                            },
                        );
                    }
                },
                Item::ExternCrate(item) => {
                    let state = cfg_state(&item.attrs);
                    if state == CfgState::Disabled {
                        continue;
                    }
                    let target = source::unraw(&item.ident);
                    let alias = item
                        .rename
                        .as_ref()
                        .map(|(_, r)| source::unraw(r))
                        .unwrap_or_else(|| target.clone());
                    if alias == "_" {
                        continue;
                    }
                    if target == "self" {
                        if is_root {
                            let ok = state == CfgState::Always;
                            self.self_aliases
                                .entry(alias)
                                .and_modify(|slot| *slot &= ok)
                                .or_insert(ok);
                        }
                    } else if state == CfgState::Conditional {
                        merge_type_item(&mut scope, alias, TypeItemDecl::Gated);
                    } else {
                        merge_type_item(
                            &mut scope,
                            alias,
                            TypeItemDecl::Unconditional {
                                kind: TypeItemKind::ExternCrate { target },
                                limit: vis_limit(module, &item.vis),
                                enum_variants: None,
                                also_value: false,
                            },
                        );
                    }
                }
                Item::Fn(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.sig.ident),
                            ValueItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.sig.ident),
                            ValueItemDecl::Unconditional {
                                limit: vis_limit(module, &item.vis),
                            },
                        );
                    }
                },
                Item::Const(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            ValueItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            ValueItemDecl::Unconditional {
                                limit: vis_limit(module, &item.vis),
                            },
                        );
                    }
                },
                Item::Static(item) => match cfg_state(&item.attrs) {
                    CfgState::Disabled => {}
                    CfgState::Conditional => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            ValueItemDecl::Gated,
                        );
                    }
                    CfgState::Always => {
                        merge_value_item(
                            &mut scope,
                            source::unraw(&item.ident),
                            ValueItemDecl::Unconditional {
                                limit: vis_limit(module, &item.vis),
                            },
                        );
                    }
                },
                Item::Mod(item) => {
                    let state = cfg_state(&item.attrs);
                    if state == CfgState::Disabled {
                        continue;
                    }
                    let name = source::unraw(&item.ident);
                    let cond_path = has_conditional_path_attr(&item.attrs);
                    if state == CfgState::Conditional || cond_path {
                        merge_submodule(&mut scope, name, SubmoduleDecl::Gated);
                        continue;
                    }
                    merge_submodule(
                        &mut scope,
                        name.clone(),
                        SubmoduleDecl::Unconditional {
                            limit: vis_limit(module, &item.vis),
                        },
                    );
                    if let Some((_, inner)) = &item.content {
                        if let Some(parent_dir) = dir {
                            let explicit = explicit_path_attr(&item.attrs);
                            let child_dir = if let Some(rel) = explicit {
                                here.unwrap_or(parent_dir).join(rel)
                            } else {
                                parent_dir.join(&name)
                            };
                            scope.child_sources.insert(
                                name.clone(),
                                ChildModSource::Inline {
                                    items: inner.clone(),
                                    dir: child_dir.clone(),
                                },
                            );
                            self.declare_with_dir(
                                &format!("{module}::{name}"),
                                inner,
                                Some(&child_dir),
                                Some(&child_dir),
                            );
                        } else {
                            self.declare(&format!("{module}::{name}"), inner);
                        }
                    } else if let (Some(here_dir), Some(parent_dir)) = (here, dir) {
                        let (child_path, child_dir) =
                            if let Some(rel) = explicit_path_attr(&item.attrs) {
                                let child = here_dir.join(rel);
                                let cdir = child.parent().unwrap_or(here_dir).to_owned();
                                (child, cdir)
                            } else if parent_dir.join(format!("{name}.rs")).is_file() {
                                (
                                    parent_dir.join(format!("{name}.rs")),
                                    parent_dir.join(&name),
                                )
                            } else {
                                (
                                    parent_dir.join(&name).join("mod.rs"),
                                    parent_dir.join(&name),
                                )
                            };
                        scope.child_sources.insert(
                            name,
                            ChildModSource::File {
                                path: child_path,
                                dir: child_dir,
                            },
                        );
                    }
                }
                Item::Use(item) => {
                    let state = cfg_state(&item.attrs);
                    if state == CfgState::Disabled {
                        continue;
                    }
                    let prefix = if item.leading_colon.is_some() {
                        Prefix::External(Vec::new())
                    } else {
                        Prefix::Root
                    };
                    let limit = vis_limit(module, &item.vis);
                    let gated = state == CfgState::Conditional;
                    walk_use(&item.tree, module, &prefix, &limit, gated, &mut scope);
                }
                _ => {}
            }
        }
        self.modules.insert(module.to_owned(), scope);
    }

    /// Structural pass-through parent rule: a parent module that declares at least one
    /// unconditional child module, re-exports its own parent with a single unconditional
    /// `use super::*;`, and carries no other non-test-only glob imports.
    fn pass_through_parent(&self, module: &str, viewer: Viewer<'_>) -> Option<String> {
        let Viewer::Inside(viewer_mod) = viewer else {
            return None;
        };
        if !within(viewer_mod, module) {
            return None;
        }
        let parent = parent_module(module)?;
        let scope = self.modules.get(module)?;
        let has_child = scope
            .submodules
            .values()
            .any(|decl| matches!(decl, SubmoduleDecl::Unconditional { .. }));
        if !has_child {
            return None;
        }
        let [glob] = scope.globs.as_slice() else {
            return None;
        };
        if !glob.is_direct_super || glob.gated || !visible_to(viewer, &glob.limit) {
            return None;
        }
        Some(parent)
    }

    fn has_self_alias(&self, name: &str) -> Option<bool> {
        self.self_aliases.get(name).copied()
    }
}

impl WorkspaceTraits {
    /// Empty workspace resolver for single-crate tests where every trait is in `OwnerTypes`.
    pub fn empty() -> Self {
        Self {
            id: NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed),
            root: None,
            crates_by_id: BTreeMap::new(),
            lib_name_to_crates: BTreeMap::new(),
            loaded: RefCell::new(BTreeMap::new()),
            vocab_generated: RefCell::new(None),
        }
    }

    /// Reads workspace crate manifests under `root/crates/` so traits in other workspace crates
    /// can be loaded lazily on demand.
    pub fn load(root: &Path) -> Result<Self, String> {
        let crates_dir = root.join("crates");
        if !crates_dir.is_dir() {
            return Ok(Self::empty());
        }
        let mut raw_packages: Vec<(PathBuf, String, String, PathBuf, toml::Table)> = Vec::new();
        let mut dir_to_crate: BTreeMap<PathBuf, (String, String)> = BTreeMap::new();
        for entry in
            fs::read_dir(&crates_dir).map_err(|e| format!("{}: {e}", crates_dir.display()))?
        {
            let package_dir = entry
                .map_err(|e| format!("{}: {e}", crates_dir.display()))?
                .path();
            if !package_dir.join("Cargo.toml").is_file() {
                continue;
            }
            let manifest = crate::replace::manifest(&package_dir)?;
            let Some(pkg_name) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
            else {
                continue;
            };
            let lib_rel = manifest
                .get("lib")
                .and_then(|l| l.get("path"))
                .and_then(toml::Value::as_str)
                .unwrap_or("src/lib.rs");
            let lib_path = lexical(&package_dir.join(lib_rel));
            if !lib_path.is_file() {
                continue;
            }
            let lib_name = manifest
                .get("lib")
                .and_then(|l| l.get("name"))
                .and_then(toml::Value::as_str)
                .unwrap_or(pkg_name)
                .replace('-', "_");
            let crate_id = pkg_name.replace('-', "_");
            let canon_dir = lexical(&package_dir);
            dir_to_crate.insert(canon_dir.clone(), (crate_id.clone(), lib_name.clone()));
            raw_packages.push((canon_dir, crate_id, lib_name, lib_path, manifest));
        }
        let mut crates_by_id = BTreeMap::new();
        let mut lib_name_to_crates: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (pkg_dir, crate_id, lib_name, lib_path, manifest) in raw_packages {
            let mut deps = BTreeMap::new();
            let mut extern_crates = BTreeSet::new();
            if let Some(table) = manifest.get("dependencies").and_then(toml::Value::as_table) {
                for (dep_key, spec) in table {
                    let renamed = spec.get("package").is_some();
                    if let Some(path) = spec.get("path").and_then(toml::Value::as_str) {
                        let target_dir = lexical(&pkg_dir.join(path));
                        if let Some((target_id, target_lib)) = dir_to_crate.get(&target_dir) {
                            let extern_ident = if renamed {
                                dep_key.replace('-', "_")
                            } else {
                                target_lib.clone()
                            };
                            extern_crates.insert(extern_ident.clone());
                            deps.insert(extern_ident, target_id.clone());
                            continue;
                        }
                    }
                    extern_crates.insert(dep_key.replace('-', "_"));
                }
            }
            lib_name_to_crates
                .entry(lib_name.clone())
                .or_default()
                .push(crate_id.clone());
            crates_by_id.insert(
                crate_id,
                WorkspaceCrateInfo {
                    lib_path,
                    deps,
                    extern_crates,
                },
            );
        }
        Ok(Self {
            id: NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed),
            root: Some(root.to_owned()),
            crates_by_id,
            lib_name_to_crates,
            loaded: RefCell::new(BTreeMap::new()),
            vocab_generated: RefCell::new(None),
        })
    }

    fn extern_crates_of(&self, krate: &str) -> impl Iterator<Item = &str> {
        self.crates_by_id
            .get(krate)
            .into_iter()
            .flat_map(|info| info.extern_crates.iter().map(String::as_str))
    }

    fn dependency_crate(&self, from_crate: &str, extern_ident: &str) -> Option<String> {
        if let Some(info) = self.crates_by_id.get(from_crate)
            && let Some(target) = info.deps.get(extern_ident)
        {
            return Some(target.clone());
        }
        // Fallback for minimal fixtures whose manifest omits `[dependencies]`: match another crate by lib name.
        if let Some(matches) = self.lib_name_to_crates.get(extern_ident)
            && let [only] = matches.as_slice()
            && only != from_crate
        {
            return Some(only.clone());
        }
        None
    }

    fn vocab_generated_file(&self, name: &str) -> Option<String> {
        let root = self.root.as_deref()?;
        if !root.join(crate::exports::DIRECTORY).is_dir() {
            return None;
        }
        let mut cache = self.vocab_generated.borrow_mut();
        if cache.is_none() {
            let mut files = BTreeMap::new();
            if let Ok(exported) = crate::exports::sources(root) {
                files.extend(exported);
            }
            if let Ok(parts) = crate::parts::source(root) {
                files.insert(crate::parts::OUTPUT.to_owned(), parts);
            }
            if let Ok(generated) = crate::sources(root) {
                files.insert("mod_api_generated.rs".to_owned(), generated.hooks);
                files.insert("mod_api_generated_host.rs".to_owned(), generated.host);
                files.insert(crate::host::AREAS_OUTPUT.to_owned(), generated.areas);
            }
            *cache = Some(files);
        }
        cache.as_ref().and_then(|map| map.get(name).cloned())
    }

    fn parse_and_expand(&self, krate: &str, file_path: &Path) -> Option<Vec<Item>> {
        let text = fs::read_to_string(file_path).ok()?;
        let parsed = syn::parse_file(&text).ok()?;
        if cfg_state(&parsed.attrs) == CfgState::Disabled {
            return Some(Vec::new());
        }
        let here = file_path.parent().unwrap_or(file_path);
        let mut ancestors = vec![file_path.to_path_buf()];
        Some(self.expand_items(krate, parsed.items, here, &mut ancestors))
    }

    fn expand_items(
        &self,
        krate: &str,
        items: Vec<Item>,
        here: &Path,
        ancestors: &mut Vec<PathBuf>,
    ) -> Vec<Item> {
        let shadowed = mod_source::modules::include_shadowed(&items);
        let mut out = Vec::with_capacity(items.len());
        for mut item in items {
            if let Some((relative, attrs)) = mod_source::modules::literal_include(&item, shadowed) {
                if cfg_state(attrs) == CfgState::Always {
                    let included = lexical(&here.join(relative.value()));
                    if !ancestors.contains(&included)
                        && let Ok(text) = fs::read_to_string(&included)
                        && let Ok(parsed) = syn::parse_file(&text)
                        && cfg_state(&parsed.attrs) == CfgState::Always
                    {
                        ancestors.push(included.clone());
                        let inc_here = included.parent().unwrap_or(here);
                        let expanded = self.expand_items(krate, parsed.items, inc_here, ancestors);
                        ancestors.pop();
                        out.extend(expanded);
                    }
                }
                continue;
            }
            if krate == "sunrise_mod_vocab"
                && let Item::Macro(mac) = &item
                && mac.mac.path.is_ident("include")
                && let Some(name) = out_dir_include_name(&mac.mac.tokens)
                && let Some(code) = self.vocab_generated_file(&name)
                && let Ok(parsed) = syn::parse_file(&code)
            {
                let expanded = self.expand_items(krate, parsed.items, here, ancestors);
                out.extend(expanded);
                continue;
            }
            if let Item::Mod(module) = &mut item
                && let Some((_, inner)) = &mut module.content
            {
                let taken = std::mem::take(inner);
                *inner = self.expand_items(krate, taken, here, ancestors);
            }
            out.push(item);
        }
        out
    }

    fn ensure_module_loaded(&self, current_crate: &str, current: &OwnerTypes, module: &str) {
        let krate = crate_of(module);
        if krate == current_crate {
            return;
        }
        let segments: Vec<&str> = module.split("::").collect();
        let mut cur_mod = krate.to_owned();
        // Ensure crate root is loaded first.
        {
            let loaded = self.loaded.borrow();
            let has_root = loaded
                .get(krate)
                .is_some_and(|ot| ot.modules.contains_key(krate));
            drop(loaded);
            if !has_root {
                let Some(info) = self.crates_by_id.get(krate) else {
                    return;
                };
                let lib_path = info.lib_path.clone();
                let dir = lib_path.parent().unwrap_or(&lib_path).to_owned();
                if let Some(items) = self.parse_and_expand(krate, &lib_path) {
                    let mut loaded = self.loaded.borrow_mut();
                    let ot = loaded.entry(krate.to_owned()).or_default();
                    ot.declare_with_dir(krate, &items, Some(&dir), Some(&dir));
                } else {
                    return;
                }
            }
        }
        for seg in segments.iter().skip(1) {
            let next_mod = format!("{cur_mod}::{seg}");
            let source = {
                let loaded = self.loaded.borrow();
                let Some(ot) = loaded.get(krate) else {
                    return;
                };
                if ot.modules.contains_key(&next_mod) {
                    cur_mod = next_mod;
                    continue;
                }
                ot.modules
                    .get(&cur_mod)
                    .and_then(|s| s.child_sources.get(*seg))
                    .cloned()
            };
            let Some(source) = source else {
                return;
            };
            match source {
                ChildModSource::Inline { items, dir } => {
                    let mut loaded = self.loaded.borrow_mut();
                    let ot = loaded.entry(krate.to_owned()).or_default();
                    ot.declare_with_dir(&next_mod, &items, Some(&dir), Some(&dir));
                }
                ChildModSource::File { path, dir } => {
                    let Some(items) = self.parse_and_expand(krate, &path) else {
                        return;
                    };
                    let here = path.parent().unwrap_or(&dir).to_owned();
                    let mut loaded = self.loaded.borrow_mut();
                    let ot = loaded.entry(krate.to_owned()).or_default();
                    ot.declare_with_dir(&next_mod, &items, Some(&here), Some(&dir));
                }
            }
            cur_mod = next_mod;
        }
        let _ = current;
    }

    fn with_crate<R>(
        &self,
        current_crate: &str,
        current: &OwnerTypes,
        module: &str,
        f: impl FnOnce(&OwnerTypes) -> R,
    ) -> Option<R> {
        let krate = crate_of(module);
        if krate == current_crate {
            return Some(f(current));
        }
        self.ensure_module_loaded(current_crate, current, module);
        let loaded = self.loaded.borrow();
        loaded.get(krate).map(f)
    }

    fn with_scope<R>(
        &self,
        current_crate: &str,
        current: &OwnerTypes,
        module: &str,
        f: impl FnOnce(&OwnerTypes, &ModuleScope) -> R,
    ) -> Option<R> {
        let krate = crate_of(module);
        if krate == current_crate {
            return current.modules.get(module).map(|scope| f(current, scope));
        }
        self.ensure_module_loaded(current_crate, current, module);
        let loaded = self.loaded.borrow();
        loaded
            .get(krate)
            .and_then(|ot| ot.modules.get(module).map(|scope| f(ot, scope)))
    }
}

fn out_dir_include_name(tokens: &proc_macro2::TokenStream) -> Option<String> {
    let text = tokens.to_string();
    if !text.contains("OUT_DIR") {
        return None;
    }
    let start = text.find("\"/")? + 2;
    let end = text[start..].find('"')? + start;
    Some(text[start..end].to_owned())
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn kind_widened_by_staging(kind: &TypeItemKind) -> bool {
    matches!(
        kind,
        TypeItemKind::Nominal | TypeItemKind::Trait | TypeItemKind::TypeAlias
    )
}

fn is_prelude_type(name: &str) -> bool {
    matches!(
        name,
        "AsMut"
            | "AsRef"
            | "AsyncFn"
            | "AsyncFnMut"
            | "AsyncFnOnce"
            | "Box"
            | "Clone"
            | "Copy"
            | "Default"
            | "DoubleEndedIterator"
            | "Drop"
            | "Eq"
            | "ExactSizeIterator"
            | "Extend"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "FromIterator"
            | "Future"
            | "Into"
            | "IntoFuture"
            | "IntoIterator"
            | "Iterator"
            | "Option"
            | "Ord"
            | "PartialEq"
            | "PartialOrd"
            | "Result"
            | "Send"
            | "Sized"
            | "String"
            | "Sync"
            | "ToOwned"
            | "ToString"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
            | "Vec"
    )
}

fn is_prelude_value(name: &str) -> bool {
    matches!(name, "Err" | "None" | "Ok" | "Some" | "drop")
}

/// Resolution state threaded through owner and trait path lookups.
struct Resolver<'a> {
    current_crate: &'a str,
    current: &'a OwnerTypes,
    workspace: &'a WorkspaceTraits,
    mode: ResolveMode,
    widened: bool,
    active_mods: BTreeSet<(String, String)>,
    active_items: BTreeSet<(String, String)>,
}

impl<'a> Resolver<'a> {
    fn new(
        current_crate: &'a str,
        current: &'a OwnerTypes,
        workspace: &'a WorkspaceTraits,
        mode: ResolveMode,
        widened: bool,
    ) -> Self {
        Self {
            current_crate,
            current,
            workspace,
            mode,
            widened,
            active_mods: BTreeSet::new(),
            active_items: BTreeSet::new(),
        }
    }

    fn resolve_module_path(
        &mut self,
        written_in: &str,
        viewer: Viewer<'_>,
        anchor: &Anchor,
        path: &[String],
    ) -> ModuleBinding {
        match anchor {
            Anchor::Invalid => ModuleBinding::Failed(FailureReason::Unresolved),
            Anchor::External => {
                if self.mode == ResolveMode::Owner {
                    return ModuleBinding::Foreign;
                }
                let Some((first, rest)) = path.split_first() else {
                    return ModuleBinding::Foreign;
                };
                let from_crate = crate_of(written_in);
                let Some(dep) = self.workspace.dependency_crate(from_crate, first) else {
                    return ModuleBinding::Foreign;
                };
                let next_viewer = if dep == self.current_crate {
                    viewer
                } else {
                    Viewer::External
                };
                self.walk_module_rest(dep, next_viewer, rest)
            }
            Anchor::Module(base) => self.walk_module_rest(base.clone(), viewer, path),
            Anchor::Bare => {
                let Some((first, rest)) = path.split_first() else {
                    return ModuleBinding::Foreign;
                };
                match self.resolve_module_segment(written_in, viewer, first) {
                    ModuleBinding::Local(next) => self.walk_module_rest(next, viewer, rest),
                    ModuleBinding::NotFound => {
                        self.resolve_bare_crate(written_in, viewer, first, rest)
                    }
                    ModuleBinding::Failed(FailureReason::GlobInPath) => {
                        match self.resolve_bare_crate(written_in, viewer, first, rest) {
                            ModuleBinding::Foreign => {
                                ModuleBinding::Failed(FailureReason::GlobInPath)
                            }
                            other => other,
                        }
                    }
                    other => other,
                }
            }
        }
    }

    fn resolve_bare_crate(
        &mut self,
        written_in: &str,
        viewer: Viewer<'_>,
        first: &str,
        rest: &[String],
    ) -> ModuleBinding {
        let from_crate = crate_of(written_in);
        let self_alias = self
            .workspace
            .with_crate(self.current_crate, self.current, from_crate, |ot| {
                ot.has_self_alias(first)
            })
            .flatten();
        match self_alias {
            Some(true) => return self.walk_module_rest(from_crate.to_owned(), viewer, rest),
            Some(false) => return ModuleBinding::Failed(FailureReason::Conditional),
            None => {}
        }
        if self.mode == ResolveMode::Trait
            && let Some(dep) = self.workspace.dependency_crate(from_crate, first)
        {
            let next_viewer = if dep == self.current_crate {
                viewer
            } else {
                Viewer::External
            };
            return self.walk_module_rest(dep, next_viewer, rest);
        }
        ModuleBinding::Foreign
    }

    fn walk_module_rest(
        &mut self,
        mut cur: String,
        viewer: Viewer<'_>,
        rest: &[String],
    ) -> ModuleBinding {
        for seg in rest {
            match self.resolve_module_segment(&cur, viewer, seg) {
                ModuleBinding::Local(next) => cur = next,
                ModuleBinding::NonModule(id) => return ModuleBinding::NonModule(id),
                ModuleBinding::Foreign => return ModuleBinding::Foreign,
                ModuleBinding::NotVisible => return ModuleBinding::NotVisible,
                ModuleBinding::Ambiguous => return ModuleBinding::Ambiguous,
                ModuleBinding::Failed(reason) => return ModuleBinding::Failed(reason),
                ModuleBinding::NotFound => return ModuleBinding::Failed(FailureReason::Unresolved),
            }
        }
        ModuleBinding::Local(cur)
    }

    fn resolve_module_segment(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        seg: &str,
    ) -> ModuleBinding {
        let key = (in_module.to_owned(), seg.to_owned());
        if !self.active_mods.insert(key.clone()) {
            return ModuleBinding::Failed(FailureReason::Unresolved);
        }
        let result = self.resolve_module_segment_inner(in_module, viewer, seg);
        self.active_mods.remove(&key);
        result
    }

    fn resolve_module_segment_inner(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        seg: &str,
    ) -> ModuleBinding {
        let direct = format!("{in_module}::{seg}");
        // Snapshot the small lookup facts from `in_module` before any recursive call borrows RefCell.
        enum LocalStep {
            Submodule(SubmoduleDecl),
            ImplicitChild,
            TypeItem(TypeItemDecl),
            Named {
                imports: Vec<NamedImport>,
                pass_through: Option<String>,
                crate_root_globs: Vec<Glob>,
                has_other_glob: bool,
            },
            NoScope {
                has_implicit_child: bool,
            },
        }
        let step = {
            let is_crate_root = !in_module.contains("::");
            self.workspace
                .with_crate(self.current_crate, self.current, in_module, |ot| {
                    let Some(scope) = ot.modules.get(in_module) else {
                        return LocalStep::NoScope {
                            has_implicit_child: ot.modules.contains_key(&direct),
                        };
                    };
                    if let Some(decl) = scope.submodules.get(seg) {
                        return LocalStep::Submodule(decl.clone());
                    }
                    if ot.modules.contains_key(&direct) {
                        return LocalStep::ImplicitChild;
                    }
                    if let Some(decl) = scope.type_items.get(seg) {
                        return LocalStep::TypeItem(decl.clone());
                    }
                    let imports = scope.named.get(seg).cloned().unwrap_or_default();
                    let pass_through = ot.pass_through_parent(in_module, viewer);
                    let crate_root_globs = if is_crate_root && self.mode == ResolveMode::Trait {
                        scope.globs.clone()
                    } else {
                        Vec::new()
                    };
                    let has_other_glob = pass_through.is_none()
                        && scope
                            .globs
                            .iter()
                            .any(|g| g.gated || visible_to(viewer, &g.limit));
                    LocalStep::Named {
                        imports,
                        pass_through,
                        crate_root_globs,
                        has_other_glob,
                    }
                })
                .unwrap_or(LocalStep::NoScope {
                    has_implicit_child: false,
                })
        };

        match step {
            LocalStep::NoScope {
                has_implicit_child: true,
            } => ModuleBinding::Local(direct),
            LocalStep::NoScope {
                has_implicit_child: false,
            } => ModuleBinding::NotFound,
            LocalStep::Submodule(SubmoduleDecl::Gated) => {
                ModuleBinding::Failed(FailureReason::Conditional)
            }
            LocalStep::Submodule(SubmoduleDecl::Unconditional { limit }) => {
                if self.widened || visible_to(viewer, &limit) {
                    ModuleBinding::Local(direct)
                } else {
                    ModuleBinding::NotVisible
                }
            }
            LocalStep::ImplicitChild => {
                if self.widened || visible_to(viewer, &Vis::Restricted(in_module.to_owned())) {
                    ModuleBinding::Local(direct)
                } else {
                    ModuleBinding::NotVisible
                }
            }
            LocalStep::TypeItem(TypeItemDecl::Gated) => {
                ModuleBinding::Failed(FailureReason::Conditional)
            }
            LocalStep::TypeItem(TypeItemDecl::Unconditional { kind, limit, .. }) => {
                if !((self.widened && kind_widened_by_staging(&kind)) || visible_to(viewer, &limit))
                {
                    return ModuleBinding::NotVisible;
                }
                match kind {
                    TypeItemKind::ExternCrate { target } => {
                        if self.mode == ResolveMode::Trait
                            && let Some(dep) = self
                                .workspace
                                .dependency_crate(crate_of(in_module), &target)
                        {
                            ModuleBinding::Local(dep)
                        } else {
                            ModuleBinding::Foreign
                        }
                    }
                    _ => ModuleBinding::NonModule(ResolvedItem {
                        module: in_module.to_owned(),
                        item: seg.to_owned(),
                    }),
                }
            }
            LocalStep::Named {
                imports,
                pass_through,
                crate_root_globs,
                has_other_glob,
            } => {
                if let Some(binding) =
                    self.resolve_named_module_imports(in_module, viewer, &imports)
                {
                    return binding;
                }
                // Explicit single pass-through parent rule (e.g. `game::client_side`, `game::server_side`).
                if let Some(parent) = pass_through {
                    return self.resolve_module_segment(&parent, viewer, seg);
                }
                // Crate-root workspace facade re-export for cross-crate traits (e.g. `pub use sunrise_client_lib::*;`).
                if !crate_root_globs.is_empty() {
                    return self.resolve_crate_root_facade_module(
                        in_module,
                        viewer,
                        seg,
                        &crate_root_globs,
                    );
                }
                // Any other glob import in an intermediate module path is refused, never followed.
                if has_other_glob {
                    return ModuleBinding::Failed(FailureReason::GlobInPath);
                }
                ModuleBinding::NotFound
            }
        }
    }

    fn resolve_crate_root_facade_module(
        &mut self,
        crate_root: &str,
        viewer: Viewer<'_>,
        seg: &str,
        globs: &[Glob],
    ) -> ModuleBinding {
        let mut found: BTreeSet<String> = BTreeSet::new();
        let mut saw_non_module = false;
        let mut failed: Option<FailureReason> = None;
        for glob in globs {
            if !visible_to(viewer, &glob.limit) {
                continue;
            }
            if glob.gated {
                failed.get_or_insert(FailureReason::Conditional);
                continue;
            }
            let target_crate = match (&glob.anchor, glob.path.as_slice()) {
                (Anchor::Bare | Anchor::External, [ext]) => {
                    self.workspace.dependency_crate(crate_root, ext)
                }
                _ => None,
            };
            let Some(target_crate) = target_crate else {
                failed.get_or_insert(FailureReason::GlobInPath);
                continue;
            };
            match self.resolve_module_segment(&target_crate, Viewer::External, seg) {
                ModuleBinding::Local(m) => {
                    found.insert(m);
                }
                ModuleBinding::NonModule(_) => saw_non_module = true,
                ModuleBinding::NotFound | ModuleBinding::NotVisible => {}
                ModuleBinding::Ambiguous => return ModuleBinding::Ambiguous,
                ModuleBinding::Foreign => failed.get_or_insert(FailureReason::Foreign).pipe_unit(),
                ModuleBinding::Failed(r) => failed.get_or_insert(r).pipe_unit(),
            }
        }
        if found.len() > 1 || (found.len() == 1 && saw_non_module) {
            return ModuleBinding::Ambiguous;
        }
        if let Some(reason) = failed {
            return ModuleBinding::Failed(reason);
        }
        if let Some(only) = found.into_iter().next() {
            ModuleBinding::Local(only)
        } else {
            ModuleBinding::NotFound
        }
    }

    fn resolve_named_module_imports(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        imports: &[NamedImport],
    ) -> Option<ModuleBinding> {
        let mut resolved: Option<ModuleBinding> = None;
        for imp in imports {
            if !visible_to(viewer, &imp.limit) {
                continue;
            }
            if imp.gated {
                return Some(ModuleBinding::Failed(FailureReason::Conditional));
            }
            let mut narrowed_buf = String::new();
            let narrowed = narrow_viewer(in_module, viewer, &mut narrowed_buf);
            let candidate = match (&imp.original, &imp.anchor, imp.path.as_slice()) {
                (Some(orig), Anchor::Bare, []) => {
                    match self.resolve_bare_crate(in_module, Viewer::Inside(in_module), orig, &[]) {
                        ModuleBinding::Local(m) => ModuleBinding::Local(m),
                        other => other,
                    }
                }
                (None, _, _) => match self.resolve_module_path(
                    in_module,
                    Viewer::Inside(in_module),
                    &imp.anchor,
                    &imp.path,
                ) {
                    ModuleBinding::NotFound => ModuleBinding::Failed(FailureReason::Unresolved),
                    other => other,
                },
                (Some(orig), _, _) => {
                    match self.resolve_module_path(
                        in_module,
                        Viewer::Inside(in_module),
                        &imp.anchor,
                        &imp.path,
                    ) {
                        ModuleBinding::Local(target_mod) => {
                            match self.resolve_module_segment(&target_mod, narrowed, orig) {
                                ModuleBinding::NotFound => {
                                    if self.is_proven_value_only(&target_mod, narrowed, orig) {
                                        continue;
                                    }
                                    ModuleBinding::Failed(FailureReason::Unresolved)
                                }
                                other => other,
                            }
                        }
                        ModuleBinding::NotFound => ModuleBinding::Failed(FailureReason::Unresolved),
                        other => other,
                    }
                }
            };
            resolved = match (resolved, candidate) {
                (None, c) => Some(c),
                (Some(ModuleBinding::Local(a)), ModuleBinding::Local(b)) if a == b => {
                    Some(ModuleBinding::Local(a))
                }
                _ => return Some(ModuleBinding::Ambiguous),
            };
        }
        resolved
    }

    fn is_proven_value_only(&mut self, in_module: &str, viewer: Viewer<'_>, name: &str) -> bool {
        let widened = self.widened;
        let has_value = self
            .workspace
            .with_scope(self.current_crate, self.current, in_module, |_, scope| {
                matches!(
                    scope.value_items.get(name),
                    Some(ValueItemDecl::Unconditional { limit })
                        if widened || visible_to(viewer, limit)
                )
            })
            .unwrap_or(false);
        if !has_value {
            return false;
        }
        matches!(
            self.resolve_type_ns(in_module, viewer, name),
            TypeNsResult::NotFound
        )
    }

    fn resolve_type_ns(&mut self, in_module: &str, viewer: Viewer<'_>, name: &str) -> TypeNsResult {
        let key = (in_module.to_owned(), name.to_owned());
        if !self.active_items.insert(key.clone()) {
            return TypeNsResult::Cycle;
        }
        let result = self.resolve_type_ns_inner(in_module, viewer, name);
        self.active_items.remove(&key);
        result
    }

    fn resolve_type_ns_inner(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        name: &str,
    ) -> TypeNsResult {
        let direct = format!("{in_module}::{name}");
        enum DirectStep {
            NoScope {
                has_implicit_child: bool,
            },
            Submodule(SubmoduleDecl),
            TypeItem {
                decl: TypeItemDecl,
                has_gated_named_conflict: bool,
            },
            ImportsAndGlobs {
                imports: Vec<NamedImport>,
                pass_through: Option<String>,
                globs: Vec<Glob>,
            },
        }
        let step = {
            self.workspace
                .with_crate(self.current_crate, self.current, in_module, |ot| {
                    let Some(scope) = ot.modules.get(in_module) else {
                        return DirectStep::NoScope {
                            has_implicit_child: ot.modules.contains_key(&direct),
                        };
                    };
                    if let Some(decl) = scope.submodules.get(name) {
                        return DirectStep::Submodule(decl.clone());
                    }
                    if let Some(decl) = scope.type_items.get(name) {
                        let has_gated_named_conflict = scope.named.get(name).is_some_and(|list| {
                            list.iter().any(|i| i.gated && visible_to(viewer, &i.limit))
                        });
                        return DirectStep::TypeItem {
                            decl: decl.clone(),
                            has_gated_named_conflict,
                        };
                    }
                    DirectStep::ImportsAndGlobs {
                        imports: scope.named.get(name).cloned().unwrap_or_default(),
                        pass_through: ot.pass_through_parent(in_module, viewer),
                        globs: scope.globs.clone(),
                    }
                })
                .unwrap_or(DirectStep::NoScope {
                    has_implicit_child: false,
                })
        };

        match step {
            DirectStep::NoScope {
                has_implicit_child: true,
            } => TypeNsResult::Resolved(TypeNsItem::Module(direct)),
            DirectStep::NoScope {
                has_implicit_child: false,
            } => TypeNsResult::NotFound,
            DirectStep::Submodule(SubmoduleDecl::Gated) => {
                TypeNsResult::Failed(FailureReason::Conditional)
            }
            DirectStep::Submodule(SubmoduleDecl::Unconditional { limit }) => {
                if self.widened || visible_to(viewer, &limit) {
                    TypeNsResult::Resolved(TypeNsItem::Module(direct))
                } else {
                    TypeNsResult::NotVisible
                }
            }
            DirectStep::TypeItem {
                decl,
                has_gated_named_conflict,
            } => {
                if has_gated_named_conflict {
                    return TypeNsResult::Failed(FailureReason::Conditional);
                }
                match decl {
                    TypeItemDecl::Gated => TypeNsResult::Failed(FailureReason::Conditional),
                    TypeItemDecl::Unconditional { kind, limit, .. } => {
                        if !((self.widened && kind_widened_by_staging(&kind))
                            || visible_to(viewer, &limit))
                        {
                            return TypeNsResult::NotVisible;
                        }
                        let id = ResolvedItem {
                            module: in_module.to_owned(),
                            item: name.to_owned(),
                        };
                        TypeNsResult::Resolved(match kind {
                            TypeItemKind::Nominal => TypeNsItem::Nominal(id),
                            TypeItemKind::Trait => TypeNsItem::Trait(id),
                            TypeItemKind::TypeAlias => TypeNsItem::TypeAlias(id),
                            TypeItemKind::TraitAlias => TypeNsItem::TraitAlias(id),
                            TypeItemKind::ExternCrate { target } => {
                                TypeNsItem::ExternCrate { target }
                            }
                        })
                    }
                }
            }
            DirectStep::ImportsAndGlobs {
                imports,
                pass_through,
                globs,
            } => {
                if let Some(res) = self.resolve_named_type_imports(in_module, viewer, &imports) {
                    return res;
                }
                if let Some(parent) = pass_through {
                    return self.resolve_type_ns(&parent, viewer, name);
                }
                self.resolve_glob_type_imports(in_module, viewer, name, &globs)
            }
        }
    }

    fn resolve_named_type_imports(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        imports: &[NamedImport],
    ) -> Option<TypeNsResult> {
        let mut resolved: Option<TypeNsResult> = None;
        for imp in imports {
            if !visible_to(viewer, &imp.limit) {
                continue;
            }
            if imp.gated {
                return Some(TypeNsResult::Failed(FailureReason::Conditional));
            }
            let mut narrowed_buf = String::new();
            let narrowed = narrow_viewer(in_module, viewer, &mut narrowed_buf);
            let candidate = match (&imp.original, &imp.anchor, imp.path.as_slice()) {
                (Some(orig), Anchor::Bare, []) => {
                    match self.resolve_bare_crate(in_module, Viewer::Inside(in_module), orig, &[]) {
                        ModuleBinding::Local(m) => TypeNsResult::Resolved(TypeNsItem::Module(m)),
                        ModuleBinding::Foreign => TypeNsResult::Foreign,
                        ModuleBinding::Failed(r) => TypeNsResult::Failed(r),
                        _ => TypeNsResult::Failed(FailureReason::Unresolved),
                    }
                }
                (None, _, _) => match self.resolve_module_path(
                    in_module,
                    Viewer::Inside(in_module),
                    &imp.anchor,
                    &imp.path,
                ) {
                    ModuleBinding::Local(m) => TypeNsResult::Resolved(TypeNsItem::Module(m)),
                    ModuleBinding::Foreign => TypeNsResult::Foreign,
                    ModuleBinding::NotVisible => TypeNsResult::NotVisible,
                    ModuleBinding::Ambiguous => TypeNsResult::Ambiguous,
                    ModuleBinding::Failed(r) => TypeNsResult::Failed(r),
                    ModuleBinding::NonModule(_) | ModuleBinding::NotFound => {
                        TypeNsResult::Failed(FailureReason::Unresolved)
                    }
                },
                (Some(orig), _, _) => {
                    match self.resolve_module_path(
                        in_module,
                        Viewer::Inside(in_module),
                        &imp.anchor,
                        &imp.path,
                    ) {
                        ModuleBinding::Local(target_mod) => {
                            if self.mode == ResolveMode::Owner
                                && crate_of(&target_mod) != self.current_crate
                            {
                                TypeNsResult::Foreign
                            } else {
                                match self.resolve_type_ns(&target_mod, narrowed, orig) {
                                    TypeNsResult::NotFound => {
                                        if self.is_proven_value_only(&target_mod, narrowed, orig) {
                                            continue;
                                        }
                                        TypeNsResult::Failed(FailureReason::Unresolved)
                                    }
                                    TypeNsResult::Cycle => {
                                        TypeNsResult::Failed(FailureReason::Unresolved)
                                    }
                                    other => other,
                                }
                            }
                        }
                        ModuleBinding::NonModule(enum_id) => {
                            match self.enum_variant_binding(&enum_id, orig) {
                                Some(variant) => TypeNsResult::Resolved(variant),
                                None => TypeNsResult::Failed(FailureReason::Unresolved),
                            }
                        }
                        ModuleBinding::Foreign => TypeNsResult::Foreign,
                        ModuleBinding::NotVisible => TypeNsResult::NotVisible,
                        ModuleBinding::Ambiguous => TypeNsResult::Ambiguous,
                        ModuleBinding::Failed(r) => TypeNsResult::Failed(r),
                        ModuleBinding::NotFound => TypeNsResult::Failed(FailureReason::Unresolved),
                    }
                }
            };
            resolved = match (resolved, candidate) {
                (None, c) => Some(c),
                (Some(TypeNsResult::Resolved(a)), TypeNsResult::Resolved(b)) if a == b => {
                    Some(TypeNsResult::Resolved(a))
                }
                _ => return Some(TypeNsResult::Ambiguous),
            };
        }
        resolved
    }

    fn enum_variant_binding(&self, enum_id: &ResolvedItem, variant: &str) -> Option<TypeNsItem> {
        self.workspace
            .with_scope(
                self.current_crate,
                self.current,
                &enum_id.module,
                |_, scope| {
                    let Some(TypeItemDecl::Unconditional {
                        enum_variants: Some((variants, _)),
                        ..
                    }) = scope.type_items.get(&enum_id.item)
                    else {
                        return None;
                    };
                    variants.contains(variant).then(|| {
                        TypeNsItem::EnumVariant(ResolvedItem {
                            module: format!("{}::{}", enum_id.module, enum_id.item),
                            item: variant.to_owned(),
                        })
                    })
                },
            )
            .flatten()
    }

    fn resolve_glob_type_imports(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        name: &str,
        globs: &[Glob],
    ) -> TypeNsResult {
        let mut found: BTreeSet<GlobCandidate> = BTreeSet::new();
        let mut failed: Option<FailureReason> = None;
        let mut narrowed_buf = String::new();
        let glob_viewer = narrow_viewer(in_module, viewer, &mut narrowed_buf);
        for glob in globs {
            if !visible_to(viewer, &glob.limit) {
                continue;
            }
            if glob.gated {
                failed.get_or_insert(FailureReason::Conditional);
                continue;
            }
            match self.resolve_module_path(
                in_module,
                Viewer::Inside(in_module),
                &glob.anchor,
                &glob.path,
            ) {
                ModuleBinding::Local(target_mod) => {
                    if self.mode == ResolveMode::Owner
                        && crate_of(&target_mod) != self.current_crate
                    {
                        failed.get_or_insert(FailureReason::Foreign);
                        continue;
                    }
                    match self.resolve_type_ns(&target_mod, glob_viewer, name) {
                        TypeNsResult::Resolved(item) => {
                            found.insert(GlobCandidate::Local(item));
                        }
                        TypeNsResult::Foreign => {
                            found.insert(GlobCandidate::Foreign);
                        }
                        TypeNsResult::NotFound | TypeNsResult::NotVisible | TypeNsResult::Cycle => {
                        }
                        TypeNsResult::Ambiguous => return TypeNsResult::Ambiguous,
                        TypeNsResult::Failed(reason) => {
                            failed.get_or_insert(reason);
                        }
                    }
                }
                ModuleBinding::NonModule(item_id) => {
                    let enum_info = self
                        .workspace
                        .with_scope(
                            self.current_crate,
                            self.current,
                            &item_id.module,
                            |_, scope| match scope.type_items.get(&item_id.item) {
                                Some(TypeItemDecl::Unconditional {
                                    enum_variants: Some((variants, has_gated)),
                                    ..
                                }) => Some((variants.contains(name), *has_gated)),
                                _ => None,
                            },
                        )
                        .flatten();
                    match enum_info {
                        Some((true, _)) => {
                            found.insert(GlobCandidate::Local(TypeNsItem::EnumVariant(
                                ResolvedItem {
                                    module: format!("{}::{}", item_id.module, item_id.item),
                                    item: name.to_owned(),
                                },
                            )));
                        }
                        Some((false, true)) => {
                            failed.get_or_insert(FailureReason::Conditional);
                        }
                        Some((false, false)) => {}
                        None => {
                            failed.get_or_insert(FailureReason::Unresolved);
                        }
                    }
                }
                ModuleBinding::Foreign => {
                    failed.get_or_insert(FailureReason::Foreign);
                }
                ModuleBinding::Ambiguous => return TypeNsResult::Ambiguous,
                ModuleBinding::Failed(reason) => {
                    failed.get_or_insert(reason);
                }
                ModuleBinding::NotFound | ModuleBinding::NotVisible => {
                    failed.get_or_insert(FailureReason::Unresolved);
                }
            }
        }
        if found.len() > 1 {
            return TypeNsResult::Ambiguous;
        }
        if let Some(reason) = failed {
            return TypeNsResult::Failed(reason);
        }
        match found.into_iter().next() {
            Some(GlobCandidate::Local(item)) => TypeNsResult::Resolved(item),
            Some(GlobCandidate::Foreign) => TypeNsResult::Foreign,
            None => TypeNsResult::NotFound,
        }
    }

    fn resolve_owner(&mut self, module: &str, owner: &str) -> Result<ResolvedItem, &'static str> {
        match self.resolve_type_ns(module, Viewer::Inside(module), owner) {
            TypeNsResult::Resolved(TypeNsItem::Nominal(id))
                if crate_of(&id.module) == self.current_crate =>
            {
                Ok(id)
            }
            TypeNsResult::Resolved(TypeNsItem::TypeAlias(_)) => Err("type alias trait owner"),
            TypeNsResult::Resolved(_) | TypeNsResult::Foreign => Err("imported trait owner"),
            TypeNsResult::NotFound => Err("foreign trait owner"),
            TypeNsResult::NotVisible => Err("inaccessible trait owner"),
            TypeNsResult::Ambiguous => Err(AMBIGUOUS_OWNER),
            TypeNsResult::Cycle => Err("imported trait owner"),
            TypeNsResult::Failed(FailureReason::Conditional) => Err("conditional trait owner"),
            TypeNsResult::Failed(FailureReason::GlobInPath) => Err("glob import in owner path"),
            TypeNsResult::Failed(FailureReason::Foreign | FailureReason::Unresolved) => {
                Err("imported trait owner")
            }
        }
    }

    fn resolve_trait(
        &mut self,
        module: &str,
        path: &syn::Path,
    ) -> Result<ResolvedItem, &'static str> {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|s| source::unraw(&s.ident))
            .collect();
        let Some((last, prefix)) = segments.split_last() else {
            return Err("unresolved trait");
        };
        let result = if path.leading_colon.is_none() && prefix.is_empty() {
            self.resolve_type_ns(module, Viewer::Inside(module), last)
        } else {
            let (anchor, mod_path) = if path.leading_colon.is_some() {
                (Anchor::External, prefix.to_vec())
            } else {
                parse_relative_path(module, prefix)
            };
            match self.resolve_module_path(module, Viewer::Inside(module), &anchor, &mod_path) {
                ModuleBinding::Local(target_mod) => {
                    let viewer = if crate_of(&target_mod) == self.current_crate {
                        Viewer::Inside(module)
                    } else {
                        Viewer::External
                    };
                    self.resolve_type_ns(&target_mod, viewer, last)
                }
                ModuleBinding::Foreign => TypeNsResult::Foreign,
                ModuleBinding::NotVisible => TypeNsResult::NotVisible,
                ModuleBinding::Ambiguous => TypeNsResult::Ambiguous,
                ModuleBinding::Failed(r) => TypeNsResult::Failed(r),
                ModuleBinding::NonModule(_) | ModuleBinding::NotFound => {
                    TypeNsResult::Failed(FailureReason::Unresolved)
                }
            }
        };
        match result {
            TypeNsResult::Resolved(TypeNsItem::Trait(id)) => Ok(id),
            TypeNsResult::Foreign | TypeNsResult::NotFound => Err("foreign trait"),
            TypeNsResult::NotVisible => Err("inaccessible trait"),
            TypeNsResult::Ambiguous => Err("ambiguous trait"),
            TypeNsResult::Failed(FailureReason::Conditional) => Err("conditional trait"),
            TypeNsResult::Failed(FailureReason::GlobInPath) => Err("glob import in trait path"),
            TypeNsResult::Failed(FailureReason::Foreign) => Err("foreign trait"),
            TypeNsResult::Failed(FailureReason::Unresolved)
            | TypeNsResult::Cycle
            | TypeNsResult::Resolved(_) => Err("unresolved trait"),
        }
    }
    fn collect_widened_glob_exposures(
        &mut self,
        in_module: &str,
        viewer: Viewer<'_>,
        globs: &[Glob],
        visited: &mut BTreeSet<(String, String)>,
        newly_exposed_types: &mut BTreeSet<String>,
        newly_exposed_values: &mut BTreeSet<String>,
        has_foreign_glob: &mut bool,
    ) {
        let mut narrowed_buf = String::new();
        let glob_viewer = narrow_viewer(in_module, viewer, &mut narrowed_buf);
        let viewer_key = match glob_viewer {
            Viewer::Inside(m) => m.to_owned(),
            Viewer::External => String::new(),
        };
        for glob in globs {
            if !visible_to(viewer, &glob.limit) || glob.gated {
                continue;
            }
            match self.resolve_module_path(
                in_module,
                Viewer::Inside(in_module),
                &glob.anchor,
                &glob.path,
            ) {
                ModuleBinding::Local(target_mod) => {
                    if crate_of(&target_mod) != self.current_crate {
                        *has_foreign_glob = true;
                        continue;
                    }
                    if !visited.insert((target_mod.clone(), viewer_key.clone())) {
                        continue;
                    }
                    let next_globs = if let Some(scope) = self.current.modules.get(&target_mod) {
                        let target_private = Vis::Restricted(target_mod.clone());
                        let hidden_if_private = !visible_to(glob_viewer, &target_private);
                        for (name, decl) in &scope.submodules {
                            match decl {
                                SubmoduleDecl::Unconditional { limit }
                                    if !visible_to(glob_viewer, limit) =>
                                {
                                    newly_exposed_types.insert(name.clone());
                                }
                                SubmoduleDecl::Gated if hidden_if_private => {
                                    newly_exposed_types.insert(name.clone());
                                }
                                _ => {}
                            }
                        }
                        for (name, decl) in &scope.type_items {
                            match decl {
                                TypeItemDecl::Unconditional {
                                    kind,
                                    limit,
                                    also_value,
                                    ..
                                } if kind_widened_by_staging(kind)
                                    && !visible_to(glob_viewer, limit) =>
                                {
                                    newly_exposed_types.insert(name.clone());
                                    if *also_value {
                                        newly_exposed_values.insert(name.clone());
                                    }
                                }
                                TypeItemDecl::Gated if hidden_if_private => {
                                    newly_exposed_types.insert(name.clone());
                                }
                                _ => {}
                            }
                        }
                        for (name, decl) in &scope.value_items {
                            match decl {
                                ValueItemDecl::Unconditional { limit }
                                    if !visible_to(glob_viewer, limit) =>
                                {
                                    newly_exposed_values.insert(name.clone());
                                }
                                ValueItemDecl::Gated if hidden_if_private => {
                                    newly_exposed_values.insert(name.clone());
                                }
                                _ => {}
                            }
                        }
                        scope.globs.clone()
                    } else {
                        Vec::new()
                    };
                    if !next_globs.is_empty() {
                        self.collect_widened_glob_exposures(
                            &target_mod,
                            glob_viewer,
                            &next_globs,
                            visited,
                            newly_exposed_types,
                            newly_exposed_values,
                            has_foreign_glob,
                        );
                    }
                }
                ModuleBinding::Foreign
                | ModuleBinding::Failed(FailureReason::Foreign | FailureReason::GlobInPath) => {
                    *has_foreign_glob = true;
                }
                _ => {}
            }
        }
    }

    fn is_explicitly_bound_in_type_ns(&mut self, module: &str, name: &str) -> bool {
        let Some(scope) = self.current.modules.get(module) else {
            return false;
        };
        if matches!(
            scope.submodules.get(name),
            Some(SubmoduleDecl::Unconditional { .. })
        ) || matches!(
            scope.type_items.get(name),
            Some(TypeItemDecl::Unconditional { .. })
        ) {
            return true;
        }
        let Some(imports) = scope.named.get(name) else {
            return false;
        };
        let imports: Vec<NamedImport> = imports
            .iter()
            .filter(|imp| !imp.gated && visible_to(Viewer::Inside(module), &imp.limit))
            .cloned()
            .collect();
        if imports.is_empty() {
            return false;
        }
        matches!(
            self.resolve_named_type_imports(module, Viewer::Inside(module), &imports),
            Some(TypeNsResult::Resolved(_))
        )
    }

    fn is_explicitly_bound_in_value_ns(&self, module: &str, name: &str) -> bool {
        let Some(scope) = self.current.modules.get(module) else {
            return false;
        };
        matches!(
            scope.value_items.get(name),
            Some(ValueItemDecl::Unconditional { .. })
        ) || matches!(
            scope.type_items.get(name),
            Some(TypeItemDecl::Unconditional {
                also_value: true,
                ..
            })
        )
    }
}

/// Refuses impls in `module` if staging widens a hidden glob item over a prelude or external name.
pub(crate) fn module_widened_glob_shadow_refusal(
    module: &str,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Option<&'static str> {
    let cache_key = (module.to_owned(), workspace.id);
    if let Some(&cached) = types.widened_glob_shadow.borrow().get(&cache_key) {
        return cached.then_some(WIDENED_GLOB_SHADOW);
    }
    let has_hazard = compute_module_widened_glob_shadow(module, types, workspace);
    types
        .widened_glob_shadow
        .borrow_mut()
        .insert(cache_key, has_hazard);
    has_hazard.then_some(WIDENED_GLOB_SHADOW)
}

fn compute_module_widened_glob_shadow(
    module: &str,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> bool {
    let Some(globs) = types.modules.get(module).map(|s| s.globs.clone()) else {
        return false;
    };
    if globs.is_empty() {
        return false;
    }
    let current_crate = crate_of(module);
    let mut resolver = Resolver::new(current_crate, types, workspace, ResolveMode::Trait, true);
    let mut visited = BTreeSet::new();
    let mut newly_exposed_types = BTreeSet::new();
    let mut newly_exposed_values = BTreeSet::new();
    let mut has_foreign_glob = false;
    resolver.collect_widened_glob_exposures(
        module,
        Viewer::Inside(module),
        &globs,
        &mut visited,
        &mut newly_exposed_types,
        &mut newly_exposed_values,
        &mut has_foreign_glob,
    );
    if newly_exposed_types.is_empty() && newly_exposed_values.is_empty() {
        return false;
    }
    let mut extern_prelude: BTreeSet<&str> =
        BTreeSet::from(["alloc", "core", "proc_macro", "std", "test"]);
    extern_prelude.extend(workspace.extern_crates_of(current_crate));
    extern_prelude.extend(types.self_aliases.keys().map(String::as_str));
    if let Some(root_scope) = types.modules.get(current_crate) {
        for (name, decl) in &root_scope.type_items {
            if matches!(
                decl,
                TypeItemDecl::Unconditional {
                    kind: TypeItemKind::ExternCrate { .. },
                    ..
                }
            ) {
                extern_prelude.insert(name.as_str());
            }
        }
    }
    let mut pristine = Resolver::new(current_crate, types, workspace, ResolveMode::Trait, false);
    for name in &newly_exposed_types {
        if pristine.is_explicitly_bound_in_type_ns(module, name) {
            continue;
        }
        if is_prelude_type(name) || extern_prelude.contains(name.as_str()) || has_foreign_glob {
            return true;
        }
    }
    for name in &newly_exposed_values {
        if pristine.is_explicitly_bound_in_value_ns(module, name) {
            continue;
        }
        if is_prelude_value(name) || has_foreign_glob {
            return true;
        }
    }
    false
}

trait PipeUnit {
    fn pipe_unit(self);
}

impl<T> PipeUnit for T {
    fn pipe_unit(self) {}
}

fn parse_relative_path(module: &str, prefix: &[String]) -> (Anchor, Vec<String>) {
    let Some((first, rest)) = prefix.split_first() else {
        return (Anchor::Module(module.to_owned()), Vec::new());
    };
    match first.as_str() {
        "crate" => (Anchor::Module(crate_of(module).to_owned()), rest.to_vec()),
        "self" => (Anchor::Module(module.to_owned()), rest.to_vec()),
        "super" => {
            let Some(mut base) = parent_module(module) else {
                return (Anchor::Invalid, Vec::new());
            };
            let mut idx = 0;
            while rest.get(idx).map(String::as_str) == Some("super") {
                let Some(parent) = parent_module(&base) else {
                    return (Anchor::Invalid, Vec::new());
                };
                base = parent;
                idx += 1;
            }
            (Anchor::Module(base), rest[idx..].to_vec())
        }
        _ => (Anchor::Bare, prefix.to_vec()),
    }
}

impl OwnerTypes {
    pub(crate) fn declaring_module(&self, module: &str, name: &str) -> Option<String> {
        let workspace = WorkspaceTraits::empty();
        Resolver::new(
            crate_of(module),
            self,
            &workspace,
            ResolveMode::Owner,
            false,
        )
        .resolve_owner(module, name)
        .ok()
        .map(|owner| owner.module)
    }
}

pub(crate) fn is_game_nominal(
    module: &str,
    name: &str,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> bool {
    let krate = crate_of(module);
    let mut resolver = Resolver::new(krate, types, workspace, ResolveMode::Owner, false);
    resolver
        .resolve_owner(module, name)
        .is_ok_and(|owner| owner.module == format!("{krate}::game") && owner.item == name)
}

/// Resolves a trait `impl`'s trait and owner declarations or returns the named refusal reason.
pub(crate) fn resolve_trait_impl(
    module: &str,
    block: &syn::ItemImpl,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Result<ResolvedTraitImpl, &'static str> {
    let Some((negative, trait_path, _)) = block.trait_.as_ref() else {
        return Err("inherent impl");
    };
    let last = trait_path.segments.last().expect("a trait path has a name");
    let syn::Type::Path(owner) = &*block.self_ty else {
        return Err("non-path trait owner");
    };
    if owner.qself.is_some() {
        return Err("non-path trait owner");
    }
    if owner.path.leading_colon.is_some() || owner.path.segments.len() != 1 {
        return Err("path-written trait owner");
    }
    if negative.is_some() {
        return Err("negative trait impl");
    }
    if block.unsafety.is_some() {
        return Err("unsafe trait impl");
    }
    if !last.arguments.is_none() {
        return Err("trait generic arguments");
    }
    if let Some(reason) = module_widened_glob_shadow_refusal(module, types, workspace) {
        return Err(reason);
    }
    let current_crate = crate_of(module);
    let mut trait_resolver =
        Resolver::new(current_crate, types, workspace, ResolveMode::Trait, false);
    let trait_decl = trait_resolver.resolve_trait(module, trait_path)?;
    let mut widened_trait_resolver =
        Resolver::new(current_crate, types, workspace, ResolveMode::Trait, true);
    match widened_trait_resolver.resolve_trait(module, trait_path) {
        Ok(widened_decl) if widened_decl == trait_decl => {}
        Err("ambiguous trait") => return Err("ambiguous trait after staging"),
        Ok(_) | Err(_) => return Err("trait changes after staging"),
    }
    let owner_name = source::unraw(&owner.path.segments[0].ident);
    let mut owner_resolver =
        Resolver::new(current_crate, types, workspace, ResolveMode::Owner, false);
    let owner_decl = owner_resolver.resolve_owner(module, &owner_name)?;
    let mut widened_owner_resolver =
        Resolver::new(current_crate, types, workspace, ResolveMode::Owner, true);
    match widened_owner_resolver.resolve_owner(module, &owner_name) {
        Ok(widened_decl) if widened_decl == owner_decl => {}
        Err(AMBIGUOUS_OWNER) => return Err("ambiguous trait owner after staging"),
        Ok(_) | Err(_) => return Err("trait owner changes after staging"),
    }
    Ok(ResolvedTraitImpl {
        owner: owner_decl,
        trait_decl,
    })
}
