//! Generate the typed mod interface from catalog contracts and signatures.
mod catalog;
mod dispatch_fixtures;
mod exports;
mod game_modules;
mod gameplay;
pub use gameplay::{GAMEPLAY_LAYOUT, GameplayAudit, GameplayPackage, RoutingIssue, audit_gameplay};
#[cfg(test)]
pub(crate) use source::LEGACY_LAYOUT;
pub use source::SourceLayout;
mod hook_wiring;
pub use game_modules::materialize_generated_host;
mod hook_fill;
mod host;
#[cfg(test)]
mod layout;
mod parts;
pub use catalog::DECLARATIONS;
use catalog::{Catalog, Contract, DrawContext, Hook, HostAccess, PartKind, PhaseContext, Side};
use heck::ToSnakeCase;
pub use hook_wiring::{check_gameplay_hook_wiring, check_hook_wiring};
use quote::ToTokens;
#[cfg(test)]
use std::path::PathBuf;
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    process::Command,
};
#[cfg(test)]
use syn::ImplItem;
use syn::{FnArg, Item, Pat, Type, visit_mut::VisitMut};

#[cfg(test)]
fn impl_functions(item: syn::ItemImpl) -> impl Iterator<Item = syn::ItemFn> {
    item.items.into_iter().filter_map(|item| match item {
        ImplItem::Fn(method) => Some(syn::ItemFn {
            attrs: method.attrs,
            vis: method.vis,
            sig: method.sig,
            block: Box::new(method.block),
        }),
        _ => None,
    })
}

fn doc_comments(
    attrs: &[syn::Attribute],
    name: &str,
    preserve_empty: bool,
) -> Result<String, String> {
    let mut docs = String::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("doc")) {
        let syn::Meta::NameValue(value) = &attr.meta else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(doc),
            ..
        }) = &value.value
        else {
            return Err(format!("unsupported doc attribute in {name}"));
        };
        let value = doc.value();
        for line in value.split('\n') {
            if preserve_empty || !line.is_empty() {
                docs.push_str(&format!("///{line}\n"));
            }
        }
    }
    Ok(docs)
}

/// The mod API's public `*Ref`, `*Key` and `*Ticket` handles, by their receiver name.
struct ApiTypes {
    handles: BTreeMap<String, String>,
}

fn api_types(source: &str) -> Result<ApiTypes, String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut handles = BTreeMap::new();
    for item in file.items {
        let Item::Struct(item) = item else {
            continue;
        };
        if !matches!(item.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let name = item.ident.to_string();
        let base = name
            .strip_suffix("Ref")
            .map(str::to_owned)
            .or_else(|| (name.ends_with("Key") || name.ends_with("Ticket")).then(|| name.clone()));
        if let Some(base) = base.filter(|base| !base.is_empty()) {
            let receiver = base.to_snake_case();
            if handles.insert(receiver.clone(), name).is_some() {
                return Err(format!("duplicate handle receiver {receiver}"));
            }
        }
    }
    Ok(ApiTypes { handles })
}

fn has_call(block: &syn::Block, expected: &str) -> bool {
    use hook_wiring::Callee;
    hook_wiring::has_call_matching(block, |call| match call {
        Callee::Method(method) => method == expected,
        Callee::Function(path) => path.segments.last().is_some_and(|s| s.ident == expected),
    })
}

fn fresh_name(base: &str, used: &mut HashSet<String>) -> String {
    let mut name = base.to_owned();
    while !used.insert(name.clone()) {
        name.push('_');
    }
    name
}

/// Turns a hook argument's type into its event field's. A `&mut` primitive scalar becomes the value
/// itself, written back after every mod has run; any other `&mut` stays a reference to the caller's
/// value. Either way one event goes through every mod, so each sees the changes before it. Every
/// elided reference lifetime becomes the event's `'a`, or `'b` inside a `&mut`, which can't shorten
/// it.
#[derive(Default)]
struct EventLifetimes {
    a: bool,
    b: bool,
    in_mut: bool,
}

/// The primitive types a `&mut` argument crosses as a copied value — every one of them `Copy`.
pub(crate) const PRIMITIVE_SCALARS: &[&str] = &[
    "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128",
    "usize", "f32", "f64",
];

/// Whether `ty` names one of [`PRIMITIVE_SCALARS`].
pub(crate) fn is_primitive_scalar(ty: &Type) -> bool {
    matches!(ty, Type::Path(path)
        if path.qself.is_none()
            && path
                .path
                .get_ident()
                .is_some_and(|ident| PRIMITIVE_SCALARS.contains(&ident.to_string().as_str())))
}

impl EventLifetimes {
    /// The field's type, and whether it is a scalar copied in and written back.
    fn field(&mut self, ty: &Type) -> (Type, bool) {
        let scalar = match ty {
            Type::Reference(reference)
                if reference.mutability.is_some() && is_primitive_scalar(&reference.elem) =>
            {
                Some(&*reference.elem)
            }
            _ => None,
        };
        if let Some(scalar) = scalar {
            return (scalar.clone(), true);
        }
        let mut field = ty.clone();
        self.visit_type_mut(&mut field);
        (field, false)
    }
}

impl VisitMut for EventLifetimes {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(syn::parse_quote!('_));
        }
        let outer = self.in_mut;
        self.visit_lifetime_mut(reference.lifetime.as_mut().unwrap());
        self.in_mut |= reference.mutability.is_some();
        self.visit_type_mut(&mut reference.elem);
        self.in_mut = outer;
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = if self.in_mut {
                self.b = true;
                syn::parse_quote!('b)
            } else {
                self.a = true;
                syn::parse_quote!('a)
            };
        }
    }
}

impl DrawContext {
    /// A declared borrowed draw trait whose backing game surface is known to the catalog.
    fn of<'a>(contexts: &'a [DrawContext], ty: &Type) -> Option<&'a DrawContext> {
        let Type::Reference(reference) = ty else {
            return None;
        };
        reference.mutability?;
        let Type::TraitObject(object) = &*reference.elem else {
            return None;
        };
        let mut bounds = object.bounds.iter();
        let Some(syn::TypeParamBound::Trait(bound)) = bounds.next() else {
            return None;
        };
        if bounds.next().is_some() {
            return None;
        }
        contexts
            .iter()
            .find(|context| bound.path.is_ident(context.trait_name.as_str()))
    }
    /// The game struct implementing the trait.
    fn game_struct(&self) -> String {
        format!("Game{}", self.trait_name)
    }
}

fn hook_id(name: &str) -> String {
    name.split('_')
        .map(|word| word[..1].to_uppercase() + &word[1..])
        .collect()
}

fn complete_hook_constants(
    code: String,
    trait_name: &str,
    module: &str,
    id: &str,
) -> Result<String, String> {
    let file = syn::parse_file(&code).map_err(|e| e.to_string())?;
    let methods = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Trait(item) if item.ident == trait_name => Some(&item.items),
            _ => None,
        })
        .ok_or("generated hook trait missing")?;
    let existing: HashSet<_> = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Mod(item) if item.ident == module => {
                item.content.as_ref().map(|(_, items)| items)
            }
            _ => None,
        })
        .ok_or("generated hook constants missing")?
        .iter()
        .filter_map(|item| match item {
            Item::Const(item) => Some(item.ident.to_string()),
            _ => None,
        })
        .collect();
    let mut defaults = String::new();
    for method in methods {
        if let syn::TraitItem::Fn(method) = method {
            let name = method.sig.ident.to_string();
            if !existing.contains(&name) {
                defaults.push_str(&format!("pub const {name}: Option<super::{id}> = None;\n"));
            }
        }
    }
    Ok(code.replacen(
        &format!("pub mod {module} {{"),
        &format!("pub mod {module} {{\n{defaults}"),
        1,
    ))
}

/// Serialize hook names and NeoForge counterparts for the port-pack event report.
/// The declaration loader remains the single source of these labels.
pub fn neoforge_catalog(root: &Path) -> Result<String, String> {
    let hooks: Vec<_> = Catalog::load(root)?
        .hooks
        .into_iter()
        .map(|hook| {
            serde_json::json!({
                "name": hook.name,
                "neoforge": hook.counterparts.neoforge.join(" / "),
            })
        })
        .collect();
    serde_json::to_string(&serde_json::json!({ "hooks": hooks })).map_err(|e| e.to_string())
}

/// The Cargo package and module path of a hook target that is a free function outside `Game`.
fn free_function_source(target: &str) -> Option<(&'static str, &str)> {
    let prefixes = [
        ("sunrise_world::", "sunrise-world"),
        ("mod_api::", "sunrise-mod-vocab"),
        ("mod_render::", "sunrise-mod-render"),
        ("sunrise_client_lib::", "sunrise-client-lib"),
        ("sunrise_client_render::", "sunrise-client-render"),
        ("sunrise_sim::", "sunrise-sim"),
    ];
    prefixes
        .into_iter()
        .find_map(|(prefix, krate)| target.strip_prefix(prefix).map(|path| (krate, path)))
}

/// Whether a hook function needs its dispatch `call` filled in: false when it
/// makes the call; true when it is a `Game` or `GameLevel` method whose body is
/// only the placeholder a new hook starts with, `let _ = (every, parameter);`
/// (`let _ = parameter;` for one). Any other body is an error, left alone.
fn needs_fill(method: &syn::ItemFn, call: &str, in_game: bool) -> Result<bool, String> {
    if has_call(&method.block, call)
        || call
            .strip_prefix("dispatch_")
            .is_some_and(|name| has_call(&method.block, &format!("fire_{name}")))
    {
        return Ok(false);
    }
    let params: Vec<String> = method
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(arg) => Some(arg.pat.to_token_stream().to_string()),
            FnArg::Receiver(_) => None,
        })
        .collect();
    let named = match &method.block.stmts[..] {
        [
            syn::Stmt::Local(syn::Local {
                pat: Pat::Wild(_),
                init:
                    Some(syn::LocalInit {
                        expr,
                        diverge: None,
                        ..
                    }),
                ..
            }),
        ] => match &**expr {
            syn::Expr::Tuple(tuple) => Some(
                tuple
                    .elems
                    .iter()
                    .map(|e| e.to_token_stream().to_string())
                    .collect(),
            ),
            single @ syn::Expr::Path(_) => Some(vec![single.to_token_stream().to_string()]),
            _ => None,
        },
        _ => None,
    };
    if in_game && named == Some(params) {
        return Ok(true);
    }
    Err(format!(
        "missing {call} call in {}: add it, or leave a Game hook's body as the let _ = (…); placeholder for cargo run -p xtask -- mod-api to fill",
        method.sig.ident
    ))
}

/// A hook function whose placeholder body becomes `call`.
#[derive(Clone)]
struct Fill {
    native: String,
    call: String,
}

/// `source` with each fill's placeholder statement replaced by its call,
/// formatted as `cargo fmt` would.
#[cfg(test)]
fn filled_hooks(source: &str, fills: &[Fill]) -> Result<Vec<u8>, String> {
    let mut source = source.to_owned();
    for Fill { native, call } in fills {
        let start = source
            .find(&format!("fn {native}("))
            .ok_or_else(|| format!("{native} not found"))?;
        let placeholder = start
            + source[start..]
                .find("let _ =")
                .ok_or_else(|| format!("{native} has no placeholder"))?;
        let end = placeholder
            + source[placeholder..]
                .find(';')
                .ok_or_else(|| format!("{native} has no placeholder"))?
            + 1;
        source.replace_range(placeholder..end, call);
    }
    rustfmt(&source)
}

/// The workspace's edition, which rustfmt formats generated code for.
const EDITION: &str = "2024";

/// `code` as `cargo fmt` formats it.
fn rustfmt(code: &str) -> Result<Vec<u8>, String> {
    let mut child = Command::new("rustfmt")
        .args(["--emit", "stdout", "--edition", EDITION])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    use std::io::Write;
    child
        .stdin
        .take()
        .ok_or("rustfmt stdin")?
        .write_all(code.as_bytes())
        .map_err(|e| e.to_string())?;
    let formatted = child.wait_with_output().map_err(|e| e.to_string())?;
    if !formatted.status.success() {
        return Err("rustfmt failed".into());
    }
    Ok(formatted.stdout)
}

/// The doc of [`BlockPart`], the first part trait in the generated file.
const PART_DOCS: &str = "/// A block's part: code attached through [`Registrar`] to one block, a mod's own or vanilla's.\n/// Its methods are the scoped hooks', asked only for the blocks it is attached to: a scoped\n/// hook has no [`Hooks`] method. They may run on the window's thread or the server thread.\n/// Annotate each part trait implementation with [`sunrise_part`] to derive its subscriptions.\n";

/// The doc of [`EntityPart`], the entity-type part trait whose methods are catalog-owned scoped
/// hooks (`register_goals`, `register_behaviors`, `spawn_cluster_size`, `is_shearable`,
/// `on_sheared`, `general_entity_tick`, `can_rider_interact`).
const ENTITY_PART_DOCS: &str = "/// An entity type's part: code attached through\n/// `Registrar::entity_part` to one entity type, a mod's own or vanilla's\n/// (`minecraft:zombie`). Its methods are the entity-scoped hooks', asked only for the entity\n/// types and hooks the part implements: `register_goals` runs at every `Mob` construction,\n/// `register_behaviors` runs wherever a brain-driven species' brain is built, `spawn_cluster_size`\n/// answers how large a spawn group may grow, `is_shearable` and `on_sheared` handle shearing,\n/// `general_entity_tick` ticks general entity rows, and `can_rider_interact` answers client\n/// rider interaction. Annotate each part trait implementation with [`sunrise_part`] to derive\n/// its subscriptions.\n";

/// The crate whose generated sources are being built.
#[derive(Clone, Copy)]
pub enum Target {
    /// Engine vocabulary: hook and host traits, values, and dispatch.
    Vocab,
    GameplayCommon,
    GameplayServer,
    GameplayClient,
}

/// Fill new hooks' placeholders in `game/native_hooks/`. This explicit authoring command
/// reads game bodies; build scripts and API generation never do.
pub fn fill_hooks(root: &Path) -> Result<(), String> {
    let layout = SourceLayout::read(root)?;
    if !layout.is_split() {
        return Err("hook authoring requires a split gameplay source tree".into());
    }
    hook_fill::fill(root, layout)
}

#[cfg(test)]
/// The hand-written sources, in deterministic order; `fire.rs` includes generated dispatch.
fn native_hook_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = root.join("crates/sunrise-client/src/game/native_hooks");
    let mut paths = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|entry| entry.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.retain(|path| {
        path.extension().is_some_and(|ext| ext == "rs")
            && path.file_name().is_some_and(|name| name != "fire.rs")
    });
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
fn fill_fixture_hooks(root: &Path) -> Result<(), String> {
    let fills: BTreeMap<_, _> = game_hook_sources(root)?
        .fills
        .into_iter()
        .map(|fill| (fill.native.clone(), fill))
        .collect();
    let mut outputs = Vec::new();
    for path in native_hook_files(root)? {
        let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
        let mut local = Vec::new();
        for item in file.items {
            if let Item::Impl(item) = item {
                for method in impl_functions(item) {
                    let name = method.sig.ident.to_string();
                    if let Some(hook) = name.strip_prefix("native_")
                        && let Some(fill) = fills.get(&name)
                        && needs_fill(&method, &format!("dispatch_{hook}"), true)?
                    {
                        local.push(fill.clone());
                    }
                }
            }
        }
        if !local.is_empty() {
            outputs.push((path, filled_hooks(&source, &local)?));
        }
    }
    for (path, output) in outputs {
        fs::write(&path, output).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
fn game_hook_sources(root: &Path) -> Result<dispatch::HookSources, String> {
    let catalog = Catalog::load(root)?;
    dispatch::emit(
        catalog
            .hooks
            .iter()
            .filter(|hook| !dispatch::is_render(hook)),
        &catalog.draw_contexts,
        true,
    )
}

#[cfg(test)]
fn game_fire_source(root: &Path) -> Result<String, String> {
    Ok(dispatch::render_fire(&game_hook_sources(root)?.fire))
}

#[derive(Debug)]
struct Sources {
    hooks: String,
    host: String,
    /// The area modules `host.rs` includes.
    areas: String,
}

/// Generate the vocabulary from declarations without consulting their engine implementations.
fn sources(root: &Path) -> Result<Sources, String> {
    let catalog = Catalog::load(root)?;
    sources_from_catalog(root, &catalog)
}

fn sources_from_catalog(root: &Path, catalog: &Catalog) -> Result<Sources, String> {
    let dispatch::HookSources {
        traits,
        dispatch,
        part_traits,
        hook_ids,
        hook_constants,
        cancel_hooks,
        client_hooks,
        hook_count,
        ..
    } = dispatch::emit(
        catalog
            .hooks
            .iter()
            .filter(|hook| !dispatch::is_render(hook)),
        &catalog.draw_contexts,
        false,
    )?;
    if catalog.hooks.is_empty() {
        return Err(catalog::CatalogError::NoHooks.into());
    }
    let part = |name: &str| part_traits.get(name).map_or("", String::as_str);
    let code = format!(
        "// @generated by cargo run -p xtask -- mod-api; do not edit.\nuse super::*;\n/// A process-local dispatch index, not a serialized or cross-build identity.\n/// SDK records identify hooks by name (`hook:<name>`). Discriminants follow lexical declaration\n/// order and can change between builds; native binaries rebuild for the selected game build.\n#[doc(hidden)]\n#[non_exhaustive]\n#[repr(usize)]\n#[derive(Clone, Copy)]\npub enum HookId {{ {hook_ids} }}\n#[doc(hidden)]\n#[allow(non_upper_case_globals)]\npub mod hook_ids {{ {hook_constants} }}\n#[doc(hidden)]\n#[allow(non_upper_case_globals)]\npub mod cancel_hooks {{ {cancel_hooks} }}\n/// The client-side hooks, by name: what a `side = \"server\"` mod may not subscribe to.\n#[doc(hidden)]\npub const CLIENT_HOOKS: &[(HookId, &str)] = &[{client_hooks}];\n/// Local dispatch metadata from a trait mod's `#[listener]` attribute. A method without one has priority 0 and is not told of cancelled events.\n#[doc(hidden)]\n#[derive(Clone, Copy)]\npub struct Listener {{ pub hook: Option<HookId>, pub priority: i32, pub receive_cancelled: bool }}\npub(crate) const HOOK_COUNT: usize = {hook_count};\n/// The hooks a trait mod implements: one method per catalog hook, each documented from its `doc` field.\n/// Annotate the `impl` with `#[mod_api::sunrise_mod]`, which subscribes the mod to just the methods it implements.\n///\n/// A panicking callback loses its unfinished edits and is disabled for that mod and hook\n/// for the rest of the session. The game names the mod and hook in an error log and,\n/// when a local client is present, an error toast. Ordinary phase and output refusals\n/// are reported every time without disabling the callback.\n///\n/// Later eligible listeners start from the last accepted result. A refused independent\n/// edit does not discard another edit or a veto. Atomic domain proposals still commit\n/// together. Successful cancellations still use listener\n/// priority and `receive_cancelled`, and a first-answer hook still stops at an accepted\n/// answer. Staged registration publishes only completed batches. After a chunk-disk\n/// panic or codec failure, other listeners can run, but the failed job cannot publish.\npub trait Hooks: std::any::Any + Send + Sync {{\n#[doc(hidden)] fn subscribed(&self) -> &'static [Option<HookId>];\n#[doc(hidden)] fn listeners(&self) -> &'static [Listener] {{ &[] }}\n/// Once, at startup: attach this mod's parts (`let shaft = r.block_key(\"mymod:shaft\"); r.block_part(shaft, Shaft)`).\n/// If this callback panics, its pending registrations are discarded and its recorded refusals are logged.\nfn register(&self, r: &mut dyn Registrar, cx: &RegistrationContext) {{ let _ = (r, cx); }}\n{traits}}}\n{PART_DOCS}pub trait BlockPart: Send + Sync + 'static {{\n#[doc(hidden)] fn block_subscribed(&self) -> &'static [Option<HookId>];\n/// Whether this part's `explosion_resistance` answer depends only on the state and the explosion, not on the position or the level around it: then a batched read (`LevelRef::explosion_resistances`) asks it once a state per call instead of once a cell. False unless the part says so; the answer stays a pure function of its inputs either way.\nfn explosion_resistance_is_pure(&self) -> bool {{ false }}\n{}}}\n/// A block's part that also answers for its block entity, and for the block like a [`BlockPart`].\npub trait BlockEntityPart: BlockPart {{\n#[doc(hidden)] fn block_entity_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n/// An item's part.\npub trait ItemPart: Send + Sync + 'static {{\n#[doc(hidden)] fn item_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n/// A menu type's part: for a mod menu type or a stock menu type opened by mod blocks.\npub trait MenuPart: Send + Sync + 'static {{\n#[doc(hidden)] fn menu_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n/// A registered mod effect\'s behaviour: first insertion (`on_effect_added`) and ticking. Each Part keeps its own tick cadence.\npub trait MobEffectPart: Send + Sync + \'static {{\n#[doc(hidden)] fn effect_subscribed(&self) -> &\'static [Option<HookId>];\n{}}}\n/// A mod world fluid\'s part.\npub trait FluidPart: Send + Sync + 'static {{\n#[doc(hidden)] fn fluid_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n{ENTITY_PART_DOCS}pub trait EntityPart: Send + Sync + 'static {{\n#[doc(hidden)] fn entity_subscribed(&self) -> &'static [Option<HookId>];\n{}}}\n{dispatch}",
        part("BlockPart"),
        part("BlockEntityPart"),
        part("ItemPart"),
        part("MenuPart"),
        part("MobEffectPart"),
        part("FluidPart"),
        part("EntityPart"),
    );
    let code = complete_hook_constants(code, "Hooks", "hook_ids", "HookId")?;
    let host = host::generate(root)?;
    let areas = host::areas_source(root)?;
    for code in [&code, &host, &areas] {
        syn::parse_file(code).map_err(|e| format!("generated Rust: {e}"))?;
    }
    Ok(Sources {
        hooks: code,
        host,
        areas,
    })
}

pub mod api_reference;
mod build;
mod capability_keys;
pub mod counterparts;
pub mod crossing;
mod dispatch;
mod host_param;
mod index;

static GENERATED_ON_STDOUT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// For a tool that prints generated text on stdout (`xtask mod-api coverage`): the generator's
/// `cargo:rerun-if-changed` lines, meant for a build script's Cargo, are left out of it.
pub fn generated_on_stdout() {
    GENERATED_ON_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(test)]
thread_local! {
    /// The paths `rerun_if_changed` was given while a test holds the capture.
    static CAPTURED_RERUNS: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

/// Tracks the stager's stamp, `.sunrise-source-staged`, only in a staged copy. Cargo reruns a build
/// script on every build while a tracked path is missing, so tracking an absent stamp rebuilt
/// sunrise-client and every crate above them on each `cargo` invocation of an
/// ordinary checkout. A stamp never appears in a tree that was already built: staging writes it
/// first, into a fresh copy, and refuses a tree that already has one.
pub(crate) fn rerun_if_staged(root: &std::path::Path) {
    let stamp = root.join(".sunrise-source-staged");
    if stamp.is_file() {
        rerun_if_changed(&stamp.display());
    }
}

/// Tells the build script's Cargo to rerun it when `path` changes.
pub(crate) fn rerun_if_changed(path: &impl std::fmt::Display) {
    #[cfg(test)]
    CAPTURED_RERUNS.with_borrow_mut(|capture| {
        if let Some(captured) = capture.as_mut() {
            captured.push(path.to_string());
        }
    });
    if !GENERATED_ON_STDOUT.load(std::sync::atomic::Ordering::Relaxed) {
        println!("cargo:rerun-if-changed={path}");
    }
}
pub mod functions;
pub mod internals;
pub mod internals_view;
pub mod loops;
pub mod notes;
pub mod patch;
pub mod replace;
mod replace_resolve;
#[cfg(test)]
mod replace_resolve_tests;
pub(crate) mod source;
pub mod staging;
pub mod surface;
pub use api_reference::generate as generate_api_reference;
pub use build::{
    generate, generate_api_parts, generate_gameplay_vocab, generate_render, write_if_changed,
};
pub use counterparts::generate as generate_counterparts;
pub use notes::generate as generate_hook_notes;

/// Generate the dispatch test fixture without preventing xtask from filling new placeholders.
#[doc(hidden)]
pub fn generate_test_hooks(root: &Path, out_dir: &Path) -> Result<(), String> {
    build::track(root)?;
    rerun_if_changed(&root.join(source::MARKER).display());
    let catalog = if SourceLayout::read(root)?.is_split() {
        gameplay::Routing::load(root)?.catalog(root)?
    } else {
        Catalog::load(root)?
    };
    dispatch_fixtures::generate(root, out_dir, &catalog)?;
    let code = sources_from_catalog(root, &catalog)?.hooks;
    fs::write(out_dir.join("mod_api_generated.rs"), code).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::{PartKind, PhaseContext};
    use quote::ToTokens;
    #[test]
    fn hooks_keep_their_versions_targets_and_scopes() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let points = super::gameplay::Routing::load(&root)
            .unwrap()
            .catalog(&root)
            .unwrap()
            .hooks;
        let frame = points
            .iter()
            .find(|p| p.name == "render_frame_pre")
            .unwrap();
        assert_eq!(frame.version, 2);
        let post = points
            .iter()
            .find(|p| p.name == "render_frame_post")
            .unwrap();
        assert_eq!(post.version, 2);
        assert_eq!(
            frame.target,
            "sunrise_client_gameplay::game::native_hooks::render_frame_pre::ClientGame<'_>::native_render_frame_pre"
        );
        assert!(frame.scope.is_none());
        let scoped = points
            .iter()
            .find(|p| p.name == "pre_remove_side_effects")
            .unwrap();
        assert_eq!(scoped.scope.as_ref().unwrap().kind, PartKind::BlockEntity);
        assert_eq!(scoped.version, 6);
        for (name, version, context) in [
            (
                "add_client_reload_listeners",
                4,
                Some(PhaseContext::ClientReloadListener),
            ),
            ("add_server_reload_listeners", 4, Some(PhaseContext::Reload)),
            (
                "register_entity_layers",
                6,
                Some(PhaseContext::Registration),
            ),
            (
                "register_entity_renderers",
                5,
                Some(PhaseContext::Registration),
            ),
            ("section_visibility", 3, None),
            ("movement_input_update", 1, None),
        ] {
            let hook = points.iter().find(|hook| hook.name == name).unwrap();
            assert_eq!(hook.version, version, "{name}");
            assert_eq!(hook.context, context, "{name}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn fragment_hooks_generate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = super::Catalog::load(&root).unwrap();
        let hook = catalog
            .hooks
            .iter()
            .find(|h| h.name == "recipes_received")
            .unwrap();
        assert_eq!(hook.contract, super::Contract::Listen);
        let generated = super::sources(&root).unwrap().hooks;
        assert!(generated.contains("fn recipes_received("));
        assert!(generated.contains("/// Side: client."));
    }

    #[test]
    fn generated_locals_do_not_shadow_hook_arguments() {
        let mut used = ["mods", "mods_", "mod_hook", "replacement", "host"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(super::fresh_name("mods", &mut used), "mods__");
        assert_eq!(super::fresh_name("mod_hook", &mut used), "mod_hook_");
        assert_eq!(super::fresh_name("replacement", &mut used), "replacement_");
        assert_eq!(super::fresh_name("host", &mut used), "host_");
    }

    #[test]
    fn level_hosts_match_receiver_mutability() {
        use super::{Hook, HostAccess, Side};
        let level = Hook::sample(
            "sunrise_client::game::GameLevel<'_>::native_test",
            Side::Server,
        );
        let game = Hook::sample("sunrise_client::game::Game<'_>::native_test", Side::Both);
        use super::host_param::{HostArgument::*, HostParam, host_param};
        let param = |ty: &str, argument| {
            Some(HostParam {
                ty: ty.into(),
                argument,
            })
        };
        assert_eq!(
            host_param(&level, Some(false), false),
            param("&dyn LevelHost", Receiver)
        );
        assert_eq!(
            host_param(&level, Some(true), false),
            param("&mut dyn LevelRandomHost", Receiver)
        );
        assert_eq!(
            host_param(&level, Some(false), true),
            param("&mut dyn LevelTicksHost", LevelTicks)
        );
        assert_eq!(
            host_param(&level, Some(true), true),
            param("&mut dyn LevelWriteHost", LevelTicks)
        );
        assert_eq!(host_param(&level, None, false), None);
        let level_both = Hook {
            side: Side::Both,
            ..level.clone()
        };
        assert_eq!(
            host_param(&level_both, Some(false), false),
            param("LevelSide", LevelSide)
        );
        for (mutable, ticks) in [(Some(true), false), (Some(false), true)] {
            assert_eq!(
                super::host_param::validate(&level_both, "test", true, mutable, ticks).unwrap_err(),
                "test: a both-side level hook takes the read kind until a client level seam exists"
            );
        }
        assert_eq!(
            host_param(&game, Some(true), false),
            param("HostSide", GameSide)
        );
        assert_eq!(
            super::host_param::validate(&game, "test", false, Some(false), false).unwrap_err(),
            "test: a both-side hook with a &self target takes host \"read\""
        );
        let read = Hook {
            host: Some(HostAccess::Read),
            ..game.clone()
        };
        assert_eq!(
            host_param(&read, Some(false), false),
            param("&dyn Host", SharedGame)
        );
        let client = Hook {
            side: Side::Client,
            ..game.clone()
        };
        assert_eq!(
            host_param(&client, Some(false), false),
            param("&dyn ClientHost", SharedReborrow)
        );
        assert_eq!(
            host_param(&client, Some(true), false),
            param("&mut dyn ClientHost", Receiver)
        );
        let client_read = Hook {
            side: Side::Client,
            ..read.clone()
        };
        assert_eq!(
            super::host_param::validate(&client_read, "test", false, Some(false), false)
                .unwrap_err(),
            "test: this entry takes no host marker"
        );
        assert_eq!(
            super::host_param::validate(&read, "test", true, Some(false), false).unwrap_err(),
            "test: this entry takes no host marker"
        );
        // A hit's level: another receiver than `GameLevel`, marked as a level host.
        let hit = Hook {
            host: Some(HostAccess::Level),
            ..Hook::sample(
                "sunrise_client::game::native_hooks::HitLevel<'_>::native_test",
                Side::Server,
            )
        };
        assert!(super::host_param::is_level(&hit));
        assert_eq!(
            host_param(&hit, Some(true), false),
            param("&mut dyn LevelRandomHost", Receiver)
        );
        super::host_param::validate(&hit, "test", true, Some(true), false).unwrap();
        // A read-only level view that is neither `GameLevel` nor a hit: a server `&self`
        // receiver reads the level through the same `&dyn LevelHost`.
        let read_level = Hook {
            host: Some(HostAccess::Level),
            ..Hook::sample(
                "sunrise_client::game::native_hooks::block_path_type::ReadLevel<'_>::native_test",
                Side::Server,
            )
        };
        assert!(super::host_param::is_level(&read_level));
        assert_eq!(
            host_param(&read_level, Some(false), false),
            param("&dyn LevelHost", Receiver)
        );
        super::host_param::validate(&read_level, "test", true, Some(false), false).unwrap();
        // The hit capability: a server hit with a `&mut self` receiver only.
        let capable = Hook {
            host: Some(HostAccess::Hit),
            ..hit.clone()
        };
        assert!(super::host_param::is_level(&capable));
        assert_eq!(
            host_param(&capable, Some(true), false),
            param("&mut dyn HitHost", Receiver)
        );
        super::host_param::validate(&capable, "test", true, Some(true), false).unwrap();
        for (hook, mutable, ticks) in [
            (capable.clone(), Some(false), false),
            (capable.clone(), Some(true), true),
            (
                Hook {
                    side: Side::Both,
                    ..capable.clone()
                },
                Some(false),
                false,
            ),
        ] {
            assert_eq!(
                super::host_param::validate(&hook, "test", true, mutable, ticks).unwrap_err(),
                "test: this entry takes no host marker"
            );
        }
        // On both sides a hit's level reads, and its host says which side it is.
        let hit_both = Hook {
            side: Side::Both,
            ..hit.clone()
        };
        super::host_param::validate(&hit_both, "test", true, Some(false), false).unwrap();
        assert_eq!(
            host_param(&hit_both, Some(false), false),
            param("LevelSide", LevelSide)
        );
        assert_eq!(
            super::host_param::validate(&hit_both, "test", true, Some(false), true).unwrap_err(),
            "test: this entry takes no host marker"
        );
        // A read-only level receiver is not tied to the hit capability: any
        // non-`GameLevel` level may take it.
        assert_eq!(
            host_param(&hit, Some(false), false),
            param("&dyn LevelHost", Receiver)
        );
        super::host_param::validate(&hit, "test", true, Some(false), false).unwrap();
        for (hook, mutable) in [
            (
                Hook {
                    side: Side::Both,
                    ..hit.clone()
                },
                Some(true),
            ),
            (
                Hook {
                    host: Some(HostAccess::Level),
                    ..level.clone()
                },
                Some(true),
            ),
        ] {
            assert_eq!(
                super::host_param::validate(&hook, "test", true, mutable, false).unwrap_err(),
                "test: this entry takes no host marker"
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_game_hooks_placeholder_body_is_filled() {
        let fill = |body: &str, in_game: bool| {
            let method: syn::ItemFn = syn::parse_str(&format!(
                "fn native_h(&mut self, player: u128, cancel: &mut bool) {{ {body} }}"
            ))
            .unwrap();
            super::needs_fill(&method, "dispatch_h", in_game)
        };
        assert_eq!(fill("let _ = (player, cancel);", true), Ok(true));
        assert_eq!(
            fill("mod_api::dispatch_h(self, mods(), player, cancel);", true),
            Ok(false)
        );
        for (body, in_game) in [
            ("let _ = (player, cancel);", false),
            ("let _ = (cancel, player);", true),
            ("let _ = player;", true),
            ("let _ = (player, cancel); *cancel = true;", true),
            ("", true),
        ] {
            assert!(
                fill(body, in_game)
                    .unwrap_err()
                    .starts_with("missing dispatch_h call in native_h: "),
                "{body}"
            );
        }
        let one: syn::ItemFn =
            syn::parse_quote! { fn native_one(&self, from: &str) { let _ = from; } };
        assert_eq!(super::needs_fill(&one, "dispatch_one", true), Ok(true));
        let source = "impl Game<'_> {\n    fn native_other(&self, a: u32) {\n        let _ = a;\n    }\n    fn native_h(&mut self, player: u128, name: &str, cancel: &mut bool) {\n        let _ = (player, name, cancel);\n    }\n}\n";
        let filled = super::filled_hooks(
            source,
            &[super::Fill {
                native: "native_h".into(),
                call:
                    "mod_api::dispatch_h(self, mod_api::__private::mods(), player, name, cancel);"
                        .into(),
            }],
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(filled).unwrap(),
            "impl Game<'_> {\n    fn native_other(&self, a: u32) {\n        let _ = a;\n    }\n    fn native_h(&mut self, player: u128, name: &str, cancel: &mut bool) {\n        mod_api::dispatch_h(self, mod_api::__private::mods(), player, name, cancel);\n    }\n}\n"
        );
    }

    /// Vocabulary generation needs no game sources. The wiring check refuses an unfinished
    /// pickup hook; the authoring command fills its placeholder without changing the vocabulary.
    #[test]
    #[ignore = "daily"]
    fn new_hook_placeholder_is_reported_then_filled() {
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            if from.is_dir() {
                for entry in std::fs::read_dir(from).unwrap() {
                    let entry = entry.unwrap();
                    copy(&entry.path(), &to.join(entry.file_name()));
                }
            } else {
                std::fs::create_dir_all(to.parent().unwrap()).unwrap();
                std::fs::copy(from, to).unwrap();
            }
        }
        let real = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let root = real
            .join("target")
            .join(format!("mod-api-fill-{}", std::process::id()));
        let vocabulary = "crates/sunrise-mod-vocab/src";
        copy(&real.join(vocabulary), &root.join(vocabulary));
        // Find hook declarations in the vocabulary-only checkout.
        let catalog = super::Catalog::load(&real).unwrap();
        let declaring = |name: &str| {
            std::fs::read_dir(root.join(super::catalog::DECLARATIONS))
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .find(|path| {
                    std::fs::read_to_string(path)
                        .unwrap()
                        .contains(&format!("fn {name}("))
                })
                .unwrap()
        };
        let first = declaring(&catalog.hooks[0].name);
        let original = std::fs::read_to_string(&first).unwrap();
        let without_counterparts = super::sources(&root).unwrap().hooks;
        std::fs::write(
            &first,
            original.replacen(
                "#[hook(",
                "#[hook(fabric = [\"Example.ONE\", \"Example.TWO\"], paper = \"ExampleEvent\", ",
                1,
            ),
        )
        .unwrap();
        assert_eq!(super::sources(&root).unwrap().hooks, without_counterparts);
        std::fs::write(&first, original).unwrap();
        let hook_dir = root.join("crates/sunrise-client/src/game/native_hooks");
        std::fs::create_dir_all(&hook_dir).unwrap();
        for (package, file) in [
            ("sunrise-gameplay", "item_picked_up.rs"),
            ("sunrise-client-gameplay", "client_tick_pre.rs"),
        ] {
            let source = std::fs::read_to_string(
                real.join(format!("crates/{package}/src/game/native_hooks/{file}")),
            )
            .unwrap();
            std::fs::write(hook_dir.join(file), super::rustfmt(&source).unwrap()).unwrap();
        }
        std::fs::write(
            hook_dir.join("mod.rs"),
            "mod item_picked_up; mod client_tick_pre;",
        )
        .unwrap();
        let client = root.join("crates/sunrise-client");
        std::fs::write(client.join("Cargo.toml"), "").unwrap();
        std::fs::write(client.join("src/lib.rs"), "mod game;").unwrap();
        std::fs::write(client.join("src/game.rs"), "mod native_hooks;").unwrap();
        let hooks = hook_dir.join("item_picked_up.rs");
        let mut pickup = catalog
            .hooks
            .iter()
            .find(|hook| hook.name == "item_picked_up")
            .unwrap()
            .clone();
        pickup.target = pickup
            .target
            .replacen("sunrise_gameplay::", "sunrise_client::", 1);
        let shipped = std::fs::read_to_string(&hooks).unwrap();
        let call = "self.fire_item_picked_up(player, item, can_pickup);";
        assert_eq!(shipped.matches(call).count(), 1);
        let unfinished = shipped.replace(call, "let _ = (player, item);");
        std::fs::write(&hooks, &unfinished).unwrap();
        assert!(
            super::fill_fixture_hooks(&root)
                .unwrap_err()
                .starts_with("missing dispatch_item_picked_up call in native_item_picked_up: ")
        );
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), unfinished);
        let placeholder = shipped.replace(call, "let _ = (player, item, can_pickup);");
        std::fs::write(&hooks, &placeholder).unwrap();
        assert_eq!(
            super::hook_wiring::check(&root, std::slice::from_ref(&pickup)).unwrap_err(),
            concat!(
                "item_picked_up (sunrise_client::game::native_hooks::item_picked_up::ServerGame<'_>::native_item_picked_up): ",
                "target does not call fire_item_picked_up or Self::fire_item_picked_up or ",
                "super::fire::fire_item_picked_up or fire::fire_item_picked_up or ",
                "crate::game::native_hooks::fire::fire_item_picked_up",
            )
        );
        assert_eq!(std::fs::read_to_string(&hooks).unwrap(), placeholder);
        // A hook without its attribute fails by name.
        let declared = declaring("item_picked_up");
        let shipped_declarations = std::fs::read_to_string(&declared).unwrap();
        let declaration = shipped_declarations.find("fn item_picked_up(").unwrap();
        let attribute = shipped_declarations[..declaration]
            .rfind("#[hook(")
            .unwrap();
        std::fs::write(
            &declared,
            format!(
                "{}{}",
                &shipped_declarations[..attribute],
                &shipped_declarations[declaration..]
            ),
        )
        .unwrap();
        assert!(
            super::fill_fixture_hooks(&root)
                .unwrap_err()
                .contains("item_picked_up: needs a #[hook(...)] attribute")
        );
        std::fs::write(&declared, shipped_declarations).unwrap();
        let second = hook_dir.join("client_tick_pre.rs");
        let second_shipped = std::fs::read_to_string(&second).unwrap();
        std::fs::write(
            &second,
            second_shipped.replace("self.fire_client_tick_pre();", "let _ = ();"),
        )
        .unwrap();
        let decoy = hooks.with_file_name("decoy.rs");
        let decoy_source = "impl Game<'_> { /// mentions fn native_item_picked_up(\n fn unrelated(&self) { let _ = self; } }\n";
        std::fs::write(&decoy, decoy_source).unwrap();
        super::fill_fixture_hooks(&root).unwrap();
        super::hook_wiring::check(&root, std::slice::from_ref(&pickup)).unwrap();
        // The generator writes LF; a Windows checkout (core.autocrlf) has CRLF.
        let lf = |s: &str| s.replace("\r\n", "\n");
        assert_eq!(
            lf(&std::fs::read_to_string(&second).unwrap()),
            lf(&second_shipped)
        );
        assert_eq!(std::fs::read_to_string(decoy).unwrap(), decoy_source);
        assert_eq!(lf(&std::fs::read_to_string(&hooks).unwrap()), lf(&shipped));
        assert_eq!(
            super::sources(&root).unwrap().hooks,
            super::sources(&real).unwrap().hooks,
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn event_fields_name_their_lifetimes() {
        for (ty, field, scalar, a, b) in [
            (
                "& mut WorldDrawn < '_ >",
                "& 'a mut WorldDrawn < 'b >",
                false,
                true,
                true,
            ),
            ("& str", "& 'a str", false, true, false),
            (
                "& mut Vec < (& 'static str , u32) >",
                "& 'a mut Vec < (& 'static str , u32) >",
                false,
                true,
                false,
            ),
            (
                "Option < & ItemStack >",
                "Option < & 'a ItemStack >",
                false,
                true,
                false,
            ),
            ("u32", "u32", false, false, false),
            ("& mut f32", "f32", true, false, false),
            (
                "& mut Option < bool >",
                "& 'a mut Option < bool >",
                false,
                true,
                false,
            ),
        ] {
            let mut lifetimes = super::EventLifetimes::default();
            let (field_type, is_scalar) = lifetimes.field(&syn::parse_str(ty).unwrap());
            assert_eq!(field_type.to_token_stream().to_string(), field);
            assert_eq!((is_scalar, lifetimes.a, lifetimes.b), (scalar, a, b));
        }
    }

    #[test]
    fn library_crate_targets_resolve_to_their_source() {
        assert_eq!(
            super::free_function_source("sunrise_client_lib::menu::native_menu_slot_rules"),
            Some(("sunrise-client-lib", "menu::native_menu_slot_rules"))
        );
        assert_eq!(
            super::free_function_source("sunrise_client_lib::breathing::native_living_drown"),
            Some(("sunrise-client-lib", "breathing::native_living_drown"))
        );
        assert_eq!(
            super::free_function_source(
                "sunrise_client_render::sound_engine::hooks::native_play_sound"
            ),
            Some((
                "sunrise-client-render",
                "sound_engine::hooks::native_play_sound"
            ))
        );
        for target in [
            "sunrise_client::menu::native_menu_slot_rules",
            "sunrise_client::breathing::native_living_drown",
        ] {
            assert_eq!(super::free_function_source(target), None);
        }
        assert_eq!(
            super::free_function_source("sunrise_client_lib::crafting::native_ingredient"),
            Some(("sunrise-client-lib", "crafting::native_ingredient"))
        );
        assert_eq!(
            super::free_function_source("sunrise_sim::breathing::native_living_drown"),
            Some(("sunrise-sim", "breathing::native_living_drown"))
        );
        assert_eq!(
            super::free_function_source("sunrise_sim::crafting::native_hooks::native_ingredient"),
            Some(("sunrise-sim", "crafting::native_hooks::native_ingredient"))
        );
        assert_eq!(
            super::free_function_source(
                "sunrise_client::game::native_hooks::Game<'_>::native_compute_fov"
            ),
            None
        );
    }

    #[test]
    fn contracted_hook_requires_dispatch_call() {
        let missing: syn::ImplItemFn =
            syn::parse_quote! { fn native_compute_fov(&self) { unrelated(); } };
        let present: syn::ImplItemFn =
            syn::parse_quote! { fn native_compute_fov(&self) { mod_api::dispatch_compute_fov(); } };
        assert!(!super::has_call(&missing.block, "dispatch_compute_fov"));
        assert!(super::has_call(&present.block, "dispatch_compute_fov"));
        let generated: syn::ItemFn = syn::parse_quote! {
            fn native_compute_fov(&self) { self.fire_compute_fov(); }
        };
        assert_eq!(
            super::needs_fill(&generated, "dispatch_compute_fov", true),
            Ok(false)
        );
    }
}
