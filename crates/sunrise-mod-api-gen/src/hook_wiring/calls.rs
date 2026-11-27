//! Calls reached by entering the checked function or callback, not merely written inside it.
use super::{Build, source};
use mod_source::index::{Found, Index, ItemId};
use std::{collections::HashSet, fmt, ops::ControlFlow};
use syn::{Block, Expr, FnArg, Item, ItemFn, Pat, Stmt, UseTree, visit::Visit};

pub(crate) enum Callee<'ast> {
    Method(&'ast syn::Ident),
    Function(&'ast syn::Path),
}

impl<'ast> Callee<'ast> {
    pub(super) fn from_expr(expr: &'ast Expr) -> Option<Self> {
        match expr {
            Expr::MethodCall(call) => Some(Self::Method(&call.method)),
            Expr::Call(call) => {
                let Expr::Path(path) = ungroup(&call.func) else {
                    return None;
                };
                if let Some(qualified) = &path.qself
                    && (qualified.as_token.is_none()
                        || qualified.position + 1 != path.path.segments.len()
                        || !matches!(&*qualified.ty, syn::Type::Path(ty) if ty.path.is_ident("Self"))
                        || !call.args.first().is_some_and(|arg| {
                            matches!(ungroup(arg), Expr::Path(expr) if expr.path.is_ident("self"))
                        }))
                {
                    return None;
                }
                Some(Self::Function(&path.path))
            }
            _ => None,
        }
    }
}

pub(super) fn ungroup(expr: &Expr) -> &Expr {
    match expr {
        Expr::Paren(paren) => ungroup(&paren.expr),
        Expr::Group(group) => ungroup(&group.expr),
        _ => expr,
    }
}

fn function_path(expr: &Expr) -> Option<&syn::Path> {
    match ungroup(expr) {
        Expr::Path(path) if path.qself.is_none() => Some(&path.path),
        _ => None,
    }
}

pub(super) fn path_is(path: &syn::Path, expected: &str) -> bool {
    let mut names = expected.split("::");
    path.segments
        .iter()
        .all(|segment| names.next().is_some_and(|name| segment.ident == name))
        && names.next().is_none()
}

/// Authoring's placeholder check only detects whether a call is already written. It does not
/// certify a production route; the independent wiring check below does that.
pub(crate) fn has_call_matching<'ast>(
    block: &'ast Block,
    matches: impl Fn(Callee<'ast>) -> bool,
) -> bool {
    let build = Build::default();
    let mut calls = Calls::new(&build);
    calls.syntax_only = true;
    calls
        .in_block(block)
        .into_iter()
        .filter_map(|call| Callee::from_expr(call.expr))
        .any(matches)
}

struct Scope<'ast> {
    functions: Vec<&'ast ItemFn>,
    imports: Vec<&'ast UseTree>,
    locals: Vec<(&'ast syn::Ident, Option<&'ast Expr>)>,
}

pub(super) struct Invocation<'ast> {
    pub(super) expr: &'ast Expr,
    imports: Vec<&'ast UseTree>,
}

type NoHelpers = fn(&syn::Path, usize, &[&UseTree]) -> bool;

/// The calls in one active cfg assignment. Closures (including stored local closures) each
/// start a new body, entered only when invoked or when passed to a helper that invokes them.
pub(super) struct Calls<'b, 'ast, F> {
    build: &'b Build,
    runs_arg: F,
    scopes: Vec<Scope<'ast>>,
    active_closures: Vec<&'ast Expr>,
    active_functions: Vec<&'ast ItemFn>,
    found: Vec<Invocation<'ast>>,
    syntax_only: bool,
}

impl<'b, 'ast> Calls<'b, 'ast, NoHelpers> {
    fn new(build: &'b Build) -> Self {
        Self::with_helpers(build, |_, _, _| false)
    }
}

impl<'b, 'ast, F: FnMut(&syn::Path, usize, &[&UseTree]) -> bool> Calls<'b, 'ast, F> {
    fn with_helpers(build: &'b Build, runs_arg: F) -> Self {
        Self {
            build,
            runs_arg,
            scopes: Vec::new(),
            active_closures: Vec::new(),
            active_functions: Vec::new(),
            found: Vec::new(),
            syntax_only: false,
        }
    }

    pub(super) fn in_block(mut self, block: &'ast Block) -> Vec<Invocation<'ast>> {
        self.visit_block(block);
        self.found
    }

    fn in_expr(
        mut self,
        expression: &'ast Expr,
        imports: &[&'ast UseTree],
    ) -> Vec<Invocation<'ast>> {
        self.scopes.push(Scope {
            functions: Vec::new(),
            imports: imports.to_vec(),
            locals: Vec::new(),
        });
        self.visit_expr(expression);
        self.found
    }

    fn imports(&self) -> Vec<&'ast UseTree> {
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.imports.iter().copied())
            .collect()
    }

    fn record(&mut self, expr: &'ast Expr) {
        let imports = self.imports();
        self.found.push(Invocation { expr, imports });
    }

    fn local_function(&self, path: &syn::Path) -> Option<&'ast ItemFn> {
        let name = path.get_ident()?;
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| &scope.functions)
            .find(|helper| helper.sig.ident == *name)
            .copied()
    }

    fn constant_condition(&self, expr: &Expr) -> Option<bool> {
        match ungroup(expr) {
            Expr::Macro(expr) if expr.mac.path.is_ident("cfg") => {
                self.build.evaluate(&expr.mac.tokens.to_string())
            }
            Expr::Lit(expr) => match &expr.lit {
                syn::Lit::Bool(value) => Some(value.value),
                _ => None,
            },
            Expr::Unary(expr) if matches!(expr.op, syn::UnOp::Not(_)) => {
                self.constant_condition(&expr.expr).map(|value| !value)
            }
            Expr::Binary(expr) => {
                let short = match expr.op {
                    syn::BinOp::And(_) => false,
                    syn::BinOp::Or(_) => true,
                    _ => return None,
                };
                let left = self.constant_condition(&expr.left);
                if left == Some(short) {
                    left
                } else {
                    let right = self.constant_condition(&expr.right);
                    if left.is_some() || right == Some(short) {
                        right
                    } else {
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn local(&self, path: &syn::Path) -> Option<Option<&'ast Expr>> {
        let name = path.get_ident()?;
        self.scopes
            .iter()
            .rev()
            .flat_map(|scope| scope.locals.iter().rev())
            .find_map(|(local, init)| (*local == name).then_some(*init))
    }

    fn closure(&self, expression: &'ast Expr) -> Option<&'ast Expr> {
        let expression = ungroup(expression);
        let resolved = if let Some(path) = function_path(expression) {
            ungroup(self.local(path)??)
        } else {
            expression
        };
        matches!(resolved, Expr::Closure(_)).then_some(resolved)
    }

    fn invoke(&mut self, expression: &'ast Expr) {
        if let Expr::Closure(closure) = expression
            && self.build.enabled(&closure.attrs)
            && !self
                .active_closures
                .iter()
                .any(|active| std::ptr::eq(*active, expression))
        {
            self.active_closures.push(expression);
            self.visit_expr(&closure.body);
            self.active_closures.pop();
        }
    }

    fn helper_runs(&mut self, path: &syn::Path, argument: usize) -> bool {
        // The crash boundary invokes its closure through a standard-library body outside the index.
        if path_is(path, "sunrise_client::crash::contain")
            || path_is(path, "sunrise_level::crash::contain")
        {
            return argument == 0;
        }
        if let Some(helper) = self.local_function(path) {
            return parameter_called(self.build, helper, argument, |_, _, _| false);
        }
        let imports = self.imports();
        (self.runs_arg)(path, argument, &imports)
    }
}

impl<'ast, F: FnMut(&syn::Path, usize, &[&UseTree]) -> bool> Visit<'ast> for Calls<'_, 'ast, F> {
    fn visit_block(&mut self, block: &'ast Block) {
        let mut scope = Scope {
            functions: Vec::new(),
            imports: Vec::new(),
            locals: Vec::new(),
        };
        for stmt in &block.stmts {
            match stmt {
                Stmt::Item(Item::Fn(item)) if self.build.enabled(&item.attrs) => {
                    scope.functions.push(item)
                }
                Stmt::Item(Item::Use(item)) if self.build.enabled(&item.attrs) => {
                    scope.imports.push(&item.tree)
                }
                _ => (),
            }
        }
        self.scopes.push(scope);
        syn::visit::visit_block(self, block);
        self.scopes.pop();
    }

    fn visit_item(&mut self, _: &'ast Item) {}

    fn visit_local(&mut self, local: &'ast syn::Local) {
        if self.build.enabled(&local.attrs) {
            syn::visit::visit_local(self, local);
            if let Pat::Ident(pat) = &local.pat
                && let Some(scope) = self.scopes.last_mut()
            {
                scope.locals.push((
                    &pat.ident,
                    local.init.as_ref().map(|init| init.expr.as_ref()),
                ));
            }
        }
    }

    fn visit_arm(&mut self, arm: &'ast syn::Arm) {
        if self.build.enabled(&arm.attrs) {
            syn::visit::visit_arm(self, arm);
        }
    }

    fn visit_field_value(&mut self, field: &'ast syn::FieldValue) {
        if self.build.enabled(&field.attrs) {
            syn::visit::visit_field_value(self, field);
        }
    }

    fn visit_expr(&mut self, expr: &'ast Expr) {
        if !self.build.enabled(source::expr_attrs(expr)) {
            return;
        }
        if self.syntax_only {
            if matches!(expr, Expr::Call(_) | Expr::MethodCall(_)) {
                self.record(expr);
            }
        } else {
            match expr {
                Expr::Closure(_) | Expr::Async(_) => return,
                Expr::If(expr) => {
                    self.visit_expr(&expr.cond);
                    let condition = self.constant_condition(&expr.cond);
                    if condition != Some(false) {
                        self.visit_block(&expr.then_branch);
                    }
                    if condition != Some(true)
                        && let Some((_, other)) = &expr.else_branch
                    {
                        self.visit_expr(other);
                    }
                    return;
                }
                Expr::While(expr) => {
                    self.visit_expr(&expr.cond);
                    if self.constant_condition(&expr.cond) != Some(false) {
                        self.visit_block(&expr.body);
                    }
                    return;
                }
                Expr::Binary(expr) if matches!(expr.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) => {
                    self.visit_expr(&expr.left);
                    let short = matches!(expr.op, syn::BinOp::Or(_));
                    if self.constant_condition(&expr.left) != Some(short) {
                        self.visit_expr(&expr.right);
                    }
                    return;
                }
                Expr::Call(call) => {
                    let path = function_path(&call.func);
                    if path.is_none_or(|path| self.local(path).is_none()) {
                        if let Some(helper) = path.and_then(|path| self.local_function(path)) {
                            if !self
                                .active_functions
                                .iter()
                                .any(|active| std::ptr::eq(*active, helper))
                            {
                                self.active_functions.push(helper);
                                self.visit_block(&helper.block);
                                self.active_functions.pop();
                            }
                        } else {
                            self.record(expr);
                        }
                    }
                    if let Some(closure) = self.closure(&call.func) {
                        self.invoke(closure);
                    }
                    if let Some(path) = path
                        && self.local(path).is_none()
                    {
                        for (index, argument) in call.args.iter().enumerate() {
                            if let Some(closure) = self.closure(argument)
                                && self.helper_runs(path, index)
                            {
                                self.invoke(closure);
                            }
                        }
                    }
                }
                Expr::MethodCall(call) => {
                    self.record(expr);
                    // The effect dispatcher filters its parts by cadence. Creating that iterator
                    // does not ask the parts; advancing it runs the predicate.
                    if call.method == "next" {
                        let receiver = function_path(&call.receiver)
                            .and_then(|path| self.local(path).flatten())
                            .unwrap_or(&call.receiver);
                        if let Expr::MethodCall(filter) = ungroup(receiver)
                            && filter.method == "filter"
                            && let Expr::Call(parts) = ungroup(&filter.receiver)
                            && function_path(&parts.func)
                                .is_some_and(|path| path_is(path, "crate::entered_parts"))
                            && let Some(closure) =
                                filter.args.first().and_then(|arg| self.closure(arg))
                        {
                            self.invoke(closure);
                        }
                    }
                }
                _ => {}
            }
        }
        syn::visit::visit_expr(self, expr);
    }
}

fn parameter_called(
    build: &Build,
    helper: &ItemFn,
    argument: usize,
    mut runs_arg: impl FnMut(&syn::Path, usize, &[&UseTree]) -> bool,
) -> bool {
    let Some(FnArg::Typed(arg)) = helper.sig.inputs.iter().nth(argument) else {
        return false;
    };
    let Pat::Ident(pat) = arg.pat.as_ref() else {
        return false;
    };
    let is_parameter =
        |expr: &Expr| function_path(expr).is_some_and(|path| path.is_ident(&pat.ident));
    Calls::new(build).in_block(&helper.block).iter().any(|invocation| {
        let Expr::Call(call) = invocation.expr else {
            return false;
        };
        let Some(path) = function_path(&call.func) else {
            return false;
        };
        if path.is_ident(&pat.ident) {
            return true;
        }
        if path_is(path, "std::panic::catch_unwind") {
            return call.args.first().is_some_and(|arg| {
                is_parameter(arg)
                    || matches!(ungroup(arg), Expr::Call(wrap)
                        if function_path(&wrap.func).is_some_and(|path| path_is(path, "std::panic::AssertUnwindSafe"))
                            && wrap.args.first().is_some_and(is_parameter))
            });
        }
        call.args.iter().enumerate().any(|(i, arg)| {
            is_parameter(arg) && runs_arg(path, i, &invocation.imports)
        })
    })
}

#[derive(Clone, Copy)]
pub(super) enum Target<'a> {
    Path(&'a str),
    Definition {
        package: &'a str,
        module: &'a str,
        name: &'a str,
    },
}

impl fmt::Display for Target<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => f.write_str(path),
            Self::Definition {
                package,
                module,
                name,
            } => write!(f, "{package}::{module}::{name}"),
        }
    }
}

/// Follow invoked functions through the shared resolver.
pub(super) struct Routes<'b, 'i, 's> {
    index: &'i mut Index<'s>,
    build: &'b Build,
}

impl<'b, 'i, 's> Routes<'b, 'i, 's> {
    pub(super) fn new(index: &'i mut Index<'s>, build: &'b Build) -> Self {
        Self { index, build }
    }

    pub(super) fn module(&mut self, package: &str, module: &str) -> Result<usize, String> {
        let path: Vec<_> = module.split("::").map(str::to_owned).collect();
        self.index.module(package, &path)
    }

    pub(super) fn bridge_module(
        &mut self,
        package: &str,
        name: &str,
    ) -> Result<(String, usize), String> {
        let root = self.index.module(package, &[])?;
        match self.index.resolve_path(root, &[name.to_owned()])? {
            Found::Module(scope) => Ok((self.index.modules[scope].krate.clone(), scope)),
            _ => Err(format!("{package}::{name} does not resolve to a module")),
        }
    }

    pub(super) fn functions(&self, scope: usize, name: &str) -> Vec<Block> {
        super::functions(self.index.modules[scope].items.iter(), name, self.build)
            .cloned()
            .collect()
    }

    pub(super) fn installers(&mut self, scope: usize, name: &str) -> Vec<(usize, Block)> {
        let local = self.functions(scope, name);
        if !local.is_empty() {
            return local.into_iter().map(|body| (scope, body)).collect();
        }
        if let Ok(Found::Item(id)) = self.index.resolve_path(scope, &[name.to_owned()])
            && let Item::Fn(installer) = self.index.item(id)
            && self.build.enabled(&installer.attrs)
        {
            return vec![(id.module, (*installer.block).clone())];
        }
        Vec::new()
    }

    pub(super) fn has_slot(&self, scope: usize, table: &str, name: &str) -> bool {
        self.index.modules[scope].items.iter().any(|item| {
            matches!(item, Item::Struct(item) if item.ident == table && self.build.enabled(&item.attrs) && item.fields.iter().any(|field| {
                field.ident.as_ref().is_some_and(|ident| ident == name)
                    && self.build.enabled(&field.attrs)
            }))
        })
    }

    pub(super) fn calls<'r, 'ast>(
        &'r mut self,
        scope: usize,
    ) -> Calls<'r, 'ast, impl FnMut(&syn::Path, usize, &[&UseTree]) -> bool + 'r> {
        Calls::with_helpers(self.build, move |path, argument, imports| {
            self.runs_arg(scope, path, argument, imports, &mut HashSet::new())
        })
    }

    fn runs_arg(
        &mut self,
        scope: usize,
        path: &syn::Path,
        argument: usize,
        imports: &[&UseTree],
        active: &mut HashSet<(ItemId, usize)>,
    ) -> bool {
        let Some(Found::Item(id)) = self.resolve(scope, path, imports) else {
            return false;
        };
        let Item::Fn(helper) = self.index.item(id) else {
            return false;
        };
        if !active.insert((id, argument)) {
            return false;
        }
        let helper = helper.clone();
        let invoked = parameter_called(self.build, &helper, argument, |path, argument, imports| {
            self.runs_arg(id.module, path, argument, imports, active)
        });
        active.remove(&(id, argument));
        invoked
    }

    pub(super) fn block_reaches(
        &mut self,
        scope: usize,
        block: &Block,
        target: Target<'_>,
    ) -> bool {
        let calls = self.calls(scope).in_block(block);
        self.calls_reach(scope, &calls, target)
    }

    fn calls_reach(&mut self, scope: usize, calls: &[Invocation<'_>], target: Target<'_>) -> bool {
        self.visit_routes(
            scope,
            calls,
            target,
            &mut HashSet::new(),
            &mut |_, _, _, _| ControlFlow::Break(()),
        )
        .is_break()
    }

    pub(super) fn callback_reaches(
        &mut self,
        scope: usize,
        imports: &[&UseTree],
        callback: &Expr,
        target: Target<'_>,
    ) -> bool {
        match ungroup(callback) {
            Expr::Path(path) if path.qself.is_none() => {
                if matches!(target, Target::Path(expected) if path_is(&path.path, expected)) {
                    return true;
                }
                let Some(found) = self.resolve(scope, &path.path, imports) else {
                    return false;
                };
                if self.is_target(&found, target) {
                    return true;
                }
                if let Found::Item(id) = found
                    && let Item::Fn(callback) = self.index.item(id)
                {
                    let body = callback.block.clone();
                    return self.block_reaches(id.module, &body, target);
                }
                false
            }
            Expr::Closure(callback) if self.build.enabled(&callback.attrs) => {
                let calls = self.calls(scope).in_expr(&callback.body, imports);
                self.calls_reach(scope, &calls, target)
            }
            _ => false,
        }
    }

    /// Unlike dispatch reachability, every invoked binding is inspected: installers may keep
    /// their first table, so a later wired table cannot excuse an earlier disconnected one.
    pub(super) fn check_bindings(
        &mut self,
        scope: usize,
        block: &Block,
        installer: Target<'_>,
        mut inspect: impl FnMut(&mut Self, usize, &syn::ExprCall, &[&UseTree]) -> Result<(), String>,
    ) -> Result<bool, String> {
        let mut installed = false;
        let calls = self.calls(scope).in_block(block);
        match self.visit_routes(
            scope,
            &calls,
            installer,
            &mut HashSet::new(),
            &mut |routes, scope, call, imports| {
                installed = true;
                match inspect(routes, scope, call, imports) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(error) => ControlFlow::Break(error),
                }
            },
        ) {
            ControlFlow::Break(error) => Err(error),
            ControlFlow::Continue(()) => Ok(installed),
        }
    }

    fn visit_routes<B>(
        &mut self,
        scope: usize,
        calls: &[Invocation<'_>],
        target: Target<'_>,
        seen: &mut HashSet<ItemId>,
        hit: &mut impl FnMut(&mut Self, usize, &syn::ExprCall, &[&UseTree]) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        let paths = calls.iter().filter_map(|invocation| {
            let Expr::Call(call) = invocation.expr else {
                return None;
            };
            Some((
                call,
                function_path(&call.func)?,
                invocation.imports.as_slice(),
            ))
        });
        // Generated dispatch paths don't have source bodies. Try direct calls before following
        // incidental helpers (e.g. data conversion), which might visit much of the engine.
        let is_direct = |path| matches!(target, Target::Path(expected) if path_is(path, expected));
        for (call, _, imports) in paths.clone().filter(|(_, path, _)| is_direct(path)) {
            hit(self, scope, call, imports)?;
        }
        for (call, path, imports) in paths.filter(|(_, path, _)| !is_direct(path)) {
            let Some(found) = self.resolve(scope, path, imports) else {
                continue;
            };
            if self.is_target(&found, target) {
                hit(self, scope, call, imports)?;
            } else if let Found::Item(id) = found
                && seen.insert(id)
                && let Item::Fn(helper) = self.index.item(id)
            {
                let body = helper.block.clone();
                let calls = self.calls(id.module).in_block(&body);
                self.visit_routes(id.module, &calls, target, seen, hit)?;
            }
        }
        ControlFlow::Continue(())
    }

    fn is_target(&self, found: &Found, target: Target<'_>) -> bool {
        match (found, target) {
            (
                Found::Item(id),
                Target::Definition {
                    package,
                    module: expected,
                    name,
                },
            ) => {
                let module = &self.index.modules[id.module];
                module.krate == package
                    && module
                        .path
                        .iter()
                        .map(String::as_str)
                        .eq(expected.split("::"))
                    && matches!(self.index.item(*id), Item::Fn(function) if function.sig.ident == name)
            }
            (Found::External { path, .. }, Target::Path(expected)) => {
                path.iter().map(String::as_str).eq(expected.split("::"))
            }
            _ => false,
        }
    }

    fn resolve(&mut self, scope: usize, path: &syn::Path, imports: &[&UseTree]) -> Option<Found> {
        let path: Vec<_> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        self.index.resolve_with_imports(scope, &path, imports).ok()
    }
}
