//! The hook catalog as typed data: the declaration files under `crates/sunrise-mod-vocab/src/hooks`, read once.
//!
//! The loader here is the only code that reads the declarations. Every generated output
//! (the `Hooks` dispatch, the Host traits, the game's entry points, the rustdoc, the mixin hook
//! names) reads these types.

#[cfg(test)]
mod collect_validation_tests;

mod gameplay;
pub(crate) use gameplay::{Binding as GameplayBinding, BindingKind};

use quote::ToTokens;
use serde::Deserialize;
use std::{collections::HashSet, fmt, fs, path::Path};

/// Which side of the connection a hook or helper runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Server,
    Client,
    Both,
}

/// What a hook's listeners may do with its event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Contract {
    /// Told about a moment; nothing comes back.
    Listen,
    /// May veto it; once cancelled, later listeners run only if they receive cancelled events,
    /// and those may undo it.
    Cancel,
    /// Every listener adds to, changes or removes from one output.
    Collect,
    /// The first listener to answer sets the output.
    Replace,
}

/// The host a hook takes when its receiver does not say which.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAccess {
    /// The shared read-only `Host`.
    Read,
    /// A level host, for a receiver that is a level but not `GameLevel`: read-only on `&self`,
    /// with the random host on `&mut self`.
    Level,
    /// A mutable random level on the server and a read-only level on the client.
    LevelRandomSide,
    /// A hit's host: a level host that can also play level sounds and draw from the
    /// random of the entities the hit lends (`HitHost`). Server hooks only.
    Hit,
}

/// A hook with a restricted phase context instead of a game or level host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseContext {
    Registration,
    DataLoad,
    /// A hook fired inside a menu, recipe or player-list loop of one side.
    Loop,
    /// A hook fired while a feature generates, on a worldgen worker or the server thread.
    Worldgen,
    /// A chunk disk job, with no live world authority.
    ChunkDisk,
    /// A configuration registration or loading hook.
    Config,
    /// A server or shared data-reload hook.
    Reload,
    /// A client resource-reload hook.
    ClientReload,
    /// The client reload-listener registration hook (`add_client_reload_listeners`).
    ClientReloadListener,
    /// The client setup hook after initial resources load.
    ClientSetup,
    /// Initial client configuration, with only the join target available.
    ClientIdentity,
    /// Repository discovery before any world is opened.
    PackDiscovery,
    /// An item ability check from gameplay (`Host` + tags) or a loot predicate (tags + `LootParamsView`).
    ItemAbility,
}

/// How a game hook borrows its receiver. Free dispatchers have no game receiver.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Receiver {
    Read,
    #[default]
    Write,
    Associated,
}

impl Receiver {
    pub(super) fn mutability(self) -> Option<bool> {
        match self {
            Self::Read => Some(false),
            Self::Write => Some(true),
            Self::Associated => None,
        }
    }
}

/// How a game target calls generated dispatch. This never changes the listener event.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStyle {
    /// The target takes the generated engine carrier.
    #[default]
    Carrier,
    /// The target already owns the converted values and takes the vocabulary carrier.
    ApiCarrier,
    /// The target takes individual engine parameters.
    Flat,
    /// The target owns the whole crossing and calls the vocabulary dispatcher itself.
    Direct,
}

pub(super) const EFFECT_TICK_TARGET: &str = "mod_api::parts::effects::dispatch_mob_effect_tick";

/// The kind of thing whose Parts a scoped hook calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Block,
    BlockEntity,
    Item,
    Menu,
    Effect,
    Fluid,
    Entity,
}

/// A scoped hook: a Part method, called on the Parts attached to what `key` names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartScope {
    pub kind: PartKind,
    /// The hook argument naming the thing whose Parts are called.
    pub key: String,
}

/// The names a hook has in other mod APIs, for the rustdoc and for finding gaps.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counterparts {
    pub neoforge: Vec<String>,
    pub fabric: Vec<String>,
    pub paper: Vec<String>,
}

/// Count admission for stack-list outputs. Trimming is deferred to placement.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StackCount {
    Trim,
}

/// How a collector exposes its append-only output.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationAppend {
    /// Generate an event method that stages one entry.
    Register,
    /// The vocabulary crate supplies the event methods in an inherent impl.
    Custom,
    /// The vocabulary crate supplies attributed append-only methods in an inherent impl;
    /// dispatch sets `pub(super) mod_id: &'static str` on the event before each listener.
    Attributed,
}

/// When an individual listener gets its own server journal.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ListenerTransaction {
    Always,
    WhenOpen,
}

/// One hook point: a moment the game tells mods about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hook {
    pub name: String,
    /// The rustdoc of its trait method.
    pub doc: String,
    /// The prose of `docs/modding/src/reference/hook-notes.md` for this hook, heading included.
    pub notes: Option<String>,
    pub side: Side,
    /// The game function that fires it.
    pub target: String,
    pub receiver_type: Option<String>,
    pub(crate) default_module: Option<String>,
    pub contract: Contract,
    /// The event field a cancel, collect or replace hook writes; `None` for a listener.
    pub out: Option<String>,
    /// A replace hook whose output also names the mod that answered, or a collect hook whose
    /// entries each name the mod that pushed it.
    pub answered_by_mod: bool,
    /// A listener about one mod's own things: the event field naming that mod, whose
    /// subscription alone is told.
    pub owner: Option<String>,
    /// Hide the output vector of a collector behind append-only methods.
    pub registration_append: Option<RegistrationAppend>,
    /// Mutable registrar field, or `host` for a direct listener whose owner journals its writes.
    pub staged_surface: Option<String>,
    /// The inert answer for a disabled content owner's subscribed block Part: Rust statements
    /// setting the answer, or `signal` to report engine inactivity separately from listener output.
    pub inert: Option<String>,
    /// A global cancel hook's per-listener check of the outputs it wrote: a path to
    /// `fn(&accepted, ..., &proposed, ...) -> Result<(), &'static str>` taking each set of
    /// vector/scalar outputs in declaration order and naming the first refused one. A refused
    /// result, including its cancellation, rolls back; later listeners still run.
    pub output_validator: Option<String>,
    pub scope: Option<PartScope>,
    /// Additional Part trait this global hook also dispatches to (same `HookId` and event).
    pub part: Option<PartKind>,
    pub host: Option<HostAccess>,
    pub context: Option<PhaseContext>,
    pub counterparts: Counterparts,
    /// The hook's contract version.
    pub version: u64,
    /// Whether the NeoForge counterpart event is cancellable, written by the
    /// stamper from the counterpart's javadoc; documentation metadata, not the
    /// contract (a hook that listens can still name a cancellable event).
    pub cancellable: Option<bool>,
    /// Dispatch parameters (name and vocabulary type in order), apart from the Host/phase context.
    pub call: Vec<(String, String)>,
    pub receiver: Receiver,
    pub dispatch: DispatchStyle,
    /// The engine passes its tick queue to the level host, not to the listener.
    pub engine_ticks: bool,
    /// Whether dispatch signals an inactive content owner to the engine.
    pub engine_inert: bool,
    /// Admission policy for the mutable stack-list output; placement owns trimming.
    pub stack_count: Option<StackCount>,
    /// Rebuild a listener event from its original inputs for every subscriber.
    pub event_per_listener: bool,
    /// Undo a failing listener's writes before the dispatcher contains its unwind.
    pub listener_transaction: Option<ListenerTransaction>,
    /// A setter callback skips its own reentry. A fault quarantines that subscriber without
    /// aborting an enclosing action that called the setter.
    pub independent_listener: bool,
}

/// A drawing surface of the game's that drawing hooks get as a trait object (the catalog's
/// `draw_contexts`): a hook argument of the surface's type is the trait object, declared in
/// `host/draw.rs`, in the hook's event.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DrawContext {
    /// The game's type (`crate::hud::HudBatch`).
    pub surface: String,
    /// The trait mods draw through.
    #[serde(rename = "trait")]
    pub trait_name: String,
}

/// Why the catalog was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum CatalogError {
    Read { path: String, error: String },
    Declaration { path: String, error: String },
    NamelessHook,
    DuplicateHook(String),
    InvalidHook { hook: String, error: String },
    MissingDoc(String),
    Counterpart { hook: String, key: &'static str },
    InvalidContract(String),
    AnsweredNeedsGlobalOutput(String),
    OwnerNeedsGlobalListener(String),
    ScopeNeedsKey(String),
    NoHooks,
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, error } => write!(f, "{path}: {error}"),
            Self::Declaration { path, error } => write!(f, "{path}: {error}"),
            Self::NamelessHook => f.write_str("hook name missing"),
            Self::DuplicateHook(name) => write!(f, "duplicate hook {name}"),
            Self::InvalidHook { hook, error } => write!(f, "hook {hook}: {error}"),
            Self::MissingDoc(name) => write!(
                f,
                "hook {name}: doc missing: add `///` lines, the rustdoc of its trait method"
            ),
            Self::Counterpart { hook, key } => {
                write!(f, "hook {hook}: {key} must be a string or list of strings")
            }
            Self::InvalidContract(name) => write!(f, "invalid contract for {name}"),
            Self::AnsweredNeedsGlobalOutput(name) => {
                write!(
                    f,
                    "{name}: answered_by needs a global replace or collect hook"
                )
            }
            Self::OwnerNeedsGlobalListener(name) => {
                write!(f, "{name}: owner needs a global listener")
            }
            Self::ScopeNeedsKey(name) => write!(f, "{name} needs both scope and key, or neither"),
            Self::NoHooks => f.write_str("no native hooks"),
        }
    }
}

impl std::error::Error for CatalogError {}

impl From<CatalogError> for String {
    fn from(error: CatalogError) -> Self {
        error.to_string()
    }
}

/// A counterpart name or a list of them.
#[derive(Deserialize)]
#[serde(untagged)]
enum Names {
    One(String),
    Many(Vec<String>),
}

fn counterparts(
    hook: &str,
    key: &'static str,
    names: Option<serde_json::Value>,
) -> Result<Vec<String>, CatalogError> {
    let invalid = || CatalogError::Counterpart {
        hook: hook.to_owned(),
        key,
    };
    match names {
        None => Ok(Vec::new()),
        Some(value) => match serde_json::from_value(value).map_err(|_| invalid())? {
            Names::One(name) => Ok(vec![name]),
            Names::Many(names) => Ok(names),
        },
    }
}

/// A hook entry as the JSON spells it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HookEntry {
    #[serde(rename = "name")]
    _name: String,
    doc: Option<String>,
    notes: Option<String>,
    side: Side,
    target: String,
    receiver_type: Option<String>,
    default_module: Option<String>,
    mod_api: ModApiEntry,
    version: Option<u64>,
    cancellable: Option<bool>,
    host: Option<HostAccess>,
    context: Option<PhaseContext>,
    #[serde(default)]
    call: Vec<(String, String)>,
    #[serde(default)]
    receiver: Receiver,
    #[serde(default)]
    dispatch: DispatchStyle,
    #[serde(default)]
    engine: Vec<(String, String)>,
    stack_count: Option<StackCount>,
    #[serde(default)]
    event_per_listener: bool,
    listener_transaction: Option<ListenerTransaction>,
    #[serde(default)]
    independent_listener: bool,
    inert: Option<String>,
    neoforge: Option<serde_json::Value>,
    fabric: Option<serde_json::Value>,
    paper: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModApiEntry {
    contract: Contract,
    out: Option<String>,
    scope: Option<PartKind>,
    key: Option<String>,
    part: Option<PartKind>,
    answered_by: Option<AnsweredBy>,
    owner: Option<String>,
    registration_append: Option<RegistrationAppend>,
    staged_surface: Option<String>,
    output_validator: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum AnsweredBy {
    Mod,
}

impl Hook {
    fn parse(value: serde_json::Value) -> Result<Self, CatalogError> {
        let name = value
            .get("name")
            .and_then(serde_json::Value::as_str)
            .ok_or(CatalogError::NamelessHook)?
            .to_owned();
        let entry: HookEntry =
            serde_json::from_value(value).map_err(|error| CatalogError::InvalidHook {
                hook: name.clone(),
                error: error.to_string(),
            })?;
        let doc = entry
            .doc
            .filter(|doc| !doc.trim().is_empty())
            .ok_or_else(|| CatalogError::MissingDoc(name.clone()))?;
        let api = entry.mod_api;
        let scope = match (api.scope, api.key) {
            (None, None) => None,
            (Some(kind), Some(key)) => Some(PartScope { kind, key }),
            _ => return Err(CatalogError::ScopeNeedsKey(name)),
        };
        if let Some(part_kind) = api.part {
            if part_kind != PartKind::Item {
                let kind_name = match part_kind {
                    PartKind::Block => "block",
                    PartKind::BlockEntity => "block_entity",
                    PartKind::Item => "item",
                    PartKind::Menu => "menu",
                    PartKind::Effect => "effect",
                    PartKind::Fluid => "fluid",
                    PartKind::Entity => "entity",
                };
                return Err(CatalogError::InvalidHook {
                    hook: name,
                    error: format!(
                        "part = \"{kind_name}\" is not supported; only part = \"item\" is implemented"
                    ),
                });
            }
            if scope.is_some()
                || api.contract != Contract::Cancel
                || api.answered_by.is_some()
                || api.owner.is_some()
                || api.output_validator.is_some()
                || entry.context.is_some()
                || entry.event_per_listener
                || entry.independent_listener
            {
                return Err(CatalogError::InvalidHook {
                    hook: name,
                    error:
                        "part requires a plain global cancel hook without scope or phase context"
                            .into(),
                });
            }
        }
        if matches!(
            (api.contract, &api.out),
            (Contract::Listen, Some(_))
                | (
                    Contract::Cancel | Contract::Collect | Contract::Replace,
                    None
                )
        ) {
            return Err(CatalogError::InvalidContract(name));
        }
        if let Some(inert) = &entry.inert
            && (!scope
                .as_ref()
                .is_some_and(|scope| scope.kind == PartKind::Block)
                || !matches!(api.contract, Contract::Cancel | Contract::Replace)
                || (inert != "signal"
                    && syn::parse_str::<syn::Block>(&format!("{{ {inert} }}")).is_err()))
        {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error:
                    "inert requires a block-scoped cancel/replace hook and Rust statements or `signal`"
                        .into(),
            });
        }
        if entry.stack_count.is_some() {
            if api.contract != Contract::Collect || api.out.is_none() || api.answered_by.is_some() {
                return Err(CatalogError::InvalidHook {
                    hook: name,
                    error: "stack_count requires a mutable Vec<ItemStack> collect output".into(),
                });
            }
            let output = entry
                .call
                .iter()
                .find(|(parameter, _)| Some(parameter) == api.out.as_ref())
                .and_then(|(_, ty)| syn::parse_str::<syn::Type>(ty).ok());
            validate_stack_count_output(&name, output.as_ref())?;
        }
        let effect = scope
            .as_ref()
            .is_some_and(|scope| scope.kind == PartKind::Effect);
        let is_effect_tick = effect
            && matches!(
                name.as_str(),
                "apply_effect_tick" | "should_apply_effect_tick_this_tick"
            );
        if is_effect_tick != (entry.target == EFFECT_TICK_TARGET) {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error: format!("effect tick hooks must target {EFFECT_TICK_TARGET}"),
            });
        }
        let mut engine_ticks = false;
        let mut engine_inert = false;
        let mut effect_factory = false;
        let mut effect_count = false;
        for (param, ty) in &entry.engine {
            match (param.as_str(), ty.as_str()) {
                ("ticks", "& mut Ticks") if !is_effect_tick => engine_ticks = true,
                ("inert", "& mut bool") if !is_effect_tick => engine_inert = true,
                ("tick_count", "i32") if name == "apply_effect_tick" && is_effect_tick => {
                    effect_count = true
                }
                ("make_call", "impl FnOnce (EffectKey , i32) -> ApplyEffectTickCall < '_ >")
                    if is_effect_tick =>
                {
                    effect_factory = true
                }
                _ => {
                    return Err(CatalogError::InvalidHook {
                        hook: name,
                        error: format!("unsupported #[engine] parameter: {param}: {ty}"),
                    });
                }
            }
        }
        if engine_inert != (entry.inert.as_deref() == Some("signal")) {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error: "inert = \"signal\" requires #[engine] inert: &mut bool and vice versa"
                    .into(),
            });
        }
        if is_effect_tick && (!effect_factory || effect_count != (name == "apply_effect_tick")) {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error:
                    "effect dispatch requires the engine-only factory and an action-only tick count"
                        .into(),
            });
        }
        let answered_by_mod = api.answered_by.is_some();
        if answered_by_mod
            && (!matches!(api.contract, Contract::Replace | Contract::Collect) || scope.is_some())
        {
            return Err(CatalogError::AnsweredNeedsGlobalOutput(name));
        }
        if api.owner.is_some() && (api.contract != Contract::Listen || scope.is_some()) {
            return Err(CatalogError::OwnerNeedsGlobalListener(name));
        }
        if let Some(field) = &api.staged_surface
            && (api.contract != Contract::Listen
                || api.owner.as_ref() == Some(field)
                || scope.as_ref().is_some_and(|scope| scope.key == *field))
        {
            return Err(CatalogError::InvalidHook {
                hook: name,
                error:
                    "staged_surface requires a listener and cannot be its owner filter or Part key"
                        .into(),
            });
        }
        if api.registration_append == Some(RegistrationAppend::Attributed)
            && (api.contract != Contract::Collect || scope.is_some() || answered_by_mod)
        {
            return Err(CatalogError::InvalidHook {
                hook: name.clone(),
                error:
                    "registration_append = \"attributed\" requires a global non-answered collect hook"
                        .into(),
            });
        }
        if let Some(validator) = &api.output_validator
            && (!matches!(
                api.contract,
                Contract::Cancel | Contract::Listen | Contract::Collect
            ) || (api.contract == Contract::Collect
                && (answered_by_mod || api.registration_append.is_some()))
                || scope.is_some()
                || entry.event_per_listener
                || (api.contract == Contract::Listen
                    && (api.staged_surface.is_some()
                        || api.owner.is_some()
                        || entry.context.is_some()))
                || syn::parse_str::<syn::Path>(validator).is_err())
        {
            return Err(CatalogError::InvalidHook {
                hook: name.clone(),
                error: "output_validator is a path on a global cancel, plain global collect or plain global listen hook with shared outputs"
                    .into(),
            });
        }
        let counterparts = Counterparts {
            neoforge: counterparts(&name, "neoforge", entry.neoforge)?,
            fabric: counterparts(&name, "fabric", entry.fabric)?,
            paper: counterparts(&name, "paper", entry.paper)?,
        };
        Ok(Self {
            name,
            doc,
            notes: entry.notes,
            side: entry.side,
            target: entry.target,
            receiver_type: entry.receiver_type,
            default_module: entry.default_module,
            contract: api.contract,
            out: api.out,
            answered_by_mod,
            owner: api.owner,
            registration_append: api.registration_append,
            staged_surface: api.staged_surface,
            inert: entry.inert,
            output_validator: api.output_validator,
            scope,
            part: api.part,
            host: entry.host,
            context: entry.context,
            counterparts,
            version: entry.version.unwrap_or(1),
            cancellable: entry.cancellable,
            call: entry.call,
            receiver: entry.receiver,
            dispatch: entry.dispatch,
            engine_ticks,
            engine_inert,
            stack_count: entry.stack_count,
            event_per_listener: entry.event_per_listener,
            listener_transaction: entry.listener_transaction,
            independent_listener: entry.independent_listener,
        })
    }
}

/// Validate a declared or resolved native output at the catalog boundary.
pub(crate) fn validate_stack_count_output(
    name: &str,
    output: Option<&syn::Type>,
) -> Result<(), CatalogError> {
    let valid = (|| {
        let syn::Type::Reference(reference) = output? else {
            return None;
        };
        reference.mutability?;
        let syn::Type::Path(path) = &*reference.elem else {
            return None;
        };
        let vec = path.path.segments.last()?;
        if vec.ident != "Vec" {
            return None;
        }
        let syn::PathArguments::AngleBracketed(args) = &vec.arguments else {
            return None;
        };
        if args.args.len() != 1 {
            return None;
        }
        let syn::GenericArgument::Type(syn::Type::Path(item)) = args.args.first()? else {
            return None;
        };
        (item.path.segments.last()?.ident == "ItemStack").then_some(())
    })()
    .is_some();
    if valid {
        Ok(())
    } else {
        Err(CatalogError::InvalidHook {
            hook: name.into(),
            error: "stack_count requires a mutable Vec<ItemStack> collect output".into(),
        })
    }
}

/// Everything the generator reads from the catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// In declaration order: files by name, each file's declarations in order.
    pub hooks: Vec<Hook>,
    pub draw_contexts: Vec<DrawContext>,
    /// Prose that belongs to no one hook, from the `Notes` trait's `#[notes]`.
    pub general_notes: Option<String>,
}

/// Where the declarations live, relative to the checkout root.
pub const DECLARATIONS: &str = "crates/sunrise-mod-vocab/src/hooks";

/// The default binding for a game hook in the same-named `native_hooks` module.
fn default_target(name: &str, area: &str, side: Option<&str>) -> String {
    let owner = match side {
        Some("server") => "ServerGame<'_>",
        Some("client") => "ClientGame<'_>",
        Some("both") => "Game<'_, S>",
        _ => "Game<'_>",
    };
    format!("sunrise_client::game::native_hooks::{area}::{owner}::native_{name}")
}

/// An attribute argument as the entry structs read it.
fn argument(expr: &syn::Expr) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match expr {
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Str(text) => Ok(Value::String(text.value())),
            syn::Lit::Bool(flag) => Ok(Value::Bool(flag.value)),
            syn::Lit::Int(number) => number
                .base10_parse::<u64>()
                .map(Value::from)
                .map_err(|e| e.to_string()),
            _ => Err("expected a string, a bool or an integer".into()),
        },
        syn::Expr::Array(array) => array.elems.iter().map(argument).collect(),
        _ => Err("expected a literal or an array of literals".into()),
    }
}

/// The `key = value` arguments of the attribute named `attribute` on a declaration, as the JSON
/// object the entry structs deserialize; `None` when it carries no such attribute.
fn arguments(
    attrs: &[syn::Attribute],
    attribute: &str,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    let Some(attr) = attrs.iter().find(|a| a.path().is_ident(attribute)) else {
        return Ok(None);
    };
    let mut map = serde_json::Map::new();
    attr.parse_nested_meta(|meta| {
        let key = meta
            .path
            .get_ident()
            .ok_or_else(|| meta.error("expected a key"))?
            .to_string();
        let value = argument(&meta.value()?.parse()?).map_err(|e| meta.error(e))?;
        if map.contains_key(&key) {
            return Err(meta.error(format!("duplicate key {key}")));
        }
        map.insert(key, value);
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    Ok(Some(map))
}

/// The `#[name = "..."]` string attributes on a declaration, in order, joined by a blank line.
fn string_attribute(attrs: &[syn::Attribute], name: &str) -> Result<Option<String>, String> {
    let mut values = Vec::new();
    for attr in attrs.iter().filter(|a| a.path().is_ident(name)) {
        let syn::Meta::NameValue(syn::MetaNameValue {
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }),
            ..
        }) = &attr.meta
        else {
            return Err(format!("#[{name}] takes a string"));
        };
        values.push(text.value());
    }
    Ok((!values.is_empty()).then(|| values.join("\n\n")))
}

/// The rustdoc of a declaration: its `///` lines, one leading space off each, joined by newlines.
fn doc(attrs: &[syn::Attribute]) -> Option<String> {
    let lines: Vec<String> = attrs
        .iter()
        .filter_map(|attr| match &attr.meta {
            syn::Meta::NameValue(syn::MetaNameValue {
                path,
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }),
                ..
            }) if path.is_ident("doc") => {
                let text = text.value();
                Some(text.strip_prefix(' ').unwrap_or(&text).to_owned())
            }
            _ => None,
        })
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The declarations of the three kinds the files hold, as entry objects in file order.
#[derive(Default)]
struct Declared {
    hooks: Vec<serde_json::Value>,
    draw_contexts: Vec<serde_json::Value>,
    general_notes: Vec<String>,
}

impl Declared {
    /// Reads one declaration file: its `Hooks`, `DrawContexts` and `Notes` traits.
    fn read(&mut self, path: &str, text: &str) -> Result<(), CatalogError> {
        let invalid = |error: String| CatalogError::Declaration {
            path: path.to_owned(),
            error,
        };
        let area = Path::new(path)
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("declaration file has no area name".into()))?;
        let file = syn::parse_file(text).map_err(|e| invalid(e.to_string()))?;
        for item in file.items {
            let syn::Item::Trait(declarations) = item else {
                continue;
            };
            let kind = declarations.ident.to_string();
            if kind == "Helpers" {
                return Err(invalid(
                    "helpers are hand-written host traits now: declare the method in `crates/sunrise-mod-vocab/src/host/<area>.rs`"
                        .into(),
                ));
            }
            for member in declarations.items {
                let syn::TraitItem::Fn(function) = member else {
                    return Err(invalid(format!(
                        "{kind} holds only declarations: fn name();"
                    )));
                };
                let name = function.sig.ident.to_string();
                if kind == "Notes" {
                    if let Some(notes) = string_attribute(&function.attrs, "notes")
                        .map_err(|e| invalid(format!("{name}: {e}")))?
                    {
                        self.general_notes.push(notes);
                    }
                    continue;
                }
                let attribute = match kind.as_str() {
                    "Hooks" => "hook",
                    "DrawContexts" => "draw_context",
                    _ => continue,
                };
                let mut entry = arguments(&function.attrs, attribute)
                    .map_err(|e| invalid(format!("{name}: {e}")))?
                    .ok_or_else(|| {
                        invalid(format!("{name}: needs a #[{attribute}(...)] attribute"))
                    })?;
                let is_free_assoc = entry.get("receiver").and_then(|v| v.as_str())
                    == Some("associated")
                    && !matches!(
                        entry.get("context").and_then(|v| v.as_str()),
                        Some(
                            "config"
                                | "reload"
                                | "client_reload"
                                | "client_reload_listener"
                                | "client_setup"
                                | "pack_discovery"
                        )
                    );
                let side = entry
                    .get("side")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                if !entry.contains_key("target") {
                    entry.insert("default_module".into(), area.into());
                }
                entry.entry("target").or_insert_with(|| {
                    if is_free_assoc {
                        format!("sunrise_client::game::native_hooks::{area}::native_{name}").into()
                    } else {
                        default_target(&name, area, side.as_deref()).into()
                    }
                });
                if let Some(notes) = string_attribute(&function.attrs, "notes")
                    .map_err(|e| invalid(format!("{name}: {e}")))?
                {
                    entry.insert("notes".into(), notes.into());
                }
                match kind.as_str() {
                    "Hooks" => {
                        entry.insert("name".into(), name.clone().into());
                        let mut call = Vec::new();
                        let mut engine = Vec::new();
                        let mut names = HashSet::new();
                        for input in &function.sig.inputs {
                            let syn::FnArg::Typed(input) = input else {
                                return Err(invalid(format!(
                                    "{name}: a declaration has no receiver"
                                )));
                            };
                            let syn::Pat::Ident(pat) = &*input.pat else {
                                return Err(invalid(format!("{name}: expected a named parameter")));
                            };
                            if pat.by_ref.is_some()
                                || pat.mutability.is_some()
                                || pat.subpat.is_some()
                                || !names.insert(&pat.ident)
                            {
                                return Err(invalid(format!(
                                    "{name}: expected a unique parameter name"
                                )));
                            }
                            let params = match &input.attrs[..] {
                                [] => &mut call,
                                [attr] if matches!(&attr.meta, syn::Meta::Path(path) if path.is_ident("engine")) => {
                                    &mut engine
                                }
                                _ => {
                                    return Err(invalid(format!(
                                        "{name}: only #[engine] is allowed on parameters"
                                    )));
                                }
                            };
                            params.push(serde_json::json!([
                                pat.ident.to_string(),
                                input.ty.to_token_stream().to_string()
                            ]));
                        }
                        entry.insert("call".into(), call.into());
                        entry.insert("engine".into(), engine.into());
                        if let Some(doc) = doc(&function.attrs) {
                            entry.insert("doc".into(), doc.into());
                        }
                        // The contract's arguments are the entry's `mod_api` object.
                        let mut api = serde_json::Map::new();
                        for key in [
                            "contract",
                            "out",
                            "scope",
                            "key",
                            "part",
                            "answered_by",
                            "owner",
                            "registration_append",
                            "staged_surface",
                            "output_validator",
                        ] {
                            if let Some(value) = entry.remove(key) {
                                api.insert(key.into(), value);
                            }
                        }
                        entry.insert("mod_api".into(), api.into());
                        self.hooks.push(entry.into());
                    }
                    _ => {
                        entry.remove("target");
                        entry.remove("default_module");
                        if let Some(trait_name) = entry.remove("name") {
                            entry.insert("trait".into(), trait_name);
                        }
                        self.draw_contexts.push(entry.into());
                    }
                }
            }
        }
        Ok(())
    }
}

impl Catalog {
    /// The catalog of the checkout at `root`: every file of the declarations directory, sorted by
    /// name, so the order never depends on a list a PR edits.
    pub fn load(root: &Path) -> Result<Self, CatalogError> {
        let read_error = |path: &Path, error: String| CatalogError::Read {
            path: path.display().to_string(),
            error,
        };
        let dir = root.join(DECLARATIONS);
        let modules =
            sunrise_game_discovery::module_files(&dir).map_err(|error| read_error(&dir, error))?;
        let files = modules
            .iter()
            .map(|module| {
                let path = dir.join(format!("{module}.rs"));
                let text =
                    fs::read_to_string(&path).map_err(|e| read_error(&path, e.to_string()))?;
                Ok((path.display().to_string(), text))
            })
            .collect::<Result<Vec<_>, CatalogError>>()?;
        Self::parse(&files)
    }

    /// The catalog of declaration files given as (path, source), in the order given.
    pub fn parse(files: &[(String, String)]) -> Result<Self, CatalogError> {
        let mut declared = Declared::default();
        for (path, text) in files {
            declared.read(path, text)?;
        }
        let mut seen = HashSet::new();
        let hooks = declared
            .hooks
            .into_iter()
            .map(|raw| {
                let hook = Hook::parse(raw)?;
                if seen.insert(hook.name.clone()) {
                    Ok(hook)
                } else {
                    Err(CatalogError::DuplicateHook(hook.name))
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let draw_contexts = declared
            .draw_contexts
            .into_iter()
            .map(|raw| {
                serde_json::from_value(raw).map_err(|e| CatalogError::Declaration {
                    path: "draw context".into(),
                    error: e.to_string(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            hooks,
            draw_contexts,
            general_notes: (!declared.general_notes.is_empty())
                .then(|| declared.general_notes.join("\n\n")),
        })
    }
}

#[cfg(test)]
impl Hook {
    /// A global listener with no counterparts, for tests of what reads a hook.
    pub(crate) fn sample(target: &str, side: Side) -> Self {
        Self {
            name: "test".into(),
            doc: "Test.".into(),
            notes: None,
            side,
            target: target.into(),
            receiver_type: None,
            default_module: None,
            contract: Contract::Listen,
            out: None,
            answered_by_mod: false,
            owner: None,
            registration_append: None,
            staged_surface: None,
            inert: None,
            output_validator: None,
            scope: None,
            part: None,
            host: None,
            context: None,
            counterparts: Counterparts::default(),
            version: 1,
            cancellable: None,
            call: Vec::new(),
            receiver: Receiver::default(),
            dispatch: DispatchStyle::default(),
            engine_ticks: false,
            engine_inert: false,
            stack_count: None,
            event_per_listener: false,
            listener_transaction: None,
            independent_listener: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hook(name: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut hook = json!({
            "name": name, "doc": "Doc.", "side": "server",
            "target": "sunrise_client::game::native_hooks::Game<'_>::native_x",
            "mod_api": {"contract": "listen"}
        });
        hook.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        hook
    }

    #[test]
    fn a_helpers_trait_is_refused_for_the_host_traits() {
        let source = r#"pub trait Helpers { #[helper(side = "server")] fn example(); }"#;
        assert!(
            super::Catalog::parse(&[("test.rs".into(), source.into())])
                .unwrap_err()
                .to_string()
                .contains("host/<area>.rs")
        );
    }

    #[test]
    fn the_shipped_catalog_loads_with_typed_contracts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = Catalog::load(&root).unwrap();
        let find = |name: &str| catalog.hooks.iter().find(|h| h.name == name).unwrap();
        let scoped = find("pre_remove_side_effects");
        assert_eq!(scoped.scope.as_ref().unwrap().kind, PartKind::BlockEntity);
        assert_eq!(find("recipes_received").contract, Contract::Listen);
        // `can_eat` changes Player.canEat's answer, so tools route an answer onto it.
        let can_eat = find("can_eat");
        assert_eq!(can_eat.contract, Contract::Replace);
        assert_eq!(can_eat.out.as_deref(), Some("can_eat"));
        assert!(
            catalog
                .hooks
                .iter()
                .any(|h| h.contract == Contract::Replace)
        );
        assert!(!catalog.draw_contexts.is_empty());
    }

    fn checkout(files: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "catalog-load-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join(DECLARATIONS);
        fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        root
    }

    const HOOK: &str = "pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn one(); }";

    #[test]
    fn metadata_generators_need_only_the_hook_declarations() {
        let root = checkout(&[(
            &format!("{DECLARATIONS}/base.rs"),
            r###"
            pub trait Hooks {
                /// Example hook.
                #[hook(side = "server", contract = "listen", neoforge = "ExampleEvent")]
                #[notes = "## Example\n\nA hook note."]
                fn example();
            }
        "###,
        )]);
        let json = crate::neoforge_catalog(&root);
        let notes = crate::generate_hook_notes(&root);
        fs::remove_dir_all(root).unwrap();

        let actual: serde_json::Value = serde_json::from_str(&json.unwrap()).unwrap();
        let expected: serde_json::Value =
            serde_json::from_str(r#"{"hooks":[{"name":"example","neoforge":"ExampleEvent"}]}"#)
                .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(
            notes.unwrap(),
            "# Hook notes\n\n## Example\n\nA hook note.\n"
        );
    }

    #[test]
    fn the_order_comes_from_the_directory_without_a_shared_module_list() {
        let zeta = "pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn zeta(); }";
        let alpha = r#"
            pub trait Hooks {
                /// Doc.
                #[hook(side = "server", contract = "listen")]
                fn alpha_second();
                /// Doc.
                #[hook(side = "server", contract = "listen")]
                fn alpha_first();
            }
        "#;
        let ignored = "pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn ignored(); }";
        let index = format!("{DECLARATIONS}/mod.rs");
        let root = checkout(&[
            (&format!("{DECLARATIONS}/zeta.rs"), zeta),
            (&format!("{DECLARATIONS}/base.rs"), HOOK),
            (&format!("{DECLARATIONS}/alpha.rs"), alpha),
            (&index, ignored),
        ]);
        let names = |catalog: Catalog| {
            catalog
                .hooks
                .into_iter()
                .map(|h| h.name)
                .collect::<Vec<_>>()
        };
        let expected = ["alpha_second", "alpha_first", "one", "zeta"];
        assert_eq!(names(Catalog::load(&root).unwrap()), expected);
        fs::remove_file(root.join(&index)).unwrap();
        assert_eq!(names(Catalog::load(&root).unwrap()), expected);
        fs::write(root.join(DECLARATIONS).join("not-valid.rs"), HOOK).unwrap();
        let error = Catalog::load(&root).unwrap_err();
        let expected_dir = root.join(DECLARATIONS).display().to_string();
        assert!(
            matches!(&error, CatalogError::Read { path, .. } if path.as_str() == expected_dir.as_str()),
            "{error}"
        );
        assert!(
            error.to_string().contains("not a Rust identifier"),
            "{error}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn file(name: &str, source: &str) -> (String, String) {
        (name.to_owned(), source.to_owned())
    }

    #[test]
    fn a_key_given_twice_in_one_attribute_is_refused() {
        let error = Catalog::parse(&[file(
            "a.rs",
            r#"
            pub trait Hooks {
                /// Twice.
                #[hook(side = "server", version = 2, contract = "listen", version = 3)]
                fn twice();
            }
            "#,
        )])
        .unwrap_err()
        .to_string();
        assert!(error.contains("duplicate key version"), "{error}");
    }

    #[test]
    fn declarations_become_entries_with_the_defaults_a_declaration_leaves_out() {
        let catalog = Catalog::parse(&[file(
            "a.rs",
            r#"
            pub trait Hooks {
                /// First line.
                ///
                ///  Indented.
                #[hook(side = "both", contract = "replace", out = "o", answered_by = "mod",
                       neoforge = ["One", "Two"], cancellable = true, version = 2)]
                fn thing();
                /// Scoped.
                #[hook(side = "server", contract = "listen", scope = "item", key = "stack",
                       target = "sunrise_world::loot::native_scoped")]
                fn scoped();
            }
            pub trait DrawContexts {
                /// Draws.
                #[draw_context(surface = "crate::s::S", name = "Draw")]
                fn draw();
            }
            "#,
        )])
        .unwrap();
        let thing = &catalog.hooks[0];
        assert_eq!(thing.doc, "First line.\n\n Indented.");
        assert_eq!(
            thing.target,
            "sunrise_client::game::native_hooks::a::Game<'_, S>::native_thing"
        );
        assert!(thing.answered_by_mod && thing.cancellable == Some(true));
        assert_eq!((thing.version, thing.counterparts.neoforge.len()), (2, 2));
        assert_eq!(
            catalog.hooks[1].target,
            "sunrise_world::loot::native_scoped"
        );
        assert_eq!(
            catalog.hooks[1].scope.as_ref().unwrap().kind,
            PartKind::Item
        );
        assert_eq!(catalog.draw_contexts[0].trait_name, "Draw");
    }

    #[test]
    fn a_declaration_repeating_a_hook_is_refused_even_across_files() {
        let declared = |name: &str| {
            format!(
                "pub trait Hooks {{ /// Doc.\n #[hook(side = \"server\", contract = \"listen\")] fn {name}(); }}"
            )
        };
        assert_eq!(
            Catalog::parse(&[
                file("a.rs", &declared("same")),
                file("b.rs", &declared("same"))
            ])
            .unwrap_err(),
            CatalogError::DuplicateHook("same".into())
        );
        assert_eq!(
            Catalog::parse(&[
                file("a.rs", &declared("one")),
                file("b.rs", &declared("two"))
            ])
            .unwrap()
            .hooks
            .len(),
            2
        );
    }

    #[test]
    fn a_declaration_needs_its_attribute_and_its_doc() {
        let parse = |source: &str| {
            Catalog::parse(&[file("a.rs", source)])
                .unwrap_err()
                .to_string()
        };
        assert!(
            parse("pub trait Hooks { fn bare(); }")
                .contains("bare: needs a #[hook(...)] attribute")
        );
        assert_eq!(
            parse(
                "pub trait Hooks { #[hook(side = \"server\", contract = \"listen\")] fn bare(); }"
            ),
            "hook bare: doc missing: add `///` lines, the rustdoc of its trait method"
        );
        assert!(
            parse("pub trait Hooks { /// Doc.\n #[hook(side = \"server\", contract = \"listen\", unknown = 1)] fn bare(); }")
                .contains("unknown field `unknown`")
        );
    }

    #[test]
    fn a_hook_without_a_doc_is_refused() {
        for doc in [json!(null), json!("  ")] {
            assert_eq!(
                Hook::parse(hook("bare", json!({"doc": doc})))
                    .unwrap_err()
                    .to_string(),
                "hook bare: doc missing: add `///` lines, the rustdoc of its trait method"
            );
        }
    }

    #[test]
    fn counterparts_accept_strings_lists_and_absence() {
        let one = Hook::parse(hook(
            "one",
            json!({"fabric": "Event.ONE", "neoforge": "OneEvent"}),
        ))
        .unwrap();
        assert_eq!(one.counterparts.fabric, ["Event.ONE"]);
        assert_eq!(one.counterparts.neoforge, ["OneEvent"]);
        let two =
            Hook::parse(hook("two", json!({"fabric": ["Event.TWO", "Event.THREE"]}))).unwrap();
        assert_eq!(two.counterparts.fabric.len(), 2);
        assert_eq!(
            Hook::parse(hook("none", json!({}))).unwrap().counterparts,
            Counterparts::default()
        );
        for key in ["fabric", "neoforge", "paper"] {
            assert_eq!(
                Hook::parse(hook("bad", json!({key: ["ok", 4]})))
                    .unwrap_err()
                    .to_string(),
                format!("hook bad: {key} must be a string or list of strings")
            );
        }
    }

    #[test]
    fn a_hook_needs_a_side_a_target_and_a_contract() {
        for side in [json!(null), json!("invalid")] {
            assert!(Hook::parse(hook("bad", json!({"side": side}))).is_err());
        }
        for field in ["side", "target", "mod_api"] {
            let mut value = hook("bad", json!({}));
            value.as_object_mut().unwrap().remove(field);
            assert!(
                Hook::parse(value)
                    .unwrap_err()
                    .to_string()
                    .contains(&format!("missing field `{field}`"))
            );
        }
        assert_eq!(
            Hook::parse(json!({})).unwrap_err(),
            CatalogError::NamelessHook
        );
    }

    #[test]
    fn contracts_and_outputs_go_together() {
        let parse = |contract: &str, out: Option<&str>| {
            Hook::parse(hook(
                "c",
                json!({"mod_api": {"contract": contract, "out": out}}),
            ))
        };
        assert_eq!(parse("listen", None).unwrap().contract, Contract::Listen);
        for contract in ["cancel", "collect", "replace"] {
            assert_eq!(
                parse(contract, Some("o")).unwrap().out.as_deref(),
                Some("o")
            );
            assert_eq!(
                parse(contract, None).unwrap_err(),
                CatalogError::InvalidContract("c".into())
            );
        }
        assert_eq!(
            parse("listen", Some("o")).unwrap_err(),
            CatalogError::InvalidContract("c".into())
        );
        assert!(parse("unknown", None).is_err());
    }

    #[test]
    fn a_scope_and_its_key_go_together() {
        let scoped = |api: serde_json::Value| Hook::parse(hook("s", json!({"mod_api": api})));
        let hook_scope = scoped(json!({"contract": "listen", "scope": "item", "key": "stack"}))
            .unwrap()
            .scope
            .unwrap();
        assert_eq!(
            (hook_scope.kind, hook_scope.key.as_str()),
            (PartKind::Item, "stack")
        );
        for api in [
            json!({"contract": "listen", "scope": "item"}),
            json!({"contract": "listen", "key": "stack"}),
        ] {
            assert_eq!(
                scoped(api).unwrap_err(),
                CatalogError::ScopeNeedsKey("s".into())
            );
        }
        assert!(scoped(json!({"contract": "listen", "scope": "nothing", "key": "k"})).is_err());
    }

    #[test]
    fn owner_needs_a_global_listener() {
        let owned = |api: serde_json::Value| Hook::parse(hook("o", json!({"mod_api": api})));
        assert_eq!(
            owned(json!({"contract": "listen", "owner": "mod_id"}))
                .unwrap()
                .owner
                .as_deref(),
            Some("mod_id")
        );
        for api in [
            json!({"contract": "cancel", "out": "c", "owner": "mod_id"}),
            json!({"contract": "listen", "owner": "mod_id", "scope": "item", "key": "k"}),
        ] {
            assert_eq!(
                owned(api).unwrap_err(),
                CatalogError::OwnerNeedsGlobalListener("o".into())
            );
        }
    }

    #[test]
    fn answered_by_needs_a_global_replace_or_collect_hook() {
        let answered = |api: serde_json::Value| Hook::parse(hook("a", json!({"mod_api": api})));
        for contract in ["replace", "collect"] {
            assert!(
                answered(json!({"contract": contract, "out": "o", "answered_by": "mod"}))
                    .unwrap()
                    .answered_by_mod
            );
        }
        for api in [
            json!({"contract": "cancel", "out": "o", "answered_by": "mod"}),
            json!({"contract": "replace", "out": "o", "answered_by": "mod", "scope": "item", "key": "k"}),
            json!({"contract": "collect", "out": "o", "answered_by": "mod", "scope": "item", "key": "k"}),
        ] {
            assert_eq!(
                answered(api).unwrap_err(),
                CatalogError::AnsweredNeedsGlobalOutput("a".into())
            );
        }
    }

    #[test]
    fn part_option_requires_a_plain_global_cancel_hook() {
        let parsed = Hook::parse(hook(
            "stacked",
            json!({"mod_api": {"contract": "cancel", "out": "canceled", "part": "item"}}),
        ))
        .unwrap();
        assert_eq!(parsed.part, Some(PartKind::Item));
        assert!(parsed.scope.is_none());

        for bad in [
            json!({"mod_api": {"contract": "cancel", "out": "canceled", "part": "item", "scope": "item", "key": "stack"}}),
            json!({"mod_api": {"contract": "listen", "part": "item"}}),
            json!({"mod_api": {"contract": "collect", "out": "drops", "part": "item"}}),
            json!({"mod_api": {"contract": "cancel", "out": "canceled", "part": "item"}, "event_per_listener": true}),
            json!({"mod_api": {"contract": "cancel", "out": "canceled", "part": "item"}, "independent_listener": true}),
        ] {
            assert!(Hook::parse(hook("bad", bad)).is_err());
        }
        for kind in ["block", "block_entity", "menu", "effect", "fluid", "entity"] {
            let err = Hook::parse(hook(
                "bad_part_kind",
                json!({"mod_api": {"contract": "cancel", "out": "canceled", "part": kind}}),
            ))
            .unwrap_err()
            .to_string();
            assert!(
                err.contains(kind),
                "expected unsupported part kind {kind:?} to be named in error: {err}"
            );
        }
    }
}

#[cfg(test)]
mod stack_count_tests {
    use super::*;

    #[test]
    fn stack_count_requires_a_stack_collect_output_and_known_policy() {
        let parse = |declaration: &str| Catalog::parse(&[("test.rs".into(), declaration.into())]);
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="listen", stack_count="trim")]
            fn notice(); }"#
            )
            .is_err()
        );
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="values", stack_count="trim")]
            fn numbers(values: &mut Vec<u32>); }"#
            )
            .is_err()
        );
        assert!(
            parse(
                r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="drops", stack_count="unknown")]
            fn fill(drops: &mut Vec<ItemStack>); }"#
            )
            .is_err()
        );
        let catalog = parse(
            r#"trait Hooks { /// Doc.
            #[hook(side="server", contract="collect", out="drops", stack_count="trim")]
            fn fill(drops: &mut Vec<ItemStack>); }"#,
        )
        .unwrap();
        assert_eq!(catalog.hooks[0].stack_count, Some(StackCount::Trim));
    }
}
