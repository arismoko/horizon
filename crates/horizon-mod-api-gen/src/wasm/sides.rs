//! The two sides' code: the guest's `Guest` (every imported trait) and exports, and the game's
//! imports and proxies (every exported trait).
use super::glue::{
    InPlace, conversions, flat_element, flat_method, lift, lower, lower_owned, rust_path, unpack,
};
use super::ty::{Kind, List, Str, Ty, Types, bound_ident, wit_ident};
use super::wit::{self, Signature, interface, signature};
use super::{Method, exports};
use heck::{ToSnakeCase, ToUpperCamelCase};
use std::fmt::Write;

/// `ty` as Rust names it in a trait method's signature.
pub(super) fn rust_ty(types: &Types<'_>, ty: &Ty, ret: bool) -> String {
    match ty {
        Ty::Unit => "()".into(),
        Ty::Prim(rust, _) | Ty::Wide(rust) => (*rust).into(),
        Ty::Duration => "std::time::Duration".into(),
        Ty::Handler(name, false) => format!("std::sync::Arc<dyn {}>", trait_path(name)),
        Ty::Handler(name, true) => format!("Box<dyn {}>", trait_path(name)),
        Ty::Str(Str::Owned) => "String".into(),
        Ty::Str(Str::Borrowed) => "&str".into(),
        Ty::Str(Str::Static) => "&'static str".into(),
        Ty::Str(Str::Boxed) => "Box<str>".into(),
        Ty::Str(Str::Shared) => "std::sync::Arc<str>".into(),
        Ty::Str(Str::Cow) => "std::borrow::Cow<'static, str>".into(),
        Ty::List(item, List::Set(set)) => format!("{set}<{}>", rust_ty(types, item, ret)),
        Ty::List(item, List::Vec) => format!("Vec<{}>", rust_ty(types, item, ret)),
        Ty::List(item, List::Slice) => format!("&[{}]", rust_ty(types, item, ret)),
        Ty::List(item, List::Boxed) => format!("Box<[{}]>", rust_ty(types, item, ret)),
        Ty::List(item, List::Shared) => format!("std::sync::Arc<[{}]>", rust_ty(types, item, ret)),
        Ty::List(item, List::Map(map)) => match &**item {
            Ty::Tuple(pair) => format!(
                "{map}<{}, {}>",
                rust_ty(types, &pair[0], ret),
                rust_ty(types, &pair[1], ret)
            ),
            _ => unreachable!("a map's items are pairs"),
        },
        Ty::Array(item, n) => format!("[{}; {n}]", rust_ty(types, item, ret)),
        Ty::Option(item) => format!("Option<{}>", rust_ty(types, item, ret)),
        Ty::Result(a, b) => format!(
            "Result<{}, {}>",
            rust_ty(types, a, ret),
            rust_ty(types, b, ret)
        ),
        Ty::Tuple(items) => format!(
            "({},)",
            items
                .iter()
                .map(|i| rust_ty(types, i, ret))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Ty::Ref(inner, true) => format!("&mut {}", rust_ty(types, inner, ret)),
        Ty::Ref(inner, false) => format!("&{}", rust_ty(types, inner, ret)),
        Ty::Boxed(inner) => format!("Box<{}>", rust_ty(types, inner, ret)),
        Ty::Shared(inner) => format!("std::sync::Arc<{}>", rust_ty(types, inner, ret)),
        Ty::Named(def) => rust_path(&types.defs[*def]),
        Ty::Dyn(name, mutable) => {
            // A returned shared one is the guest's unit, for any borrow; a mutable one, being
            // invariant, keeps the borrow's own lifetime.
            let lifetime = if ret && !mutable { "'static " } else { "" };
            let m = if *mutable { "mut " } else { "" };
            format!("&{lifetime}{m}dyn {}", trait_path(name))
        }
    }
}

pub(super) fn trait_path(owner: &str) -> String {
    if owner == super::ERASED {
        return "std::any::Any + Send + Sync".into();
    }
    // Every name in it is `mod_api`'s, but the prelude's (`ResourceHandler<ItemResource>`,
    // `EntityCapabilityProvider<dyn EnergyHandler, Option<Direction>>`).
    struct Qualify;
    impl syn::visit_mut::VisitMut for Qualify {
        fn visit_path_mut(&mut self, path: &mut syn::Path) {
            syn::visit_mut::visit_path_mut(self, path);
            let outside = ["Option", "Vec", "Box", "String", "Result", "crate", "std"];
            if path.leading_colon.is_none()
                && !outside.contains(&path.segments[0].ident.to_string().as_str())
            {
                path.segments.insert(0, syn::parse_quote!(crate));
            }
        }
    }
    let mut path: syn::Path = syn::parse_str(owner).expect("an owner is a path");
    syn::visit_mut::VisitMut::visit_path_mut(&mut Qualify, &mut path);
    super::ty::written(&path)
}

/// A method's Rust signature, its arguments named `p0…`.
fn rust_sig(types: &Types<'_>, method: &Method) -> String {
    let mut params = Vec::new();
    match method.receiver {
        Some(true) => params.push("&mut self".to_string()),
        Some(false) => params.push("&self".to_string()),
        None => {}
    }
    for (i, (_, ty)) in method.params.iter().enumerate() {
        params.push(format!("p{i}: {}", rust_ty(types, ty, false)));
    }
    let ret = match &method.ret {
        Ty::Unit => String::new(),
        ty => format!(" -> {}", rust_ty(types, ty, true)),
    };
    format!("fn {}({}){ret}", method.name, params.join(", "))
}

fn header(wit_text: &str, macro_call: &str, options: &str) -> String {
    format!(
        "// @generated by horizon-mod-api-gen from the mod API's traits; do not edit.\n{macro_call}!({{ inline: r####\"{wit_text}\"####, world: \"mod\", {options} }});\nuse self::horizon::mod_api as api;\n#[allow(unused_imports)]\nuse super::Refused;\n"
    )
}

/// A copy of what `local`, a `&mut` record argument, holds before the call, so only what the
/// call changed is sent back.
fn before(types: &Types<'_>, sig: &Signature, ty: &Ty, local: &str, out: &mut String) {
    if let Ty::Named(def) = ty
        && sig.results.contains(&wit::out_name(&types.defs[*def]))
    {
        writeln!(out, "let b_{local} = before_{def}(&{local});").unwrap();
    }
}

/// The function returning what the method's results tuple holds, after the call.
fn results(
    types: &Types<'_>,
    method: &Method,
    sig: &Signature,
    ret: &str,
    outs: &[(usize, String, Ty)],
) -> String {
    let mut parts = Vec::new();
    if wit::wit(types, &method.ret).ok().flatten().is_some() {
        parts.push(super::glue::lower_owned(types, &method.ret, ret));
    }
    for (_, local, ty) in outs {
        match ty {
            Ty::Named(def) if sig.results.contains(&wit::out_name(&types.defs[*def])) => {
                parts.push(format!("lower_out_{def}(&{local}, &b_{local})?"))
            }
            Ty::Named(def) if matches!(types.defs[*def].kind, Kind::Record(_)) => {}
            ty => parts.push(lower(types, ty, &format!("(&{local})"))),
        }
    }
    match parts.len() {
        0 => "()".into(),
        1 => parts.remove(0),
        _ => format!("({})", parts.join(", ")),
    }
}

/// The guest side: `Guest` implementing every imported trait, and the exports calling the mod.
pub(super) fn guest(
    types: &Types<'_>,
    methods: &[&Method],
    parts: &[(&'static str, String)],
    wit_text: &str,
    hooks: &[(String, String)],
) -> Result<String, String> {
    let (defs, outs) = wit::reached(types, methods);
    let mut code = header(wit_text, "wit_bindgen::generate", "generate_all");
    types.guest.set(true);
    code.push_str(&conversions(types, &defs, &outs));
    code.push_str(
        "/// The host as the guest reaches it: each method calls its import.\npub struct Guest;\n",
    );
    let mut owners: Vec<&str> = Vec::new();
    for method in methods {
        if !owners.contains(&method.owner) {
            owners.push(method.owner);
        }
    }
    // Every imported trait, a method or not: a trait with none bound still holds up its subtraits.
    for owner in super::IMPORTS {
        if !owners.contains(owner) {
            writeln!(code, "impl {} for Guest {{}}", trait_path(owner)).unwrap();
            for (_, via) in parts.iter().filter(|(of, _)| of == owner) {
                writeln!(code, "impl {} for Guest {{}}", trait_path(via)).unwrap();
            }
        }
    }
    for owner in owners.iter().filter(|o| !exports().contains(o)) {
        writeln!(code, "impl {} for Guest {{", trait_path(owner)).unwrap();
        match *owner {
            "Host" => code.push_str("fn backend(&self) -> Option<crate::BackendRef<'_>> { Some(crate::BackendRef::of(self)) }\nfn capability_providers(&self) -> &'static dyn crate::CapabilityProviders { &Guest }\n"),
            "ServerHost" => code.push_str("fn backend_mut(&mut self) -> Option<crate::BackendMutRef<'_>> { Some(crate::BackendMutRef::of(self)) }\n"),
            "ClientHost" => code.push_str("fn client_backend_mut(&mut self) -> Option<crate::ClientBackendMutRef<'_>> { Some(crate::ClientBackendMutRef::of(self)) }\n"),
            _ => {}
        }
        for method in methods
            .iter()
            .filter(|m| m.owner == *owner && m.via.is_none())
        {
            code.push_str(&guest_import_call(types, method)?);
        }
        code.push_str("}\n");
        // The area traits that are parts of it: each implemented, those with no method that
        // crosses included.
        for (_, via) in parts.iter().filter(|(of, _)| of == owner) {
            writeln!(code, "impl {} for Guest {{", trait_path(via)).unwrap();
            for method in methods
                .iter()
                .filter(|m| m.owner == *owner && m.via.as_deref() == Some(via.as_str()))
            {
                code.push_str(&guest_import_call(types, method)?);
            }
            code.push_str("}\n");
        }
    }
    // The game's objects the mod holds, each method calling its `held-` import; and the mod's
    // own, boxed again.
    for owner in super::exports().iter().filter(|o| super::held(o)) {
        let path = trait_path(owner);
        let held = bound_ident(&format!("held-{}", interface(owner)));
        let mut forward = String::new();
        writeln!(code, "impl {path} for super::Held {{").unwrap();
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let callee = format!("api::{held}::{}", bound_ident(&wit_ident(&method.name)));
            code.push_str(&guest_call(types, method, &callee, Some("self.0"))?);
            let args: Vec<String> = (0..method.params.len()).map(|i| format!("p{i}")).collect();
            writeln!(
                forward,
                "{} {{ self.0.{}({}) }}",
                rust_sig(types, method),
                method.name,
                args.join(", ")
            )
            .unwrap();
        }
        code.push_str("}\n");
        writeln!(
            code,
            "impl {path} for super::Shared<dyn {path}> {{\n{forward}}}"
        )
        .unwrap();
    }
    let arms: Vec<String> = hooks
        .iter()
        .map(|(v, n)| format!("crate::HookId::{v} => \"{n}\""))
        .collect();
    writeln!(
        code,
        "fn hook_name(id: crate::HookId) -> &'static str {{ match id {{ {} }} }}",
        arms.join(", ")
    )
    .unwrap();
    // Exports.
    code.push_str("struct Exports;\nexport!(Exports);\n");
    for owner in owners.iter().filter(|o| exports().contains(o)) {
        let iface = bound_ident(&interface(owner));
        writeln!(
            code,
            "impl exports::horizon::mod_api::{iface}::Guest for Exports {{"
        )
        .unwrap();
        if let Some((_, list)) = super::SUBSCRIBED.iter().find(|(o, _)| o == owner) {
            let (param, receiver) = match super::mod_receiver(owner) {
                Some(receiver) => ("", receiver.to_string()),
                None => (
                    "handler: u32",
                    format!("super::handler::<dyn {}>(handler)", trait_path(owner)),
                ),
            };
            writeln!(code, "fn subscribed({param}) -> Vec<String> {{ {receiver}.{list}().iter().flatten().map(|id| hook_name(*id).to_string()).collect() }}").unwrap();
        }
        if super::mod_receiver(owner).is_some() {
            code.push_str(
                "fn release_handlers(handlers: Vec<u32>) { super::release_handlers(handlers) }\n",
            );
        }
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let sig = signature(types, method)?;
            let object = |_: &str, mutable: bool| {
                if mutable {
                    "keep.val_mut(Guest)".to_string()
                } else {
                    "&Guest".to_string()
                }
            };
            let place = InPlace::new(types, &object);
            let mut out = String::from("let keep = super::Keep::default();");
            let mut call_args = Vec::new();
            let mut mut_locals = Vec::new();
            for (name, ty) in &method.params {
                let source = if wit::wit(types, ty)?.is_some() {
                    bound_ident(&wit_ident(name))
                } else {
                    "()".into()
                };
                match ty {
                    Ty::Ref(inner, true) if !matches!(**inner, Ty::Dyn(..)) => {
                        let value = place.value(inner, &source)?;
                        let local = format!("m{}", mut_locals.len());
                        writeln!(out, "let mut {local} = {value};").unwrap();
                        before(types, &sig, inner, &local, &mut out);
                        call_args.push(format!("&mut {local}"));
                        mut_locals.push((0, local, (**inner).clone()));
                    }
                    ty => call_args.push(place.value(ty, &source)?),
                }
            }
            let receiver = match super::mod_receiver(owner) {
                Some(receiver) => receiver.to_string(),
                None => format!("super::handler::<dyn {}>(handler)", trait_path(owner)),
            };
            let result = results(types, method, &sig, "r", &mut_locals);
            writeln!(
                code,
                "#[allow(unused_variables, unused_mut, clippy::all)] fn {}({}) {} {{ super::expect((|| -> Result<_, Refused> {{ {} let r = {receiver}.{}({}); Ok({result}) }})()) }}",
                bound_ident(&wit_ident(&method.name)),
                export_params(types, method, &sig)?,
                export_ret(&sig),
                out,
                method.name,
                call_args.join(", ")
            )
            .unwrap();
        }
        code.push_str("}\n");
    }
    Ok(code)
}

/// A guest method calling import `callee` (with `lead`, the held object's index, first).
fn guest_call(
    types: &Types<'_>,
    method: &Method,
    callee: &str,
    lead: Option<&str>,
) -> Result<String, String> {
    // `Guest` is a unit type: even a mutable frame view allocates no memory.
    let frame_view = if matches!(method.ret, Ty::Dyn(_, true)) {
        "Box::leak(Box::new(Guest))"
    } else {
        "&Guest"
    };
    if super::frame_accessor(method) {
        return Ok(format!("{} {{ {frame_view} }}\n", rust_sig(types, method)));
    }
    let sig = signature(types, method)?;
    let mut prep = String::new();
    let mut args = Vec::new();
    for (i, (_, ty)) in method.params.iter().enumerate() {
        if wit::wit(types, ty)?.is_some() {
            writeln!(
                prep,
                "let a{i} = {};",
                lower(types, ty, &format!("(&p{i})"))
            )
            .unwrap();
            args.push(arg(types, ty, &format!("a{i}")));
        }
    }
    let (pattern, names) = unpack(&sig);
    let mut back = String::new();
    let mut next = usize::from(wit::wit(types, &method.ret)?.is_some());
    for (i, (_, ty)) in method.params.iter().enumerate() {
        if let Ty::Ref(inner, true) = ty {
            match &**inner {
                Ty::Named(def) if sig.results.contains(&wit::out_name(&types.defs[*def])) => {
                    writeln!(back, "write_back_{def}(p{i}, {})?;", names[next]).unwrap();
                    next += 1;
                }
                Ty::Dyn(..) => {}
                Ty::Named(def) if matches!(types.defs[*def].kind, Kind::Record(_)) => {}
                inner => {
                    writeln!(back, "*p{i} = {};", lift(types, inner, &names[next])).unwrap();
                    next += 1;
                }
            }
        }
    }
    let value = if wit::wit(types, &method.ret)?.is_some() {
        guest_ret(types, &method.ret, &names[0])
    } else if let Ty::Dyn(..) = method.ret {
        format!("{frame_view} as {}", rust_ty(types, &method.ret, true))
    } else {
        "()".to_string()
    };
    if let Some(lead) = lead {
        args.insert(0, lead.to_string());
    }
    Ok(format!(
        "#[allow(unused_variables, unused_mut, clippy::all)] {} {{ super::expect((|| -> Result<_, Refused> {{ {prep} let {pattern} = {callee}({}); {back} Ok({value}) }})()) }}\n",
        rust_sig(types, method),
        args.join(", ")
    ))
}

/// The Rust type wit-bindgen gives the WIT type `wire`.
fn wire_rust(wire: &str) -> String {
    fn parse(text: &mut &str) -> String {
        *text = text.trim_start();
        // `result<_, e>`: no ok value.
        if let Some(rest) = text.strip_prefix('_') {
            *text = rest;
            return "()".into();
        }
        let end = text
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '%'))
            .unwrap_or(text.len());
        let name = &text[..end];
        *text = &text[end..];
        let mut args = Vec::new();
        if text.starts_with('<') {
            *text = &text[1..];
            loop {
                args.push(parse(text));
                *text = text.trim_start();
                let c = text.as_bytes()[0];
                *text = &text[1..];
                if c == b'>' {
                    break;
                }
            }
        }
        match name {
            "bool" | "u8" | "u16" | "u32" | "u64" | "f32" | "f64" | "char" => name.into(),
            "s8" | "s16" | "s32" | "s64" => format!("i{}", &name[1..]),
            "string" => "String".into(),
            "list" => format!("Vec<{}>", args[0]),
            "option" => format!("Option<{}>", args[0]),
            "tuple" => format!("({},)", args.join(", ")),
            "result" => format!(
                "Result<{}, {}>",
                args[0],
                args.get(1).map_or("()", String::as_str)
            ),
            _ => format!(
                "horizon::mod_api::types::{}",
                heck::ToUpperCamelCase::to_upper_camel_case(name.trim_start_matches('%'))
            ),
        }
    }
    let mut text = wire;
    parse(&mut text)
}

/// An export's parameters as the guest bindings declare them: owned wire values.
fn export_params(types: &Types<'_>, method: &Method, sig: &Signature) -> Result<String, String> {
    let mut params = Vec::new();
    if sig.params.first().is_some_and(|(n, _)| n == "handler") {
        params.push("handler: u32".to_string());
    }
    for (name, ty) in &method.params {
        if wit::wit(types, ty)?.is_some() {
            params.push(format!(
                "{}: {}",
                bound_ident(&wit_ident(name)),
                wire_type(types, ty)
            ));
        }
    }
    Ok(params.join(", "))
}

fn export_ret(sig: &Signature) -> String {
    match sig.results.as_slice() {
        [] => String::new(),
        [one] => format!("-> {}", wire_rust(one)),
        many => format!(
            "-> ({},)",
            many.iter()
                .map(|r| wire_rust(r))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// The owned wire Rust type of `ty`.
fn wire_type(types: &Types<'_>, ty: &Ty) -> String {
    match ty {
        Ty::Unit | Ty::Dyn(..) => "()".into(),
        Ty::Prim(_, wire) => match *wire {
            "s8" => "i8",
            "s16" => "i16",
            "s32" => "i32",
            "s64" => "i64",
            w => w,
        }
        .into(),
        Ty::Wide(_) => "(u64, u64)".into(),
        Ty::Duration => "u64".into(),
        Ty::Handler(..) => format!("{}::Handler", super::glue::WIRE),
        Ty::Str(Str::Static) => "u32".into(),
        Ty::Str(_) => "String".into(),
        Ty::Array(item, n) if (1..=12).contains(n) => {
            format!("({},)", vec![wire_type(types, item); *n].join(", "))
        }
        Ty::List(item, _) | Ty::Array(item, _) => format!("Vec<{}>", wire_type(types, item)),
        Ty::Option(item) => format!("Option<{}>", wire_type(types, item)),
        Ty::Result(a, b) => format!("Result<{}, {}>", wire_type(types, a), wire_type(types, b)),
        Ty::Tuple(items) => format!(
            "({},)",
            items
                .iter()
                .filter(|i| wit::wit(types, i).ok().flatten().is_some())
                .map(|i| wire_type(types, i))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Ty::Ref(inner, _) | Ty::Boxed(inner) | Ty::Shared(inner) => wire_type(types, inner),
        Ty::Named(def) => format!(
            "{}::{}",
            "horizon::mod_api::types",
            types.defs[*def]
                .wit
                .trim_start_matches('%')
                .to_upper_camel_case()
        ),
    }
}

/// The guest's value for an import's result: owned, or kept for the call where the trait
/// method returns a borrow.
fn guest_ret(types: &Types<'_>, ty: &Ty, e: &str) -> String {
    match ty {
        Ty::Str(Str::Borrowed) => format!("super::intern({e})"),
        Ty::Option(inner) if matches!(**inner, Ty::Str(Str::Borrowed)) => {
            format!(
                "match {e} {{ Some(x) => Some({}), None => None }}",
                guest_ret(types, inner, "x")
            )
        }
        ty => lift(types, ty, e),
    }
}

/// The game's side: each import forwarding to the frame's host, and the proxies.
pub(super) fn host(
    types: &Types<'_>,
    methods: &[&Method],
    wit_text: &str,
    hooks: &[(String, String)],
    servers: &[(&str, Vec<&str>)],
) -> Result<String, String> {
    let (defs, outs) = wit::reached(types, methods);
    let mut code = header(
        wit_text,
        "wasmtime::component::bindgen",
        "imports: { default: trappable }",
    );
    types.guest.set(false);
    code.push_str(&conversions(types, &defs, &outs));
    let mut owners: Vec<&str> = Vec::new();
    for method in methods {
        if !owners.contains(&method.owner) {
            owners.push(method.owner);
        }
    }
    code.push_str("impl api::types::Host for super::State {}\n");
    code.push_str(&frame(servers, methods));
    let arms: Vec<String> = hooks
        .iter()
        .map(|(v, n)| format!("\"{n}\" => Some(crate::HookId::{v})"))
        .collect();
    writeln!(code, "/// The hook a mod names in its subscriptions, if this game has it.\npub(super) fn hook_id(name: &str) -> Option<crate::HookId> {{ match name {{ {}, _ => None }} }}", arms.join(", ")).unwrap();
    for owner in owners.iter().filter(|o| !exports().contains(o)) {
        let iface = bound_ident(&interface(owner));
        writeln!(code, "impl api::{iface}::Host for super::State {{").unwrap();
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let receiver = format!(
                "frame::{}{}()?",
                owner.rsplit("::").next().unwrap().to_snake_case(),
                if method.receiver == Some(true) {
                    "_mut"
                } else {
                    ""
                }
            );
            code.push_str(&host_import(types, method, &receiver, false)?);
        }
        code.push_str("}\n");
    }
    // The game's objects a mod holds, called by their index.
    for owner in exports().iter().filter(|o| super::held(o)) {
        let iface = bound_ident(&format!("held-{}", interface(owner)));
        writeln!(code, "impl api::{iface}::Host for super::State {{").unwrap();
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let receiver = format!("&*super::held::<dyn {}>(handler)?", trait_path(owner));
            code.push_str(&host_import(types, method, &receiver, true)?);
        }
        code.push_str("}\n");
    }
    code.push_str(&store_imports(types, methods, wit_text)?);
    // Proxies: the mod's traits, each method calling its export.
    for owner in owners.iter().filter(|o| exports().contains(o)) {
        let iface = bound_ident(&interface(owner));
        let proxy = if super::mod_receiver(owner).is_some() {
            "super::WasmMod"
        } else {
            "super::WasmHandler"
        };
        writeln!(code, "impl {} for {proxy} {{", trait_path(owner)).unwrap();
        if let Some((_, list)) = super::SUBSCRIBED.iter().find(|(o, _)| o == owner) {
            writeln!(
                code,
                "fn {list}(&self) -> &'static [Option<crate::HookId>] {{ self.subscribed }}"
            )
            .unwrap();
        }
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let sig = signature(types, method)?;
            let mut prep = String::new();
            let mut frame = String::new();
            let mut args = Vec::new();
            if sig.params.first().is_some_and(|(n, _)| n == "handler") {
                args.push("self.index".to_string());
            }
            // The arguments lowered before the call, by name.
            let mut sent = Vec::new();
            for (i, (_, ty)) in method.params.iter().enumerate() {
                frame_objects(types, ty, &format!("p{i}"), &mut frame);
                if wit::wit(types, ty)?.is_some() {
                    writeln!(
                        prep,
                        "let a{i} = {};",
                        lower(types, ty, &format!("(&p{i})"))
                    )
                    .unwrap();
                    sent.push(format!("a{i}"));
                }
            }
            args.extend(sent.iter().cloned());
            let (pattern, names) = unpack(&sig);
            let mut back = String::new();
            let mut next = usize::from(wit::wit(types, &method.ret)?.is_some());
            for (i, (_, ty)) in method.params.iter().enumerate() {
                if let Ty::Ref(inner, true) = ty {
                    match &**inner {
                        Ty::Named(def)
                            if sig.results.contains(&wit::out_name(&types.defs[*def])) =>
                        {
                            writeln!(back, "write_back_{def}(p{i}, {})?;", names[next]).unwrap();
                            next += 1;
                        }
                        Ty::Dyn(..) => {}
                        Ty::Named(def) if matches!(types.defs[*def].kind, Kind::Record(_)) => {}
                        inner => {
                            writeln!(back, "*p{i} = {};", lift(types, inner, &names[next]))
                                .unwrap();
                            next += 1;
                        }
                    }
                }
            }
            let value = if wit::wit(types, &method.ret)?.is_some() {
                lift(types, &method.ret, &names[0])
            } else {
                "()".into()
            };
            // A fallible export answers its error when the call doesn't run, or success where
            // that is doing nothing (a piece that places nothing); the rest, nothing.
            let no_answer = match &method.ret {
                Ty::Result(ok, _) if **ok == Ty::Unit => "|| Ok(())",
                Ty::Result(..) => "|| Err(super::refused())",
                _ => "super::NoAnswer::no_answer",
            };
            // An argument that can't cross (a tag past the limits) isn't sent: the call answers
            // as one that didn't run, and the reason is logged.
            if !sent.is_empty() {
                let sent = sent.join(", ");
                prep = format!(
                    "let ({sent},) = match super::lowering(&self.module, || -> Result<_, super::Refused> {{ {prep} Ok(({sent},)) }}) {{ Ok(a) => a, Err(why) => return super::unsent(&self.module, why, {no_answer}) }};"
                );
            }
            writeln!(
                code,
                "#[allow(unused_variables, unused_mut, clippy::all)] {} {{ let mut frame = super::Frame::default(); {frame} {prep} self.call_or(frame, {}, {no_answer}, |m, store| -> wasmtime::Result<_> {{ let {pattern} = m.horizon_mod_api_{iface}().call_{}(store, {})?; {back} Ok({value}) }}) }}",
                rust_sig(types, method),
                super::call_class(types, method),
                bound_ident(&wit_ident(&method.name)),
                args.join(", ")
            )
            .unwrap();
        }
        code.push_str("}\n");
    }
    // Each mod's exports, found by name in its group's instance: the build renames a mod's
    // interfaces to `<mod id>-<interface>`, so one group holds every member's.
    let mut fields = String::new();
    let mut inits = String::new();
    let mut getters = String::new();
    for owner in owners.iter().filter(|o| exports().contains(o)) {
        let kebab = interface(owner);
        let iface = bound_ident(&kebab);
        let ty = format!("{}Exports", kebab.to_upper_camel_case());
        writeln!(
            code,
            "/// A mod's `{kebab}` exports.\npub(super) struct {ty} {{"
        )
        .unwrap();
        let mut finds = String::new();
        let mut calls = String::new();
        let mut names = Vec::new();
        let mut entries: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
        for method in methods.iter().filter(|m| m.owner == *owner) {
            let sig = signature(types, method)?;
            entries.push((
                wit_ident(&method.name),
                sig.params.iter().map(|(_, w)| wire_rust(w)).collect(),
                sig.results.iter().map(|r| wire_rust(r)).collect(),
            ));
        }
        if super::SUBSCRIBED.iter().any(|(o, _)| o == owner) {
            let params = if super::mod_receiver(owner).is_some() {
                vec![]
            } else {
                vec!["u32".to_string()]
            };
            entries.push(("subscribed".into(), params, vec!["Vec<String>".into()]));
        }
        if super::mod_receiver(owner).is_some() {
            entries.push(("release-handlers".into(), vec!["Vec<u32>".into()], vec![]));
        }
        for (wit_name, params, results) in entries {
            let name = bound_ident(&wit_name);
            // A function's name in the component, without the `%` a WIT keyword is escaped with.
            let wit_name = wit_name.trim_start_matches('%');
            let ret = match results.as_slice() {
                [] => "()".to_string(),
                [one] => one.clone(),
                many => format!("({},)", many.join(", ")),
            };
            let result = if results.is_empty() {
                "()".to_string()
            } else {
                format!("({ret},)")
            };
            let tuple = |items: &[String]| {
                if items.is_empty() {
                    "()".to_string()
                } else {
                    format!("({},)", items.join(", "))
                }
            };
            writeln!(
                code,
                "{name}: wasmtime::component::TypedFunc<{}, {result}>,",
                tuple(&params)
            )
            .unwrap();
            writeln!(
                finds,
                "let {name} = instance.get_export_index(&mut *store, Some(&interface), \"{wit_name}\").ok_or_else(|| super::LoadError::MissingExport(format!(\"{{name}}#{wit_name}\")))?; let {name} = instance.get_typed_func(&mut *store, &{name}).map_err(|e| super::LoadError::ChangedExport(format!(\"{{name}}#{wit_name}\"), e.to_string()))?;"
            )
            .unwrap();
            let args: Vec<String> = (0..params.len()).map(|i| format!("p{i}")).collect();
            let typed: Vec<String> = params
                .iter()
                .enumerate()
                .map(|(i, p)| format!("p{i}: {p}"))
                .collect();
            let unwrap = if results.is_empty() {
                "Ok(())"
            } else {
                "Ok(r.0)"
            };
            writeln!(
                calls,
                "pub(super) fn call_{name}(&self, store: &mut impl wasmtime::AsContextMut<Data = super::State>, {}) -> wasmtime::Result<{ret}> {{ let r = self.{name}.call(&mut *store, {})?; {unwrap} }}",
                typed.join(", "),
                tuple(&args)
            )
            .unwrap();
            names.push(name);
        }
        code.push_str("}\n");
        writeln!(
            code,
            "impl {ty} {{\nfn find(store: &mut wasmtime::Store<super::State>, instance: &wasmtime::component::Instance, prefix: &str) -> Result<Self, super::LoadError> {{ let name = format!(\"{{prefix}}-{kebab}\"); let interface = instance.get_export_index(&mut *store, None, &name).ok_or_else(|| super::LoadError::MissingExport(name.clone()))?; {finds} Ok(Self {{ {} }}) }}\n{calls}}}",
            names.join(", ")
        )
        .unwrap();
        writeln!(fields, "{iface}: {ty},").unwrap();
        writeln!(inits, "{iface}: {ty}::find(store, instance, prefix)?,").unwrap();
        writeln!(
            getters,
            "pub(super) fn horizon_mod_api_{iface}(&self) -> &{ty} {{ &self.{iface} }}"
        )
        .unwrap();
    }
    writeln!(
        code,
        "/// One mod's exports, typed once at load.\npub(super) struct ModExports {{ {fields} }}\nimpl ModExports {{\npub(super) fn find(store: &mut wasmtime::Store<super::State>, instance: &wasmtime::component::Instance, prefix: &str) -> Result<Self, super::LoadError> {{ Ok(Self {{ {inits} }}) }}\n{getters}}}"
    )
    .unwrap();
    Ok(code)
}

/// The guest's method for an import: it calls the import its interface names.
fn guest_import_call(types: &Types<'_>, method: &Method) -> Result<String, String> {
    let callee = format!(
        "api::{}::{}",
        bound_ident(&interface(method.owner)),
        bound_ident(&wit_ident(&method.name))
    );
    guest_call(types, method, &callee, None)
}

/// The game's side of an import: `method` called on `receiver` (taking the held object's index
/// first when `held`).
fn host_import(
    types: &Types<'_>,
    method: &Method,
    receiver: &str,
    held: bool,
) -> Result<String, String> {
    let (params, ret, body) = host_import_body(types, method, receiver, held)?;
    Ok(format!(
        "#[allow(unused_variables, unused_mut, clippy::all)] fn {}(&mut self, {}) -> wasmtime::Result<{ret}> {{ {body} }}",
        bound_ident(&wit_ident(&method.name)),
        params.join(", ")
    ))
}

fn host_import_body(
    types: &Types<'_>,
    method: &Method,
    receiver: &str,
    held: bool,
) -> Result<(Vec<String>, String, String), String> {
    let sig = signature(types, method)?;
    let object = |name: &str, mutable: bool| {
        format!(
            "frame::{}{}()?",
            name.rsplit("::").next().unwrap().to_snake_case(),
            if mutable { "_mut" } else { "" }
        )
    };
    let place = InPlace::new(types, &object);
    let mut out = String::from("let keep = super::Keep::default();");
    let mut call_args = Vec::new();
    let mut mut_locals = Vec::new();
    let mut params = Vec::new();
    if held {
        params.push("handler: u32".to_string());
    }
    for (name, ty) in &method.params {
        let wire = wit::wit(types, ty)?;
        let source = if wire.is_some() {
            bound_ident(&wit_ident(name))
        } else {
            "()".into()
        };
        if wire.is_some() {
            params.push(format!("{source}: {}", wire_type(types, ty)));
        }
        match ty {
            Ty::Ref(inner, true) if !matches!(**inner, Ty::Dyn(..)) => {
                let value = place.value(inner, &source)?;
                let local = format!("m{}", mut_locals.len());
                writeln!(out, "let mut {local} = {value};").unwrap();
                before(types, &sig, inner, &local, &mut out);
                call_args.push(format!("&mut {local}"));
                mut_locals.push((0, local, (**inner).clone()));
            }
            ty => call_args.push(place.value(ty, &source)?),
        }
    }
    let result = results(types, method, &sig, "r", &mut_locals);
    let body = format!(
        "{out} let r = <_ as {}>::{}({receiver}, {}); Ok({result})",
        trait_path(method.via.as_deref().unwrap_or(method.owner)),
        method.name,
        call_args.join(", ")
    );
    Ok((params, results_type(types, method, &sig, &mut_locals), body))
}

fn store_imports(types: &Types<'_>, methods: &[&Method], wit_text: &str) -> Result<String, String> {
    let mut registrations = std::collections::BTreeMap::new();
    for method in methods.iter().filter(|m| {
        (!exports().contains(&m.owner) || super::held(m.owner)) && flat_method(types, m)
    }) {
        let held = exports().contains(&method.owner);
        let interface = if held {
            format!("held-{}", interface(method.owner))
        } else {
            interface(method.owner)
        };
        let receiver = if held {
            format!(
                "&*super::held::<dyn {}>(handler)?",
                trait_path(method.owner)
            )
        } else {
            format!(
                "frame::{}{}()?",
                method.owner.rsplit("::").next().unwrap().to_snake_case(),
                if method.receiver == Some(true) {
                    "_mut"
                } else {
                    ""
                }
            )
        };
        let object = |name: &str, mutable: bool| {
            format!(
                "frame::{}{}()?",
                name.rsplit("::").next().unwrap().to_snake_case(),
                if mutable { "_mut" } else { "" }
            )
        };
        let place = InPlace::new(types, &object);
        let mut params = Vec::new();
        let mut args = Vec::new();
        let mut scratch = Vec::new();
        let mut prep = String::from("let keep = super::Keep::default();");
        if held {
            params.push("handler: u32".to_string());
        }
        for (name, ty) in &method.params {
            let wire = wit::wit(types, ty)?;
            let source = if wire.is_some() {
                bound_ident(&wit_ident(name))
            } else {
                "()".into()
            };
            if let Ty::List(item, List::Slice) = ty
                && flat_element(types, item, &mut Default::default())
            {
                params.push(format!(
                    "{source}: wasmtime::component::WasmList<{}>",
                    wire_type(types, item)
                ));
                let var = format!("b{}", scratch.len());
                scratch.push((
                    var.clone(),
                    format!("super::FlatInput<{}>", rust_ty(types, item, false)),
                ));
                writeln!(prep, "let {var} = super::FlatInputGuard(&mut {var}.0); {var}.0.clear(); for value in {source}.iter(wasmtime::AsContextMut::as_context_mut(&mut store))? {{ let value = value?; {var}.0.push({}); }}", lift(types, item, "value")).unwrap();
                args.push(format!("{var}.0.as_slice()"));
            } else {
                if wire.is_some() {
                    params.push(format!("{source}: {}", wire_type(types, ty)));
                }
                args.push(place.value(ty, &source)?);
            }
        }
        let mut output = lower_owned(types, &method.ret, "r");
        if let Ty::List(item, List::Vec) = &method.ret
            && flat_element(types, item, &mut Default::default())
        {
            let var = format!("b{}", scratch.len());
            scratch.push((
                var.clone(),
                format!("std::sync::Arc<Vec<{}>>", wire_type(types, item)),
            ));
            output = format!(
                "{{ let values = std::sync::Arc::get_mut({var}).unwrap(); values.clear(); for value in &r {{ values.push({}); }} {var}.clone() }}",
                lower(types, item, "value")
            );
        }
        let mut body = format!(
            "{prep} let r = <_ as {}>::{}({receiver}, {}); Ok(({output},))",
            trait_path(method.via.as_deref().unwrap_or(method.owner)),
            method.name,
            args.join(", ")
        );
        for (var, ty) in scratch.iter().rev() {
            body = format!(
                "super::State::with_buffer::<{ty}, _>(store, |{var}, mut store| {{ {body} }})"
            );
        }
        let (names, types): (Vec<_>, Vec<_>) =
            params.iter().map(|p| p.split_once(": ").unwrap()).unzip();
        let pattern = if names.is_empty() {
            "()".to_string()
        } else {
            format!("({},)", names.join(", "))
        };
        let param_types = if types.is_empty() {
            "()".to_string()
        } else {
            format!("({},)", types.join(", "))
        };
        let key = (interface, wit_ident(&method.name));
        if registrations
            .insert(
                key.clone(),
                crate::wasm_imports::Registration {
                    parameters: format!("{pattern}: {param_types}"),
                    body,
                    shadow: true,
                },
            )
            .is_some()
        {
            return Err(format!("duplicate flat-list import {key:?}"));
        }
    }
    for method in methods.iter().filter(|m| {
        (!exports().contains(&m.owner) || super::held(m.owner))
            && !flat_method(types, m)
            && m.params.iter().any(|(_, ty)| matches!(ty, Ty::Handler(name, _) if super::CALLBACKS.contains(&name.as_str())))
    }) {
        let held = exports().contains(&method.owner);
        let receiver = if held {
            format!(
                "&*super::held::<dyn {}>(handler)?",
                trait_path(method.owner)
            )
        } else {
            format!(
                "frame::{}{}()?",
                method.owner.rsplit("::").next().unwrap().to_snake_case(),
                if method.receiver == Some(true) {
                    "_mut"
                } else {
                    ""
                }
            )
        };
        let (params, _, body) = host_import_body(types, method, &receiver, held)?;
        let (names, wire_types): (Vec<_>, Vec<_>) =
            params.iter().map(|p| p.split_once(": ").unwrap()).unzip();
        let tuple = |items: Vec<&str>| {
            if items.is_empty() {
                "()".into()
            } else {
                format!("({},)", items.join(", "))
            }
        };
        let result = if signature(types, method)?.results.is_empty() {
            "()"
        } else {
            "(r,)"
        };
        let interface = if held {
            format!("held-{}", interface(method.owner))
        } else {
            interface(method.owner)
        };
        registrations.insert((interface, wit_ident(&method.name)), crate::wasm_imports::Registration {
            parameters: format!("{}: {}", tuple(names), tuple(wire_types)),
            body: format!("super::with_callback_store(store, || {{ let r = (|| -> wasmtime::Result<_> {{ {body} }})()?; Ok({result}) }})"),
            shadow: true,
        });
    }
    let mut resolve = wit_parser::Resolve::default();
    let package = resolve
        .push_str("mod-api.wit", wit_text)
        .map_err(|e| e.to_string())?;
    let world = resolve
        .select_world(&[package], Some("mod"))
        .map_err(|e| e.to_string())?;
    let allowed: std::collections::BTreeSet<String> = resolve.packages[package]
        .interfaces
        .keys()
        .cloned()
        .collect();
    let emitted = crate::wasm_imports::emit(
        &resolve,
        world,
        |name| {
            let short = name
                .split('/')
                .next_back()
                .unwrap()
                .split('@')
                .next()
                .unwrap();
            allowed
                .contains(short)
                .then_some(())
                .ok_or_else(|| format!("unhandled {name}"))
        },
        |interface, function| {
            let short = interface
                .split('/')
                .next_back()
                .unwrap()
                .split('@')
                .next()
                .unwrap();
            Ok(registrations.remove(&(short.to_string(), function.name.clone())))
        },
    )?;
    if !registrations.is_empty() {
        return Err(format!(
            "flat-list imports missing from WIT: {:?}",
            registrations.keys().collect::<Vec<_>>()
        ));
    }
    Ok(format!(
        "pub(super) fn add_store_imports(linker: &mut wasmtime::component::Linker<super::State>) -> wasmtime::Result<()> {{ {emitted} Ok(()) }}\n"
    ))
}

/// The Rust type of an import's results.
fn results_type(
    types: &Types<'_>,
    method: &Method,
    sig: &Signature,
    outs: &[(usize, String, Ty)],
) -> String {
    let mut parts = Vec::new();
    if wit::wit(types, &method.ret).ok().flatten().is_some() {
        parts.push(wire_type(types, &method.ret));
    }
    for (_, _, ty) in outs {
        match ty {
            Ty::Named(def) if sig.results.contains(&wit::out_name(&types.defs[*def])) => parts
                .push(format!(
                    "horizon::mod_api::types::{}",
                    wit::out_name(&types.defs[*def]).to_upper_camel_case()
                )),
            Ty::Named(def) if matches!(types.defs[*def].kind, Kind::Record(_)) => {}
            ty => parts.push(wire_type(types, ty)),
        }
    }
    match parts.len() {
        0 => "()".into(),
        1 => parts.remove(0),
        _ => format!("({})", parts.join(", ")),
    }
}

/// Whether `ty`'s wire value is a string or list, which the bindings borrow as a slice.
fn wire_is_list(types: &Types<'_>, ty: &Ty) -> bool {
    match ty {
        Ty::Str(Str::Static) => false,
        Ty::Array(item, n) if (1..=12).contains(n) => heap(types, item),
        Ty::Str(_) | Ty::List(..) | Ty::Array(..) => true,
        Ty::Named(def) => {
            matches!(&types.defs[*def].kind, Kind::Chain { .. })
                || matches!(&types.defs[*def].kind, Kind::Via(form) if wire_is_list(types, form))
        }
        _ => false,
    }
}

/// The form the bindings take an argument in: borrowed where it holds a string or list.
fn arg(types: &Types<'_>, ty: &Ty, e: &str) -> String {
    match ty {
        Ty::Str(Str::Static) => e.into(),
        Ty::Array(item, n) if (1..=12).contains(n) => format!(
            "({},)",
            (0..*n)
                .map(|i| arg(types, item, &format!("{e}.{i}")))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Ty::Str(_) | Ty::List(..) | Ty::Array(..) => format!("&{e}"),
        Ty::Named(_) if wire_is_list(types, ty) => format!("&{e}"),
        Ty::Option(item) => match &**item {
            Ty::Str(Str::Static) => e.into(),
            Ty::Array(item, n) if (1..=12).contains(n) => format!(
                "{e}.as_ref().map(|v| ({},))",
                (0..*n)
                    .map(|i| arg(types, item, &format!("v.{i}")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Ty::Str(_) | Ty::List(..) | Ty::Array(..) => format!("{e}.as_deref()"),
            Ty::Ref(inner, _) | Ty::Boxed(inner) | Ty::Shared(inner) => {
                arg(types, &Ty::Option(inner.clone()), e)
            }
            item if wire_is_list(types, item) => {
                format!("{e}.as_deref()")
            }
            item if heap(types, item) => format!("{e}.as_ref()"),
            _ => e.into(),
        },
        Ty::Ref(inner, _) | Ty::Boxed(inner) | Ty::Shared(inner) => arg(types, inner, e),
        Ty::Tuple(items) if items.iter().any(|i| heap(types, i)) => {
            let crossing: Vec<&Ty> = items
                .iter()
                .filter(|i| wit::wit(types, i).ok().flatten().is_some())
                .collect();
            let parts: Vec<String> = crossing
                .iter()
                .enumerate()
                .map(|(i, item)| arg(types, item, &format!("{e}.{i}")))
                .collect();
            format!("({},)", parts.join(", "))
        }
        Ty::Result(..) if heap(types, ty) => format!("{e}.as_ref()"),
        ty if heap(types, ty) => format!("&{e}"),
        _ => e.into(),
    }
}

/// Whether a type's wire value holds a string or list.
fn heap(types: &Types<'_>, ty: &Ty) -> bool {
    match ty {
        Ty::Str(Str::Static) => false,
        Ty::Array(item, n) if (1..=12).contains(n) => heap(types, item),
        Ty::Str(_) | Ty::List(..) | Ty::Array(..) | Ty::Handler(..) => true,
        Ty::Option(item) | Ty::Ref(item, _) | Ty::Boxed(item) | Ty::Shared(item) => {
            heap(types, item)
        }
        Ty::Result(a, b) => heap(types, a) || heap(types, b),
        Ty::Tuple(items) => items.iter().any(|i| heap(types, i)),
        Ty::Named(def) => {
            let def = &types.defs[*def];
            match &def.kind {
                super::ty::Kind::Json => true,
                _ => wit::def_fields(def).into_iter().any(|t| heap(types, t)),
            }
        }
        _ => false,
    }
}

/// The call's frame: a slot for each imported trait's object, set by the export that hands it
/// over, and the getters the imports reach it by. A trait is served by its own slot, else by a
/// subtrait's (`servers`), and mutably only by a slot set mutably; a trait no call hands over is
/// reached through another's method (`REACHED`).
fn frame(servers: &[(&str, Vec<&str>)], methods: &[&Method]) -> String {
    let snake = |owner: &str| owner.rsplit("::").next().unwrap().to_snake_case();
    let path = |owner: &str| format!("dyn crate::{owner}");
    let mut fields = String::new();
    let mut setters = String::new();
    let mut getters = String::new();
    for (owner, list) in servers {
        let name = snake(owner);
        let ty = path(owner);
        if let Some((_, from, expr)) = super::REACHED.iter().find(|(o, ..)| o == owner) {
            // An empty source reaches through the frame expression. Its trait's
            // receivers determine whether the resource needs an exclusive borrow.
            let mutable = from.ends_with("_mut")
                || (from.is_empty()
                    && methods
                        .iter()
                        .any(|method| method.owner == *owner && method.receiver == Some(true)));
            let body = match *from {
                "" => expr.to_string(),
                from => format!("let h = {from}()?; {expr}"),
            };
            writeln!(
                getters,
                "pub(crate) fn {name}{m}() -> wasmtime::Result<&'static {mm}{ty}> {{ {body} }}",
                m = if mutable { "_mut" } else { "" },
                mm = if mutable { "mut " } else { "" },
            )
            .unwrap();
            continue;
        }
        writeln!(fields, "{name}: Option<(*mut ({ty} + 'static), bool)>,").unwrap();
        // SAFETY (setters): only the lifetime is erased, and a shared borrow is never written
        // through (`mutable` false); the getters hand the pointer out only while the call that
        // set it runs.
        writeln!(
            setters,
            "#[allow(clippy::transmute_ptr_to_ptr)] pub(crate) fn {name}(&mut self, v: &({ty} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute::<*const ({ty} + '_), *mut ({ty} + 'static)>(v) }}, false)); }}\n#[allow(clippy::transmute_ptr_to_ptr)] pub(crate) fn {name}_mut(&mut self, v: &mut ({ty} + '_)) {{ self.{name} = Some((unsafe {{ std::mem::transmute::<*mut ({ty} + '_), *mut ({ty} + 'static)>(v) }}, true)); }}"
        )
        .unwrap();
        let shared: Vec<String> = list
            .iter()
            .filter(|s| !super::REACHED.iter().any(|(o, ..)| o == *s))
            .map(|s| format!("f.{}.map(|(p, _)| unsafe {{ &*p as &{ty} }})", snake(s)))
            .collect();
        let exclusive: Vec<String> = list
            .iter()
            .filter(|s| !super::REACHED.iter().any(|(o, ..)| o == *s))
            .map(|s| {
                format!(
                    "f.{}.filter(|(_, m)| *m).map(|(p, _)| unsafe {{ &mut *p as &mut {ty} }})",
                    snake(s)
                )
            })
            .collect();
        let what = name.replace('_', " ");
        writeln!(
            getters,
            "pub(crate) fn {name}() -> wasmtime::Result<&'static {ty}> {{ let f = current()?; {}.ok_or_else(|| missing(\"{what}\")) }}\npub(crate) fn {name}_mut() -> wasmtime::Result<&'static mut {ty}> {{ let f = current()?; {}.ok_or_else(|| missing(\"mutable {what}\")) }}",
            chain(&shared),
            chain(&exclusive)
        )
        .unwrap();
    }
    format!(
        "/// The objects the call in progress handed over, by imported trait.\n#[derive(Default)]\npub(crate) struct Frame {{ {fields} }}\nimpl Frame {{ {setters} }}\n/// The call's objects, as the imports ask for them; one the call didn't hand over is a typed trap.\npub(crate) mod frame {{ #![allow(clippy::all, dead_code)] use super::super::{{current, missing}};\n{getters} }}\n"
    )
}

/// The first of `options` that is some: `a.or_else(|| b.or_else(|| c))`.
fn chain(options: &[String]) -> String {
    let (last, rest) = options.split_last().expect("a trait serves itself");
    rest.iter().rev().fold(last.clone(), |inner, option| {
        format!("{option}.or_else(|| {inner})")
    })
}

/// Statements putting the trait objects a parameter holds into the frame.
fn frame_objects(types: &Types<'_>, ty: &Ty, e: &str, out: &mut String) {
    match ty {
        Ty::Dyn(name, mutable) => {
            writeln!(
                out,
                "frame.{}{}({e});",
                name.to_snake_case(),
                if *mutable { "_mut" } else { "" }
            )
            .unwrap();
        }
        Ty::Ref(inner, _) => {
            if let Ty::Named(def) = &**inner
                && let super::ty::Kind::Record(fields) = &types.defs[*def].kind
            {
                for field in fields {
                    if let Ty::Dyn(..) = field.ty {
                        frame_objects(types, &field.ty, &format!("&mut *{e}.{}", field.rust), out);
                    }
                }
            }
        }
        Ty::Named(def) => {
            if let super::ty::Kind::Variant(cases) = &types.defs[*def].kind {
                let path = rust_path(&types.defs[*def]);
                let path = path.trim_end_matches("<'_>");
                let mut arms = Vec::new();
                for case in cases {
                    if let super::ty::Payload::Tuple(items) = &case.payload
                        && let [Ty::Dyn(name, mutable)] = items.as_slice()
                    {
                        arms.push(format!(
                            "{path}::{}(h) => frame.{}{}({}**h)",
                            case.rust,
                            name.to_snake_case(),
                            if *mutable { "_mut" } else { "" },
                            if *mutable { "&mut " } else { "&" }
                        ));
                    }
                }
                if !arms.is_empty() {
                    writeln!(
                        out,
                        "let mut {e} = {e}; match &mut {e} {{ {} }}",
                        arms.join(", ")
                    )
                    .unwrap();
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "boundary_tests.rs"]
mod boundary_tests;
