use super::*;

pub(super) struct HookSources {
    pub traits: String,
    pub dispatch: String,
    pub render_traits: String,
    pub render_dispatch: String,
    pub part_traits: BTreeMap<&'static str, String>,
    pub fills: Vec<Fill>,
    pub fire: String,
    pub hook_ids: String,
    pub hook_constants: String,
    pub hook_count: usize,
    pub render_hook_ids: String,
    pub render_hook_constants: String,
    pub render_hook_count: usize,
}

/// The most parameters a `native_*` target takes, receiver included (clippy's limit): a hook
/// with more declares its parameters in `hooks/*.rs` and its target takes one `<Name>Call`.
const MAX_TARGET_PARAMETERS: usize = 7;

/// Refuses a flat target that takes more than [`MAX_TARGET_PARAMETERS`].
fn target_parameters(name: &str, native: &str, count: usize, carrier: &str) -> Result<(), String> {
    if count > MAX_TARGET_PARAMETERS {
        return Err(format!(
            "{name}: {native} takes {count} parameters, more than {MAX_TARGET_PARAMETERS}: declare them on the hook in hooks/*.rs so it takes one {carrier}"
        ));
    }
    Ok(())
}

pub(super) fn is_render(hook: &Hook) -> bool {
    hook.target.starts_with("mod_api::render::")
}

/// Render emission reads only the catalog and its API-owned raw target signatures.
pub(super) fn render(root: &Path, catalog: &Catalog) -> Result<HookSources, String> {
    emit(
        root,
        catalog.hooks.iter().filter(|hook| is_render(hook)),
        &[],
        &catalog.draw_contexts,
    )
}

pub(super) fn emit<'a>(
    root: &Path,
    hooks: impl Iterator<Item = &'a Hook>,
    methods: &[syn::ItemFn],
    contexts: &[DrawContext],
) -> Result<HookSources, String> {
    let mut hook_ids = String::new();
    let mut hook_constants = String::new();
    let mut hook_count = 0;
    let mut render_hook_ids = String::new();
    let mut render_hook_constants = String::new();
    let mut render_hook_count = 0;
    let mut traits = String::new();
    let mut dispatch = String::new();
    let mut render_traits = String::new();
    let mut render_dispatch = String::new();
    let mut part_traits: BTreeMap<&str, String> = BTreeMap::new();
    let mut fills = Vec::new();
    let mut fire = String::new();
    let mut calls = String::new();
    for hook in hooks {
        let name = hook.name.as_str();
        let native = format!("native_{name}");
        let target = hook.target.as_str();
        let is_render = is_render(hook);
        let external = free_function_source(target);
        let in_game = external.is_none();
        let (method, call) = if let Some((krate, path)) = external {
            let Some((module, function)) = path.rsplit_once("::") else {
                return Err(format!("wrong target for {name}"));
            };
            if function != native {
                return Err(format!("wrong target for {name}"));
            }
            let path = root
                .join("crates")
                .join(krate)
                .join("src")
                .join(format!("{}.rs", module.replace("::", "/")));
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let file = syn::parse_file(&source).map_err(|e| e.to_string())?;
            let functions: Vec<_> = file
                .items
                .into_iter()
                .filter_map(|item| match item {
                    Item::Fn(function) => Some(function),
                    _ => None,
                })
                .collect();
            (
                one_function(functions.iter(), &native)?.clone(),
                // A hook in the mod API crate, or in a crate above it, calls its dispatch
                // directly; one below it calls the callback the game installs.
                if matches!(krate, "horizon-mod-api" | "horizon-client-render") {
                    format!("dispatch_{name}")
                } else {
                    name.to_owned()
                },
            )
        } else {
            if !target.starts_with("horizon_client::game::native_hooks::")
                || !target.ends_with(&format!("::{native}"))
            {
                return Err(format!("wrong target for {name}"));
            }
            (
                one_function(methods.iter(), &native)?.clone(),
                format!("dispatch_{name}"),
            )
        };
        let fill = needs_fill(&method, &call, in_game)?;
        let sig = &method.sig;
        let mut native_args = Vec::new();
        let mut receiver_mut = None;
        for arg in &sig.inputs {
            match arg {
                FnArg::Receiver(r) => {
                    receiver_mut = Some(r.mutability.is_some());
                }
                FnArg::Typed(arg) => {
                    let Pat::Ident(pat) = &*arg.pat else {
                        return Err(format!("unsupported argument in {name}"));
                    };
                    native_args.push((pat.ident.to_string(), &*arg.ty));
                }
            }
        }
        // A hook that declares its parameters is called with them as one `<Name>Call`.
        let carrier = format!("{}Call", hook_id(name));
        let declared = hook
            .call
            .iter()
            .map(|(arg, ty)| Ok((arg.clone(), syn::parse_str::<Type>(ty)?)))
            .collect::<Result<Vec<_>, syn::Error>>()
            .map_err(|e| format!("{name}: {e}"))?;
        let mut args: Vec<(String, &Type)> = if declared.is_empty() {
            target_parameters(name, &native, sig.inputs.len(), &carrier)?;
            native_args
        } else {
            let taken_ticks = native_args.iter().position(|(arg, _)| arg == "ticks");
            let others: Vec<_> = native_args
                .iter()
                .filter(|(arg, _)| arg != "ticks")
                .collect();
            let [(call_arg, call_ty)] = others[..] else {
                return Err(format!(
                    "{name}: {native} takes its declared parameters as one `call: {carrier}`"
                ));
            };
            if call_arg != "call" || !call_ty.to_token_stream().to_string().contains(&carrier) {
                return Err(format!(
                    "{name}: {native} takes its declared parameters as one `call: {carrier}`"
                ));
            }
            taken_ticks
                .map(|index| native_args[index].clone())
                .into_iter()
                .chain(declared.iter().map(|(arg, ty)| (arg.clone(), ty)))
                .collect()
        };
        let is_level = host_param::is_level(hook);
        let ticks = is_level
            && args.iter().any(|(name, ty)| {
                name == "ticks" && ty.to_token_stream().to_string() == "& mut Ticks"
            });
        host_param::validate(hook, name, is_level, receiver_mut, ticks)?;
        let context = host_param::context_param(hook, &mut args)?;
        if ticks {
            args.retain(|(name, _)| name != "ticks");
        }
        let carried = args.clone();
        let mut used: HashSet<_> = args.iter().map(|(name, _)| name.clone()).collect();
        let mod_item = fresh_name("mod_hook", &mut used);
        let part_list = fresh_name("parts", &mut used);
        let part_item = fresh_name("part", &mut used);
        let part_table = fresh_name("part_table", &mut used);
        let replacement = fresh_name("replacement", &mut used);
        let contract = hook.contract;
        let out = hook.out.as_deref();
        let answered = hook.answered_by_mod;
        let output = if let Some(out) = out {
            let index = args
                .iter()
                .position(|(n, _)| n == out)
                .ok_or_else(|| format!("missing output {out} in {name}"))?;
            Some(args.remove(index))
        } else {
            None
        };
        let result = if let Some((_, ty)) = &output {
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
                Contract::Collect => String::new(),
                _ => return Err(format!("unexpected output type for {name}")),
            }
        } else {
            String::new()
        };
        let host = context
            .or_else(|| host_param::host_param(hook, receiver_mut, ticks))
            .map(|param| {
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
        // A scoped hook's parts: the trait its method goes on, and the lookup
        // of the parts attached to the thing its `key` argument names.
        let scoped = match &hook.scope {
            None => None,
            Some(scope) => {
                let key = scope.key.as_str();
                let (part_trait, table) = match scope.kind {
                    PartKind::Block => ("BlockPart", "block"),
                    PartKind::BlockEntity => ("BlockEntityPart", "block_entity"),
                    PartKind::Item => ("ItemPart", "item"),
                    PartKind::Menu => ("MenuPart", "menu"),
                };
                if is_render {
                    return Err(format!("render hook {name} cannot be scoped"));
                }
                let (_, ty) = args
                    .iter()
                    .find(|(arg, _)| arg == key)
                    .ok_or_else(|| format!("key {key} is not an argument of {name}"))?;
                let key_type = ty.to_token_stream().to_string().replace("mod_api :: ", "");
                let lookup = match key_type.as_str() {
                    "BlockStateId" if table == "block" => format!("block_state({key})"),
                    "BlockStateId" => {
                        let Some((level, _)) = host.as_ref().filter(|_| is_level) else {
                            return Err(format!("{name} has no level to name {key}'s block"));
                        };
                        if hook.side == Side::Both {
                            format!(
                                "{table}(|| match &{level} {{ LevelSide::Server({level}) | LevelSide::Client({level}) => {level}.state_block({key}), }})"
                            )
                        } else {
                            format!("{table}(|| {level}.state_block({key}))")
                        }
                    }
                    "& ItemStack" => {
                        format!("{table}(|| super::registry_keys::item_id({key}.item()))")
                    }
                    "& str" => format!("{table}(|| Some({key}))"),
                    // An entity's type id need not be its block's: its own table.
                    "& ModBlockEntity" | "& mut ModBlockEntity" => {
                        format!("block_entity_kind(|| Some({key}.id()))")
                    }
                    other => return Err(format!("key {key} of {name} has type {other}")),
                };
                Some((part_trait, lookup))
            }
        };
        let event = fresh_name("event", &mut used);
        let event_type = format!("{}Event", hook_id(name));
        let hook_id = event_type.strip_suffix("Event").unwrap();
        if is_render {
            render_hook_ids.push_str(&format!("{hook_id},\n"));
            render_hook_constants.push_str(&format!(
                "pub const {name}: Option<super::RenderHookId> = Some(super::RenderHookId::{hook_id});\n"
            ));
            render_hook_count += 1;
        } else {
            hook_ids.push_str(&format!("{hook_id},\n"));
            hook_constants.push_str(&format!(
                "pub const {name}: Option<super::HookId> = Some(super::HookId::{hook_id});\n"
            ));
            hook_count += 1;
        }
        let mut trait_params = vec!["&self".to_owned()];
        let mut dispatch_params = Vec::new();
        let mut call_args = Vec::new();
        if let Some((name, param)) = &host {
            trait_params.push(format!("{name}: {}", param.ty));
            let by_side = param.ty == "HostSide" || param.ty == "LevelSide";
            dispatch_params.push(format!(
                "{}{name}: {}",
                if by_side { "mut " } else { "" },
                param.ty
            ));
            call_args.push(if by_side {
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
        let collected = output.as_ref().filter(|_| contract == Contract::Collect);
        // A draw surface reaches mods as its context's trait object, which the native call
        // builds from `Game` and the surface.
        let mut surface_calls = std::collections::HashMap::new();
        for (name, ty) in args.iter().chain(collected) {
            let drawn = DrawContext::of(contexts, ty);
            if let Some((context, mutable)) = drawn {
                if !mutable || receiver_mut != Some(false) {
                    return Err(format!(
                        "{}: a drawing hook takes &self and its surface as &mut",
                        hook.name
                    ));
                }
                surface_calls.insert(
                    name.as_str(),
                    format!(
                        "&mut crate::game::host::draw::{} {{ game: self, surface: {name} }}",
                        context.game_struct()
                    ),
                );
            }
            let drawn: Option<Type> = drawn.map(|(context, _)| {
                syn::parse_str(&format!("&mut dyn {}", context.trait_name)).expect("a trait name")
            });
            let ty = drawn.as_ref().unwrap_or(ty);
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
            let mutable_entity = ty.to_token_stream().to_string() == "& mut ModBlockEntity";
            let field_ty = if mutable_entity {
                syn::parse_quote!(ModBlockEntityView<'_>)
            } else {
                ty.clone()
            };
            let (field, scalar) = lifetimes.field(&field_ty);
            let visibility = if event_type == "ApplyEffectTickEvent" && name == "changes" {
                "pub(super)"
            } else {
                "pub"
            };
            fields.push_str(&format!(
                "{visibility} {name}: {},\n",
                field.to_token_stream()
            ));
            if scalar {
                inits.push(format!("{name}: *{name}"));
                write_back.push_str(&format!("*{name} = {event}.{name};"));
            } else if mutable_entity {
                inits.push(format!("{name}: ModBlockEntityView::new({name})"));
            } else {
                inits.push(name.clone());
            }
        }
        if let Some((name, ty)) = output.as_ref().filter(|_| contract != Contract::Collect) {
            dispatch_params.push(format!("{name}: {}", ty.to_token_stream()));
        }
        if !declared.is_empty() {
            dispatch_params.truncate(usize::from(host.is_some()));
            let (fields, generics) = carrier_fields(&carried);
            dispatch_params.push(format!(
                "call: calls::{carrier}{}",
                if generics.is_empty() { "" } else { "<'_>" }
            ));
            calls.push_str(&format!(
                "/// What the game passes `dispatch_{name}`: the hook's declared parameters.\n#[doc(hidden)]\n#[allow(missing_docs)]\npub struct {carrier}{generics} {{\n{fields}}}\n"
            ));
        }
        let (generics, elided) = match (lifetimes.a, lifetimes.b) {
            (_, true) => ("<'a, 'b>", "<'_, '_>"),
            (true, false) => ("<'a>", "<'_>"),
            (false, false) => ("", ""),
        };
        trait_params.push(format!("{event}: &mut {event_type}{elided}"));
        call_args.push(format!("&mut {event}"));
        let unused = trait_params[1..]
            .iter()
            .map(|param| format!("let _ = {};", param.split(':').next().unwrap()))
            .collect::<String>();
        let default = match contract {
            Contract::Cancel => "Flow::Continue",
            Contract::Replace => "None",
            _ => "",
        };
        // Reuse the exact host and drawing conversions for generated game-side calls.
        if in_game {
            let passed = host
                .as_ref()
                .map(|(_, param)| {
                    if param.call == "&*self" && receiver_mut == Some(false) {
                        "self".to_owned()
                    } else {
                        param.call.to_owned()
                    }
                })
                .into_iter()
                .chain(if declared.is_empty() {
                    args.iter()
                        .chain(output.as_ref())
                        .map(|(n, _)| {
                            surface_calls
                                .get(n.as_str())
                                .cloned()
                                .unwrap_or_else(|| n.clone())
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec!["call".to_owned()]
                })
                .collect::<Vec<_>>()
                .join(", ");
            let mut signature = sig.clone();
            signature.ident = syn::Ident::new(&format!("fire_{name}"), signature.ident.span());
            let target_path: syn::Path = syn::parse_str(target)
                .map_err(|error| format!("invalid target for {name}: {error}"))?;
            let receiver = &target_path.segments[target_path.segments.len() - 2].ident;
            let crossing = if declared.is_empty() {
                stack_crossing(
                    &args,
                    output.as_ref(),
                    hook_id,
                    scoped.is_some(),
                    &surface_calls,
                )
            } else {
                None
            };
            let body = if let Some(crossing) = crossing {
                let passed = host
                    .as_ref()
                    .map(|(_, param)| {
                        if param.call == "&*self" && receiver_mut == Some(false) {
                            "self".to_owned()
                        } else {
                            param.call.to_owned()
                        }
                    })
                    .into_iter()
                    .chain(crossing.arguments.into_iter().map(|argument| {
                        surface_calls
                            .get(argument.as_str())
                            .cloned()
                            .unwrap_or(argument)
                    }))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} mod_api::{call}({passed}); {}",
                    crossing.before, crossing.after
                )
            } else {
                format!("mod_api::{call}({passed});")
            };
            fire.push_str(&format!(
                "impl {receiver}<'_> {{ #[inline] pub(super) {signature} {{ {body} }} }}\n",
                signature = signature.to_token_stream()
            ));
            if fill {
                let passed = sig
                    .inputs
                    .iter()
                    .filter_map(|arg| match arg {
                        FnArg::Typed(arg) => Some(arg.pat.to_token_stream().to_string()),
                        FnArg::Receiver(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                fills.push(Fill {
                    native: native.clone(),
                    call: format!("self.fire_{name}({passed});"),
                });
            }
        }
        let invoke = call_args.join(", ");
        let trait_params = trait_params.join(", ");
        let dispatch_params = dispatch_params.join(", ");
        let trait_name = if is_render { "RenderHooks" } else { "Hooks" };
        let traits = if is_render {
            &mut render_traits
        } else {
            &mut traits
        };
        let dispatch = if is_render {
            &mut render_dispatch
        } else {
            &mut dispatch
        };
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
            (Contract::Cancel, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Flow::Cancel` to take it: this sets `{out}`, and later {callee}s are skipped.\n"
            )),
            (Contract::Replace, Some(out)) => method_source.push_str(&format!(
                "///\n/// Return `Some` to set `{out}`; the first {callee} returning `Some` wins.\n"
            )),
            (Contract::Collect, Some(out)) => method_source.push_str(&format!(
                "///\n/// Every {callee} gets `{event}.{out}` in order and may add, change or remove {out}.\n"
            )),
            _ => {}
        }
        method_source.push_str(&format!(
            "fn {name}({trait_params}){result} {{ {unused} {default} }}\n"
        ));
        match &scoped {
            Some((part_trait, _)) => part_traits
                .entry(*part_trait)
                .or_default()
                .push_str(&method_source),
            None => traits.push_str(&method_source),
        }
        let trait_name = scoped
            .as_ref()
            .map_or(trait_name, |(part_trait, _)| part_trait);
        dispatch.push_str(&format!(
            "/// What [`{trait_name}::{name}`] is told. Each field is that hook's argument of the same name, described in the hook's doc.\n#[allow(missing_docs)]\npub struct {event_type}{generics} {{\n{fields}}}\n"
        ));
        let build = format!("let mut {event} = {event_type} {{ {} }};", inits.join(", "));
        // A trait mod's call is marked with the mod for a panic to name; render hooks are called bare.
        let each = if is_render {
            format!("super::entered_render(RenderHookId::{hook_id})")
        } else if answered {
            format!("super::entered_with_id(HookId::{hook_id}, \"{name}\")")
        } else {
            format!("super::entered(HookId::{hook_id}, \"{name}\")")
        };
        // One callee's call: a mod's, or a scoped hook's part's.
        let call = |callee: &str, exit: &str| match contract {
            Contract::Cancel => format!(
                "if matches!({callee}.{name}({invoke}), Flow::Cancel) {{ *{o} = true; {exit}; }}",
                o = out.unwrap()
            ),
            Contract::Replace if answered => format!(
                "if let Some({replacement}) = {callee}.{name}({invoke}) {{ *{o} = Some(Answered {{ answer: {replacement}, mod_id }}); {exit}; }}",
                o = out.unwrap()
            ),
            Contract::Replace => format!(
                "if let Some({replacement}) = {callee}.{name}({invoke}) {{ *{o} = Some({replacement}); {exit}; }}",
                o = out.unwrap()
            ),
            _ => format!("{callee}.{name}({invoke});"),
        };
        let binding = if answered {
            format!("(mod_id, {mod_item})")
        } else {
            mod_item.clone()
        };
        let (lookup, calls) = match &scoped {
            None => (
                String::new(),
                format!("for {binding} in {each} {{ {} }}", call(&mod_item, "break")),
            ),
            Some((_, lookup)) => (
                // The parts are looked up first: the key may be moved into the event.
                format!(
                    "\nif !super::has_part_hook(HookId::{hook_id}) {{ return; }} let {part_table} = super::parts(HookId::{hook_id}, \"{name}\"); let {part_list} = {part_table}.for_hook(HookId::{hook_id}).map_or(&[][..], |table| table.{lookup}); if {part_list}.is_empty() {{ return; }}"
                ),
                format!(
                    "for {part_item} in super::entered_parts({part_list}, \"{name}\") {{ {} }}",
                    call(&part_item, "break"),
                ),
            ),
        };
        let body = match contract {
            Contract::Cancel => format!(
                "if *{o} {{ return; }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            Contract::Replace => format!(
                "if {o}.is_some() {{ return; }} {lookup} {build} {calls} {write_back}",
                o = out.unwrap()
            ),
            _ => format!("{lookup} {build} {calls} {write_back}"),
        };
        let early_return = if scoped.is_some() {
            String::new()
        } else if is_render {
            format!("if !super::render_listened(RenderHookId::{hook_id}) {{ return; }}")
        } else {
            format!("if !super::listened(HookId::{hook_id}) {{ return; }}")
        };
        let unpack = if declared.is_empty() {
            String::new()
        } else {
            let names: Vec<_> = carried.iter().map(|(arg, _)| arg.as_str()).collect();
            format!("let calls::{carrier} {{ {} }} = call;\n", names.join(", "))
        };
        dispatch.push_str(&format!(
            "#[doc(hidden)] pub fn dispatch_{name}({dispatch_params}) {{ {unpack}{early_return}\n{body} }}\n"
        ));
    }
    if !calls.is_empty() {
        dispatch.push_str(&format!(
            "#[doc(hidden)]\n#[allow(missing_docs)]\npub mod calls {{\nuse super::*;\n{calls}}}\n"
        ));
    }
    Ok(HookSources {
        traits,
        dispatch,
        render_traits,
        render_dispatch,
        part_traits,
        fills,
        fire,
        hook_ids,
        hook_constants,
        hook_count,
        render_hook_ids,
        render_hook_constants,
        render_hook_count,
    })
}

/// A carrier's fields, each type with its elided lifetimes named `'a`, and its generics.
struct StackCrossing {
    before: String,
    arguments: Vec<String>,
    after: String,
}

/// Only stack-bearing declared argument types select this seam. Event records
/// are built explicitly at their game call sites, not inferred from game bodies.
fn stack_crossing(
    args: &[(String, &Type)],
    output: Option<&(String, &Type)>,
    hook_id: &str,
    scoped: bool,
    surfaces: &std::collections::HashMap<&str, String>,
) -> Option<StackCrossing> {
    fn contains_stack(ty: &Type) -> bool {
        use syn::visit::Visit;
        struct Stack(bool);
        impl<'ast> Visit<'ast> for Stack {
            fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
                // Bare names resolve to the owned records in the emitted API module.
                // Qualified names do not acquire that provenance from their last segment.
                self.0 |= path.path.segments.len() == 1
                    && path.path.segments.last().is_some_and(|s| {
                        matches!(
                            s.ident.to_string().as_str(),
                            "ItemStack" | "ItemCost" | "MerchantOffer"
                        )
                    });
                syn::visit::visit_type_path(self, path);
            }
        }
        let mut stack = Stack(false);
        stack.visit_type(ty);
        stack.0
    }
    let parameters = args.iter().chain(output).collect::<Vec<_>>();
    if !parameters.iter().any(|(_, ty)| contains_stack(ty)) {
        return None;
    }
    let guard = if scoped { "has_part_hook" } else { "listened" };
    let mut before = format!(
        "if !mod_api::__private::{guard}(mod_api::HookId::{hook_id}) {{ return; }} use crate::game::api_values::TryIntoApi; "
    );
    if parameters.iter().any(|(_, ty)| {
        contains_stack(ty) && matches!(ty, Type::Reference(r) if r.mutability.is_some())
    }) {
        before.push_str("use crate::game::api_values::{TryIntoEngine, ItemBoundary}; ");
    }
    let mut arguments = Vec::new();
    let mut validations = String::new();
    let mut writes = String::new();
    for (name, ty) in parameters {
        let stack = contains_stack(ty);
        let mutable = matches!(ty, Type::Reference(r) if r.mutability.is_some());
        if surfaces.contains_key(name.as_str()) || (!stack && !mutable) {
            arguments.push(name.clone());
            continue;
        }
        let local = format!("__api_{name}");
        let value = if stack {
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
        if mutable {
            let converted = format!("__engine_{name}");
            let value = if stack {
                let result = format!("__checked_{name}");
                validations.push_str(&format!("let {result}: Result<_, mod_api::StackWriteError> = self.boundary_items().ok_or(mod_api::StackWriteError::Unavailable).and_then(|items| {local}.try_into_engine(items)); "));
                format!("match {result} {{ Ok(value) => value, Err(_) => return }}")
            } else {
                local
            };
            validations.push_str(&format!("let {converted} = {value}; "));
            writes.push_str(&format!("*{name} = {converted}; "));
        }
    }
    Some(StackCrossing {
        before,
        arguments,
        after: format!("{validations}{writes}"),
    })
}

#[cfg(test)]
mod stack_crossing_tests {
    use super::*;
    #[test]
    fn stack_hook_gates_before_conversion_and_stages_every_write() {
        let stack: Type = syn::parse_quote!(&mut ItemStack);
        let power: Type = syn::parse_quote!(&mut u32);
        let args = [("stack".to_owned(), &stack), ("power".to_owned(), &power)];
        let crossing =
            stack_crossing(&args, None, "ItemDamage", true, &Default::default()).unwrap();
        assert!(
            crossing
                .before
                .starts_with("if !mod_api::__private::has_part_hook")
        );
        assert!(crossing.before.contains("stack.try_into_api()"));
        assert_eq!(crossing.arguments, ["&mut __api_stack", "&mut __api_power"]);
        assert!(
            crossing
                .after
                .contains("Result<_, mod_api::StackWriteError>")
        );
        let validation = crossing.after.find("try_into_engine").unwrap();
        let first_write = crossing.after.find("*stack =").unwrap();
        assert!(validation < first_write);
        assert!(crossing.after.find("let __engine_power").unwrap() < first_write);
        let body = format!(
            "{{ {} {}; {} }}",
            crossing.before, "dispatch(&mut __api_stack, &mut __api_power)", crossing.after
        );
        syn::parse_str::<syn::Block>(&body).unwrap();
    }
    #[test]
    fn unsubscribed_item_use_never_converts() {
        let stack: Type = syn::parse_quote!(&ItemStack);
        let claimed: Type = syn::parse_quote!(&mut bool);
        let args = [
            ("stack".to_owned(), &stack),
            ("claimed".to_owned(), &claimed),
        ];
        let crossing = stack_crossing(&args, None, "UseItem", true, &Default::default()).unwrap();
        let body = format!(
            "{} mod_api::dispatch_use_item(self, {}); {}",
            crossing.before,
            crossing.arguments.join(", "),
            crossing.after
        );
        let source = r#"
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    for _ in 0..1_000 { game.fire_use_item(&ItemStack, &mut claimed); }
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
        .replace("FIRE_BODY", &body);
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
    fn a_new_hook_over_the_limit_needs_the_carrier() {
        assert!(target_parameters("new_hook", "native_new_hook", 7, "NewHookCall").is_ok());
        let refused = target_parameters("new_hook", "native_new_hook", 8, "NewHookCall");
        assert!(refused.unwrap_err().contains("one NewHookCall"));
    }
}
