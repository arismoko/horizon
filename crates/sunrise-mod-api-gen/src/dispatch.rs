//! Generate hook traits, event structs, and dispatch functions from the catalog.
//!
//! Every generated `dispatch_*` function routes listener calls through `runtime::invoke`,
//! enforcing one failure rule across the entire mod API: a panic in a mod callback disables that
//! mod's callback for that hook for the rest of the session (leaving other hooks and other mods
//! running) and reports it once via an error log line and error toast naming the mod and hook;
//! any other failure or refusal is reported each time and never latched.
use super::*;
use crate::catalog::{
    BindingKind, DispatchStyle, EFFECT_TICK_TARGET, ListenerTransaction, Receiver,
    RegistrationAppend,
};

pub(super) struct HookSources {
    pub traits: String,
    pub dispatch: String,
    pub render_dispatch: String,
    pub part_traits: BTreeMap<&'static str, String>,
    pub fills: Vec<Fill>,
    pub fire: Vec<FireUnit>,
    pub render_fire: String,
    pub hook_ids: String,
    pub hook_constants: String,
    /// A `()` constant named after each hook a mod may cancel, which `#[listener(receive_cancelled)]`
    /// names so that the option on any other hook fails to compile.
    pub cancel_hooks: String,
    /// `(HookId::X, "x"),` for each client-side hook: what a `side = "server"` mod may not hear.
    pub client_hooks: String,
    pub hook_count: usize,
}

pub(super) struct FireUnit {
    name: String,
    owner: FireOwner,
    package: Option<crate::GameplayPackage>,
    side: Side,
    carrier: Option<FireCarrier>,
    receiver: Receiver,
    parameters: Vec<String>,
    before: String,
    dispatch: String,
    after: String,
}

enum FireOwner {
    Free,
    Shared,
    Game,
    Scoped(syn::PathSegment),
}

struct FireCarrier {
    name: String,
    generics: &'static str,
    fields: String,
}

pub(super) fn render_fire(units: &[FireUnit]) -> String {
    use std::fmt::Write as _;
    let mut code = String::new();
    let mut shared = String::new();
    for unit in units {
        if let Some(FireCarrier {
            name,
            generics,
            fields,
        }) = &unit.carrier
        {
            let visibility = fire_visibility(unit.package);
            writeln!(code, "{visibility} struct {name}{generics} {{ {fields} }}").unwrap();
        }
        let receiver = match unit.receiver {
            Receiver::Read => Some("&self"),
            Receiver::Write => Some("&mut self"),
            Receiver::Associated => None,
        };
        let signature = receiver
            .into_iter()
            .chain(unit.parameters.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(", ");
        let FireUnit {
            name,
            before,
            dispatch,
            after,
            ..
        } = unit;
        let visibility = if matches!(unit.owner, FireOwner::Shared) {
            ""
        } else {
            fire_visibility(unit.package)
        };
        let visibility = if visibility.is_empty() {
            String::new()
        } else {
            format!("{visibility} ")
        };
        let function = format!(
            "#[inline] {visibility}fn fire_{name}({signature}) {{ {before} {dispatch} {after} }}"
        );
        let header = match &unit.owner {
            FireOwner::Free => {
                writeln!(code, "{function}").unwrap();
                continue;
            }
            FireOwner::Shared => {
                writeln!(shared, "{function}").unwrap();
                continue;
            }
            FireOwner::Game if unit.package.is_some() => format!(
                "impl crate::game::{}<'_>",
                unit.package.unwrap().game_type()
            ),
            FireOwner::Game => match unit.side {
                Side::Server => "impl crate::game::ServerGame<'_>".into(),
                Side::Client => {
                    "#[cfg(feature = \"client\")] impl crate::game::ClientGame<'_>".into()
                }
                Side::Both => "impl<S: crate::game::Side> crate::game::Game<'_, S>".into(),
            },
            FireOwner::Scoped(owner) if owner.ident == "ReadLevel" && unit.package.is_some() => {
                "impl<'g, G: crate::game::SideGame<'g> + ?Sized> ReadLevel<'_, 'g, G>".into()
            }
            FireOwner::Scoped(owner) if owner.ident == "ReadLevel" => {
                "impl<S: crate::game::Side> ReadLevel<'_, '_, S>".into()
            }
            FireOwner::Scoped(owner) => format!("impl {}", owner.to_token_stream()),
        };
        writeln!(code, "{header} {{ {function} }}").unwrap();
    }
    if !shared.is_empty() {
        code.push_str(&format!(
            "pub trait FireHooks<'g>: crate::game::SideGame<'g> {{\n{shared}}}\nimpl<'g, T: crate::game::SideGame<'g> + ?Sized> FireHooks<'g> for T {{}}\n"
        ));
    }
    code
}

fn fire_visibility(package: Option<crate::GameplayPackage>) -> &'static str {
    if package == Some(crate::GameplayPackage::Common) {
        "pub"
    } else {
        "pub(in crate::game)"
    }
}

/// Whether a mutable-reference field can be saved and restored by copying its pointee.
fn rollback_pointee_is_copy(field_ty: &Type) -> bool {
    match field_ty {
        Type::Reference(reference) => rollback_pointee_is_copy(&reference.elem),
        Type::Array(array) => is_primitive_scalar(&array.elem),
        other => is_primitive_scalar(other),
    }
}

fn part_trait_and_table(kind: PartKind) -> (&'static str, &'static str) {
    match kind {
        PartKind::Block => ("BlockPart", "block"),
        PartKind::BlockEntity => ("BlockEntityPart", "block_entity"),
        PartKind::Item => ("ItemPart", "item"),
        PartKind::Menu => ("MenuPart", "menu"),
        PartKind::Effect => ("MobEffectPart", "effect"),
        PartKind::Fluid => ("FluidPart", "fluid"),
        PartKind::Entity => ("EntityPart", "entity"),
    }
}

pub(super) fn is_render(hook: &Hook) -> bool {
    hook.target.starts_with("mod_render::render::")
}

fn receiver_mut(hook: &Hook) -> Option<bool> {
    if hook.target == EFFECT_TICK_TARGET {
        Some(false)
    } else if hook.target.starts_with("sunrise_client::game::")
        || hook
            .gameplay_binding()
            .is_ok_and(|binding| binding.is_some())
    {
        hook.receiver.mutability()
    } else {
        None
    }
}

pub(super) fn listener_host_or_context(hook: &Hook) -> Option<host_param::HostParam> {
    host_param::select(hook, receiver_mut(hook))
}

/// Render records only, without game binding or listener dispatch.
pub(super) fn render(catalog: &Catalog) -> Result<HookSources, String> {
    emit(
        catalog.hooks.iter().filter(|hook| is_render(hook)),
        &catalog.draw_contexts,
        false,
    )
}

/// Emit trait methods, event types, game adapters, and `dispatch_*` functions for `hooks`.
///
/// Unless an engine adapter owns recovery, generated dispatchers invoke callbacks through
/// `runtime::invoke`: panics disable only `(hook, mod)` for the rest of the session and report once,
/// while ordinary refusals report every time and are never latched.
pub(super) fn emit<'a>(
    hooks: impl Iterator<Item = &'a Hook>,
    contexts: &[DrawContext],
    emit_fire: bool,
) -> Result<HookSources, String> {
    let mut hook_ids = String::new();
    let mut hook_constants = String::new();
    let mut cancel_hooks = String::new();
    let mut client_hooks = String::new();
    let mut hook_count = 0;
    let mut traits = String::new();
    let mut dispatch = String::new();
    let mut render_dispatch = String::new();
    let mut part_traits: BTreeMap<&str, String> = BTreeMap::new();
    let mut fills = Vec::new();
    let mut fire = Vec::new();
    let mut render_fire = String::new();
    let mut calls = String::new();
    for hook in hooks {
        let name = hook.name.as_str();
        let single_part_slot = name == "on_sheared";
        let native = format!("native_{name}");
        let target = hook.target.as_str();
        let is_render = is_render(hook);
        let binding = hook.gameplay_binding()?;
        crate::gameplay::validate_shared_hook(
            hook,
            binding.as_ref().map(|binding| binding.package),
            contexts,
        )?;
        let in_game = target.starts_with("sunrise_client::game::") || binding.is_some();
        let declaration_only = target == EFFECT_TICK_TARGET;
        let expected = if is_render { name } else { &native };
        if !declaration_only
            && (!(in_game || free_function_source(target).is_some())
                || !target
                    .rsplit_once("::")
                    .is_some_and(|(_, function)| function == expected))
        {
            return Err(format!("wrong target for {name}: {target}"));
        }
        let receiver_mut = receiver_mut(hook);
        let ticks = hook.engine_ticks;
        let inert_signal = hook.engine_inert;
        // Only a quick-move Part's panic can refuse its unanswered click.
        let panic_signal = name == "menu_quick_move";
        let is_level = host_param::is_level(hook);
        if (ticks && !is_level) || ((ticks || inert_signal) && !in_game) {
            return Err(format!(
                "{name}: engine-only ticks need a level receiver and inert needs a game target"
            ));
        }
        host_param::validate(hook, name, is_level, receiver_mut, ticks)?;
        let declared = hook
            .call
            .iter()
            .map(|(arg, ty)| Ok((arg.clone(), syn::parse_str::<Type>(ty)?)))
            .collect::<Result<Vec<_>, syn::Error>>()
            .map_err(|e| format!("{name}: {e}"))?;
        let carried: Vec<_> = declared.iter().map(|(arg, ty)| (arg.clone(), ty)).collect();
        let mut args = carried.clone();
        if hook.independent_listener
            && (is_render
                || declaration_only
                || hook.scope.is_some()
                || hook.owner.is_some()
                || hook.context == Some(PhaseContext::ChunkDisk)
                || name == "consume_effect"
                || !matches!(hook.contract, Contract::Listen | Contract::Cancel))
        {
            return Err(format!(
                "{name}: independent_listener requires a global listen or cancel dispatcher"
            ));
        }
        let carrier = format!("{}Call", hook_id(name));
        let uses_carrier = !declared.is_empty()
            && (name == "apply_effect_tick" && declaration_only
                || in_game
                    && matches!(
                        hook.dispatch,
                        DispatchStyle::Carrier | DispatchStyle::ApiCarrier
                    ));
        let mut used: HashSet<_> = args.iter().map(|(name, _)| name.clone()).collect();
        let mod_item = fresh_name("mod_hook", &mut used);
        let listener_id = fresh_name("listener_id", &mut used);
        let part_list = fresh_name("parts", &mut used);
        let part_item = fresh_name("part", &mut used);
        let part_table = fresh_name("part_table", &mut used);
        let replacement = fresh_name("replacement", &mut used);
        let receives = fresh_name("receives_cancelled", &mut used);
        let mod_entry = fresh_name("mod_entry", &mut used);
        let entered = fresh_name("_entered", &mut used);
        let batch = fresh_name("batch", &mut used);
        let contract = hook.contract;
        let out = hook.out.as_deref();
        let answered = hook.answered_by_mod;
        let answered_return = answered
            && contract == Contract::Collect
            && matches!(name, "section_visibility" | "player_commands_sent");
        let output = if let Some(out) = out {
            let index = args
                .iter()
                .position(|(n, _)| n == out)
                .ok_or_else(|| format!("missing output {out} in {name}"))?;
            Some(args.remove(index))
        } else {
            None
        };
        if hook.stack_count.is_some() {
            crate::catalog::validate_stack_count_output(name, output.as_ref().map(|(_, ty)| *ty))
                .map_err(|error| error.to_string())?;
        }
        let collect_cancel = contract == Contract::Collect
            && hook.cancellable == Some(true)
            && args.iter().any(|(name, ty)| {
                name == "cancel" && ty.to_token_stream().to_string() == "& mut bool"
            });
        let cancel_out = if collect_cancel { Some("cancel") } else { out };
        let result = if collect_cancel {
            " -> Flow".to_owned()
        } else if let Some((_, ty)) = &output {
            let Type::Reference(reference) = ty else {
                return Err(format!("output must be a mutable reference in {name}"));
            };
            if reference.mutability.is_none() {
                return Err(format!("output must be a mutable reference in {name}"));
            }
            match contract {
                Contract::Cancel if reference.elem.to_token_stream().to_string() == "bool" => {
                    " -> Flow".to_owned()
                }
                Contract::Replace if answered => {
                    let answer = answered_type(&reference.elem)
                        .ok_or_else(|| format!("{name}: output must be Option<Answered<T>>"))?;
                    format!(" -> Option<{}>", answer.to_token_stream())
                }
                Contract::Replace => format!(" -> {}", reference.elem.to_token_stream()),
                Contract::Collect if answered_return => {
                    let answer = answered_sink(&reference.elem)
                        .filter(|(_, output)| matches!(output, syn::ReturnType::Default))
                        .map(|(answer, _)| answer)
                        .ok_or_else(|| {
                            format!("{name}: output must be &mut dyn FnMut(Answered<T>)")
                        })?;
                    format!(" -> Option<{}>", answer.to_token_stream())
                }
                Contract::Collect
                    if answered
                        && answered_batch(&reference.elem).is_none()
                        && !(matches!(name, "blocks" | "items")
                            && declaration_sink(&reference.elem).is_some()) =>
                {
                    return Err(format!("{name}: output must be Vec<Answered<T>>"));
                }
                Contract::Collect => String::new(),
                _ => return Err(format!("unexpected output type for {name}")),
            }
        } else {
            String::new()
        };
        let host = host_param::select(hook, receiver_mut).map(|param| {
            (
                fresh_name(
                    if hook.context.is_some() {
                        "context"
                    } else if is_level {
                        "level"
                    } else {
                        "host"
                    },
                    &mut used,
                ),
                param,
            )
        });
        if hook.listener_transaction.is_some()
            && (hook.dispatch != DispatchStyle::Flat
                || !host
                    .as_ref()
                    .is_some_and(|(_, param)| param.ty == "&mut dyn ServerHost"))
        {
            return Err(format!(
                "{name}: listener_transaction requires a flat hook with a mutable ServerHost"
            ));
        }
        let staged_host = hook.staged_surface.as_deref() == Some("host")
            && !args.iter().any(|(arg, _)| arg == "host");
        let mut staged_draw = false;
        let staged = if staged_host {
            let (host_name, param) = host
                .as_ref()
                .ok_or_else(|| format!("{name}: staged host needs a Host"))?;
            if hook.dispatch != DispatchStyle::Direct
                || !in_game
                || is_render
                || contract != Contract::Listen
                || hook.scope.is_some()
                || hook.owner.is_some()
                || hook.context.is_some()
                || !param.ty.starts_with("&mut dyn ")
                || args.iter().any(|(_, ty)| DrawContext::of(contexts, ty).is_some()
                    || matches!(ty, Type::Reference(reference) if reference.mutability.is_some()))
            {
                return Err(format!("{name}: staged host needs a direct global game listener with a mutable Host and read-only inputs"));
            }
            let stage = fresh_name("stage_listener", &mut used);
            Some((
                host_name,
                stage,
                format!("impl FnMut(&mut dyn FnMut({}) -> bool)", param.ty),
            ))
        } else if let Some(field) = &hook.staged_surface {
            let Some((_, ty)) = args.iter().find(|(arg, _)| arg == field) else {
                return Err(format!("{name}: missing staged_surface field {field}"));
            };
            staged_draw = DrawContext::of(contexts, ty).is_some_and(|draw| {
                draw.trait_name == "DrawContext" && contract == Contract::Listen
            });
            if hook.context == Some(PhaseContext::ChunkDisk)
                || !in_game
                || is_render
                || hook.dispatch != DispatchStyle::Flat
                || args.iter().any(|(arg, ty)| {
                    if staged_draw && arg == field {
                        return false;
                    }
                    DrawContext::of(contexts, ty).is_some()
                        || (!staged_draw
                            && arg != field
                            && matches!(ty, Type::Reference(reference) if reference.mutability.is_some()))
                })
                || !matches!(ty, Type::Reference(reference)
                    if reference.mutability.is_some() && matches!(*reference.elem, Type::TraitObject(_)))
            {
                return Err(format!(
                    "{name}: staged_surface needs a flat game listener with a mutable surface"
                ));
            }
            let stage = fresh_name("stage_listener", &mut used);
            let signature = format!(
                "impl FnMut(&mut dyn FnMut({}) -> bool)",
                ty.to_token_stream()
            );
            Some((field, stage, signature))
        } else {
            None
        };
        if answered_return
            && (hook.context.is_some()
                || host.as_ref().is_some_and(|(_, host)| {
                    host.ty.starts_with("&mut ")
                        || matches!(
                            host.ty.as_str(),
                            "HostSide" | "LevelSide" | "LevelRandomSide"
                        )
                }))
        {
            return Err(format!(
                "{name}: an isolated collector needs a read-only world Host"
            ));
        }
        // A block notice may identify a watching block entity. Its Parts receive
        // the source payload without changing the declared source key.
        let observer_routed = hook
            .scope
            .as_ref()
            .is_some_and(|scope| scope.kind == PartKind::Block)
            && args.iter().any(|(arg, ty)| {
                arg == "observer"
                    && ty
                        .to_token_stream()
                        .to_string()
                        .replace("mod_api :: ", "")
                        .split_whitespace()
                        .collect::<String>()
                        == "Option<BlockEntityRef>"
            });
        // Menu button hooks name the opener block and reach only the active
        // menu's owner (`HookParts::menu_for_opener`).
        let opener_routed = hook
            .scope
            .as_ref()
            .is_some_and(|scope| scope.kind == PartKind::Menu)
            && args.iter().any(|(arg, ty)| {
                arg == "block"
                    && ty.to_token_stream().to_string().replace(' ', "") == "Option<&str>"
            });
        // A scoped hook's parts: the trait its method goes on, and the lookup
        // of the parts attached to the thing its `key` argument names.
        let mut entity_kind_param = false;
        let scoped = match &hook.scope {
            None => None,
            Some(scope) => {
                let key = scope.key.as_str();
                let (part_trait, table) = part_trait_and_table(scope.kind);
                if is_render {
                    return Err(format!("render hook {name} cannot be scoped"));
                }
                let (_, ty) = args
                    .iter()
                    .find(|(arg, _)| arg == key)
                    .ok_or_else(|| format!("key {key} is not an argument of {name}"))?;
                let key_type = ty.to_token_stream().to_string().replace("mod_api :: ", "");
                if hook.inert.is_some() && (observer_routed || key_type != "BlockState") {
                    return Err(format!(
                        "inert hook {name} requires a block scope routed by a BlockState key"
                    ));
                }
                let lookup = match key_type.as_str() {
                    "BlockState" if hook.inert.is_some() => format!("block_entry({key})"),
                    "BlockState" if table == "block" && observer_routed => {
                        let host = &host
                            .as_ref()
                            .ok_or("block observer routing needs a Host")?
                            .0;
                        format!(
                            "block_key(match observer {{ Some(observer) => observer.level.block(&*{host}, observer.position), None => Some({key}.block()) }})"
                        )
                    }
                    "BlockState" if table == "block" => format!("block_value({key})"),
                    "BlockState" => {
                        format!(
                            "{table}(|| super::content::registry_keys::block_id({key}.block()))"
                        )
                    }
                    "& ItemStack" | "ItemStackView < '_ >" => {
                        format!("{table}(|| super::content::registry_keys::item_id({key}.item()))")
                    }
                    "ItemKey" if table == "item" => {
                        format!("item(|| super::content::registry_keys::item_id({key}))")
                    }
                    "EffectKey" if table == "effect" => {
                        format!("effect(|| super::content::registry_keys::effect_id({key}))")
                    }
                    "BlockEntityTypeKey" if table == "block_entity" => format!(
                        "block_entity_kind(|| super::content::registry_keys::block_entity_type_name({key}))"
                    ),
                    "& str" if opener_routed => format!("menu_for_opener({key}, block)"),
                    "& str" => format!("{table}(|| Some({key}))"),
                    "EntityRef" | "MobRef" | "UnspawnedMobRef" if table == "entity" => {
                        entity_kind_param = true;
                        "entity(|| Some(__entity_kind))".to_owned()
                    }
                    "& mut GoalRegistrar < '_ >" | "& mut BehaviorRegistrar < '_ >"
                        if table == "entity" =>
                    {
                        format!("entity(|| Some({key}.route_kind()))")
                    }
                    "BlockEntityRef" if table == "block_entity" => {
                        let host = &host.as_ref().ok_or("block entity handle needs a Host")?.0;
                        format!(
                            "block_entity_kind(|| {key}.kind({host}).and_then(super::content::registry_keys::block_entity_type_name))"
                        )
                    }
                    // An entity's type id need not be its block's: its own table.
                    "& ModBlockEntity" | "BlockEntityDataView < '_ >" => {
                        format!("block_entity_kind(|| Some({key}.id()))")
                    }
                    "BlockEntityClientScope < '_ >" | "BlockEntityClientReadScope < '_ >"
                        if table == "block_entity" =>
                    {
                        format!("block_entity_kind(|| {key}.route_kind())")
                    }
                    "EntityClientScope < '_ >" if table == "entity" => {
                        format!("entity(|| {key}.route_kind())")
                    }
                    "ItemEntityServerScope < '_ >" if table == "item" => {
                        format!("item(|| Some({key}.route_item()))")
                    }
                    other => return Err(format!("key {key} of {name} has type {other}")),
                };
                Some((part_trait, lookup))
            }
        };
        let event = fresh_name("event", &mut used);
        let event_type = format!("{}Event", hook_id(name));
        let hook_id = event_type.strip_suffix("Event").unwrap();
        // The checked declaration view replaces the old plain collect output.
        // Surface export reads the input tree's types, including older trees.
        let declaration = match (name, output.as_ref()) {
            ("blocks" | "items", Some((_, Type::Reference(output)))) => {
                declaration_sink(&output.elem)
                    .or_else(|| answered_batch(&output.elem))
                    .map(ToTokens::to_token_stream)
            }
            _ => None,
        };
        let registration_append = if let Some(mode) = hook.registration_append {
            let entry = match output.as_ref() {
                Some((_, Type::Reference(reference))) => {
                    if answered {
                        answered_batch(&reference.elem)
                    } else {
                        generic_of(&reference.elem, "Vec")
                    }
                }
                _ => None,
            };
            let context_ok = mode == RegistrationAppend::Attributed
                || hook.context == Some(PhaseContext::Registration);
            let attributed_ok = mode != RegistrationAppend::Attributed
                || (scoped.is_none() && !answered && !hook.event_per_listener);
            let Some(entry) = entry.filter(|_| {
                context_ok
                    && attributed_ok
                    && contract == Contract::Collect
                    && !collect_cancel
                    && declaration.is_none()
                    && !is_render
            }) else {
                return Err(format!(
                    "{name}: registration_append requires a non-cancelling collect with a mutable Vec output (and RegistrationContext unless attributed)"
                ));
            };
            Some((mode, entry))
        } else {
            None
        };
        let attributed_append = matches!(
            registration_append,
            Some((RegistrationAppend::Attributed, _))
        );
        if attributed_append && args.iter().any(|(arg, _)| arg == "mod_id") {
            return Err(format!(
                "{name}: an attributed append hook's event has its own `mod_id` field"
            ));
        }
        if !is_render {
            hook_ids.push_str(&format!("{hook_id},\n"));
            hook_constants.push_str(&format!(
                "pub const {name}: Option<super::HookId> = Some(super::HookId::{hook_id});\n"
            ));
            hook_count += 1;
            if hook.side == Side::Client {
                client_hooks.push_str(&format!("(HookId::{hook_id}, \"{name}\"),\n"));
            }
        }
        // A global listener may opt in to cancelled events. A Part is never told of one.
        let mod_cancel = (contract == Contract::Cancel || collect_cancel) && scoped.is_none();
        if mod_cancel && !is_render {
            cancel_hooks.push_str(&format!("pub const {name}: () = ();\n"));
        }
        let registration = hook.context == Some(PhaseContext::Registration);
        let mut trait_params = vec!["&self".to_owned()];
        let mut dispatch_params = Vec::new();
        let mut call_args = Vec::new();
        if let Some((name, param)) = &host {
            trait_params.push(format!("{name}: {}", param.ty));
            let by_side = matches!(
                param.ty.as_str(),
                "HostSide" | "LevelSide" | "LevelRandomSide"
            );
            if let Some((_, stage, signature)) = staged.as_ref().filter(|_| staged_host) {
                dispatch_params.push(format!("mut {stage}: {signature}"));
            } else {
                dispatch_params.push(format!(
                    "{}{name}: {}",
                    if by_side { "mut " } else { "" },
                    param.ty
                ));
            }
            call_args.push(if registration {
                format!("&{name}.for_mod({listener_id})")
            } else if by_side {
                format!("{name}.reborrow()")
            } else if param.ty.starts_with("&mut ") {
                format!("&mut *{name}")
            } else {
                name.clone()
            });
        }

        let mut lifetimes = EventLifetimes::default();
        let mut fields = String::new();
        let mut inits = Vec::new();
        let mut write_back = String::new();
        // Fields restored to the last accepted values on a listener failure, each with whether
        // the event holds it behind a `&mut`.
        let mut rollback: Vec<(String, bool)> = Vec::new();
        let mut cloned: Vec<String> = Vec::new();
        let mut surfaces: Vec<String> = Vec::new();
        // Domain batches already have a deliberate atomic contract. Only ordinary
        // engine conversions use per-output admission; direct and API-carrier owners
        // (notably the resident menu routes) already own their writes.
        let checked_crossing = in_game
            && matches!(hook.dispatch, DispatchStyle::Carrier | DispatchStyle::Flat)
            && !matches!(name, "fall_on" | "entity_inside");
        let list_type = |ty: &Type| checked_crossing.then(|| reply_list(ty)).flatten();
        let collected = output
            .as_ref()
            .filter(|_| contract == Contract::Collect && !answered_return);
        let checked_fields: Vec<_> = args
            .iter()
            .chain(collected)
            .filter_map(|(field, ty)| {
                (checked_crossing && list_type(ty).is_none() && boundary_kind(ty).is_some())
                    .then(|| mutable_pointee(ty).map(|ty| (field, ty)))
                    .flatten()
            })
            .collect();
        let checked_replacement = output.as_ref().filter(|(_, ty)| {
            checked_crossing && contract == Contract::Replace && boundary_kind(ty).is_some()
        });
        // Swapping hands is one operation. Keep its two proposed writes together,
        // but do not make the independent veto depend on the validity of the pair.
        let paired_hands = name == "player_swap_hands" && checked_crossing;
        let mut lists = Vec::new();
        if attributed_append {
            fields.push_str("pub(super) mod_id: &'static str,\n");
            inits.push("mod_id: \"\"".to_owned());
        }
        for (name, ty) in args.iter().chain(collected) {
            let projected_list = list_type(ty);
            let dispatch_ty = projected_list.as_ref().map(ListReply::adapter_type);
            let ty = dispatch_ty.as_ref().unwrap_or(ty);
            if projected_list.is_some() {
                lists.push(name.clone());
            }
            let block_entity_scope = matches!(
                ty,
                Type::Path(path)
                    if path.path.segments.last().is_some_and(|part| part.ident == "BlockEntityScope")
            );
            if answered_return {
                struct MutableBorrow(bool);
                impl<'ast> syn::visit::Visit<'ast> for MutableBorrow {
                    fn visit_type_reference(&mut self, reference: &'ast syn::TypeReference) {
                        self.0 |= reference.mutability.is_some();
                        syn::visit::visit_type_reference(self, reference);
                    }
                }
                let mut mutable = MutableBorrow(false);
                syn::visit::Visit::visit_type(&mut mutable, ty);
                if mutable.0 {
                    return Err(format!(
                        "{}: an isolated collector cannot also write to {name}",
                        hook.name
                    ));
                }
            }
            if let Some(entry) = declaration.as_ref().filter(|_| Some(name.as_str()) == out) {
                dispatch_params.push(format!(
                    "{name}: &mut super::parts::DeclarationStage<'_, {entry}>"
                ));
                lifetimes.a = true;
                fields.push_str(&format!(
                    "mod_id: &'static str,\nrefusals: &'a mut Vec<super::parts::DeclarationRefusal>,\n{name}: &'a mut super::parts::DeclarationSink<'a, {entry}>,\n"
                ));
                continue;
            }
            if let Some((_, stage, signature)) = staged
                .as_ref()
                .filter(|(field, _, _)| field.as_str() == name.as_str())
            {
                dispatch_params.push(format!("mut {stage}: {signature}"));
            } else if block_entity_scope {
                dispatch_params.push(format!("mut {name}: {}", ty.to_token_stream()));
            } else {
                dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
            }
            if collect_cancel && name == "cancel" {
                continue;
            }
            // An answered collect hook's mods each push plain entries into their own batch;
            // dispatch, not the mod, names the mod on each.
            let batch_entry =
                (answered && contract == Contract::Collect && Some(name.as_str()) == out)
                    .then(|| match ty {
                        Type::Reference(reference) => answered_batch(&reference.elem),
                        _ => None,
                    })
                    .flatten();
            let field_ty = if let Some(entry) = batch_entry {
                syn::parse_quote!(&mut Vec<#entry>)
            } else {
                (*ty).clone()
            };
            let (field, scalar) = lifetimes.field(&field_ty);
            if hook.output_validator.is_some()
                && matches!(&field_ty, Type::Reference(reference)
                    if reference.mutability.is_some() && !rollback_pointee_is_copy(&field_ty))
            {
                return Err(format!(
                    "{}: output_validator requires Copy outputs; mutable argument {name} \
                     must be a Copy primitive or a fixed array of them",
                    hook.name
                ));
            }
            // The task sink checkpoints its append-only queue and diagnostic;
            // inside-block callbacks stage per listener instead of snapshotting.
            let task_registration =
                event_type == "RegisterConfigurationTasksEvent" && name == "registration";
            let custom_spawners = event_type == "ModifyCustomSpawnersEvent" && name == "spawners";
            // The lent click checkpoints its working stacks and edit log; it cannot be cloned.
            let stacked_click = event_type == "ItemStackedOnOtherEvent" && name == "click";
            let staged_mob_registrar = (event_type == "RegisterGoalsEvent" && name == "goals")
                || (event_type == "RegisterBehaviorsEvent" && name == "behaviors");
            let inside_effects = event_type == "EntityInsideEvent" && name == "effects";
            if scalar || task_registration || custom_spawners || stacked_click {
                rollback.push((name.clone(), false));
            } else if staged_mob_registrar {
                surfaces.push(name.clone());
                rollback.push((name.clone(), false));
            } else if !inside_effects
                && matches!(&field_ty, Type::Reference(reference) if reference.mutability.is_some())
            {
                if matches!(&field_ty, Type::Reference(reference) if matches!(*reference.elem, Type::TraitObject(_)))
                {
                    surfaces.push(name.clone());
                }
                if !rollback_pointee_is_copy(&field_ty) {
                    cloned.push(name.clone());
                }
                rollback.push((name.clone(), true));
            }
            let append = registration_append.filter(|_| Some(name.as_str()) == out);
            let visibility = match append {
                Some((RegistrationAppend::Register, _)) => "",
                Some((RegistrationAppend::Custom | RegistrationAppend::Attributed, _)) => {
                    "pub(super)"
                }
                None if task_registration
                    || inside_effects
                    || matches!(
                        (event_type.as_str(), name.as_str()),
                        ("ApplyEffectTickEvent", "changes")
                            | ("StepOnEvent", "writes")
                            | ("ExtractBlockScreenEffectEvent", "renderer")
                            | ("RegisterDebugRenderersEvent", "entries")
                            | ("RegisterRenderPipelinesEvent", "render_types")
                            | ("RegisterRenderPipelinesEvent", "compute_pipelines")
                    ) =>
                {
                    "pub(super)"
                }
                None => "pub",
            };
            fields.push_str(&format!(
                "{visibility} {name}: {},\n",
                field.to_token_stream()
            ));
            if scalar {
                inits.push(format!("{name}: *{name}"));
                write_back.push_str(&format!("*{name} = {event}.{name};"));
            } else if block_entity_scope {
                inits.push(format!("{name}: {name}.reborrow()"));
            } else if batch_entry.is_some() {
                inits.push(format!("{name}: &mut {batch}"));
            } else if (hook.event_per_listener || staged_draw)
                && matches!(field_ty, Type::Reference(ref reference) if reference.mutability.is_some())
            {
                // Keep write-through access, but not changes to the reference itself.
                inits.push(format!("{name}: &mut *{name}"));
            } else {
                inits.push(name.clone());
            }
        }
        if contract == Contract::Listen && hook.output_validator.is_some() && rollback.is_empty() {
            return Err(format!("{name}: output_validator requires mutable outputs"));
        }
        if mod_cancel {
            if args.iter().any(|(arg, _)| arg == "cancelled") {
                return Err(format!(
                    "{name}: a cancel hook's event has its own `cancelled` field"
                ));
            }
            fields.push_str("/// Whether the event is cancelled: by the game or an earlier listener, or by this one\n/// setting it. Only a listener that receives cancelled events (`#[listener(receive_cancelled)]`)\n/// sees `true`.\n");
            if hook.output_validator.is_some() {
                fields.push_str("/// An accepted cancellation cannot be cleared.\n");
            } else {
                fields.push_str("/// Setting it back to `false` lets the game's behaviour run.\n");
            }
            fields.push_str("pub cancelled: bool,\n");
            inits.push(format!("cancelled: *{}", cancel_out.unwrap()));
        }
        if let Some((name, ty)) = output
            .as_ref()
            .filter(|_| contract != Contract::Collect || answered_return)
        {
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
        }
        if uses_carrier {
            dispatch_params.truncate(usize::from(host.is_some()));
            let api_carried = carried
                .iter()
                .map(|(field, ty)| {
                    (
                        field.clone(),
                        list_type(ty).map_or_else(|| (*ty).clone(), |list| list.adapter_type()),
                    )
                })
                .collect::<Vec<_>>();
            let refs = api_carried
                .iter()
                .map(|(field, ty)| (field.clone(), ty))
                .collect::<Vec<_>>();
            let (fields, generics) = carrier_fields(&refs);
            dispatch_params.push(format!(
                "call: calls::{carrier}{}",
                if generics.is_empty() { "" } else { "<'_>" }
            ));
            calls.push_str(&format!(
                "/// The hook's declared parameters at the engine boundary.\n#[doc(hidden)]\n#[allow(missing_docs)]\n// no wire: internal call carrier, not an owned boundary value.\npub struct {carrier}{generics} {{\n{fields}}}\n"
            ));
        }
        if paired_hands {
            dispatch_params.push("__accept_hands: &mut dyn FnMut(Option<&Option<ItemStack>>, Option<&Option<ItemStack>>) -> Result<(), (&'static str, String)>".into());
        } else {
            for (field, ty) in checked_fields
                .iter()
                .copied()
                .chain(checked_replacement.map(|(field, ty)| (field, mutable_pointee(ty).unwrap())))
            {
                dispatch_params.push(format!(
                    "__accept_{field}: &mut dyn FnMut(&{}) -> Result<(), String>",
                    ty.to_token_stream()
                ));
            }
        }
        if inert_signal {
            dispatch_params.push("inert: &mut bool".into());
        }
        if panic_signal {
            dispatch_params.push("panicked: &mut bool".into());
        }
        let returns_slot = name == "is_shearable";
        let sets_registrar_owner = matches!(name, "register_goals" | "register_behaviors");
        if single_part_slot {
            dispatch_params.push("__part_slot: (usize, usize)".into());
        }
        if entity_kind_param {
            dispatch_params.push("__entity_kind: &str".into());
        }
        let (generics, elided) = match (lifetimes.a, lifetimes.b) {
            (_, true) => ("<'a, 'b>", "<'_, '_>"),
            (true, false) => ("<'a>", "<'_>"),
            (false, false) => ("", ""),
        };
        let wire_comment = if declaration.is_some() {
            "// no wire: this event borrows a declaration sink and its diagnostics.\n"
        } else if registration_append.is_some() {
            "// no wire: this event borrows the append-only staging vector.\n"
        } else if staged.is_some() && !staged_host {
            "// no wire: this event borrows the current listener's staging surface.\n"
        } else if !surfaces.is_empty() {
            "// no wire: this event borrows live callback surfaces.\n"
        } else {
            ""
        };
        let event_struct = format!(
            "/// An in-process callback record built by the game and passed to the listener.\n{wire_comment}#[non_exhaustive]\n#[allow(missing_docs)]\npub struct {event_type}{generics} {{\n{fields}}}\n"
        );
        if is_render {
            if hook.output_validator.is_some() {
                return Err(format!("{name}: render hooks do not use output_validator"));
            }
            render_dispatch.push_str(&event_struct);
            // The raw function is still the replaceable entry point. Its real caller goes
            // through this private typed adapter so a declaration cannot drift from it.
            let forwarded = carried
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            render_fire.push_str(&format!(
                "#[inline] pub(super) fn fire_{name}({}) {{ super::{name}({forwarded}); }}\n",
                dispatch_params.join(", ")
            ));
            continue;
        }
        trait_params.push(format!("{event}: &mut {event_type}{elided}"));
        call_args.push(format!("&mut {event}"));
        let unused = trait_params[1..]
            .iter()
            .map(|param| format!("let _ = {};", param.split(':').next().unwrap()))
            .collect::<String>();
        let default = match contract {
            _ if collect_cancel => "Flow::Continue",
            Contract::Cancel => "Flow::Continue",
            Contract::Replace => "None",
            Contract::Collect if answered_return => "None",
            _ => "",
        };
        // Direct game targets and free functions own their crossing. Everywhere else the
        // native method calls this typed adapter: rustc checks that call, not a source scan.
        if emit_fire && in_game && hook.dispatch != DispatchStyle::Direct {
            let (owner, is_game_assoc, scoped_read_host, borrowed) = if let Some(binding) =
                binding.as_ref()
            {
                match &binding.kind {
                    BindingKind::Shared => (FireOwner::Shared, false, false, None),
                    BindingKind::Nominal => (FireOwner::Game, false, false, None),
                    BindingKind::Free => (FireOwner::Free, true, false, None),
                    BindingKind::Borrowed(ty) => {
                        let scoped = matches!(ty, Type::Path(ty) if ty.path.segments.last().is_some_and(|segment| segment.ident == "NativeHookHost"));
                        (FireOwner::Free, true, scoped, Some(ty))
                    }
                    BindingKind::Scoped(owner) => (
                        FireOwner::Scoped(owner.clone()),
                        false,
                        owner.ident == "NativeHookHost",
                        None,
                    ),
                }
            } else {
                let target_path: syn::Path = syn::parse_str(target)
                    .map_err(|error| format!("invalid target for {name}: {error}"))?;
                let receiver = &target_path.segments[target_path.segments.len() - 2];
                let is_game_receiver = matches!(
                    receiver.ident.to_string().as_str(),
                    "Game" | "ServerGame" | "ClientGame"
                );
                let is_game_assoc = hook.receiver == Receiver::Associated
                    && (is_game_receiver
                        || (target.starts_with("sunrise_client::game::native_hooks::")
                            && target_path.segments.len() == 5));
                let owner = if is_game_assoc {
                    FireOwner::Free
                } else if is_game_receiver {
                    FireOwner::Game
                } else {
                    FireOwner::Scoped(receiver.clone())
                };
                let scoped_read_host = receiver.ident == "NativeHookHost";
                (owner, is_game_assoc, scoped_read_host, None)
            };
            let package = binding.as_ref().map(|binding| binding.package);
            let engine_receiver = EngineReceiver {
                package,
                shared: matches!(&owner, FireOwner::Shared),
                expression: if borrowed.is_some() {
                    "__fire_receiver"
                } else {
                    "self"
                },
            };
            let receiver = engine_receiver.expression;
            if scoped_read_host
                && (hook.host != Some(HostAccess::Read) || receiver_mut != Some(false))
            {
                return Err(format!(
                    "{name}: NativeHookHost requires a read-only host and receiver"
                ));
            }
            let native_host = host.as_ref().map(|(_, param)| {
                param.argument.render_for(
                    receiver,
                    package.is_some(),
                    scoped_read_host,
                    name == "place_liquid",
                    receiver_mut,
                )
            });
            let mut fire_carrier = None;
            let mut parameters = Vec::new();
            let mut forwarded = Vec::new();
            let mut parameter = |name: &str, ty: &str| {
                parameters.push(format!("{name}: {ty}"));
                forwarded.push(name.to_owned());
            };
            if let Some(ty) = borrowed {
                let mutability = if receiver_mut == Some(true) {
                    "mut "
                } else {
                    ""
                };
                parameter(receiver, &format!("&{mutability}{}", ty.to_token_stream()));
            }
            if let Some(ctx) = hook.context {
                parameter("context", host_param::phase_engine_type(ctx));
            }
            if ticks {
                parameter("ticks", "&mut Ticks");
            }
            let mut before = String::new();
            let mut after = String::new();
            let mut passed: Vec<_> = native_host.into_iter().collect();
            if uses_carrier && hook.dispatch == DispatchStyle::ApiCarrier {
                let (_, generics) = carrier_fields(&carried);
                parameter(
                    "call",
                    &format!("mod_api::__private::{carrier}{}", elide(generics)),
                );
                passed.push("call".to_owned());
            } else {
                let mut surfaces = std::collections::HashMap::new();
                let engine = carried
                    .iter()
                    .map(|(arg, ty)| {
                        let mut ty = (*ty).clone();
                        if let Some(draw) = DrawContext::of(contexts, &ty) {
                            if receiver_mut != Some(false) {
                                return Err(format!(
                                    "{name}: a drawing hook takes &self and its surface as &mut"
                                ));
                            }
                            surfaces.insert(
                                arg.as_str(),
                                if staged_draw {
                                    format!("|listen| crate::game::host::draw::GameDrawContext::stage({receiver}, {arg}, listen)")
                                } else {
                                    format!(
                                        "&mut crate::game::host::draw::{}::new({receiver}, {arg})",
                                        draw.game_struct()
                                    )
                                },
                            );
                            let surface = if package == Some(crate::GameplayPackage::Client) {
                                match draw.surface.as_str() {
                                    "crate::hud::HudBatch" => "sunrise_client_render::hud::HudBatch",
                                    "crate::world_geometry::WorldGeometry" => {
                                        "sunrise_client_lib::world_geometry::WorldGeometry"
                                    }
                                    other => other,
                                }
                            } else {
                                &draw.surface
                            };
                            ty = syn::parse_str(&format!("&mut {surface}"))
                                .map_err(|e| format!("{name}: invalid draw surface: {e}"))?;
                        } else {
                            engine_types(&mut ty, uses_carrier);
                        }
                        Ok((arg.clone(), ty))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if uses_carrier {
                    let refs: Vec<_> = engine.iter().map(|(arg, ty)| (arg.clone(), ty)).collect();
                    let (fields, generics) = carrier_fields(&refs);
                    fire_carrier = Some(FireCarrier {
                        name: carrier.clone(),
                        generics,
                        fields,
                    });
                    parameter("call", &format!("{carrier}{}", elide(generics)));
                    before.push_str(&format!(
                        "let {carrier} {{ {} }} = call; ",
                        carried
                            .iter()
                            .map(|(name, _)| name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                } else {
                    for (arg, ty) in &engine {
                        if let Some(entry) =
                            declaration.as_ref().filter(|_| Some(arg.as_str()) == out)
                        {
                            parameter(
                                arg,
                                &format!("&mut mod_api::__private::DeclarationStage<'_, {entry}>"),
                            );
                        } else if let Some((_, _, signature)) = staged
                            .as_ref()
                            .filter(|(field, _, _)| !staged_draw && field.as_str() == arg.as_str())
                        {
                            parameter(arg, signature);
                        } else {
                            parameter(arg, &ty.to_token_stream().to_string());
                        }
                    }
                }
                let crossing = stack_crossing_for(
                    engine_receiver,
                    &args,
                    output.as_ref(),
                    checked_crossing,
                    paired_hands,
                    hook_id,
                    scoped.is_some(),
                    hook.scope.as_ref().and_then(|scope| match scope.kind {
                        PartKind::Block => Some(scope.key.as_str()),
                        _ => None,
                    }),
                    observer_routed,
                    &surfaces,
                    match hook.stack_count {
                        Some(crate::catalog::StackCount::Trim) => hook.out.as_deref(),
                        None => None,
                    },
                );
                if package.is_some()
                    && matches!(owner, FireOwner::Free)
                    && borrowed.is_none()
                    && crossing
                        .as_ref()
                        .is_some_and(|crossing| crossing.needs_receiver)
                {
                    return Err(format!("{name}: conversion requires a receiver context"));
                }
                let (values, validators) = if let Some(crossing) = crossing {
                    before.push_str(&crossing.before);
                    after = crossing.after;
                    (crossing.arguments, crossing.validators)
                } else {
                    (
                        args.iter()
                            .chain(output.as_ref())
                            .map(|(arg, _)| arg.clone())
                            .collect(),
                        Vec::new(),
                    )
                };
                let values = values
                    .into_iter()
                    .map(|arg| surfaces.get(arg.as_str()).cloned().unwrap_or(arg));
                if uses_carrier {
                    let fields = args
                        .iter()
                        .chain(output.as_ref())
                        .zip(values)
                        .map(|((arg, _), value)| {
                            if *arg == value {
                                value
                            } else {
                                format!("{arg}: {value}")
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    passed.push(format!("mod_api::__private::{carrier} {{ {fields} }}"));
                } else {
                    passed.extend(values);
                }
                passed.extend(validators);
            }
            if inert_signal {
                parameter("inert", "&mut bool");
                passed.push("inert".to_owned());
            }
            if panic_signal {
                parameter("panicked", "&mut bool");
                passed.push("panicked".to_owned());
            }
            if name == "add_pack_finders" {
                parameter("allow", "impl FnMut(&str) -> bool");
            }
            if entity_kind_param {
                parameter("entity_kind", "&str");
                passed.push("entity_kind".to_owned());
            }
            let passed = passed.join(", ");
            let dispatch_call = if name == "add_pack_finders" {
                format!("mod_api::dispatch_{name}_for({passed}, allow);")
            } else {
                format!("mod_api::dispatch_{name}({passed});")
            };
            fire.push(FireUnit {
                name: name.to_owned(),
                package,
                owner,
                side: hook.side,
                carrier: fire_carrier,
                receiver: if borrowed.is_some() {
                    Receiver::Associated
                } else {
                    hook.receiver
                },
                parameters,
                before,
                dispatch: dispatch_call,
                after,
            });
            let receiver = if let Some(binding) = binding.as_ref() {
                match binding.kind {
                    BindingKind::Shared => {
                        forwarded.insert(0, "self".into());
                        "crate::game::native_hooks::fire::FireHooks::"
                    }
                    BindingKind::Free | BindingKind::Borrowed(_) => {
                        "crate::game::native_hooks::fire::"
                    }
                    _ => "self.",
                }
            } else if is_game_assoc {
                "super::fire::"
            } else if hook.receiver == Receiver::Associated {
                "Self::"
            } else {
                "self."
            };
            if let Some((field, _, _)) = &staged
                && !staged_draw
                && let Some(argument) = forwarded
                    .iter_mut()
                    .find(|argument| argument.as_str() == field.as_str())
            {
                *argument = format!("|listen| {field}.register_listener(listen)");
            }
            if panic_signal {
                forwarded.push("panicked".into());
            }
            if name == "add_pack_finders" {
                forwarded.push("|_| true".into());
            }
            fills.push(Fill {
                native,
                call: format!("{receiver}fire_{name}({});", forwarded.join(", ")),
            });
        }
        // Keep part lookup and first-Some selection here even when the engine
        // lends a fresh (e.g. transactional) write host to each invocation.
        let scoped_write_replace = contract == Contract::Replace
            && scoped.is_some()
            && !observer_routed
            && host
                .as_ref()
                .is_some_and(|(_, param)| param.ty == "&mut dyn LevelWriteHost");
        let scoped_random_collect = contract == Contract::Collect
            && scoped.is_some()
            && !observer_routed
            && host
                .as_ref()
                .is_some_and(|(_, param)| param.ty == "LevelRandomSide");
        let filtered_subscribers = name == "add_pack_finders";
        let movement_side_filter = matches!(
            name,
            "entity_move" | "living_travel" | "living_travel_moved"
        );
        let allow_subscriber = fresh_name("allow", &mut used);
        let client_controlled = fresh_name("client_controlled", &mut used);
        let invoke_part = fresh_name("invoke_part", &mut used);
        let invoke = call_args.join(", ");
        let trait_params = trait_params.join(", ");
        let dispatch_signature = dispatch_params.join(", ");
        let trait_name = "Hooks";
        let mut method_source = String::new();
        for line in hook.doc.trim().lines() {
            method_source.push_str(&format!(
                "///{}{line}\n",
                if line.is_empty() { "" } else { " " }
            ));
        }
        let side = match hook.side {
            Side::Server => "server",
            Side::Client => "client",
            Side::Both => "both",
        };
        method_source.push_str(&format!("/// Side: {side}.\n"));
        // A scoped hook is its part's method alone: a global moment beside it is its own hook.
        let callee = if scoped.is_some() { "part" } else { "mod" };
        match (contract, out) {
            (Contract::Cancel, Some(out)) if mod_cancel && hook.output_validator.is_some() => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to propose setting `{out}`. Once accepted, a cancellation cannot be\n/// cleared; only mods registered with `#[listener(receive_cancelled)]` still run.\n"
            )),
            (Contract::Cancel, Some(out)) if mod_cancel => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later mods are skipped unless they\n/// listen with `#[listener(receive_cancelled)]`; those see `{event}.cancelled` and may set it back to\n/// `false`.\n"
            )),
            (Contract::Cancel, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later {callee}s are skipped.\n"
            )),
            (Contract::Replace, Some(out)) if checked_replacement.is_some() => method_source.push_str(&format!(
                "///\n/// Return `Some` to propose `{out}`; the first accepted answer wins.\n"
            )),
            (Contract::Replace, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Some` to set `{out}`; the first {callee} returning `Some` wins.\n"
            )),
            (Contract::Collect, _) if declaration.is_some() => (),
            (Contract::Collect, _) if registration_append.is_some() => (),
            (Contract::Collect, Some(_)) if answered_return => method_source.push_str(
                "///\n/// Return `Some` to give one answer, or `None` to abstain. The game collects each mod's answer in order.\n"
            ),
            (Contract::Collect, Some(out)) if answered => method_source.push_str(&format!(
                "///\n/// Each mod receives its own empty `{event}.{out}` batch; every entry it adds is recorded\n/// as that mod's.\n"
            )),
            (Contract::Collect, Some(out)) => method_source.push_str(&format!(
                "///\n/// Every {callee} gets `{event}.{out}` in order and may add, change or remove {out}.\n"
            )),
            _ => {}
        }
        if !lists.is_empty() {
            method_source.push_str("///\n/// Lists in this event are checked at each edit. A refused entry does not change\n/// another entry or cancel the operation; a panic discards the listener's own list edits.\n");
        }
        if paired_hands {
            method_source.push_str("///\n/// A listener's two hand proposals are accepted together. A refused pair does not\n/// discard a veto.\n");
        } else if !checked_fields.is_empty() {
            method_source.push_str("///\n/// Each changed value is checked on return. A refused value leaves the prior\n/// value intact without discarding separate edits or a veto.\n");
        }
        method_source.push_str(&format!(
            "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
        ));
        match &scoped {
            Some((part_trait, _)) => part_traits
                .entry(*part_trait)
                .or_default()
                .push_str(&method_source),
            None => {
                traits.push_str(&method_source);
                if let Some(part_kind) = hook.part {
                    let (part_trait, _) = part_trait_and_table(part_kind);
                    let mut part_method = String::new();
                    for line in hook.doc.trim().lines() {
                        part_method.push_str(&format!(
                            "///{}{line}\n",
                            if line.is_empty() { "" } else { " " }
                        ));
                    }
                    part_method.push_str(&format!("/// Side: {side}.\n"));
                    if let Some(out) = out {
                        part_method.push_str(&format!(
                            "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later parts are skipped.\n"
                        ));
                    }
                    part_method.push_str(&format!(
                        "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
                    ));
                    part_traits
                        .entry(part_trait)
                        .or_default()
                        .push_str(&part_method);
                }
            }
        }
        let trait_name = scoped
            .as_ref()
            .map_or(trait_name, |(part_trait, _)| part_trait);
        let event_doc = match registration_append {
            Some((RegistrationAppend::Register, _)) => {
                "Use [`Self::register`] to add entries without changing earlier registrations."
            }
            Some((RegistrationAppend::Custom, _)) => {
                "Use its registration methods to add entries without changing earlier registrations."
            }
            Some((RegistrationAppend::Attributed, _)) => {
                "Use its scoped methods to record attributed entries without changing earlier ones."
            }
            None => {
                "Each field is that hook's argument of the same name, described in the hook's doc."
            }
        };
        dispatch.push_str(&format!(
            "/// What [`{trait_name}::{name}`] is told. {event_doc}\n///\n{event_struct}"
        ));
        if let Some((RegistrationAppend::Register, entry)) = registration_append {
            // An elided lifetime within the vector belongs to the entry, not the vector borrow.
            let mut entry = entry.clone();
            let mut entry_lifetimes = EventLifetimes {
                in_mut: true,
                ..EventLifetimes::default()
            };
            entry_lifetimes.visit_type_mut(&mut entry);
            let (impl_params, event_params) = if entry_lifetimes.b {
                ("<'b>", "<'_, 'b>")
            } else {
                ("", elided)
            };
            let out = out.unwrap();
            let entry = entry.to_token_stream();
            dispatch.push_str(&format!(
                "impl{impl_params} {event_type}{event_params} {{\n\
                 /// Stage one entry for registration.\n\
                 pub fn register(&mut self, entry: {entry}) {{ self.{out}.push(entry); }}\n}}\n"
            ));
        }
        if let Some(entry) = &declaration {
            let out = out.unwrap();
            dispatch.push_str(&format!(
                "impl {event_type}<'_> {{\n\
                 /// Submit a declaration in a manifest-owned namespace. A bare name or reference\n\
                 /// uses the mod's ID as its namespace. A locally invalid declaration returns\n\
                 /// a [`DeclarationError`] and is also reported at startup if ignored by a returning listener.\n\
                 /// A panicking listener's declarations and diagnostics are discarded. Collisions\n\
                 /// and checks needing other declarations or resources run after collection.\n\
                 pub fn register(&mut self, entry: {entry}) -> Result<(), DeclarationError> {{\n\
                     super::parts::submit_declaration(self.refusals, self.mod_id, self.{out}, entry)\n\
                 }}\n}}\n"
            ));
        }
        if declaration_only {
            continue;
        }
        let build = if declaration.is_some() || name == "entity_inside" {
            String::new()
        } else {
            format!(
                "{}let mut {event} = {event_type} {{ {} }};",
                if answered && contract == Contract::Collect && !answered_return {
                    format!("let mut {batch} = Vec::new(); ")
                } else {
                    String::new()
                },
                inits.join(", "),
            )
        };
        // Each listener gets a fresh event with the original inputs. Writable borrowed
        // payloads can still share their pointee. The owner filter and chunk I/O checks
        // use the outer event and cannot use a listener-local one.
        let local_cancel = hook.event_per_listener && mod_cancel;
        let (build, local_event) = if staged.is_some() {
            (String::new(), build)
        } else if hook.event_per_listener || answered_return {
            // The ordinary collect output and the accepted cancellation live outside the event.
            let reply_outside_event =
                matches!(contract, Contract::Listen | Contract::Replace) || answered_return;
            let shared_collect_output = collected.is_some() && !answered && !collect_cancel;
            let local_cancel_outputs = local_cancel
                && contract == Contract::Cancel
                && hook.output_validator.is_none()
                && surfaces.is_empty();
            if (!reply_outside_event && !shared_collect_output && !local_cancel_outputs)
                || hook.owner.is_some()
                || hook.context == Some(PhaseContext::ChunkDisk)
            {
                return Err(format!(
                    "{name}: event_per_listener requires a listen or replace hook, a collect hook with a shared borrowed output, or a global cancel hook with rollback-capable outputs and no output validator"
                ));
            }
            (String::new(), build)
        } else {
            (build, String::new())
        };
        // A per-listener event is rebuilt from the current scalars for each listener and writes
        // its own scalars back, so the listeners chain only those outputs.
        let (write_back, local_write_back) = if hook.event_per_listener {
            (String::new(), write_back)
        } else {
            (write_back, String::new())
        };
        // Restore pending event outputs on unwind; owning sinks publish only completed batches.
        let saved = fresh_name("saved", &mut used);
        let mut snaps: Vec<String> = Vec::new();
        let mut restores: Vec<String> = Vec::new();
        // The same restores for a listener's own transaction, which can refuse while the
        // snapshots are still read after it: an owned one is borrowed there, not moved.
        let mut restores_in_place: Vec<String> = Vec::new();
        // The same snapshots serve panic rollback and changed-output detection.
        let mut checked_indices = Vec::new();
        let answered_collect_batch =
            answered && contract == Contract::Collect && declaration.is_none() && !answered_return;
        // Trait-object fields have no whole value to copy. For a staged registrar the
        // owning sink handles publication; drawing and other live views are not registrars.
        for (field, by_reference) in rollback
            .iter()
            .filter(|(field, by_reference)| !surfaces.contains(field) || !*by_reference)
        {
            // A rebuilt event's owned scalars cannot escape an aborted listener.
            if hook.event_per_listener && !by_reference {
                continue;
            }
            if answered_collect_batch && Some(field.as_str()) == out {
                continue;
            }
            if lists.contains(field) {
                continue;
            }
            let index = snaps.len();
            if checked_fields.iter().any(|(name, _)| *name == field) {
                checked_indices.push((field, index));
            }
            if (event_type == "RegisterConfigurationTasksEvent" && field == "registration")
                || (event_type == "ModifyCustomSpawnersEvent" && field == "spawners")
                || (event_type == "ItemStackedOnOtherEvent" && field == "click")
                || (event_type == "RegisterGoalsEvent" && field == "goals")
                || (event_type == "RegisterBehaviorsEvent" && field == "behaviors")
            {
                snaps.push(format!("{event}.{field}.checkpoint()"));
                let restore = format!("{event}.{field}.rollback({saved}.{index});");
                restores.push(restore.clone());
                restores_in_place.push(restore);
                continue;
            }
            if (registration_append.is_some() && Some(field.as_str()) == out)
                || (event_type == "RegisterRenderPipelinesEvent" && field == "render_types")
                || (event_type == "RegisterRenderPipelinesEvent" && field == "compute_pipelines")
            {
                // Only the tail is writable; never clone or restore earlier registrations.
                snaps.push(format!("{event}.{field}.len()"));
                let restore = format!("{event}.{field}.truncate({saved}.{index});");
                restores.push(restore.clone());
                restores_in_place.push(restore);
                continue;
            }
            snaps.push(if cloned.contains(field) {
                // `to_owned`, not `clone`: this generator cannot tell a `Copy` field type from an
                // owned one, and `clippy::clone_on_copy` would flag a `clone` of the first.
                format!("ToOwned::to_owned(&*{event}.{field})")
            } else if *by_reference {
                format!("*{event}.{field}")
            } else {
                format!("{event}.{field}")
            });
            let target = format!("{}{event}.{field}", if *by_reference { "*" } else { "" });
            restores.push(format!("{target} = {saved}.{index};"));
            if cloned.contains(field) {
                restores_in_place.push(format!("{target} = {saved}.{index}.clone();"));
            } else {
                restores_in_place.push(format!("{target} = {saved}.{index};"));
            }
        }
        let previous_cancel = snaps.len();
        // A listener-local event's cancellation never reaches the dispatch's flag while its
        // listener runs, so an aborted listener has nothing to undo there.
        if mod_cancel && !local_cancel {
            restores.push(format!("{event}.cancelled = {saved}.{previous_cancel};"));
            snaps.push(format!("{event}.cancelled"));
        }
        let snapshot_statement = if snaps.is_empty() {
            String::new()
        } else {
            format!("let {saved} = ({},); ", snaps.join(", "))
        };
        let restore_statement = if snaps.is_empty() {
            String::new()
        } else {
            format!("{} ", restores.join(" "))
        };
        let restore_in_place = if snaps.is_empty() {
            String::new()
        } else {
            format!("{} ", restores_in_place.join(" "))
        };
        // Accepted batches are already in the caller's output. Detach only the failed
        // listener's private batch and run each destructor under its own panic fence.
        let discarded = fresh_name("discarded", &mut used);
        let list_enter = |owner: &str| {
            lists
                .iter()
                .map(|field| format!("{event}.{field}.enter({owner});"))
                .collect::<String>()
        };
        let list_finish = |accepted: bool| {
            lists
                .iter()
                .map(|field| format!("{event}.{field}.finish({accepted});"))
                .collect::<String>()
        };
        let refused = |owner: &str, field: &str| {
            format!(
                "log::warn!(\"mod {{}} hook {name} output {{}} refused: {{error}}\", {owner}, {field});"
            )
        };
        let admit_fields = |owner: &str| {
            if paired_hands {
                let [(main, main_index), (off, off_index)] = checked_indices.as_slice() else {
                    unreachable!("swap hands has exactly two stack proposals");
                };
                let refusal = refused(owner, "field");
                format!("let main = (*{event}.{main} != {saved}.{main_index}).then_some(&*{event}.{main});
                         let off = (*{event}.{off} != {saved}.{off_index}).then_some(&*{event}.{off});
                         if (main.is_some() || off.is_some()) && let Err((field, error)) = __accept_hands(main, off) {{
                            *{event}.{main} = {saved}.{main_index}; *{event}.{off} = {saved}.{off_index}; {refusal}
                         }}")
            } else {
                checked_indices
                    .iter()
                    .map(|(field, index)| {
                        let refusal = refused(owner, &format!("\"{field}\""));
                        // Each field's check is a statement of its own line: `} if` on one line reads
                        // as an `else if` whose `else` is missing (`clippy::suspicious_else_formatting`).
                        format!(
                            "if *{event}.{field} != {saved}.{index} && let Err(error) = __accept_{field}(&*{event}.{field}) {{ *{event}.{field} = {saved}.{index}; {refusal} }}\n"
                        )
                    })
                    .collect()
            }
        };
        let report_panic = if panic_signal {
            "*panicked = true; "
        } else {
            ""
        };
        // Collision shape answers fail only the invocation, with the same
        // per-listener scope and output rollback as other hooks.
        let transient_answer = name == "collision_shape";
        let callback = if hook.independent_listener {
            format!("super::runtime::Callback::IndependentHook(HookId::{hook_id})")
        } else {
            format!("HookId::{hook_id}")
        };
        let run = |mod_id_expr: &str, on_abort: &str, expression: &str| {
            let invocation = if transient_answer {
                format!(
                    "super::runtime::invoke_without_disabling(\"{name}\", {mod_id_expr}, || {expression})"
                )
            } else {
                format!(
                    "super::runtime::invoke({callback}, \"{name}\", {mod_id_expr}, || {expression})"
                )
            };
            let discard_batch = if answered_collect_batch {
                let output = out.unwrap();
                format!(
                    "for {discarded} in core::mem::take({event}.{output}) {{ \
                        super::runtime::discard_registration((HookId::{hook_id}, \"{name}\"), Some({mod_id_expr}), {discarded}); \
                     }} "
                )
            } else {
                String::new()
            };
            if restore_statement.is_empty()
                && discard_batch.is_empty()
                && lists.is_empty()
                && checked_indices.is_empty()
                && report_panic.is_empty()
            {
                match on_abort {
                    "false" => return format!("{invocation}.unwrap_or_default()"),
                    "" => return format!("let _ = {invocation}"),
                    _ => {}
                }
            }
            let enter = list_enter(mod_id_expr);
            let finish = list_finish(true);
            let rollback_lists = list_finish(false);
            let admit = admit_fields(mod_id_expr);
            let decision = format!(
                "match {invocation} {{ Some(answer) => {{ {finish}{admit} answer }}, None => {{ {report_panic}{restore_statement}{discard_batch}{rollback_lists}{on_abort} }} }}"
            );
            // A block that only holds the match is a statement without effect
            // (`clippy::unnecessary_operation`); it scopes the entered lists' names.
            if enter.is_empty() {
                decision
            } else {
                format!("{{ {enter} {decision} }}")
            }
        };
        let listener_call = |callee: &str, mod_id_expr: &str| {
            let expression = format!("{callee}.{name}({invoke})");
            if panic_signal {
                // Quick move chains the healthy Parts' proposals. Abort only the failing
                // Part's storage writes before the shared panic boundary restores its outputs.
                format!(
                    "{{\
                         use super::item::transaction::TransactionError;\
                         match {event}.subject.transaction(|subject, _| -> Result<_, core::convert::Infallible> {{\
                             Ok({callee}.{name}(&mut {event_type} {{\
                                 kind: {event}.kind, subject: subject.reborrow(),\
                                 container_slots: {event}.container_slots, slot: {event}.slot,\
                                 stack: &mut *{event}.stack,\
                             }}))\
                         }}) {{\
                             Ok(answer) => answer,\
                             Err(TransactionError::Aborted(never)) => match never {{}},\
                             Err(error @ (TransactionError::InvalidContext | TransactionError::Unavailable | TransactionError::CommitRefused)) => {{\
                                 log::warn!(\"mod {{}}: {name} callback refused: {{error:?}}\", {mod_id_expr});\
                                 Flow::Continue\
                             }}\
                         }}\
                     }}"
                )
            } else if name == "render_tooltip_gather_components" {
                format!(
                    "{{ let answer = {expression}; super::client::tooltip_components::retain_gather_tooltip_payloads({mod_id_expr}, {event}.elements.as_slice()); answer }}"
                )
            } else if let Some(transaction) = hook.listener_transaction.or_else(|| {
                matches!(
                    name,
                    "contents_changed"
                        | "farmland_trample"
                        | "on_sheared"
                        | "register_configuration_tasks"
                        | "register_goals"
                        | "register_behaviors"
                        | "server_about_to_start"
                        | "server_starting"
                        | "server_started"
                        | "server_stopping"
                        | "server_stopped"
                        | "config_unloading"
                        | "level_load"
                        | "level_save"
                        | "level_unload"
                        | "create_spawn_position"
                        | "player_save_to_file"
                        | "world_saved"
                )
                .then_some(ListenerTransaction::Always)
            }) {
                // Let the existing journal see the unwind before invoke catches it.
                let (host, host_param) = host.as_ref().unwrap();
                let sided = host_param.ty == "HostSide";
                let (commit, no_answer) =
                    if matches!(contract, Contract::Listen | Contract::Collect) && !answered_return
                    {
                        (format!("{expression}; Ok(())"), "")
                    } else {
                        (format!("Ok({expression})"), "Flow::Continue")
                    };
                if sided {
                    // A both-side host: the server's writes are one transaction; the client's
                    // copy has no journal to commit, so its callback runs as it is.
                    format!(
                        "{{\
                             use super::item::transaction::{{HostTransaction, TransactionError}};\
                             match &mut {host} {{\
                                 HostSide::Server(side) => match HostTransaction::transaction(&mut **side, |{host}, _| -> Result<_, core::convert::Infallible> {{ let mut {host} = HostSide::Server({host}); {commit} }}) {{\
                                     Ok(answer) => answer,\
                                     Err(TransactionError::Aborted(never)) => match never {{}},\
                                     Err(error @ (TransactionError::InvalidContext | TransactionError::Unavailable | TransactionError::CommitRefused)) => {{\
                                         log::warn!(\"mod {{}}: {name} callback refused: {{error:?}}\", {mod_id_expr});\
                                         {restore_in_place} {no_answer}\
                                     }}\
                                 }},\
                                 HostSide::Client(side) => {{ let mut {host} = HostSide::Client(&mut **side); {expression} }}\
                             }}\
                         }}"
                    )
                } else {
                    let journalled = format!(
                        "{{\
                             use super::item::transaction::{{HostTransaction, TransactionError}};\
                             match HostTransaction::transaction(&mut *{host}, |{host}, _| -> Result<_, core::convert::Infallible> {{ {commit} }}) {{\
                                 Ok(answer) => answer,\
                                 Err(TransactionError::Aborted(never)) => match never {{}},\
                                 Err(error @ (TransactionError::InvalidContext | TransactionError::Unavailable | TransactionError::CommitRefused)) => {{\
                                     log::warn!(\"mod {{}}: {name} callback refused: {{error:?}}\", {mod_id_expr});\
                                     {restore_in_place} {no_answer}\
                                 }}\
                             }}\
                         }}"
                    );
                    match transaction {
                        ListenerTransaction::Always => journalled,
                        ListenerTransaction::WhenOpen => format!(
                            "if !{host}.transaction_matches(None) {{ {journalled} }} else {{ {expression} }}"
                        ),
                    }
                }
            } else if matches!(name, "level_tick_pre" | "level_tick_post") {
                // Only the server Host owns a journal. The client side still uses invoke
                // for panic/phase refusal handling, as do the client Load and Unload hooks.
                let (host, _) = host.as_ref().unwrap();
                format!(
                    "match {host}.reborrow() {{\
                        HostSide::Server({host}) => {{\
                            use super::item::transaction::{{HostTransaction, TransactionError}};\
                            match HostTransaction::transaction({host}, |{host}, _| -> Result<(), core::convert::Infallible> {{\
                                let mut {host} = HostSide::Server({host});\
                                {expression}; Ok(())\
                            }}) {{\
                                Ok(()) => {{}}\
                                Err(TransactionError::Aborted(never)) => match never {{}},\
                                Err(error @ (TransactionError::InvalidContext | TransactionError::Unavailable | TransactionError::CommitRefused)) => {{\
                                    log::warn!(\"mod {{}}: {name} callback refused: {{error:?}}\", {mod_id_expr});\
                                }}\
                            }}\
                        }}\
                        mut {host} => {expression},\
                    }}"
                )
            } else {
                expression
            }
        };
        // Only a completed listener has a proposal for the validator to check. The same
        // snapshot that handles panics preserves earlier accepted edits on refusal.
        let validate_proposal = hook.output_validator.as_ref().map(|validator| {
            let field = fresh_name("refused_field", &mut used);
            let arguments = rollback
                .iter()
                .enumerate()
                .map(|(index, _)| format!("&{saved}.{index}"))
                .chain(rollback.iter().map(|(name, by_reference)| {
                    format!("&{}{event}.{name}", if *by_reference { "*" } else { "" })
                }))
                .collect::<Vec<_>>()
                .join(", ");
            let restore_statement = &restore_statement;
            move |mod_id_expr: &str| {
                format!(
                    "if let Err({field}) = {validator}({arguments}) {{
                         {restore_statement}
                         super::runtime::report_output_refusal({mod_id_expr}, \"{name}\", {field});
                         continue;
                     }}"
                )
            }
        });
        // One callee's call: a mod's, or a scoped hook's part's.
        let call = |callee: &str, mod_id_expr: &str, on_abort: &str, exit: &str| {
            if let Some((field, stage, _)) = &staged {
                let accepted = run(
                    mod_id_expr,
                    "false",
                    &format!("{{ {callee}.{name}({invoke}); true }}"),
                );
                return format!(
                    "{stage}(&mut |{field}| {{ {local_event} {snapshot_statement}{accepted} }});"
                );
            }
            let answer_call = if scoped_write_replace {
                run(
                    mod_id_expr,
                    on_abort,
                    &format!("{invoke_part}({callee}, &mut {event})"),
                )
            } else {
                run(mod_id_expr, on_abort, &format!("{callee}.{name}({invoke})"))
            };
            let plain_call = if scoped_random_collect {
                let level = &call_args[0];
                run(
                    mod_id_expr,
                    on_abort,
                    &format!("{invoke_part}({callee}, {level}, &mut {event})"),
                )
            } else {
                run(mod_id_expr, on_abort, &listener_call(callee, mod_id_expr))
            };
            let invocation = match contract {
                Contract::Cancel => format!(
                    "if matches!({plain_call}, Flow::Cancel) {{ *{o} = true; {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Replace if answered => format!(
                    "if let Some({replacement}) = {answer_call} {{ *{o} = Some(Answered {{ answer: {replacement}, mod_id: {mod_id_expr} }}); {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Replace if checked_replacement.is_some() => {
                    let o = out.unwrap();
                    let refusal = refused(mod_id_expr, &format!("\"{o}\""));
                    format!("let proposal = {answer_call}; if proposal.is_some() {{
                        match __accept_{o}(&proposal) {{ Ok(()) => {{ *{o} = proposal; {exit}; }}, Err(error) => {{ {refusal} }} }}
                    }}")
                }
                Contract::Replace => format!(
                    "if let Some({replacement}) = {answer_call} {{ *{o} = Some({replacement}); {exit}; }}",
                    o = out.unwrap()
                ),
                Contract::Collect if answered_return => format!(
                    "if let Some({replacement}) = {answer_call} {{ {o}(Answered {{ answer: {replacement}, mod_id: {mod_id_expr} }}); }}",
                    o = out.unwrap()
                ),
                Contract::Collect if answered => format!(
                    "{plain_call}; {o}.extend({event}.{o}.drain(..).map(|answer| Answered {{ answer, mod_id: {mod_id_expr} }}));",
                    o = out.unwrap()
                ),
                Contract::Collect if attributed_append => {
                    format!("{event}.mod_id = {mod_id_expr}; {plain_call};")
                }
                Contract::Listen | Contract::Collect => match &validate_proposal {
                    Some(validate) => format!("{plain_call}; {}", validate(mod_id_expr)),
                    None => format!("{plain_call};"),
                },
            };
            format!("{local_event} {snapshot_statement}{invocation} {local_write_back}")
        };
        let cancel_invocation =
            |callee: &str, owner: &str| run(owner, "continue;", &listener_call(callee, owner));
        let plain_cancel_step = |invocation: &str| {
            format!(
                "{snapshot_statement}if matches!({invocation}, Flow::Cancel) {{ {event}.cancelled = true; }}"
            )
        };
        // An independent listener's veto joins the shared flag as its listener returns. The other
        // global cancel hooks also snapshot the cancellation and only commit it on acceptance.
        let cancel_step = if mod_cancel {
            let mod_id_expr = format!("{mod_entry}.id");
            let invocation = cancel_invocation(&mod_item, &mod_id_expr);
            if hook.independent_listener && local_cancel {
                let out = cancel_out.unwrap();
                let vetoed = fresh_name("vetoed", &mut used);
                format!(
                    "let {vetoed} = matches!({invocation}, Flow::Cancel);
                     if {vetoed} || (!*{out} && {event}.cancelled) {{
                         log::warn!(\"mod {{}}: hook {name} requested a veto\", {mod_entry}.id);
                     }}
                     {event}.cancelled |= {vetoed};"
                )
            } else if let Some(validate) = &validate_proposal {
                let flow = fresh_name("flow", &mut used);
                let validation = validate(&mod_id_expr);
                format!(
                    "{snapshot_statement}let {flow} = {invocation};
                     {validation}
                     {event}.cancelled |= {saved}.{previous_cancel} || matches!({flow}, Flow::Cancel);"
                )
            } else {
                plain_cancel_step(&invocation)
            }
        } else {
            String::new()
        };
        let inert_owner = fresh_name("disabled_owner", &mut used);
        let empty_return = if returns_slot {
            "return None;"
        } else if single_part_slot {
            "return Ok(());"
        } else {
            "return;"
        };
        let part_routing = if hook.context == Some(PhaseContext::Worldgen) {
            "super::worldgen_parts()".to_owned()
        } else {
            format!("super::parts(HookId::{hook_id}, \"{name}\")")
        };
        let part_routing = format!(
            "\nif !super::has_part_hook(HookId::{hook_id}) {{ {empty_return} }} let {part_table} = {part_routing};"
        );
        let hook_parts = format!("{part_table}.for_hook(HookId::{hook_id})");
        let part_selection = |lookup: &str| {
            format!(
                "{part_routing} let {part_list} = {hook_parts}.map_or(&[][..], |table| table.{lookup}); if {part_list}.is_empty() {{ {empty_return} }}"
            )
        };
        let active_guard = fresh_name("_active_listener", &mut used);
        let enter_active = |owner: &str| {
            if hook.independent_listener {
                format!(
                    "let {active_guard} = super::runtime::ActiveListener::enter(HookId::{hook_id}, {owner});"
                )
            } else {
                String::new()
            }
        };
        let allow_active =
            format!("|id| super::runtime::listener_is_inactive(HookId::{hook_id}, \"{name}\", id)");
        let isolated_menu_place = name == "recipe_book_place";
        let (lookup, calls) = match &scoped {
            None if declaration.is_some() => {
                let output = out.unwrap();
                let accepted = run(
                    &listener_id,
                    "false",
                    &format!("{{ {mod_item}.{name}(&listener_context, &mut {event}); true }}"),
                );
                (String::new(), format!(
                    "for ({listener_id}, {mod_item}) in super::entered_with_id(HookId::{hook_id}, \"{name}\") {{
                        let listener_context = context.for_mod({listener_id});
                        {output}(&mut |mut submit| super::parts::declaration_listener(&listener_context, |refusals| {{
                            let mut {event} = {event_type} {{ mod_id: {listener_id}, refusals, {output}: &mut submit }};
                            {accepted}
                        }}));
                    }}"
                ))
            }
            None if name == "consume_effect" => (
                String::new(),
                format!("callback.invoke(|{mod_item}| {mod_item}.{name}({invoke}))"),
            ),
            None if hook.owner.is_some() => {
                let owner_field = hook.owner.as_deref().unwrap();
                let owner_filter = if staged.is_some() {
                    owner_field.to_owned()
                } else {
                    format!("{event}.{owner_field}")
                };
                (
                    String::new(),
                    format!(
                        "{{ let mut {entered} = super::runtime::ListenerScope::new(); for (_, {mod_entry}) in super::listening(HookId::{hook_id}) {{ if {mod_entry}.id != {owner_filter} {{ continue; }} let {mod_item} = {entered}.enter({mod_entry}, \"{name}\"); {} }} }}",
                        call(&mod_item, &format!("{mod_entry}.id"), "continue;", "break"),
                    ),
                )
            }
            None if mod_cancel => {
                let out = cancel_out.unwrap();
                let (accepted, publish_each, publish_end) = if local_cancel {
                    (
                        format!("*{out}"),
                        format!("*{out} = {event}.cancelled; {local_write_back}"),
                        String::new(),
                    )
                } else {
                    (
                        format!("{event}.cancelled"),
                        String::new(),
                        format!("*{out} = {event}.cancelled;"),
                    )
                };
                let each = if hook.independent_listener {
                    format!("super::runtime::listening_matching(HookId::{hook_id}, {allow_active})")
                } else {
                    format!("super::listening(HookId::{hook_id})")
                };
                let active = enter_active(&format!("{mod_entry}.id"));
                let side_guard = if movement_side_filter {
                    format!(
                        "if {client_controlled} && {mod_entry}.side == super::Side::Server {{ continue; }}\n"
                    )
                } else {
                    String::new()
                };
                (
                    String::new(),
                    format!(
                        "{{ let mut {entered} = super::runtime::ListenerScope::new(); for ({receives}, {mod_entry}) in {each} {{ {side_guard}if {accepted} && !{receives} {{ continue; }} {active} let {mod_item} = {entered}.enter({mod_entry}, \"{name}\"); {local_event} {cancel_step} {publish_each} }} }} {publish_end}"
                    ),
                )
            }
            None if movement_side_filter => {
                let side_guard = format!(
                    "if {client_controlled} && {mod_entry}.side == super::Side::Server {{ continue; }}"
                );
                let active = enter_active(&format!("{mod_entry}.id"));
                (
                    String::new(),
                    format!(
                        "{{ let mut {entered} = super::runtime::ListenerScope::new(); for (_, {mod_entry}) in super::listening(HookId::{hook_id}) {{ {side_guard} {active} let {mod_item} = {entered}.enter({mod_entry}, \"{name}\"); {} }} }}",
                        call(&mod_item, &format!("{mod_entry}.id"), "continue;", "break"),
                    ),
                )
            }
            None => {
                let each = if hook.independent_listener {
                    format!(
                        "super::runtime::entered_with_id_matching(HookId::{hook_id}, \"{name}\", {allow_active})"
                    )
                } else if filtered_subscribers {
                    format!(
                        "super::runtime::entered_with_id_matching(HookId::{hook_id}, \"{name}\", {allow_subscriber})"
                    )
                } else {
                    format!("super::entered_with_id(HookId::{hook_id}, \"{name}\")")
                };
                (
                    String::new(),
                    format!(
                        "for ({listener_id}, {mod_item}) in {each} {{ {} {} }}",
                        enter_active(&listener_id),
                        call(&mod_item, &listener_id, "continue;", "break")
                    ),
                )
            }
            Some((_, lookup)) => (
                // The parts are looked up first: the key may be moved into the event. A worldgen
                // Part method runs on the worker threads too: it reads the server's published table.
                if hook.inert.is_some() {
                    let entry = fresh_name("part_entry", &mut used);
                    format!(
                        "{part_routing} let Some({entry}) = {hook_parts}.and_then(|table| table.{lookup}) else {{ {empty_return} }}; let {part_list} = {entry}.parts(); let {inert_owner} = if super::any_mod_disabled() {{ {entry}.disabled_owner_slot() }} else {{ None }};"
                    )
                } else {
                    part_selection(lookup)
                },
                if isolated_menu_place {
                    let host = &host.as_ref().unwrap().0;
                    format!(
                        "super::runtime::run_isolated_part_output({part_list}, {host}, placement, \"menu part\", \"{name}\", |{part_item}, {host}, placement| {{ {build} {part_item}.{name}({host}, &mut {event}); }});"
                    )
                } else if let Some(inert) = &hook.inert {
                    let inert = if inert_signal {
                        "*inert = true;".to_owned()
                    } else if let Some((out, _)) = checked_replacement {
                        // The inert assignment only changes the API slot. Admission writes the engine slot.
                        format!("{inert} if {out}.is_some() {{ let _ = __accept_{out}(&*{out}); }}")
                    } else {
                        inert.clone()
                    };
                    // The synthetic answer occupies the owner's original place in the chain.
                    // Earlier enabled Parts still get their regular call and can answer first.
                    format!(
                        "'dispatch_parts: {{ for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, &{part_list}[..{inert_owner}.unwrap_or({part_list}.len())], \"{name}\") {{ {} }} if {inert_owner}.is_some() {{ {inert} }} }}",
                        call(
                            &part_item,
                            &listener_id,
                            "continue;",
                            "break 'dispatch_parts"
                        ),
                    )
                } else if sets_registrar_owner {
                    let registrar = hook.scope.as_ref().unwrap().key.as_str();
                    format!(
                        "for ({listener_id}, __part_name, {part_item}) in super::entered_parts_with_owner(HookId::{hook_id}, {part_list}, \"{name}\") {{ {event}.{registrar}.set_owner({listener_id}, __part_name); {} }}",
                        call(&part_item, &listener_id, "continue;", "break"),
                    )
                } else if returns_slot {
                    let answer_call = run(
                        &listener_id,
                        "continue;",
                        &format!("{part_item}.{name}({invoke})"),
                    );
                    let o = out.unwrap();
                    format!(
                        "for ({listener_id}, __slot, {part_item}) in super::entered_parts_with_slot(HookId::{hook_id}, {part_list}, \"{name}\") {{ if let Some({replacement}) = {answer_call} {{ *{o} = Some({replacement}); return Some(__slot); }} }} None"
                    )
                } else if single_part_slot {
                    format!(
                        "let Some(__selected) = super::part_slot({part_list}, __part_slot) else {{ return Ok(()); }};
                         let mut __entered = super::entered_parts(HookId::{hook_id}, __selected, \"{name}\");
                         let Some(({listener_id}, {part_item})) = __entered.next() else {{ return Err(()); }};
                         {} Ok(())",
                        call(&part_item, &listener_id, "return Err(());", ""),
                    )
                } else if name == "entity_inside" {
                    let inside_stage = fresh_name("inside_stage", &mut used);
                    let invoke_inside = call(&part_item, &listener_id, "return false;", "break");
                    format!(
                        "for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, {part_list}, \"{name}\") {{
                             effects.stage({listener_id}, |{inside_stage}| {{
                                 let mut {event} = {event_type} {{ dimension, entity, kind, state, pos, precise, effects: {inside_stage} }};
                                 {invoke_inside}
                                 true
                             }});
                         }}"
                    )
                } else if name == "tooltip_image" {
                    let o = out.unwrap();
                    // Retained payloads poison this registration rather than all the mod's item Parts.
                    format!(
                        "for {part_item}_registration in {part_list} {{ if let Some({replacement}) = super::client::tooltip_components::invoke_tooltip_image({part_item}_registration, |{part_item}| {part_item}.{name}({invoke})) {{ *{o} = Some({replacement}); break; }} }}"
                    )
                } else {
                    // A live menu click can have committed edits before a listener panics.
                    // Continue dispatch, but do not run the default after such a failure.
                    let on_abort = if name == "menu_clicked" {
                        format!("*{} = true; continue;", out.unwrap())
                    } else {
                        "continue;".to_owned()
                    };
                    let part_call = call(&part_item, &listener_id, &on_abort, "break");
                    if name == "menu_slot_rules" {
                        // A Part's refusal stands: a later Part may not turn it back. A Part may
                        // still allow what the slot's own rule refuses.
                        let refused = fresh_name("refused", &mut used);
                        let before = fresh_name("before", &mut used);
                        format!(
                            "let mut {refused} = (false, false); for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, {part_list}, \"{name}\") {{ let {before} = ({event}.may_place, {event}.may_pickup); {part_call} {refused}.0 |= {before}.0 && !{event}.may_place; {refused}.1 |= {before}.1 && !{event}.may_pickup; }} {event}.may_place &= !{refused}.0; {event}.may_pickup &= !{refused}.1;"
                        )
                    } else if name == "menu_pick_all" {
                        // As for slot rules: a Part's refusal stands.
                        let refused = fresh_name("refused", &mut used);
                        format!(
                            "let mut {refused} = false; for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, {part_list}, \"{name}\") {{ {part_call} {refused} |= !{event}.allowed; }} {event}.allowed &= !{refused};"
                        )
                    } else {
                        format!(
                            "for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, {part_list}, \"{name}\") {{ {part_call} }}",
                        )
                    }
                },
            ),
        };
        let body = match contract {
            _ if isolated_menu_place => format!("{lookup} {calls}"),
            _ if mod_cancel => format!(
                "if *{o} && !super::receives_cancelled(HookId::{hook_id}) {{ return; }}\n{build} {calls} {write_back}",
                o = cancel_out.unwrap()
            ),
            Contract::Cancel => format!(
                "if *{o} {{ {empty_return} }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            Contract::Replace => format!(
                "if {o}.is_some() {{ {empty_return} }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            _ => format!("{lookup} {build} {calls} {write_back}"),
        };
        let dispatch_return = if returns_slot {
            " -> Option<(usize, usize)>"
        } else if single_part_slot {
            " -> Result<(), ()>"
        } else {
            ""
        };
        let early_return = if scoped.is_some() {
            String::new()
        } else {
            format!("if !super::listened(HookId::{hook_id}) {{ return; }}")
        };
        let unpack = if !uses_carrier {
            String::new()
        } else {
            let names: Vec<_> = carried.iter().map(|(arg, _)| arg.as_str()).collect();
            format!("let calls::{carrier} {{ {} }} = call;\n", names.join(", "))
        };
        if name == "consume_effect" {
            dispatch.push_str(&format!(
                "#[doc(hidden)] pub fn dispatch_{name}({dispatch_signature}, callback: super::runtime::ConsumeEffectBinding) -> bool {{ {unpack}{body} }}\n"
            ));
        } else if scoped_write_replace {
            let (part_trait, _) = scoped.as_ref().unwrap();
            let args = dispatch_params[1..]
                .iter()
                .map(|param| param.split_once(':').unwrap().0)
                .collect::<Vec<_>>()
                .join(", ");
            let params = dispatch_params[1..].join(", ");
            let (host, _) = host.as_ref().unwrap();
            dispatch.push_str(&format!(
                "#[doc(hidden)] pub fn dispatch_{name}({dispatch_signature}) {{ dispatch_{name}_with({args}, |part, event| part.{name}(&mut *{host}, event)); }}\n\
                 #[doc(hidden)] pub fn dispatch_{name}_with({params}, mut {invoke_part}: impl FnMut(&dyn {part_trait}, &mut {event_type}{elided}){result}) {{ {unpack}{early_return}\n{body} }}\n"
            ));
        } else if scoped_random_collect {
            let (part_trait, _) = scoped.as_ref().unwrap();
            dispatch.push_str(&format!(
                "#[doc(hidden)] pub fn dispatch_{name}({dispatch_signature}, mut {invoke_part}: impl FnMut(&dyn {part_trait}, LevelRandomSide<'_>, &mut {event_type}{elided})) {{ {unpack}{early_return}\n{body} }}\n"
            ));
        } else if movement_side_filter {
            let wrapper_params = dispatch_params
                .iter()
                .map(|param| param.trim().strip_prefix("mut ").unwrap_or(param.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let args = dispatch_params
                .iter()
                .map(|param| {
                    let binding = param.split_once(':').unwrap().0.trim();
                    binding.strip_prefix("mut ").unwrap_or(binding)
                })
                .collect::<Vec<_>>()
                .join(", ");
            dispatch.push_str(&format!(
                "#[doc(hidden)] pub fn dispatch_{name}({wrapper_params}) {{ dispatch_{name}_for({args}, kind == \"minecraft:player\"); }}\n\
                 #[doc(hidden)] pub fn dispatch_{name}_for({dispatch_signature}, {client_controlled}: bool) {{ {unpack}{early_return}\n{body} }}\n"
            ));
        } else if filtered_subscribers {
            let wrapper_params = dispatch_params
                .iter()
                .map(|param| param.trim().strip_prefix("mut ").unwrap_or(param.trim()))
                .collect::<Vec<_>>()
                .join(", ");
            let args = dispatch_params
                .iter()
                .map(|param| {
                    let binding = param.split_once(':').unwrap().0.trim();
                    binding.strip_prefix("mut ").unwrap_or(binding)
                })
                .collect::<Vec<_>>()
                .join(", ");
            dispatch.push_str(&format!(
                "#[doc(hidden)] pub fn dispatch_{name}({wrapper_params}) {{ dispatch_{name}_for({args}, |_| true); }}\n\
                 #[doc(hidden)] pub fn dispatch_{name}_for({dispatch_signature}, {allow_subscriber}: impl FnMut(&str) -> bool) {{ {unpack}{early_return}\n{body} }}\n"
            ));
        } else {
            let lints = if single_part_slot {
                "#[allow(clippy::too_many_arguments, clippy::result_unit_err)] "
            } else {
                ""
            };
            dispatch.push_str(&format!(
                "{lints}#[doc(hidden)] pub fn dispatch_{name}({dispatch_signature}){dispatch_return} {{ {unpack}{early_return}\n{body} }}\n"
            ));
            if let Some(part_kind) = hook.part {
                if is_render {
                    return Err(format!("render hook {name} cannot have a part"));
                }
                let (_, table) = part_trait_and_table(part_kind);
                let part_key = fresh_name("part_key", &mut used);
                let host_count = usize::from(host.is_some());
                let part_dispatch_signature = dispatch_params[..host_count]
                    .iter()
                    .cloned()
                    .chain(std::iter::once(format!("{part_key}: &str")))
                    .chain(dispatch_params[host_count..].iter().cloned())
                    .collect::<Vec<_>>()
                    .join(", ");
                let lookup = part_selection(&format!("{table}(|| Some({part_key}))"));
                let part_step = plain_cancel_step(&cancel_invocation(&part_item, &listener_id));
                let o = cancel_out.unwrap();
                dispatch.push_str(&format!(
                    "#[doc(hidden)] pub fn dispatch_{name}_part({part_dispatch_signature}) {{ {unpack}if *{o} {{ return; }} {lookup} {build} for ({listener_id}, {part_item}) in super::entered_parts(HookId::{hook_id}, {part_list}, \"{name}\") {{ {part_step}\nif {event}.cancelled {{ break; }} }} *{o} = {event}.cancelled; {write_back} }}\n"
                ));
            }
        }
    }
    if !calls.is_empty() {
        dispatch.push_str(&format!(
            "#[doc(hidden)]\n#[allow(missing_docs)]\npub mod calls {{\nuse super::*;\n{calls}}}\n"
        ));
    }
    Ok(HookSources {
        traits,
        dispatch,
        render_dispatch,
        part_traits,
        fills,
        fire,
        render_fire,
        hook_ids,
        hook_constants,
        cancel_hooks,
        client_hooks,
        hook_count,
    })
}

fn elide(generics: &str) -> &'static str {
    if generics.is_empty() { "" } else { "<'_>" }
}

fn engine_types(ty: &mut Type, carrier: bool) {
    struct EngineTypes {
        carrier: bool,
    }
    impl VisitMut for EngineTypes {
        fn visit_type_mut(&mut self, ty: &mut Type) {
            let Type::Path(path) = ty else {
                return syn::visit_mut::visit_type_mut(self, ty);
            };
            *ty = if path.path.is_ident("BlockState") {
                if self.carrier {
                    syn::parse_quote!(sunrise_world::registry::BlockStateId)
                } else {
                    syn::parse_quote!(BlockStateId)
                }
            } else if path.path.is_ident("LandingWrite") {
                syn::parse_quote!(sunrise_world::level::LandingWrite)
            } else {
                return syn::visit_mut::visit_type_mut(self, ty);
            };
        }
    }
    EngineTypes { carrier }.visit_type_mut(ty);
}

/// A carrier's fields, each type with its elided lifetimes named `'a`, and its generics.
struct StackCrossing {
    needs_receiver: bool,
    before: String,
    arguments: Vec<String>,
    validators: Vec<String>,
    after: String,
}

/// Declared owned-value arguments select this guarded conversion seam. Event records
/// are built explicitly at their game call sites, not inferred from game bodies.
#[cfg(test)]
fn stack_crossing(
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    hook_id: &str,
    scoped: bool,
    surfaces: &std::collections::HashMap<&str, String>,
) -> Option<StackCrossing> {
    stack_crossing_routed(
        args, output, false, false, hook_id, scoped, None, false, surfaces, None,
    )
}

#[derive(Clone, Copy)]
struct EngineReceiver<'a> {
    package: Option<crate::GameplayPackage>,
    shared: bool,
    expression: &'a str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReplyBoundary {
    Items,
    Block,
    Text,
    Particle,
}

fn boundary_kind(ty: &Type) -> Option<ReplyBoundary> {
    use ReplyBoundary::*;
    [
        (
            Items,
            &[
                "ItemStack",
                "ItemCost",
                "MerchantOffer",
                "Suggestion",
                "GameRuleValue",
                "PlacementContext",
            ][..],
        ),
        (Block, &["BlockState", "LandingWrite", "InsideWrite"][..]),
        (Text, &["Text"][..]),
        (Particle, &["ParticleOptions"][..]),
    ]
    .into_iter()
    .find_map(|(kind, names)| contains_boundary(ty, names).then_some(kind))
}

fn mutable_pointee(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(reference) if reference.mutability.is_some() => Some(&reference.elem),
        _ => None,
    }
}

struct ListReply {
    element: Type,
    fixed: bool,
    error: &'static str,
}

impl ListReply {
    fn adapter_type(&self) -> Type {
        let Self {
            element,
            fixed,
            error,
        } = self;
        let error = syn::Ident::new(error, proc_macro2::Span::call_site());
        if *fixed {
            syn::parse_quote!(&mut HookList<'_, #element, #error, true>)
        } else {
            syn::parse_quote!(&mut HookList<'_, #element, #error>)
        }
    }
}

fn reply_list(ty: &Type) -> Option<ListReply> {
    let error = match boundary_kind(ty)? {
        ReplyBoundary::Items => "StackWriteError",
        ReplyBoundary::Text => "TextFormError",
        _ => return None,
    };
    let pointee = mutable_pointee(ty)?;
    let (element, fixed) = match pointee {
        Type::Array(array) => (&*array.elem, true),
        _ => (generic_of(pointee, "Vec")?, false),
    };
    Some(ListReply {
        element: element.clone(),
        fixed,
        error,
    })
}

#[cfg(test)]
fn stack_crossing_routed(
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    checked: bool,
    paired_hands: bool,
    hook_id: &str,
    scoped: bool,
    block_scope: Option<&str>,
    observer_routed: bool,
    surfaces: &std::collections::HashMap<&str, String>,
    trim_count_output: Option<&str>,
) -> Option<StackCrossing> {
    stack_crossing_for(
        EngineReceiver {
            package: None,
            shared: false,
            expression: "self",
        },
        args,
        output,
        checked,
        paired_hands,
        hook_id,
        scoped,
        block_scope,
        observer_routed,
        surfaces,
        trim_count_output,
    )
}

fn stack_crossing_for(
    engine_receiver: EngineReceiver<'_>,
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    checked: bool,
    paired_hands: bool,
    hook_id: &str,
    scoped: bool,
    block_scope: Option<&str>,
    observer_routed: bool,
    surfaces: &std::collections::HashMap<&str, String>,
    trim_count_output: Option<&str>,
) -> Option<StackCrossing> {
    let common = engine_receiver
        .package
        .map_or("crate", crate::GameplayPackage::common);
    let receiver = engine_receiver.expression;
    let mut needs_receiver = false;
    let parameters = args.iter().chain(output).collect::<Vec<_>>();
    if !parameters.iter().any(|(_, ty)| boundary_kind(ty).is_some()) {
        return None;
    }
    let guard = if scoped { "has_part_hook" } else { "listened" };
    let mut before =
        format!("if !mod_api::__private::{guard}(mod_api::HookId::{hook_id}) {{ return; }} ");
    if let Some(key) = block_scope {
        let source_route = if observer_routed {
            "observer.is_none() && "
        } else {
            ""
        };
        before.push_str(&format!("if {source_route}!{common}::game::api_values::block_state::block_listens(mod_api::HookId::{hook_id}, {key}) {{ return; }} "));
    }
    // A list only projects a value if the listener requests that element.
    before.push_str(&format!("use {common}::game::api_values::TryIntoApi; "));
    if parameters.iter().any(|(field, ty)| {
        trim_count_output != Some(field.as_str())
            && boundary_kind(ty).is_some()
            && mutable_pointee(ty).is_some()
    }) {
        before.push_str(&format!("use {common}::game::api_values::TryIntoEngine; "));
    }
    if trim_count_output.is_some() {
        before.push_str(&format!(
            "use {common}::game::api_values::TryIntoEnginePreservingCount; "
        ));
    }
    if parameters.iter().any(|(_, ty)| {
        boundary_kind(ty) == Some(ReplyBoundary::Items) && mutable_pointee(ty).is_some()
    }) {
        if checked {
            needs_receiver = true;
            let items = if engine_receiver.shared {
                format!(
                    "{common}::game::item_snapshot::ItemSnapshotGameExt1::stack_boundary_items({receiver})"
                )
            } else {
                format!("{receiver}.stack_boundary_items()")
            };
            before.push_str(&format!("let __boundary_items = {items}; "));
        } else {
            before.push_str(&format!("use {common}::game::api_values::ItemBoundary; "));
        }
    }
    let mut arguments = Vec::new();
    let mut stack_validators = Vec::new();
    let mut validations = String::new();
    let mut writes = String::new();
    for (name, ty) in parameters {
        let kind = boundary_kind(ty);
        let block = kind == Some(ReplyBoundary::Block);
        let stack = matches!(kind, Some(ReplyBoundary::Items | ReplyBoundary::Particle));
        let text = kind == Some(ReplyBoundary::Text);
        let mutable = mutable_pointee(ty).is_some();
        let hook = hook_id.to_snake_case();
        if checked && let Some(list) = reply_list(ty) {
            let storage = if list.fixed { "Fixed" } else { "Resizable" };
            let conversion = match (kind, trim_count_output == Some(name.as_str())) {
                (Some(ReplyBoundary::Items), true) => {
                    "value.try_into_engine_preserving_count(&**items)"
                }
                (Some(ReplyBoundary::Items), false) => "value.try_into_engine(&**items)",
                _ => "value.try_into_engine(&())",
            };
            let context = if kind == Some(ReplyBoundary::Items) {
                "let items = __boundary_items.as_ref().ok_or(mod_api::StackWriteError::Unavailable)?; "
            } else {
                ""
            };
            let fixed = if list.fixed { ", true" } else { "" };
            let api_element = qualify_reply_type(&list.element);
            let error = list.error;
            before.push_str(&format!(
                "let mut __journal_{name} = mod_api::HookListJournal::new(mod_api::HookListStorage::{storage}({name}), \
                    |value| value.try_into_api().ok(), |value: &{api_element}| {{ {context}{conversion} }}); \
                 let mut __editor_{name}: mod_api::HookList<'_, _, mod_api::{error}{fixed}> = mod_api::HookList::new(&mut __journal_{name}, \"{hook}\", \"{name}\"); "
            ));
            arguments.push(format!("&mut __editor_{name}"));
            continue;
        }
        if surfaces.contains_key(name.as_str())
            || matches!(ty, Type::Reference(r) if matches!(&*r.elem, Type::TraitObject(_)))
            || contains_boundary(ty, &["InsideEffects"])
            || (kind.is_none() && (!mutable || checked))
        {
            arguments.push(name.clone());
            continue;
        }
        let local = format!("__api_{name}");
        let value = if kind.is_some() {
            format!("match {name}.try_into_api() {{ Ok(value) => value, Err(_) => return }}")
        } else {
            format!("(*{name}).to_owned()")
        };
        before.push_str(&format!(
            "let {mutability}{local} = {value}; ",
            mutability = if mutable { "mut " } else { "" }
        ));
        arguments.push(match ty {
            Type::Reference(_) => format!("&{}{local}", if mutable { "mut " } else { "" }),
            Type::Path(_)
                if generic_of(ty, "Option").is_some_and(|ty| matches!(ty, Type::Reference(_))) =>
            {
                format!("{local}.as_ref()")
            }
            _ => local.clone(),
        });
        if mutable && checked {
            if !paired_hands {
                let pointee = qualify_reply_type(mutable_pointee(ty).unwrap());
                let conversion = if kind == Some(ReplyBoundary::Items) {
                    "__boundary_items.as_ref().ok_or(mod_api::StackWriteError::Unavailable).and_then(|items| value.try_into_engine(&**items))"
                } else {
                    "value.try_into_engine(&())"
                };
                before.push_str(&format!(
                    "let mut __accept_{name} = |value: &{pointee}| -> Result<(), String> {{
                    *{name} = {conversion}.map_err(|error| format!(\"{{error:?}}\"))?; Ok(()) }}; "
                ));
                stack_validators.push(format!("&mut __accept_{name}"));
            }
        } else if mutable {
            let comparable = contains_boundary(ty, &["ItemStack"]);
            if comparable {
                before.push_str(&format!("let __original_{name} = {local}.clone(); "));
            }
            if comparable {
                validations.push_str(&format!(
                    "let __candidate_{name} = if {local} != __original_{name} {{ "
                ));
            }
            let converted = format!("__engine_{name}");
            let value = if text {
                format!(
                    "match {local}.try_into_engine(&()) {{ Ok(value) => value, Err(_) => return }}"
                )
            } else if block {
                needs_receiver = true;
                format!(
                    "match {local}.try_into_engine(&*{receiver}) {{ Ok(value) => value, Err(error) => {{ log::warn!(\"mod block state write refused: {{error:?}}\"); return; }} }}"
                )
            } else if stack {
                let result = format!("__checked_{name}");
                if contains_boundary(ty, &["ParticleOptions"]) {
                    validations.push_str(&format!("let {result} = {local}.try_into_engine(&()); "));
                } else {
                    needs_receiver = true;
                    validations.push_str(&format!("let {result}: Result<_, mod_api::StackWriteError> = {receiver}.boundary_items().ok_or(mod_api::StackWriteError::Unavailable).and_then(|items| {local}.try_into_engine(items)); "));
                }
                format!(
                    "match {result} {{ Ok(value) => value, Err(error) => {{ log::warn!(\"mod {} write refused: {{error:?}}\"); return; }} }}",
                    hook_id.to_snake_case()
                )
            } else {
                local
            };
            if comparable {
                validations.push_str(&format!("Some({value}) }} else {{ None }}; "));
                writes.push_str(&format!(
                    "if let Some({converted}) = __candidate_{name} {{ *{name} = {converted}; }} "
                ));
            } else {
                validations.push_str(&format!("let {converted} = {value}; "));
                writes.push_str(&format!("*{name} = {converted}; "));
            }
        }
    }
    if paired_hands {
        before.push_str(r#"
            let mut __accept_hands = |main: Option<&Option<mod_api::ItemStack>>, off: Option<&Option<mod_api::ItemStack>>| {
                let convert = |value: Option<&Option<mod_api::ItemStack>>, field: &'static str| {
                    value.map(|value| {
                        let items = __boundary_items.as_ref().ok_or(mod_api::StackWriteError::Unavailable)?;
                        value.try_into_engine(&**items)
                    }).transpose().map_err(|error| (field, format!("{error:?}")))
                };
                let main = convert(main, "to_main_hand")?;
                let off = convert(off, "to_off_hand")?;
                if let Some(value) = main { *to_main_hand = value; }
                if let Some(value) = off { *to_off_hand = value; }
                Ok(())
            };
        "#);
        stack_validators.push("&mut __accept_hands".to_owned());
    }
    Some(StackCrossing {
        needs_receiver,
        before,
        arguments,
        validators: stack_validators,
        after: format!("{validations}{writes}"),
    })
}

#[cfg(test)]
#[path = "writeback_tests.rs"]
mod writeback_tests;

#[cfg(test)]
mod stack_crossing_tests {
    use super::*;

    #[test]
    fn checked_shared_stack_uses_the_original_receivers_snapshot() {
        let stack: Type = syn::parse_quote!(&mut ItemStack);
        let crossing = stack_crossing_for(
            EngineReceiver {
                package: Some(crate::GameplayPackage::Common),
                shared: true,
                expression: "self",
            },
            &[("stack".into(), &stack)],
            None,
            true,
            false,
            "Proof",
            false,
            None,
            false,
            &Default::default(),
            None,
        )
        .unwrap();
        assert!(crossing.before.contains(
            "crate::game::item_snapshot::ItemSnapshotGameExt1::stack_boundary_items(self)"
        ));
    }

    #[test]
    #[ignore = "daily"]
    fn per_block_routing_runtime_and_mutants() {
        let state: Type = syn::parse_quote!(BlockState);
        let writes: Type = syn::parse_quote!(&mut Vec<LandingWrite>);
        let crossing = stack_crossing_routed(
            &[
                ("state".into(), &state),
                ("secondary".into(), &state),
                ("writes".into(), &writes),
            ],
            None,
            false,
            false,
            "Proof",
            true,
            Some("state"),
            false,
            &Default::default(),
            None,
        )
        .unwrap();
        let global = stack_crossing_routed(
            &[
                ("state".into(), &state),
                ("secondary".into(), &state),
                ("writes".into(), &writes),
            ],
            None,
            false,
            false,
            "Proof",
            false,
            None,
            false,
            &Default::default(),
            None,
        )
        .unwrap();
        let global = format!(
            "{} callback(__api_state, __api_secondary, &mut __api_writes); {}",
            global.before, global.after
        );
        let guard = "if !crate::game::api_values::block_state::block_listens(mod_api::HookId::Proof, state) { return; }";
        let body = format!(
            "{} callback(__api_state, __api_secondary, &mut __api_writes); {}",
            crossing.before, crossing.after
        );
        let unguarded = body.replace(guard, "");
        let late = unguarded.replace("callback(", &format!("{guard} callback("));
        let after_scalar = unguarded.replace(
            "let __api_secondary",
            &format!("{guard} let __api_secondary"),
        );
        assert_ne!(body, unguarded);
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/block-routing-proof");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, body, success) in [
            ("baseline", body, true),
            ("no_guard", unguarded, false),
            ("late_guard", late, false),
            ("after_scalar_guard", after_scalar, false),
            ("global_listener", global, true),
        ] {
            let source = directory.join(format!("{name}.rs"));
            let binary = directory.join(name);
            let mut fixture =
                include_str!("../tests/fixtures/block_routing.rs").replace("FIRE_BODY", &body);
            if name == "global_listener" {
                fixture = fixture.replace("fn main()", "fn routing_main()");
                fixture.push_str(
                    r#"
fn main() {
    let b = BlockState(BlockKey(8), 0);
    game::api_values::bind(&registry::BlockRegistry(vec![Ok(b)]));
    let mut writes = vec![42];
    Game.fire(registry::BlockStateId(0), registry::BlockStateId(0), &mut writes);
    assert_eq!(CALLS.load(Ordering::Relaxed), 1);
    assert_eq!(writes, vec![42,99]);
    println!("ordinary global listener sees unrelated block B");
}
"#,
                );
            }
            std::fs::write(&source, fixture).unwrap();
            let compiled = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiled.status.success(),
                "{}",
                String::from_utf8_lossy(&compiled.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {} {}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn block_crossing_runtime_and_mutants() {
        let state: Type = syn::parse_quote!(&mut BlockState);
        let sibling: Type = syn::parse_quote!(&mut u32);
        // Sibling first catches early commits even when the checked field follows it.
        let crossing = stack_crossing(
            &[("sibling".into(), &sibling), ("state".into(), &state)],
            None,
            "BlockProof",
            true,
            &Default::default(),
        )
        .unwrap();
        let body = format!(
            "{} callback(&mut __api_state, &mut __api_sibling); {}",
            crossing.before, crossing.after
        );
        let early = body.replace(
            "let __engine_sibling = __api_sibling;",
            "let __engine_sibling = __api_sibling; *sibling = __engine_sibling;",
        );
        let guard =
            "if !mod_api::__private::has_part_hook(mod_api::HookId::BlockProof) { return; }";
        let unguarded = body.replace(guard, "");
        assert_ne!(early, body);
        assert_ne!(unguarded, body);
        let silent = body.replace(
            "log::warn!(\"mod block state write refused: {error:?}\");",
            "",
        );
        assert_ne!(silent, body);
        let fixture = include_str!("../tests/fixtures/hook_crossing.rs");
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/hook-crossing-proof");
        std::fs::create_dir_all(&directory).unwrap();
        for (name, body, success) in [
            ("baseline", body, true),
            ("early_commit", early, false),
            ("no_guard", unguarded, false),
            ("silent_refusal", silent, false),
        ] {
            let source = directory.join(format!("{name}.rs"));
            let binary = directory.join(name);
            std::fs::write(&source, fixture.replace("FIRE_BODY", &body)).unwrap();
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "-O"])
                .arg(&source)
                .arg("-o")
                .arg(&binary)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            let result = std::process::Command::new(binary).output().unwrap();
            assert_eq!(
                result.status.success(),
                success,
                "{name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            println!(
                "{name}: {}{}",
                String::from_utf8_lossy(&result.stdout),
                if success { "passed" } else { "mutant killed" }
            );
        }
    }

    #[test]
    fn boundary_draw_context_is_borrowed_not_snapshotted() {
        let state: Type = syn::parse_quote!(BlockState);
        let draw: Type = syn::parse_quote!(&mut dyn BlockEntityDrawContext);
        let replace: Type = syn::parse_quote!(&mut bool);
        let crossing = stack_crossing(
            &[
                ("state".into(), &state),
                ("draw".into(), &draw),
                ("replace".into(), &replace),
            ],
            None,
            "BlockEntitySubmit",
            false,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(
            crossing.arguments,
            ["__api_state", "draw", "&mut __api_replace"]
        );
        assert!(!crossing.before.contains("__api_draw"));
        assert!(!crossing.after.contains("*draw"));
    }

    #[test]
    fn landing_writes_are_checked_and_staged_with_siblings() {
        let writes: Type = syn::parse_quote!(&mut Vec<LandingWrite>);
        let sibling: Type = syn::parse_quote!(&mut bool);
        let crossing = stack_crossing(
            &[("sibling".into(), &sibling), ("writes".into(), &writes)],
            None,
            "BlockLanding",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(crossing.before.contains("writes.try_into_api()"));
        assert!(!crossing.before.contains("ItemBoundary"));
        assert!(
            crossing.after.find("try_into_engine(&*self)").unwrap()
                < crossing.after.find("*sibling =").unwrap()
        );
    }

    #[test]
    fn completion_hook_converts_only_after_subscription_check() {
        let ty: Type = syn::parse_quote!(&mut Vec<Suggestion>);
        let crossing = stack_crossing(
            &[("suggestions".to_owned(), &ty)],
            None,
            "TabComplete",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::listened")
        );
        assert!(crossing.before.contains("suggestions.try_into_api()"));
        assert_eq!(crossing.arguments, ["&mut __api_suggestions"]);
        let body = format!(
            "{{ {} dispatch(&mut __api_suggestions); {} }}",
            crossing.before, crossing.after
        );
        syn::parse_str::<syn::Block>(&body).unwrap();
    }
    #[test]
    #[ignore = "daily"]
    fn unsubscribed_owned_values_never_convert_or_allocate() {
        for record in ["ItemStack", "ParticleOptions"] {
            let stack: Type = syn::parse_str(&format!("&{record}")).unwrap();
            let claimed: Type = syn::parse_quote!(&mut bool);
            let args = [
                ("stack".to_owned(), &stack),
                ("claimed".to_owned(), &claimed),
            ];
            let crossing = stack_crossing(
                &args,
                None,
                "UseItem",
                record == "ItemStack",
                &Default::default(),
            )
            .unwrap();
            let body = format!(
                "{} mod_api::dispatch_use_item(self, {}); {}",
                crossing.before,
                crossing.arguments.join(", "),
                crossing.after
            );
            let source = r#"
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
struct Counter;
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
unsafe impl std::alloc::GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(pointer, layout); }
    }
}
#[global_allocator] static ALLOCATOR: Counter = Counter;
static SUBSCRIBED: AtomicBool = AtomicBool::new(false);
static CONVERSIONS: AtomicUsize = AtomicUsize::new(0);
static DISPATCHES: AtomicUsize = AtomicUsize::new(0);
pub struct ItemStack;
pub struct ApiStack;
pub struct Game;
mod game { pub mod api_values {
    pub trait TryIntoApi { fn try_into_api(&self) -> Result<crate::ApiStack, ()>; }
    impl TryIntoApi for crate::ItemStack {
        fn try_into_api(&self) -> Result<crate::ApiStack, ()> {
            crate::CONVERSIONS.fetch_add(1, crate::Ordering::Relaxed);
            Ok(crate::ApiStack)
        }
    }
} }
mod mod_api {
    pub enum HookId { UseItem }
    pub mod __private {
        pub fn has_part_hook(_: super::HookId) -> bool {
            crate::SUBSCRIBED.load(crate::Ordering::Relaxed)
        }
    }
    pub fn dispatch_use_item(_: &mut crate::Game, _: &crate::ApiStack, claimed: &mut bool) {
        crate::DISPATCHES.fetch_add(1, crate::Ordering::Relaxed);
        *claimed = true;
    }
}
impl Game {
    fn fire_use_item(&mut self, stack: &ItemStack, claimed: &mut bool) { FIRE_BODY }
}
fn main() {
    let mut game = Game;
    let mut claimed = false;
    let allocations = ALLOCATIONS.load(Ordering::Relaxed);
    for _ in 0..1_000 { game.fire_use_item(&ItemStack, &mut claimed); }
    assert_eq!(ALLOCATIONS.load(Ordering::Relaxed), allocations);
    assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 0);
    assert_eq!(DISPATCHES.load(Ordering::Relaxed), 0);
    assert!(!claimed);
    SUBSCRIBED.store(true, Ordering::Relaxed);
    game.fire_use_item(&ItemStack, &mut claimed);
    assert_eq!(CONVERSIONS.load(Ordering::Relaxed), 1);
    assert_eq!(DISPATCHES.load(Ordering::Relaxed), 1);
    assert!(claimed);
}
"#
            .replace("FIRE_BODY", &body)
            .replace("ItemStack", record)
            .replace(
                "has_part_hook",
                if record == "ItemStack" {
                    "has_part_hook"
                } else {
                    "listened"
                },
            );
            let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = target.join(format!("item-fire-{}-{unique}", std::process::id()));
            std::fs::create_dir_all(&directory).unwrap();
            let input = directory.join("fire.rs");
            let output = directory.join(format!("fire{}", std::env::consts::EXE_SUFFIX));
            std::fs::write(&input, source).unwrap();
            let compiler = std::process::Command::new("rustc")
                .args(["--edition=2024", "--crate-name=item_fire_counter"])
                .arg(&input)
                .arg("-o")
                .arg(&output)
                .output()
                .unwrap();
            assert!(
                compiler.status.success(),
                "{}",
                String::from_utf8_lossy(&compiler.stderr)
            );
            assert!(
                std::process::Command::new(output)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn qualified_names_do_not_select_a_root_record_crossing() {
        for ty in [
            syn::parse_quote!(&foreign::ItemStack),
            syn::parse_quote!(&mod_api::ItemStack),
            syn::parse_quote!(&mut Vec<foreign::MerchantOffer>),
        ] {
            assert!(
                stack_crossing(
                    &[("value".to_owned(), &ty)],
                    None,
                    "AnyName",
                    false,
                    &Default::default()
                )
                .is_none()
            );
        }
        let ty = syn::parse_quote!(&mut Option<ItemStack>);
        assert!(
            stack_crossing(
                &[("value".to_owned(), &ty)],
                None,
                "AnyName",
                false,
                &Default::default()
            )
            .is_some()
        );
    }
    #[test]
    fn rule_hook_gates_before_copying_value() {
        let ty: Type = syn::parse_quote!(GameRuleValue);
        let crossing = stack_crossing(
            &[("value".to_owned(), &ty)],
            None,
            "GameRuleChanged",
            false,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::listened")
        );
        assert!(crossing.before.contains("value.try_into_api()"));
        assert_eq!(crossing.arguments, ["__api_value"]);
        assert!(crossing.after.is_empty());
    }

    #[test]
    fn placement_hook_gates_before_snapshot_and_stages_the_answer() {
        let context: Type = syn::parse_quote!(&PlacementContext);
        let answer: Type = syn::parse_quote!(&mut Option<Option<BlockStateId>>);
        let crossing = stack_crossing(
            &[
                ("context".to_owned(), &context),
                ("answer".to_owned(), &answer),
            ],
            None,
            "StateForPlacement",
            true,
            &Default::default(),
        )
        .unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::has_part_hook")
        );
        assert!(crossing.before.contains("context.try_into_api()"));
        assert_eq!(crossing.arguments, ["&__api_context", "&mut __api_answer"]);
        assert!(crossing.after.contains("*answer = __engine_answer"));
    }

    #[test]
    fn plain_hook_has_no_stack_seam() {
        let scalar: Type = syn::parse_quote!(&mut u32);
        assert!(
            stack_crossing(
                &[("power".to_owned(), &scalar)],
                None,
                "Plain",
                false,
                &Default::default()
            )
            .is_none()
        );
    }
}

fn carrier_fields(args: &[(String, &Type)]) -> (String, &'static str) {
    let mut lifetimes = CarrierLifetimes::default();
    let fields = args
        .iter()
        .map(|(arg, ty)| {
            let mut ty = (*ty).clone();
            lifetimes.visit_type_mut(&mut ty);
            format!("pub {arg}: {},\n", ty.to_token_stream())
        })
        .collect();
    (fields, if lifetimes.used { "<'a>" } else { "" })
}

/// Names every reference and elided lifetime of a carrier's field types `'a`.
#[derive(Default)]
struct CarrierLifetimes {
    used: bool,
}

impl VisitMut for CarrierLifetimes {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        self.used = true;
        reference.lifetime = Some(syn::parse_quote!('a));
        self.visit_type_mut(&mut reference.elem);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            self.used = true;
            *lifetime = syn::parse_quote!('a);
        }
    }
}

/// `T` of an `Option<Answered<T>>` output, if that is its type.
fn answered_type(ty: &Type) -> Option<&Type> {
    generic_of(generic_of(ty, "Option")?, "Answered")
}

/// `T` of a `Vec<Answered<T>>` output, if that is its type.
fn answered_batch(ty: &Type) -> Option<&Type> {
    generic_of(generic_of(ty, "Vec")?, "Answered")
}

/// `T` of the engine callback that validates and stages an `Answered<T>`.
fn declaration_sink(ty: &Type) -> Option<&Type> {
    answered_sink(ty).map(|(answer, _)| answer)
}

/// The payload and return type of an engine-only `dyn FnMut(Answered<T>)` sink.
fn answered_sink(ty: &Type) -> Option<(&Type, &syn::ReturnType)> {
    let Type::TraitObject(object) = ty else {
        return None;
    };
    let syn::TypeParamBound::Trait(bound) = object.bounds.first()? else {
        return None;
    };
    let segment = bound.path.segments.last().filter(|s| s.ident == "FnMut")?;
    let syn::PathArguments::Parenthesized(args) = &segment.arguments else {
        return None;
    };
    if args.inputs.len() != 1 {
        return None;
    }
    Some((generic_of(args.inputs.first()?, "Answered")?, &args.output))
}

/// `T` of a `name<T>` type.
fn generic_of<'a>(ty: &'a Type, name: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else { return None };
    let segment = path.path.segments.last().filter(|s| s.ident == name)?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "daily"]
    fn setting_changes_notify_only_the_owner() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let sources = super::super::sources(&root).unwrap();
        let file = syn::parse_file(&sources.hooks).unwrap();
        let function = file
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Fn(f) if f.sig.ident == "dispatch_setting_changed" => Some(f),
                _ => None,
            })
            .unwrap();
        let function = function.to_token_stream();
        let dir = root.join("target/setting-owner-test");
        std::fs::create_dir_all(&dir).unwrap();
        let program = format!(
            r#"
#![allow(dead_code, unused_variables)]
use std::sync::atomic::{{AtomicUsize, Ordering}};
static A: AtomicUsize = AtomicUsize::new(0);
static B: AtomicUsize = AtomicUsize::new(0);
struct Owner {{ id: &'static str }}
static OWNERS: [Owner; 2] = [Owner {{ id: "a" }}, Owner {{ id: "b" }}];
fn listened(_: generated::HookId) -> bool {{ true }}
fn listening(_: generated::HookId) -> impl Iterator<Item=(bool, &'static Owner)> {{ OWNERS.iter().map(|m| (false, m)) }}
mod runtime {{
    pub fn invoke<T>(_: super::generated::HookId, _: &str, _: &str, call: impl FnOnce() -> T) -> Option<T> {{ Some(call()) }}
    pub struct ListenerScope;
    impl ListenerScope {{
        pub fn new() -> Self {{ Self }}
        pub fn enter<'a>(&mut self, owner: &'a super::Owner, _: &str) -> &'a super::Owner {{ owner }}
    }}
}}
impl Owner {{
    fn setting_changed(&self, _: &dyn generated::Host, _: &mut generated::SettingChangedEvent<'_>) {{
        if self.id == "a" {{ A.fetch_add(1, Ordering::Relaxed); }} else {{ B.fetch_add(1, Ordering::Relaxed); }}
    }}
}}
mod generated {{
    pub trait Host {{}}
    impl Host for () {{}}
    pub enum HookId {{ SettingChanged }}
    pub struct SettingChangedEvent<'a> {{ pub mod_id: &'a str, pub key: &'a str, pub value: u8 }}
    {function}
}}
fn main() {{
    generated::dispatch_setting_changed(&(), "a", "toggle", 0);
    generated::dispatch_setting_changed(&(), "a", "toggle", 0);
    assert_eq!(A.load(Ordering::Relaxed), 2);
    assert_eq!(B.load(Ordering::Relaxed), 0);
}}
"#
        );
        let source = dir.join("owners.rs");
        let binary = dir.join("owners");
        std::fs::write(&source, program).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            std::process::Command::new(binary)
                .status()
                .unwrap()
                .success()
        );
    }

    #[test]
    fn staged_surface_requires_a_flat_mutable_non_draw_registrar() {
        let mut hook = Hook {
            name: "probe".into(),
            owner: Some("owner".into()),
            staged_surface: Some("sink".into()),
            call: vec![
                ("owner".into(), "&str".into()),
                ("sink".into(), "&mut dyn Sink".into()),
            ],
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::probe::Game<'_>::native_probe",
                Side::Server,
            )
        };
        assert!(emit(std::iter::once(&hook), &[], true).is_ok());
        let contexts = [DrawContext {
            surface: "DrawBatch".into(),
            trait_name: "Sink".into(),
        }];
        assert!(emit(std::iter::once(&hook), &contexts, true).is_err());
        hook.dispatch = DispatchStyle::Carrier;
        assert!(emit(std::iter::once(&hook), &[], true).is_err());
        hook.dispatch = DispatchStyle::Flat;
        hook.call[1].1 = "&mut u32".into();
        assert!(emit(std::iter::once(&hook), &[], true).is_err());
    }

    #[test]
    fn an_output_validated_listen_hook_needs_a_mutable_output() {
        let emit_hook = |ty: &str| {
            let hook = Hook {
                name: "checked".into(),
                output_validator: Some("crate::check".into()),
                call: vec![("progress".into(), ty.into())],
                dispatch: DispatchStyle::Flat,
                ..Hook::sample(
                    "sunrise_client::game::native_hooks::checked::Game<'_>::native_checked",
                    Side::Client,
                )
            };
            emit(std::iter::once(&hook), &[], false)
        };
        if let Err(error) = emit_hook("&mut f32") {
            panic!("a listen hook validating a mutable output must generate: {error}");
        }
        assert_eq!(
            emit_hook("&f32").err().as_deref(),
            Some("checked: output_validator requires mutable outputs")
        );
    }

    #[test]
    fn inert_guard_is_owned_by_the_hook_declaration() {
        let mut hook = Hook {
            name: "probe".into(),
            contract: Contract::Cancel,
            out: Some("handled".into()),
            scope: Some(crate::catalog::PartScope {
                kind: crate::catalog::PartKind::Block,
                key: "state".into(),
            }),
            inert: Some("*handled = true;".into()),
            call: vec![
                ("state".into(), "BlockState".into()),
                ("handled".into(), "&mut bool".into()),
            ],
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::probe::Game<'_>::native_probe",
                Side::Server,
            )
        };
        let declared = emit(std::iter::once(&hook), &[], false).unwrap();
        assert!(declared.dispatch.contains("disabled_owner_slot"));
        assert!(declared.dispatch.contains("*handled = true;"));
        hook.inert = None;
        let undeclared = emit(std::iter::once(&hook), &[], false).unwrap();
        assert!(!undeclared.dispatch.contains("disabled_owner_slot"));
    }

    #[test]
    #[ignore = "daily"]
    fn an_answered_collect_names_the_mod_of_each_entry() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hook = Hook {
            name: "gather".into(),
            contract: Contract::Collect,
            out: Some("entries".into()),
            answered_by_mod: true,
            call: vec![("entries".into(), "&mut Vec<Answered<u32>>".into())],
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::gather::Game<'_>::native_gather",
                Side::Server,
            )
        };
        let sources = emit(std::iter::once(&hook), &[], false).unwrap();
        let (traits, dispatch) = (sources.traits, sources.dispatch);
        let program = format!(
            r#"
#![allow(dead_code, unused_variables)]
struct Gatherer {{ pushes: &'static [u32] }}
static MODS: [(&str, Gatherer); 3] = [("a", Gatherer {{ pushes: &[1, 2] }}), ("quiet", Gatherer {{ pushes: &[] }}), ("b", Gatherer {{ pushes: &[3] }})];
fn listened(_: generated::HookId) -> bool {{ true }}
fn entered_with_id(_: generated::HookId, _: &str) -> impl Iterator<Item = (&'static str, &'static Gatherer)> {{ MODS.iter().map(|(id, m)| (*id, m)) }}
mod runtime {{
    pub fn invoke<T>(_: super::generated::HookId, _: &str, _: &str, call: impl FnOnce() -> T) -> Option<T> {{ Some(call()) }}
    pub fn discard_registration<T>(_: (super::generated::HookId, &'static str), _: Option<&'static str>, value: T) {{ drop(value); }}
}}
impl Gatherer {{ fn gather(&self, _: &mut dyn generated::ServerHost, event: &mut generated::GatherEvent<'_>) {{ assert!(event.entries.is_empty()); event.entries.extend(self.pushes); }} }}
mod generated {{
pub trait ServerHost {{}}
impl ServerHost for () {{}}
pub enum HookId {{ Gather }}
#[derive(Debug, PartialEq)]
pub struct Answered<T> {{ pub answer: T, pub mod_id: &'static str }}
pub trait Hooks {{ {traits} }}
{dispatch}
}}
use generated::Answered;
fn main() {{
    let mut entries = vec![Answered {{ answer: 0, mod_id: "game" }}];
    generated::dispatch_gather(&mut (), &mut entries);
    let entry = |answer, mod_id| Answered {{ answer, mod_id }};
    assert_eq!(entries, [entry(0, "game"), entry(1, "a"), entry(2, "a"), entry(3, "b")]);
}}
"#
        );
        let dir = root.join("target/answered-collect-test");
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("main.rs");
        let binary = dir.join("main");
        std::fs::write(&source, program).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let run = std::process::Command::new(binary).output().unwrap();
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

    #[test]
    fn an_answered_collect_needs_answered_entries() {
        let hook = Hook {
            name: "gather".into(),
            contract: Contract::Collect,
            out: Some("entries".into()),
            answered_by_mod: true,
            call: vec![("entries".into(), "&mut Vec<u32>".into())],
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::gather::Game<'_>::native_gather",
                Side::Server,
            )
        };
        let refused = emit(std::iter::once(&hook), &[], false).err();
        assert_eq!(
            refused.as_deref(),
            Some("gather: output must be Vec<Answered<T>>")
        );
    }

    #[test]
    fn an_answered_return_only_allows_its_single_answer_sink() {
        let emit_hook = |receiver: Receiver, extra: Option<(&str, &str)>, sink: &str| {
            let mut call = Vec::new();
            if let Some((name, ty)) = extra {
                call.push((name.into(), ty.into()));
            }
            call.push(("answers".into(), sink.into()));
            let hook = Hook {
                name: "section_visibility".into(),
                contract: Contract::Collect,
                out: Some("answers".into()),
                answered_by_mod: true,
                receiver,
                dispatch: DispatchStyle::ApiCarrier,
                call,
                ..Hook::sample(
                    "sunrise_client::game::native_hooks::section_visibility::Game<'_>::native_section_visibility",
                    Side::Client,
                )
            };
            emit(std::iter::once(&hook), &[], false)
        };
        let sink = "&mut dyn FnMut(Answered<u32>)";
        if let Err(error) = emit_hook(Receiver::Read, Some(("input", "&u32")), sink) {
            panic!("a read-only collector with one answer sink must generate: {error}");
        }
        for extra in [("extra", "&mut u32"), ("extra", "Option<&mut u32>")] {
            assert_eq!(
                emit_hook(Receiver::Read, Some(extra), sink)
                    .err()
                    .as_deref(),
                Some("section_visibility: an isolated collector cannot also write to extra")
            );
        }
        for invalid_sink in [
            "&mut Vec<Answered<u32>>",
            "&mut dyn FnMut(Answered<u32>) -> Result<(), DeclarationError>",
        ] {
            assert_eq!(
                emit_hook(Receiver::Read, None, invalid_sink)
                    .err()
                    .as_deref(),
                Some("section_visibility: output must be &mut dyn FnMut(Answered<T>)")
            );
        }
        assert_eq!(
            emit_hook(Receiver::Write, Some(("input", "&u32")), sink)
                .err()
                .as_deref(),
            Some("section_visibility: an isolated collector needs a read-only world Host")
        );
    }

    #[test]
    fn fire_units_keep_side_and_owner_with_their_adapter() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let catalog = crate::gameplay::Routing::load(&root)
            .unwrap()
            .catalog(&root)
            .unwrap();
        let names = [
            "can_start_swimming",
            "block_entity_on_load",
            "tick_block",
            "render_gui_post",
            "living_drown",
        ];
        let hooks: Vec<_> = names
            .iter()
            .map(|name| {
                catalog
                    .hooks
                    .iter()
                    .find(|hook| hook.name == *name)
                    .unwrap()
            })
            .collect();
        let mut direct = hooks[0].clone();
        direct.dispatch = DispatchStyle::Direct;
        let output = emit(hooks.iter().copied(), &catalog.draw_contexts, true).unwrap();
        let units = &output.fire;
        assert_eq!(units.len(), 4);
        assert_eq!(units[0].side, Side::Both);
        assert_eq!(units[0].receiver, Receiver::Read);
        assert!(matches!(units[0].owner, FireOwner::Shared));
        assert!(matches!(units[1].owner, FireOwner::Free));
        assert!(units[1].carrier.is_none());
        assert!(render_fire(&units[1..2]).contains("mod_api::__private::BlockEntityOnLoadCall"));
        assert_eq!(units[2].side, Side::Server);
        assert!(matches!(&units[2].owner, FireOwner::Scoped(owner) if owner.ident == "GameLevel"));
        let server = render_fire(&units[2..3]);
        assert!(server.contains("LevelTicks::new(self, ticks)"));
        assert!(!server.contains("DrawContext"));
        assert_eq!(units[3].side, Side::Client);
        let client = render_fire(&units[3..]);
        assert!(client.contains("GameDrawContext::stage"));
        assert!(!client.contains("LevelTicks"));
        assert!(
            emit(std::iter::once(&direct), &catalog.draw_contexts, true)
                .unwrap()
                .fire
                .is_empty()
        );
    }

    #[test]
    #[ignore = "daily"]
    fn one_borrowed_game_adapter_compiles_for_both_concrete_callers() {
        let hook = Hook {
            receiver: Receiver::Read,
            host: Some(HostAccess::Read),
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::Game<'_>::native_test",
                Side::Both,
            )
        };
        let fire = render_fire(&emit(std::iter::once(&hook), &[], true).unwrap().fire);
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("fire.rs");
        fs::write(
            &path,
            format!(
                r#"
extern crate self as mod_api;
fn dispatch_test(_: &dyn game::Host) {{}}
mod game {{
    pub trait Host {{}}
    pub trait Side: Sized {{
        fn as_host<'a>(game: &'a Game<'_, Self>) -> &'a dyn Host;
    }}
    pub struct Client;
    pub struct Server;
    pub struct Game<'g, S>(&'g S);
    impl<S> Host for Game<'_, S> {{}}
    impl Side for Client {{
        fn as_host<'a>(game: &'a Game<'_, Self>) -> &'a dyn Host {{ game }}
    }}
    impl Side for Server {{
        fn as_host<'a>(game: &'a Game<'_, Self>) -> &'a dyn Host {{ game }}
    }}
    mod fire {{ {fire} }}
    pub fn run() {{
        let client = Game(&Client);
        let server = Game(&Server);
        let _ = (client.0, server.0);
        client.fire_test();
        server.fire_test();
    }}
}}
fn main() {{ game::run(); }}
"#
            ),
        )
        .unwrap();
        let compile = Command::new("rustc")
            .args(["--edition=2024", "-Dwarnings"])
            .arg(&path)
            .arg("--out-dir")
            .arg(fixture.path())
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        assert!(
            Command::new(
                fixture
                    .path()
                    .join(format!("fire{}", std::env::consts::EXE_SUFFIX))
            )
            .status()
            .unwrap()
            .success()
        );
    }

    #[test]
    #[ignore = "daily"]
    fn scoped_native_hook_uses_selected_host_and_game_hooks_keep_their_receiver() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fire = super::super::game_fire_source(&root).unwrap();
        let compact: String = fire.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(compact.contains("mod_api::dispatch_living_change_target(self.host,call)"));
        assert!(
            compact.contains("mod_api::dispatch_living_visibility(self,entity,looking,visibility)")
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_worldgen_part_method_takes_the_worldgen_context_and_reads_the_published_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let hooks = super::super::sources(&root).unwrap().hooks;
        let compact: String = hooks.chars().filter(|c| !c.is_whitespace()).collect();
        fn dispatch<'a>(compact: &'a str, name: &str) -> &'a str {
            let start = compact
                .find(&format!("fndispatch_{name}("))
                .unwrap_or_else(|| panic!("no dispatch for {name}"));
            let body = &compact[start..];
            &body[..body.find("#[doc(hidden)]").unwrap_or(body.len())]
        }
        let method = &compact[compact.find("fnon_tree_grow(").unwrap()..];
        assert!(method.starts_with("fnon_tree_grow(&self,context:&dynWorldgenHost,"));
        let worldgen = dispatch(&compact, "on_tree_grow");
        assert!(worldgen.contains("=super::worldgen_parts();"));
        assert!(!worldgen.contains("super::parts("));
        // Every other scoped hook keeps this thread's table, `is_ladder`'s loop context included.
        assert!(
            dispatch(&compact, "is_ladder")
                .contains("=super::parts(HookId::IsLadder,\"is_ladder\");")
        );
    }

    #[test]
    fn item_ability_context_with_api_carrier_borrows_stack_view() {
        let hook = Hook {
            name: "can_perform_action".into(),
            contract: Contract::Listen,
            receiver: Receiver::Associated,
            dispatch: DispatchStyle::ApiCarrier,
            context: Some(PhaseContext::ItemAbility),
            scope: Some(crate::catalog::PartScope {
                kind: crate::catalog::PartKind::Item,
                key: "stack".into(),
            }),
            call: vec![
                ("stack".into(), "ItemStackView<'_>".into()),
                ("ability".into(), "&str".into()),
                ("can_perform".into(), "&mut bool".into()),
            ],
            ..Hook::sample(
                "sunrise_gameplay_common::game::native_hooks::can_perform_action::native_can_perform_action",
                Side::Both,
            )
        };
        let emitted = emit(std::iter::once(&hook), &[], true).unwrap();
        let part_traits: String = emitted
            .part_traits
            .get("ItemPart")
            .unwrap()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(part_traits.contains(
            "fncan_perform_action(&self,context:&ItemAbilityContext<'_>,event:&mutCanPerformActionEvent<'_>)"
        ));
        let dispatch: String = emitted
            .dispatch
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(dispatch.contains(
            "pubfndispatch_can_perform_action(context:&ItemAbilityContext<'_>,call:calls::CanPerformActionCall<'_>)"
        ));
        assert!(dispatch.contains(
            "pubstructCanPerformActionCall<'a>{pubstack:ItemStackView<'a>,pubability:&'astr,pubcan_perform:&'amutbool,}"
        ));
        assert!(dispatch.contains(
            "pubstructCanPerformActionEvent<'a>{pubstack:ItemStackView<'a>,pubability:&'astr,pubcan_perform:bool,}"
        ));
        assert!(dispatch.contains("item_id(stack.item())"));
        let fire: String = render_fire(&emitted.fire)
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(fire.contains(
            "pubfnfire_can_perform_action(context:&mod_api::ItemAbilityContext<'_>,call:mod_api::__private::CanPerformActionCall<'_>){mod_api::dispatch_can_perform_action(context,call);}"
        ));
    }

    #[test]
    fn part_cancel_hook_emits_global_and_part_dispatch_under_one_hook_id() {
        let hook = Hook {
            name: "stacked_part_test".into(),
            contract: Contract::Cancel,
            out: Some("canceled".into()),
            part: Some(PartKind::Item),
            call: vec![
                ("value".into(), "u32".into()),
                ("canceled".into(), "&mut bool".into()),
            ],
            dispatch: DispatchStyle::Flat,
            ..Hook::sample(
                "sunrise_client::game::native_hooks::stacked_part_test::Game<'_>::native_stacked_part_test",
                Side::Server,
            )
        };
        let emitted = emit(std::iter::once(&hook), &[], true).unwrap();
        assert_eq!(emitted.hook_count, 1);
        assert!(emitted.hook_ids.contains("StackedPartTest,"));
        assert!(emitted.traits.contains("fn stacked_part_test("));
        let item_part = emitted
            .part_traits
            .get("ItemPart")
            .expect("part = \"item\" must emit an ItemPart method");
        assert!(item_part.contains("fn stacked_part_test("));

        let compact: String = emitted
            .dispatch
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        assert!(compact.contains("pubfndispatch_stacked_part_test("));
        assert!(compact.contains("pubfndispatch_stacked_part_test_part(host:&mutdynServerHost,part_key:&str,value:u32,canceled:&mutbool)"));
        assert!(compact.contains("super::has_part_hook(HookId::StackedPartTest)"));
        assert!(compact.contains("super::parts(HookId::StackedPartTest,\"stacked_part_test\")"));
        assert!(compact.contains("table.item(||Some(part_key))"));
    }
}
fn contains_boundary(ty: &Type, names: &[&str]) -> bool {
    use syn::visit::Visit;
    struct Boundary<'a> {
        found: bool,
        names: &'a [&'a str],
    }
    impl<'ast> Visit<'ast> for Boundary<'_> {
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            // Only bare names have the emitted API module's provenance.
            self.found |= path.path.segments.len() == 1
                && path
                    .path
                    .segments
                    .last()
                    .is_some_and(|s| self.names.contains(&s.ident.to_string().as_str()));
            syn::visit::visit_type_path(self, path);
        }
    }
    let mut boundary = Boundary {
        found: false,
        names,
    };
    boundary.visit_type(ty);
    boundary.found
}

fn qualify_reply_type(ty: &Type) -> String {
    struct Qualify;
    impl VisitMut for Qualify {
        fn visit_type_path_mut(&mut self, path: &mut syn::TypePath) {
            if path.path.segments.len() == 1
                && boundary_kind(&Type::Path(path.clone())).is_some()
                && path.path.segments[0].arguments.is_empty()
            {
                let name = &path.path.segments[0].ident;
                *path = syn::parse_quote!(mod_api::#name);
            } else {
                syn::visit_mut::visit_type_path_mut(self, path);
            }
        }
    }
    let mut ty = ty.clone();
    Qualify.visit_type_mut(&mut ty);
    ty.to_token_stream().to_string()
}

#[cfg(test)]
#[path = "dispatch/stack_count_tests.rs"]
mod stack_count_tests;
