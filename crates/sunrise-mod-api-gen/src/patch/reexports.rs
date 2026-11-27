//! The source-level part of path resolution that can prove a copy can't reach a name.
//! Leave external crates, macro expansions and ambiguous bindings to the compiler.

use super::{Found, Item, Source, Visit};
use crate::source::unraw;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use syn::{Attribute, ItemUse, UseTree, Visibility};

pub(super) type Scopes = HashMap<String, Rc<Scope>>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Namespace {
    Type,
    Value,
}

#[derive(Default)]
pub(super) struct Scope {
    // None means a cfg-gated declaration may bind the name in that namespace.
    items: [HashMap<String, Option<Binding>>; 2],
    imports: HashMap<String, Option<Import>>,
    globs: Vec<Import>,
    opaque: bool,
}

struct Import {
    path: Vec<String>,
    external: bool,
    conditional: bool,
    limit: Option<String>,
}

#[derive(Clone)]
struct Binding {
    module: Option<String>,
    // Which module and its descendants can see this name in the original source. `None` is pub.
    limit: Option<String>,
    blocked: Option<Blocked>,
}

#[derive(Clone)]
enum Blocked {
    // A non-public import in a module whose imports the copy cannot repeat.
    Import(String),
    // The mirror cannot re-export a non-public crate-root function.
    RootFunction(String),
}

/// A lower crate's unlisted modules may carry imports the mirror cannot access.
pub(super) struct Narrowing<'a> {
    /// The names the copies and the rows' helpers use: the imports a mirror keeps.
    pub(super) used: &'a BTreeSet<String>,
    /// Linked externs can supply traits a copy uses through method syntax.
    linked: &'a BTreeSet<String>,
}

impl<'a> Narrowing<'a> {
    pub(super) fn new(used: &'a BTreeSet<String>, linked: &'a BTreeSet<String>) -> Self {
        Self { used, linked }
    }

    /// The patch crate's extern prelude includes the linked crates, std and core.
    pub(super) fn resolves(&self, root: &syn::Ident) -> bool {
        root == "std" || root == "core" || self.linked.contains(&root.to_string())
    }
}

pub(super) fn preserve_owner_features(layout: crate::source::SourceLayout, module: &str) -> bool {
    !layout.owns(module)
}

fn within(module: &str, ancestor: &str) -> bool {
    module == ancestor
        || module
            .strip_prefix(ancestor)
            .is_some_and(|tail| tail.starts_with("::"))
}

fn limit(module: &str, vis: &Visibility) -> Option<String> {
    match vis {
        Visibility::Public(_) => None,
        Visibility::Inherited => Some(module.to_owned()),
        Visibility::Restricted(vis) => {
            let path: Vec<_> = vis.path.segments.iter().map(|s| unraw(&s.ident)).collect();
            let (base, tail) = relative(module, &path);
            Some(
                tail.iter()
                    .fold(base.to_owned(), |base, name| format!("{base}::{name}")),
            )
        }
    }
}

/// The starting scope and the rest of a path, after consuming Rust's relative qualifiers.
fn relative<'a>(module: &'a str, mut path: &'a [String]) -> (&'a str, &'a [String]) {
    let Some((first, tail)) = path.split_first() else {
        return (module, path);
    };
    match first.as_str() {
        "crate" => (module.split("::").next().unwrap_or(module), tail),
        "self" => (module, tail),
        "super" => {
            let mut base = module;
            while let Some(("super", tail)) = path.split_first().map(|(s, tail)| (s.as_str(), tail))
            {
                let Some((parent, _)) = base.rsplit_once("::") else {
                    break;
                };
                base = parent;
                path = tail;
            }
            (base, path)
        }
        _ => (module, path),
    }
}

fn conditional(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
}

fn imports(item: &ItemUse, mut insert: impl FnMut(Option<String>, Vec<String>)) {
    fn walk(
        tree: &UseTree,
        prefix: &mut Vec<String>,
        insert: &mut impl FnMut(Option<String>, Vec<String>),
    ) {
        let leaf = |name: &syn::Ident, mut path: Vec<String>| {
            if name != "self" {
                path.push(unraw(name));
            }
            path
        };
        match tree {
            UseTree::Path(next) => {
                prefix.push(unraw(&next.ident));
                walk(&next.tree, prefix, insert);
                prefix.pop();
            }
            UseTree::Name(name) => {
                let path = leaf(&name.ident, prefix.clone());
                if let Some(name) = path.last() {
                    insert(Some(name.clone()), path);
                }
            }
            UseTree::Rename(rename) if rename.rename != "_" => {
                insert(
                    Some(unraw(&rename.rename)),
                    leaf(&rename.ident, prefix.clone()),
                );
            }
            UseTree::Rename(_) => {}
            UseTree::Glob(_) => insert(None, prefix.clone()),
            UseTree::Group(group) => {
                for tree in &group.items {
                    walk(tree, prefix, insert);
                }
            }
        }
    }
    walk(&item.tree, &mut Vec::new(), &mut insert);
}

impl Scope {
    fn read<'a>(module: &str, items: impl IntoIterator<Item = &'a Item>) -> Self {
        let mut scope = Self::default();
        for item in items {
            let (ident, vis, attrs) = match item {
                Item::Use(item) if !mod_source::modules::test_only(&item.attrs) => {
                    let limit = limit(module, &item.vis);
                    imports(item, |name, path| {
                        let import = Import {
                            path,
                            external: item.leading_colon.is_some(),
                            conditional: conditional(&item.attrs),
                            limit: limit.clone(),
                        };
                        if let Some(name) = name {
                            let import = (!import.conditional).then_some(import);
                            scope
                                .imports
                                .entry(name)
                                .and_modify(|entry| *entry = None)
                                .or_insert(import);
                        } else {
                            scope.globs.push(import);
                        }
                    });
                    continue;
                }
                Item::Mod(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Fn(i) => (&i.sig.ident, &i.vis, &i.attrs),
                Item::Const(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Static(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Struct(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Enum(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Union(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Type(i) => (&i.ident, &i.vis, &i.attrs),
                Item::Trait(i) => (&i.ident, &i.vis, &i.attrs),
                Item::TraitAlias(i) => (&i.ident, &i.vis, &i.attrs),
                Item::ExternCrate(i) => (
                    i.rename.as_ref().map_or(&i.ident, |(_, name)| name),
                    &i.vis,
                    &i.attrs,
                ),
                Item::Macro(i) if i.ident.is_none() => {
                    scope.opaque |= !mod_source::modules::test_only(&i.attrs);
                    continue;
                }
                _ => continue,
            };
            if !mod_source::modules::test_only(attrs) {
                let name = unraw(ident);
                let spaces: &[_] = match item {
                    Item::Fn(_) | Item::Const(_) | Item::Static(_) => &[Namespace::Value],
                    // A tuple or unit struct has a constructor in the value namespace too.
                    Item::Struct(item) if !matches!(item.fields, syn::Fields::Named(_)) => {
                        &[Namespace::Type, Namespace::Value]
                    }
                    _ => &[Namespace::Type],
                };
                let binding = (!conditional(attrs)).then(|| Binding {
                    module: matches!(item, Item::Mod(_)).then(|| format!("{module}::{name}")),
                    limit: limit(module, vis),
                    blocked: (matches!(item, Item::Fn(_))
                        && !module.contains("::")
                        && !matches!(vis, Visibility::Public(_)))
                    .then(|| Blocked::RootFunction(name.clone())),
                });
                for &space in spaces {
                    let names = &mut scope.items[space as usize];
                    if let Some(binding) = &binding {
                        names
                            .entry(name.clone())
                            .or_insert_with(|| Some(binding.clone()));
                    } else {
                        names.insert(name.clone(), None);
                    }
                }
            }
        }
        scope
    }
}

// Not found is distinct from being unable to tell what an opaque glob supplies.
struct Unresolved;
type Lookup = Result<Option<Binding>, Unresolved>;

struct Walk<'a> {
    source: &'a mut Source,
    origin: &'a str,
    // `use super::*` and sibling globs can reach the very name already being looked up.
    active: HashSet<(String, String, Namespace)>,
}

impl Walk<'_> {
    fn scope(&mut self, module: &str) -> Option<Rc<Scope>> {
        if let Some(scope) = self.source.cache.scopes.get(module) {
            return Some(Rc::clone(scope));
        }
        let scope = Rc::new(Scope::read(module, &self.source.module(module).ok()?.items));
        self.source
            .cache
            .scopes
            .insert(module.to_owned(), Rc::clone(&scope));
        Some(scope)
    }

    fn import_path(&mut self, module: &str, import: &Import, space: Namespace) -> Lookup {
        if import.external || import.conditional {
            return Err(Unresolved);
        }
        self.path(module, &import.path, space)
    }

    fn import_block(&self, module: &str, import: &Import) -> Option<Blocked> {
        (import.limit.is_some() && !within(self.origin, module))
            .then(|| Blocked::Import(module.to_owned()))
    }

    fn binding(&mut self, module: &str, name: &str, space: Namespace) -> Lookup {
        let scope = self.scope(module).ok_or(Unresolved)?;
        if let Some(item) = scope.items[space as usize].get(name) {
            return item.clone().map(Some).ok_or(Unresolved);
        }
        if space == Namespace::Type {
            // The game module walker knows about build-generated declarations too.
            let child = format!("{module}::{name}");
            if self.source.files.contains_key(&child) {
                return Ok(Some(Binding {
                    module: Some(child),
                    limit: Some(module.to_owned()),
                    blocked: None,
                }));
            }
        }
        if scope.imports.is_empty() && scope.globs.is_empty() {
            return if scope.opaque {
                Err(Unresolved)
            } else {
                Ok(None)
            };
        }
        let key = (module.to_owned(), name.to_owned(), space);
        if !self.active.insert(key.clone()) {
            return Ok(None);
        }
        let binding = self.import_binding(&scope, module, name, space);
        self.active.remove(&key);
        binding
    }

    fn import_binding(
        &mut self,
        scope: &Scope,
        module: &str,
        name: &str,
        space: Namespace,
    ) -> Lookup {
        if let Some(import) = scope.imports.get(name) {
            let import = import.as_ref().ok_or(Unresolved)?;
            // An explicit import shadows globs only in the namespaces it actually binds.
            if let Some(destination) = self.import_path(module, import, space)? {
                return Ok(Some(Binding {
                    blocked: if within(self.origin, module) {
                        // The mirror repeats its import, so that import must also be reachable.
                        destination.blocked
                    } else {
                        self.import_block(module, import)
                    },
                    module: destination.module,
                    limit: import.limit.clone(),
                }));
            }
        }
        if scope.opaque {
            return Err(Unresolved);
        }
        let mut found = None;
        for import in &scope.globs {
            // A glob we cannot read (including an external crate) might provide this name too.
            let from = self
                .import_path(module, import, Namespace::Type)?
                .ok_or(Unresolved)?;
            let from_module = from.module.as_deref().ok_or(Unresolved)?;
            if let Some(mut binding) = self.binding(from_module, name, space)?
                && binding
                    .limit
                    .as_ref()
                    .is_none_or(|limit| within(module, limit))
            {
                if found.is_some() {
                    return Err(Unresolved);
                }
                // Preserve the provider of a restricted name even for an unqualified
                // reference through the mirror's own glob.
                binding.blocked = if within(self.origin, module) {
                    from.blocked.or(binding.blocked)
                } else {
                    self.import_block(module, import).or(binding.blocked)
                };
                if let Some(limit) = &import.limit
                    && binding.limit.as_ref().is_none_or(|old| within(limit, old))
                {
                    binding.limit = Some(limit.clone());
                }
                found = Some(binding);
            }
        }
        Ok(found)
    }

    fn path(&mut self, written_in: &str, path: &[String], space: Namespace) -> Lookup {
        let (module, tail) = relative(written_in, path);
        let mut module = module.to_owned();
        let mut blocked = None;
        for (index, name) in tail.iter().enumerate() {
            let last = index + 1 == tail.len();
            let needed = if last { space } else { Namespace::Type };
            let Some(mut binding) = self.binding(&module, name, needed)? else {
                // An unqualified initial type name may be a crate from the extern prelude.
                if index == 0 && tail.len() == path.len() && needed == Namespace::Type {
                    return Err(Unresolved);
                }
                return Ok(None);
            };
            if binding
                .limit
                .as_ref()
                .is_some_and(|limit| !within(written_in, limit))
            {
                return Ok(None);
            }
            binding.blocked = blocked.or(binding.blocked);
            if last {
                return Ok(Some(binding));
            }
            let Some(next) = binding.module else {
                // This can be an associated-item path. A known restriction on the prefix
                // already makes it unusable in the copy.
                return if binding.blocked.is_some() {
                    Ok(Some(binding))
                } else {
                    Err(Unresolved)
                };
            };
            module = next;
            blocked = binding.blocked;
        }
        Ok((space == Namespace::Type).then_some(Binding {
            module: Some(module),
            limit: None,
            blocked,
        }))
    }
}

// Syn keeps attributes on the concrete nodes, not on the enclosing enums.
macro_rules! attrs {
    ($kind:ident, $node:expr; $($variant:ident),+ $(,)?) => {
        match $node {
            $($kind::$variant(node) => node.attrs.as_slice(),)+
            _ => &[],
        }
    };
}

pub(super) fn check_paths(source: &mut Source, found: &Found) -> Result<(), String> {
    struct Check<'a> {
        walk: Walk<'a>,
        local: Vec<Scope>,
        names: Vec<(Namespace, String)>,
        in_inline_module: bool,
        root_copy: Option<String>,
        refusal: Option<String>,
    }
    impl Check<'_> {
        fn scoped(&mut self, visit: impl FnOnce(&mut Self)) {
            let start = self.names.len();
            visit(self);
            self.names.truncate(start);
        }

        fn bind(&mut self, pat: &syn::Pat) {
            // Don't guess whether a bare pattern name is a binding or a constant. Types,
            // expressions and inline const blocks inside a pattern don't bind any names.
            struct Names<'a>(&'a mut Vec<(Namespace, String)>);
            impl<'ast> Visit<'ast> for Names<'_> {
                fn visit_pat_ident(&mut self, pat: &'ast syn::PatIdent) {
                    self.0.push((Namespace::Value, unraw(&pat.ident)));
                    syn::visit::visit_pat_ident(self, pat);
                }
                fn visit_type(&mut self, _: &'ast syn::Type) {}
                fn visit_expr(&mut self, _: &'ast syn::Expr) {}
                fn visit_block(&mut self, _: &'ast syn::Block) {}
            }
            Names(&mut self.names).visit_pat(pat);
        }

        fn function(&mut self, sig: &syn::Signature, body: Option<&syn::Block>) {
            self.visit_signature(sig);
            if let Some(body) = body {
                for arg in &sig.inputs {
                    if let syn::FnArg::Typed(arg) = arg
                        && !mod_source::modules::test_only(&arg.attrs)
                    {
                        // A cfg-gated parameter can still shadow a name on some builds.
                        self.bind(&arg.pat);
                    }
                }
                self.visit_block(body);
            }
        }

        fn binding(&mut self, parts: &[String], space: Namespace) -> Lookup {
            let Some(first) = parts.first() else {
                return Err(Unresolved);
            };
            if self.in_inline_module && first != "crate" {
                return Err(Unresolved);
            }
            let head = if parts.len() == 1 {
                space
            } else {
                Namespace::Type
            };
            if !matches!(first.as_str(), "crate" | "self" | "super")
                && (self
                    .names
                    .iter()
                    .rev()
                    .any(|(space, name)| *space == head && name == first)
                    || self.local.iter().any(|scope| {
                        scope.opaque
                            || !scope.globs.is_empty()
                            || scope.items[head as usize].contains_key(first)
                            || scope.imports.contains_key(first)
                    }))
            {
                // A block-local binding can shadow the module's name. Don't guess its target.
                return Err(Unresolved);
            }
            self.walk.path(self.walk.origin, parts, space)
        }

        fn note(&mut self, parts: &[String], binding: Binding) {
            self.refusal = match binding.blocked {
                Some(Blocked::Import(module)) => Some(format!(
                    "it names `{}`, which crosses a non-public `use` in `{module}`",
                    parts.join("::")
                )),
                Some(Blocked::RootFunction(name)) if self.root_copy.as_ref() != Some(&name) => {
                    Some(format!(
                        "it names `{}`, which reaches the non-public crate-root function `{name}`",
                        parts.join("::")
                    ))
                }
                _ => None,
            };
        }

        fn path(&mut self, path: &syn::Path, qself: Option<&syn::QSelf>, space: Namespace) {
            if path.leading_colon.is_some() || self.refusal.is_some() {
                return;
            }
            let (len, space) = match qself {
                // In `<T as m::Trait>::Item` only `m::Trait` is a path we can resolve here.
                // In `<T>::Item` there is no trait prefix. Syn visits `T` separately.
                Some(qself) if qself.position == 0 => return,
                Some(qself) => (qself.position, Namespace::Type),
                None => (path.segments.len(), space),
            };
            let parts: Vec<_> = path
                .segments
                .iter()
                .take(len)
                .map(|s| unraw(&s.ident))
                .collect();
            if let Ok(Some(binding)) = self.binding(&parts, space) {
                self.note(&parts, binding);
            }
        }
    }
    impl<'ast> Visit<'ast> for Check<'_> {
        fn visit_block(&mut self, block: &'ast syn::Block) {
            let mut items = block
                .stmts
                .iter()
                .filter_map(|stmt| match stmt {
                    syn::Stmt::Item(item) => Some(item),
                    _ => None,
                })
                .peekable();
            let has_items = items.peek().is_some();
            if has_items {
                self.local.push(Scope::read(self.walk.origin, items));
            }
            self.scoped(|check| syn::visit::visit_block(check, block));
            if has_items {
                self.local.pop();
            }
        }

        fn visit_generics(&mut self, generics: &'ast syn::Generics) {
            for param in &generics.params {
                let (space, ident) = match param {
                    syn::GenericParam::Type(param) => (Namespace::Type, &param.ident),
                    syn::GenericParam::Const(param) => (Namespace::Value, &param.ident),
                    syn::GenericParam::Lifetime(_) => continue,
                };
                self.names.push((space, unraw(ident)));
            }
            syn::visit::visit_generics(self, generics);
        }

        fn visit_expr(&mut self, expr: &'ast syn::Expr) {
            if !conditional(crate::source::expr_attrs(expr)) {
                syn::visit::visit_expr(self, expr);
            }
        }

        fn visit_item(&mut self, item: &'ast Item) {
            if !conditional(attrs!(Item, item;
                Const, Enum, ExternCrate, Fn, ForeignMod, Impl, Macro, Mod, Static, Struct,
                Trait, TraitAlias, Type, Union, Use,
            )) {
                self.scoped(|check| syn::visit::visit_item(check, item));
            }
        }

        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.function(&item.sig, Some(&item.block));
        }

        fn visit_fn_arg(&mut self, arg: &'ast syn::FnArg) {
            let attrs = match arg {
                syn::FnArg::Receiver(arg) => &arg.attrs,
                syn::FnArg::Typed(arg) => &arg.attrs,
            };
            if !conditional(attrs) {
                syn::visit::visit_fn_arg(self, arg);
            }
        }

        fn visit_field(&mut self, field: &'ast syn::Field) {
            if !conditional(&field.attrs) {
                syn::visit::visit_field(self, field);
            }
        }

        fn visit_variant(&mut self, variant: &'ast syn::Variant) {
            if !conditional(&variant.attrs) {
                syn::visit::visit_variant(self, variant);
            }
        }

        fn visit_trait_item(&mut self, item: &'ast syn::TraitItem) {
            use syn::TraitItem;
            if !conditional(attrs!(TraitItem, item; Const, Fn, Type, Macro)) {
                self.scoped(|check| syn::visit::visit_trait_item(check, item));
            }
        }

        fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
            self.function(&item.sig, item.default.as_ref());
        }

        fn visit_local(&mut self, local: &'ast syn::Local) {
            if !conditional(&local.attrs) {
                syn::visit::visit_local(self, local);
            }
            // Neither the initializer nor a let-else's else block sees this binding.
            // A cfg-gated pattern can still shadow later references on some builds.
            if !mod_source::modules::test_only(&local.attrs) {
                self.bind(&local.pat);
            }
        }

        fn visit_expr_let(&mut self, expr: &'ast syn::ExprLet) {
            // A conditional binding starts after its initializer and extends through
            // the rest of the let chain and its successful branch.
            syn::visit::visit_expr_let(self, expr);
            self.bind(&expr.pat);
        }

        fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
            self.scoped(|check| {
                check.visit_expr(&expr.cond);
                check.visit_block(&expr.then_branch);
            });
            if let Some((_, otherwise)) = &expr.else_branch {
                self.visit_expr(otherwise);
            }
        }

        fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
            self.scoped(|check| {
                check.visit_expr(&expr.cond);
                check.visit_block(&expr.body);
            });
        }

        fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
            self.visit_pat(&expr.pat);
            self.visit_expr(&expr.expr);
            self.scoped(|check| {
                check.bind(&expr.pat);
                check.visit_block(&expr.body);
            });
        }

        fn visit_expr_closure(&mut self, expr: &'ast syn::ExprClosure) {
            for pat in &expr.inputs {
                self.visit_pat(pat);
            }
            self.visit_return_type(&expr.output);
            self.scoped(|check| {
                for pat in &expr.inputs {
                    check.bind(pat);
                }
                check.visit_expr(&expr.body);
            });
        }

        fn visit_arm(&mut self, arm: &'ast syn::Arm) {
            if !conditional(&arm.attrs) {
                self.visit_pat(&arm.pat);
                self.scoped(|check| {
                    check.bind(&arm.pat);
                    if let Some((_, guard)) = &arm.guard {
                        check.visit_expr(guard);
                    }
                    check.visit_expr(&arm.body);
                });
            }
        }

        fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
            if !conditional(&field.attrs) {
                syn::visit::visit_field_value(self, field);
            }
        }

        fn visit_stmt_macro(&mut self, stmt: &'ast syn::StmtMacro) {
            if !conditional(&stmt.attrs) {
                syn::visit::visit_stmt_macro(self, stmt);
            }
        }

        fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
            self.path(&expr.path, expr.qself.as_ref(), Namespace::Value);
            syn::visit::visit_expr_path(self, expr);
        }

        fn visit_type_path(&mut self, ty: &'ast syn::TypePath) {
            self.path(&ty.path, ty.qself.as_ref(), Namespace::Type);
            syn::visit::visit_type_path(self, ty);
        }

        fn visit_trait_bound(&mut self, bound: &'ast syn::TraitBound) {
            self.path(&bound.path, None, Namespace::Type);
            syn::visit::visit_trait_bound(self, bound);
        }

        fn visit_expr_struct(&mut self, expr: &'ast syn::ExprStruct) {
            self.path(&expr.path, expr.qself.as_ref(), Namespace::Type);
            syn::visit::visit_expr_struct(self, expr);
        }

        fn visit_pat_struct(&mut self, pat: &'ast syn::PatStruct) {
            self.path(&pat.path, pat.qself.as_ref(), Namespace::Type);
            syn::visit::visit_pat_struct(self, pat);
        }

        fn visit_pat_tuple_struct(&mut self, pat: &'ast syn::PatTupleStruct) {
            self.path(&pat.path, pat.qself.as_ref(), Namespace::Value);
            syn::visit::visit_pat_tuple_struct(self, pat);
        }

        fn visit_item_use(&mut self, item: &'ast ItemUse) {
            if item.leading_colon.is_none() {
                imports(item, |name, parts| {
                    if self.refusal.is_some() {
                        return;
                    }
                    let types = self.binding(&parts, Namespace::Type);
                    // A glob names a module; a single import can name both a type and a value.
                    let values = if name.is_some() {
                        self.binding(&parts, Namespace::Value)
                    } else {
                        Ok(None)
                    };
                    match (types, values) {
                        (Ok(Some(binding)), Ok(None)) | (Ok(None), Ok(Some(binding))) => {
                            self.note(&parts, binding);
                        }
                        (Ok(Some(ty)), Ok(Some(value))) if value.blocked.is_some() => {
                            self.note(&parts, ty);
                        }
                        _ => {}
                    }
                });
            }
        }

        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            // The enclosing body's bindings do not resolve names in this module. Only `crate::`
            // starts from a scope we know, regardless of the inline module's own declarations.
            let was_inline = std::mem::replace(&mut self.in_inline_module, true);
            syn::visit::visit_item_mod(self, item);
            self.in_inline_module = was_inline;
        }
    }
    let mut check = Check {
        walk: Walk {
            source,
            origin: &found.module,
            active: HashSet::new(),
        },
        local: Vec::new(),
        names: Vec::new(),
        in_inline_module: false,
        // A root function copied under its own name can still call itself.
        root_copy: (found.owner.is_none() && !found.module.contains("::"))
            .then(|| unraw(&found.function.sig.ident)),
        refusal: None,
    };
    check.function(&found.function.sig, Some(&found.function.block));
    check.refusal.map_or(Ok(()), Err)
}
