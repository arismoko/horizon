//! The per-mod wasm view, generated from the same layouts and selection as the staged table.
use crate::crossing::{Collection, Crossing, Rule, Scalar, Shape, bare};
use crate::internals::Internal;
use crate::internals_table::{
    function_scope, guest_codecs, node_fields, optional_reference, result_success, select,
    signature,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn name(path: &str) -> syn::Ident {
    format_ident!("{}", bare(path).replace("::", "__").replace("r#", ""))
}

fn node_name(path: &str, rule: &Rule) -> syn::Ident {
    let id = name(path);
    if matches!(
        rule.crossing_of(&Shape::Named(path.into(), 0)),
        Crossing::Copy(_)
    ) {
        format_ident!("{id}Node")
    } else {
        id
    }
}
fn tuple_name(shape: &Shape) -> syn::Ident {
    format_ident!(
        "Tuple{}",
        &format!("{:x}", Sha256::digest(format!("{shape:?}")))[..16]
    )
}

fn short_alias_available(name: &str) -> bool {
    ![
        "I",
        "V",
        "GameError",
        "Node",
        "Reached",
        "Arg",
        "Value",
        "CopyValue",
        "Field",
        "get_many",
        "set_many",
        "ViewType",
        "AsNode",
        "Refusal",
        "Seq",
        "Map",
        "Opt",
        "DECLARED",
        "call",
        "root",
        "flatten",
        "unflatten",
        "changed",
        "wire_refusal",
        "root_as",
    ]
    .contains(&name)
}

fn field_name(field: &crate::crossing::Field<'_>) -> String {
    match field.case {
        Some((_, case)) => format!(
            "{}_{}",
            heck::AsSnakeCase(case.to_string()),
            field.member.to_token_stream()
        ),
        None => field.member.to_token_stream().to_string(),
    }
}

fn accessor_name(field: &str) -> String {
    if field.chars().all(|c| c.is_ascii_digit()) {
        format!("field_{field}")
    } else {
        field.into()
    }
}

/// Names depend on the complete catalog layout, never on a mod's selected fields.
pub(crate) fn method_name(rule: &Rule, owner: &str, func: &str) -> String {
    let named = rule.named(owner).unwrap();
    let reserved = |func: &str| {
        (func == "case" && matches!(named.item, syn::Item::Enum(_)))
            || rule.fields(owner).iter().any(|field| {
                let crossing = rule.crossing_of(&rule.shape(&named.module, Some(owner), field.ty));
                if crossing == Crossing::Opaque {
                    return false;
                }
                let field = field_name(field);
                let get = accessor_name(&field);
                let get = get.trim_start_matches("r#");
                func.trim_start_matches("r#") == get
                    || (matches!(crossing, Crossing::Copy(_))
                        && (func == format!("set_{get}")
                            || func == descriptor_name(&field, Some(owner), rule)))
            })
    };
    let mut name = func.to_owned();
    while reserved(&name) {
        name = format!("call_{}", name.trim_start_matches("r#"));
    }
    name
}

/// Owned guest spelling: native borrows and shared pointers are copies, not guest borrows.
pub(crate) fn spell(shape: &Shape, rule: &Rule) -> TokenStream {
    match shape {
        Shape::Named(path, _) => {
            let name = name(path);
            quote!(#name)
        }
        Shape::Scalar(s) => {
            let name = format_ident!(
                "{}",
                match s {
                    Scalar::Bool => "bool",
                    Scalar::Char => "char",
                    Scalar::Signed(n)
                    | Scalar::Unsigned(n)
                    | Scalar::Float(n)
                    | Scalar::Wide(n) => n,
                    Scalar::Str | Scalar::String => "String",
                }
            );
            quote!(#name)
        }
        Shape::Unit => quote!(()),
        Shape::Ref { to, .. } | Shape::Wrapped(_, to) => {
            let ty = spell(to, rule);
            if rule.crossing_of(shape) == Crossing::Handle {
                ty
            } else {
                quote!(Box<#ty>)
            }
        }
        Shape::Tuple(_) if rule.crossing_of(shape) == Crossing::Handle => {
            let ty = tuple_name(shape);
            quote!(#ty)
        }
        Shape::Tuple(parts) => {
            let parts = parts.iter().map(|p| spell(p, rule));
            quote!((#(#parts,)*))
        }
        Shape::Array(elem, n) if matches!(rule.crossing_of(shape), Crossing::Copy(_)) => {
            let ty = spell(elem, rule);
            quote!([#ty; #n])
        }
        Shape::Array(elem, _) | Shape::Slice(elem) => {
            let ty = spell(elem, rule);
            quote!(Seq<#ty>)
        }
        Shape::Option(elem) => {
            let ty = spell(elem, rule);
            if matches!(rule.crossing_of(shape), Crossing::Copy(_)) {
                quote!(Option<#ty>)
            } else {
                quote!(Opt<#ty>)
            }
        }
        Shape::Collection(kind, args) => {
            let first = spell(&args[0], rule);
            if matches!(kind, Collection::HashMap | Collection::BTreeMap) {
                let second = spell(&args[1], rule);
                quote!(Map<#first, #second>)
            } else {
                quote!(Seq<#first>)
            }
        }
        Shape::Opaque(_) => unreachable!("selection excludes opaque shapes"),
    }
}

/// An owned catalog boundary. Static tags in `Internal` are selected here, never leaked strings.
pub fn catalog(text: &str) -> Result<Vec<Internal>, String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        path: String,
        kind: String,
        signature: String,
        hash: String,
        #[serde(default)]
        cfg: String,
        #[serde(default)]
        shape_only: bool,
        #[serde(default)]
        opaque_reason: Option<String>,
        #[serde(default)]
        refs: BTreeMap<String, String>,
    }
    let entries: Vec<Entry> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    entries
        .into_iter()
        .map(|e| {
            Ok(Internal {
                path: e.path,
                kind: match e.kind.as_str() {
                    "struct" => "struct",
                    "enum" => "enum",
                    "field" => "field",
                    "method" => "method",
                    "fn" => "fn",
                    "const" => "const",
                    "static" => "static",
                    "type" => "type",
                    "trait" => "trait",
                    "mod" => "mod",
                    _ => return Err(format!("unknown internal kind {}", e.kind)),
                },
                signature: e.signature,
                hash: e.hash,
                cfg: e.cfg,
                shape_only: e.shape_only,
                opaque_reason: e.opaque_reason.map(|_| "opaque layout"),
                refs: e.refs,
            })
        })
        .collect()
}

/// A manifest internal, fingerprinted by `internals add`.
pub struct Declaration {
    pub path: String,
    pub hash: String,
    pub kind: Option<String>,
}
/// One raw listener, in manifest dispatch order.
pub struct Injection {
    pub target: String,
    pub at_return: bool,
    pub listener: String,
}

/// The generated view and raw-listener export macro. Nothing per game type is handwritten.
pub fn generate(
    internals: &[Internal],
    declared: &[Declaration],
    injections: &[Injection],
    points: &[crate::points::GamePoint],
) -> Result<String, String> {
    let rule = Rule::new(internals);
    for d in declared {
        let candidates: Vec<_> = internals
            .iter()
            .filter(|i| {
                i.path == d.path
                    && d.kind.as_deref().is_none_or(|k| k == i.kind)
                    && !i.shape_only
                    && i.cfg.is_empty()
            })
            .collect();
        if candidates.is_empty() {
            return Err(format!("{}: missing or conditional internal", d.path));
        }
        if !candidates.iter().any(|i| i.hash == d.hash) {
            return Err(format!(
                "{}: changed internal; refresh its declaration",
                d.path
            ));
        }
    }
    let paths: BTreeSet<_> = declared
        .iter()
        .map(|d| d.path.as_str())
        .chain(injections.iter().map(|i| i.target.as_str()))
        .collect();
    let hashes = internals
        .iter()
        .filter(|i| matches!(i.kind, "struct" | "enum") && i.cfg.is_empty())
        .map(|i| (bare(&i.path), (i.path.as_str(), i.hash.as_str())))
        .collect();
    let selected = select(internals, paths, points, &rule, &hashes);
    if !selected.counts.refused.is_empty() {
        return Err(format!(
            "internals cannot cross to wasm: {:?}",
            selected.counts.refused
        ));
    }
    for d in declared {
        let entry = internals
            .iter()
            .find(|i| i.path == d.path && i.hash == d.hash)
            .expect("validated declaration");
        let bound = match entry.kind {
            "fn" | "method" => selected
                .accepted
                .iter()
                .any(|i| i.path == d.path && i.hash == d.hash),
            "field" => selected
                .members
                .iter()
                .any(|(i, _, _)| i.path == d.path && i.hash == d.hash),
            "struct" | "enum" => true,
            _ => false,
        };
        if !bound {
            return Err(format!(
                "{}: this declared {} cannot cross to wasm",
                d.path, entry.kind
            ));
        }
    }
    let mut walked: BTreeSet<_> = selected.selection.full.iter().cloned().collect();
    walked.extend(selected.selection.owners.keys().cloned());
    let mut listed: Vec<_> = declared
        .iter()
        .map(|d| (d.path.clone(), d.hash.clone()))
        .collect();
    for path in &walked {
        let (path, hash) = hashes
            .get(path)
            .ok_or_else(|| format!("{path}: no layout hash"))?;
        if !listed.iter().any(|(p, _)| p == path) {
            listed.push((path.to_string(), hash.to_string()));
        }
    }
    let mut items = Vec::new();
    let mut copies = BTreeMap::new();
    fn copy_shapes(shape: &Shape, rule: &Rule, copies: &mut BTreeMap<String, Shape>) {
        if rule.crossing_of(shape) == Crossing::Opaque {
            return;
        }
        // Handle wrappers can have the same guest spelling as their tuple referent. Keep
        // the tuple itself so its node wrapper is emitted, rather than an Arc/Ref shadow.
        if matches!(rule.crossing_of(shape), Crossing::Copy(_)) || matches!(shape, Shape::Tuple(_))
        {
            copies
                .entry(spell(shape, rule).to_string())
                .or_insert_with(|| shape.clone());
        }
        match shape {
            Shape::Named(path, _) if matches!(rule.crossing_of(shape), Crossing::Copy(_)) => {
                let named = rule.named(path).unwrap();
                for f in rule.fields(path) {
                    copy_shapes(&rule.shape(&named.module, Some(path), f.ty), rule, copies);
                }
            }
            Shape::Ref { to, .. }
            | Shape::Wrapped(_, to)
            | Shape::Option(to)
            | Shape::Array(to, _)
            | Shape::Slice(to) => copy_shapes(to, rule, copies),
            Shape::Tuple(parts) | Shape::Collection(_, parts) => {
                for p in parts {
                    copy_shapes(p, rule, copies);
                }
            }
            _ => {}
        }
    }
    let mut aliases = BTreeMap::<String, Vec<syn::Ident>>::new();
    for path in &walked {
        let named = rule.named(path).unwrap();
        let ty = name(path);
        aliases
            .entry(path.rsplit("::").next().unwrap().into())
            .or_default()
            .push(ty.clone());
        let shape = Shape::Named(path.clone(), named.lifetimes);
        if matches!(rule.crossing_of(&shape), Crossing::Copy(_)) {
            copy_shapes(&shape, &rule, &mut copies);
            let fields = |fields: &syn::Fields| -> TokenStream {
                let defs = fields.iter().enumerate().map(|(index, f)| {
                    let member = f
                        .ident
                        .clone()
                        .map(syn::Member::Named)
                        .unwrap_or_else(|| syn::Member::Unnamed(index.into()));
                    let t = spell(&rule.shape(&named.module, Some(path), &f.ty), &rule);
                    quote!(pub #member: #t)
                });
                // All copies use named members, including numeric tuple members in codecs.
                match fields {
                    syn::Fields::Named(_) => quote!({#(#defs),*}),
                    syn::Fields::Unnamed(fs) => {
                        let ts = fs
                            .unnamed
                            .iter()
                            .map(|f| spell(&rule.shape(&named.module, Some(path), &f.ty), &rule));
                        quote!((#(pub #ts),*))
                    }
                    syn::Fields::Unit => quote!(;),
                }
            };
            match &named.item {
                syn::Item::Struct(s) => {
                    let fs = fields(&s.fields);
                    let semi = matches!(s.fields, syn::Fields::Unnamed(_)).then(|| quote!(;));
                    items.push(quote!(#[derive(Clone, Debug, PartialEq)] pub struct #ty #fs #semi));
                }
                syn::Item::Enum(e) => {
                    let variants = e.variants.iter().map(|v| {
                        let id = &v.ident;
                        let fs = match &v.fields {
                            syn::Fields::Named(fs) => {
                                let fs = fs.named.iter().map(|f| {
                                    let id = &f.ident;
                                    let t =
                                        spell(&rule.shape(&named.module, Some(path), &f.ty), &rule);
                                    quote!(#id: #t)
                                });
                                quote!({#(#fs),*})
                            }
                            syn::Fields::Unnamed(fs) => {
                                let fs = fs.unnamed.iter().map(|f| {
                                    spell(&rule.shape(&named.module, Some(path), &f.ty), &rule)
                                });
                                quote!((#(#fs),*))
                            }
                            syn::Fields::Unit => quote!(),
                        };
                        quote!(#id #fs)
                    });
                    items.push(
                        quote!(#[derive(Clone, Debug, PartialEq)] pub enum #ty {#(#variants),*}),
                    );
                }
                _ => unreachable!(),
            }
        }
        {
            let node_ty = node_name(path, &rule);
            if node_ty != ty {
                aliases
                    .entry(format!("{}Node", path.rsplit("::").next().unwrap()))
                    .or_default()
                    .push(node_ty.clone());
            }
            let mut methods = Vec::new();
            for (index, f) in node_fields(&rule, path).into_iter().enumerate() {
                if !selected.selection.full.contains(path)
                    && !selected
                        .selection
                        .owners
                        .get(path)
                        .is_some_and(|fs| fs.contains(&index))
                {
                    continue;
                }
                let shape = rule.shape(&named.module, Some(path), f.ty);
                if rule.crossing_of(&shape) == Crossing::Opaque {
                    continue;
                }
                copy_shapes(&shape, &rule, &mut copies);
                let field = field_name(&f);
                methods.push(accessor(&field, index as u32, &shape, &rule, Some(path)));
            }
            if matches!(named.item, syn::Item::Enum(_)) {
                methods.push(quote!(
                    pub fn case(&self) -> Result<u32, Refusal> {
                        self.0.case()
                    }
                ));
            }
            items.push(handle(&node_ty, methods));
        }
    }
    for (alias, types) in aliases {
        if types.len() == 1 && short_alias_available(&alias) {
            let alias = format_ident!("{alias}");
            let ty = &types[0];
            items.push(quote!(pub use #ty as #alias;));
        }
    }
    let mut free_aliases = BTreeMap::<String, Vec<syn::Ident>>::new();
    let mut has_game_error = false;
    for (index, d) in declared.iter().enumerate() {
        if let Some(i) = selected
            .accepted
            .iter()
            .find(|i| i.path == d.path && d.kind.as_deref().is_none_or(|k| k == i.kind))
        {
            let sig = signature(&i.signature).expect("selection validated");
            let path = bare(&i.path);
            let (_, func) = path.rsplit_once("::").unwrap();
            let (module, this) = function_scope(&rule, i).expect("validated function scope");
            let (module, this) = (module.as_str(), this.as_deref());
            let mut args = Vec::new();
            let mut params = Vec::new();
            let mut updates = Vec::new();
            let mut decode_updates = Vec::new();
            let mut write_back = Vec::new();
            for (n, arg) in sig
                .inputs
                .iter()
                .filter_map(|a| match a {
                    syn::FnArg::Typed(a) => Some(a),
                    _ => None,
                })
                .enumerate()
            {
                let a = format_ident!("a{n}");
                let shape = rule.shape(module, this, &arg.ty);
                if let Shape::Ref { mutable: true, to } = &shape
                    && matches!(rule.crossing_of(to), Crossing::Copy(_))
                {
                    copy_shapes(to, &rule, &mut copies);
                    let ty = spell(to, &rule);
                    let update = format_ident!("update_{n}");
                    params.push(quote!(#a: &mut #ty));
                    args.push(quote!(Arg::Value(flatten(#a.to_value()))));
                    updates.push(update.clone());
                    decode_updates.push(quote!(let #update = <#ty as CopyValue>::from_value(unflatten(#update).map_err(wire_refusal)?)?;));
                    write_back.push(quote!(*#a = #update;));
                } else if rule.crossing_of(&shape) == Crossing::Handle {
                    let t = if let Shape::Ref { to, .. } = &shape {
                        to.as_ref()
                    } else {
                        &shape
                    };
                    let ty = spell(t, &rule);
                    params.push(quote!(#a: &#ty));
                    args.push(quote!(Arg::Node(#a.node())));
                } else {
                    // An immutable borrowed copy crosses as a value, without its borrow layer.
                    let shape = if let Shape::Ref { to, .. } = shape {
                        *to
                    } else {
                        shape
                    };
                    copy_shapes(&shape, &rule, &mut copies);
                    let ty = spell(&shape, &rule);
                    params.push(quote!(#a: #ty));
                    args.push(quote!(Arg::Value(flatten(#a.to_value()))));
                }
            }
            let success = match &sig.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, t) => result_success(&rule, module, this, t),
            };
            let optional = match &sig.output {
                syn::ReturnType::Default => None,
                syn::ReturnType::Type(_, t) => optional_reference(&rule, module, this, t),
            };
            let result =
                optional
                    .clone()
                    .or(success.clone())
                    .unwrap_or_else(|| match &sig.output {
                        syn::ReturnType::Default => Shape::Unit,
                        syn::ReturnType::Type(_, t) => rule.shape(module, this, t),
                    });
            copy_shapes(&result, &rule, &mut copies);
            let result = spell(&result, &rule);
            let (result, decode) = if let Some(Shape::Ref { to, .. }) = optional {
                let ty = spell(&to, &rule);
                (
                    quote!(Option<#ty>),
                    quote! {
                        match answer {
                            Reached::Node(node) => Ok(Some(<#ty as ViewType>::from_reached(Reached::Node(node))?)),
                            Reached::Value(cells) => match unflatten(cells).map_err(wire_refusal)? {
                                Value::Var(0, fields) if fields.is_empty() => Ok(None),
                                _ => Err(changed("an optional reference")),
                            },
                        }
                    },
                )
            } else if success.is_some() {
                has_game_error = true;
                (
                    quote!(Result<#result, GameError>),
                    quote! {
                        let Reached::Value(cells) = answer else { return Err(changed("a result value")); };
                        match unflatten(cells).map_err(wire_refusal)? {
                            Value::Var(0, mut fields) if fields.len() == 1 =>
                                Ok(Ok(<#result as CopyValue>::from_value(fields.remove(0))?)),
                            Value::Var(1, fields) if fields.is_empty() => Ok(Err(GameError)),
                            _ => Err(changed("a result case")),
                        }
                    },
                )
            } else {
                (
                    result.clone(),
                    quote!(<#result as ViewType>::from_reached(answer)),
                )
            };
            let func = if let Some(this) = this {
                format_ident!("{}", method_name(&rule, this, func))
            } else {
                let id = name(&path);
                free_aliases
                    .entry(func.into())
                    .or_default()
                    .push(id.clone());
                id
            };
            let index = index as u32;
            let receiver = if sig.receiver().is_some() {
                quote!(Some(self.node()))
            } else {
                quote!(None)
            };
            let self_param = sig.receiver().map(|_| quote!(&self,));
            let f = quote!(pub fn #func(#self_param #(#params),*) -> Result<#result, Refusal> {
                let reply = call(#index, #receiver, &[#(#args),*])?;
                let answer = reply.result;
                let result: #result = (|| { #decode })()?;
                let [#(#updates),*] = reply.updates.try_into().map_err(|_| changed("mutable copy outputs"))?;
                #(#decode_updates)*
                #(#write_back)*
                Ok(result)
            });
            if let Some(this) = this {
                let ty = node_name(this, &rule);
                items.push(quote!(impl #ty { #f }));
            } else {
                items.push(f);
            }
        }
    }
    if has_game_error {
        items.push(quote! {
            /// A declared game function failed; its opaque native error stays in the game.
            #[derive(Debug, Clone, Copy, PartialEq, Eq)]
            pub struct GameError;
        });
    }
    for (alias, functions) in free_aliases {
        if functions.len() == 1 && short_alias_available(&alias) {
            let alias = format_ident!("{alias}");
            let function = &functions[0];
            items.push(quote!(pub use #function as #alias;));
        }
    }
    let mut subscriptions = Vec::new();
    let mut calls = Vec::new();
    for (index, inj) in injections.iter().enumerate() {
        let (internal, point) = selected
            .raw
            .iter()
            .find(|(i, _)| i.path == inj.target)
            .ok_or_else(|| format!("{}: not a crossable raw point", inj.target))?;
        let sig = signature(&internal.signature).unwrap();
        let shape =
            point_shape::shape(quote!(name = "unused", raw), &sig).map_err(|e| e.to_string())?;
        let (_, owner) = function_scope(&rule, internal).ok_or("raw receiver did not resolve")?;
        let module = point
            .listener
            .as_ref()
            .unwrap()
            .rsplit_once("::")
            .unwrap()
            .0;
        let mut fs: Vec<_> = point_shape::raw_fields(&sig)
            .into_iter()
            .map(|f| {
                let ty = if shape.args.iter().any(|a| a.name == f.name)
                    && matches!(f.ty, syn::Type::Reference(_))
                {
                    syn::parse_quote!(::std::borrow::Cow<'static, str>)
                } else {
                    f.ty
                };
                (
                    f.name.to_string(),
                    rule.shape(module, owner.as_deref(), &ty),
                )
            })
            .collect();
        if inj.at_return {
            let result = match &sig.output {
                syn::ReturnType::Default => Shape::Unit,
                syn::ReturnType::Type(_, t) => rule.shape(module, owner.as_deref(), t),
            };
            fs = vec![("result".into(), result)];
        } else {
            match shape.raw_outcome(&sig) {
                point_shape::RawOutcome::Cancel => {
                    fs.push(("cancel".into(), Shape::Scalar(Scalar::Bool)))
                }
                point_shape::RawOutcome::Result(ty) => fs.push((
                    "result".into(),
                    Shape::Option(Box::new(rule.shape(module, owner.as_deref(), &ty))),
                )),
                point_shape::RawOutcome::None => {}
            }
        }
        let ty = format_ident!(
            "{}{}Raw{}",
            heck::AsUpperCamelCase(&point.name).to_string(),
            if inj.at_return { "Return" } else { "" },
            index
        );
        let mut methods = Vec::new();
        for (n, (field, s)) in fs.iter().enumerate() {
            if rule.crossing_of(s) == Crossing::Opaque {
                continue;
            }
            copy_shapes(s, &rule, &mut copies);
            methods.push(accessor(field, n as u32, s, &rule, None));
        }
        items.push(handle(&ty, methods));
        let alias = format_ident!(
            "{}{}Raw",
            heck::AsUpperCamelCase(&point.name).to_string(),
            if inj.at_return { "Return" } else { "" }
        );
        if !injections[..index]
            .iter()
            .any(|i| i.target == inj.target && i.at_return == inj.at_return)
        {
            items.push(quote!(pub use #ty as #alias;));
        }
        let method = if inj.at_return {
            format!("{}_return", point.name)
        } else {
            point.name.clone()
        };
        let hash = &point.wire_shape;
        subscriptions.push(quote!((#method.into(), #hash.into())));
        let has_receiver = sig.receiver().is_some() && !(inj.at_return && shape.borrowed_return);
        let root = u32::from(has_receiver);
        let recv = if has_receiver {
            let ty = node_name(owner.as_ref().unwrap(), &rule);
            quote!(Some($crate::runtime::root_as::<$crate::view::#ty>(0)))
        } else {
            quote!(None::<()>)
        };
        let idx = index as u32;
        let listener = format_ident!("{}", inj.listener);
        calls.push(quote!(#idx => crate::#listener($crate::runtime::root_as::<$crate::view::#ty>(#root), #recv),));
    }
    for shape in copies.values() {
        if let Shape::Tuple(parts) = shape
            && rule.crossing_of(shape) == Crossing::Handle
        {
            let ty = tuple_name(shape);
            let methods = parts
                .iter()
                .enumerate()
                .filter(|(_, p)| rule.crossing_of(p) != Crossing::Opaque)
                .map(|(i, p)| accessor(&i.to_string(), i as u32, p, &rule, None))
                .collect();
            items.push(handle(&ty, methods));
        }
    }
    let codecs = guest_codecs(
        &rule,
        &copies
            .into_values()
            .filter(|s| matches!(rule.crossing_of(s), Crossing::Copy(_)))
            .collect::<Vec<_>>(),
    );
    let declared = listed.iter().map(|(p, h)| quote!((#p, #h)));
    let code = quote! {
        #[cfg(target_arch = "wasm32")]
        #[allow(non_camel_case_types, non_snake_case, dead_code, unused_imports, unused_variables, unused_parens, unreachable_patterns, clippy::all)]
        pub mod view {
            use crate::runtime::*;
            pub use crate::runtime::{CopyValue, Refusal, Seq, Map, Opt, Field, get_many, set_many};
            use mod_points::internals as I;
            use I::Value as V;
            /// Manifest internals followed by every walked named layout and its hash.
            pub const DECLARED: &[(&str, &str)] = &[#(#declared),*];
            #(#items)*
            #codecs
        }
        #[macro_export]
        macro_rules! raw_listeners {
            () => {
                #[cfg(target_arch = "wasm32")]
                const _: () = {
                    use $crate::runtime::{Guest, HostKind};
                    struct RawListeners;
                    impl Guest for RawListeners {
                        fn declared() -> Vec<(String, String)> { $crate::view::DECLARED.iter().map(|(p,h)| ((*p).into(), (*h).into())).collect() }
                        fn subscribed() -> Vec<(String, String)> { vec![#(#subscriptions),*] }
                        fn fire(place: u32, _: HostKind) { match place { #(#calls)* _ => panic!("unknown raw listener") } }
                    }
                    $crate::runtime::bindings::export_raw_listeners!(RawListeners with_types_in $crate::runtime::bindings);
                };
            };
        }
    };
    Ok(code.to_string())
}

fn handle(ty: &syn::Ident, methods: Vec<TokenStream>) -> TokenStream {
    quote! {
        /// A call-scoped path into game state; every operation refuses stale nodes.
        pub struct #ty(Node);
        impl AsNode for #ty { fn node(&self) -> &Node { &self.0 } }
        impl ViewType for #ty { fn from_reached(reached: Reached) -> Result<Self, Refusal> { match reached { Reached::Node(n) => Ok(Self(n)), _ => Err(changed("a node")) } } }
        impl #ty {
            #(#methods)*
        }
    }
}
fn descriptor_name(field: &str, owner: Option<&str>, rule: &Rule) -> String {
    let mut name = format!("{}_field", accessor_name(field).trim_start_matches("r#"));
    if let Some(owner) = owner {
        let fields = rule.fields(owner);
        while fields.iter().any(|f| {
            let get = accessor_name(&field_name(f));
            name == get.trim_start_matches("r#")
                || name == format!("set_{}", get.trim_start_matches("r#"))
        }) {
            name = format!("call_{name}");
        }
    }
    name
}

fn accessor(
    field: &str,
    index: u32,
    shape: &Shape,
    rule: &Rule,
    owner: Option<&str>,
) -> TokenStream {
    let get = format_ident!("{}", accessor_name(field));
    let set = format_ident!("set_{}", get.to_string().trim_start_matches("r#"));
    let ty = spell(shape, rule);
    if matches!(rule.crossing_of(shape), Crossing::Copy(_)) {
        let descriptor = format_ident!("{}", descriptor_name(field, owner, rule));
        quote! {
            /// Select this field for a typed batched read or write.
            pub fn #descriptor(&self) -> Field<'_, #ty> { Field::new(&self.0, #index) }
            /// Read this declared field, including this listener's journaled writes.
            pub fn #get(&self) -> Result<#ty, Refusal> { <#ty as CopyValue>::from_value(unflatten(self.0.get(#index)?).map_err(wire_refusal)?) }
            /// Journal a write, applied on listener return or before a declared call.
            pub fn #set(&self, value: #ty) -> Result<(), Refusal> { self.0.set(#index, &flatten(value.to_value())) }
        }
    } else {
        quote!(pub fn #get(&self) -> Result<#ty, Refusal> { <#ty as ViewType>::from_reached(Reached::Node(self.0.field(#index)?)) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: &'static str, signature: &str) -> Internal {
        Internal {
            path: path.into(),
            kind,
            signature: signature.into(),
            hash: format!("hash-{path}"),
            cfg: String::new(),
            shape_only: false,
            opaque_reason: None,
            refs: BTreeMap::new(),
        }
    }

    fn declaration(i: &Internal) -> Declaration {
        Declaration {
            path: i.path.clone(),
            kind: Some(i.kind.into()),
            hash: i.hash.clone(),
        }
    }

    #[test]
    fn guest_copies_compile_and_round_trip_the_host_value_format() {
        let entries = [
            entry("horizon_client::Pair", "struct", "struct Pair(u8, i16);"),
            entry(
                "horizon_client::Choice",
                "enum",
                "enum Choice { Empty, Pair(u16, bool), Named { value: char } }",
            ),
            entry(
                "horizon_client::Borrowed",
                "struct",
                "struct Borrowed<'a> { text: &'a str, wide: u128 }",
            ),
            entry(
                "horizon_client::Data",
                "struct",
                "struct Data { pair: Pair, choice: Choice, array: [i32; 2], option: Option<f64>, tuple: (bool, String) }",
            ),
            entry(
                "horizon_client::Owner",
                "struct",
                "struct Owner { first: u8, second: i64, hidden: Vec<u8>, wrapped: Arc<(Vec<u8>, bool)>, mode: u8 }",
            ),
            entry("horizon_client::Owner::second", "field", "second: i64"),
            entry(
                "horizon_client::succeed",
                "fn",
                "fn succeed() -> Result<u32>",
            ),
            entry(
                "horizon_client::fail",
                "fn",
                "fn fail() -> std::result::Result<(), Box<dyn std::error::Error>>",
            ),
            entry(
                "horizon_client::find",
                "fn",
                "fn find() -> Option<&'static Owner>",
            ),
            entry(
                "horizon_client::missing",
                "fn",
                "fn missing() -> Option<&'static Owner>",
            ),
            entry(
                "horizon_client::Owner::set_mode",
                "method",
                "fn set_mode(&mut self, mode: u8) -> Result<u32>",
            ),
            entry(
                "horizon_client::mutate",
                "fn",
                "fn mutate(n: &mut u32, pair: &mut Pair) -> bool",
            ),
        ];
        let declared = [
            declaration(&entries[2]),
            declaration(&entries[3]),
            declaration(&entries[5]),
            declaration(&entries[6]),
            declaration(&entries[7]),
            declaration(&entries[8]),
            declaration(&entries[9]),
            declaration(&entries[10]),
            declaration(&entries[11]),
        ];
        let (_, counts) = crate::internals_table::table(&entries, &[entries[10].path.clone()]);
        assert_eq!(counts.renamed_methods[&entries[10].path], "call_set_mode");
        let code = generate(&entries, &declared, &[], &[])
            .unwrap()
            .replace("# [cfg (target_arch = \"wasm32\")]", "");
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let build = root
            .join("target/internals-view-test")
            .join(std::process::id().to_string());
        std::fs::create_dir_all(&build).unwrap();
        let internals = root
            .join("crates/horizon-mod-points/src/internals.rs")
            .canonicalize()
            .unwrap();
        let source = format!(
            r#"
            extern crate self as mod_points;
            #[path = {internals:?}] pub mod internals;
            mod runtime {{
                pub use crate::internals::{{Value, Refusal, changed}};
                pub trait CopyValue: Sized {{ fn to_value(&self) -> Value; fn from_value(value: Value) -> Result<Self, Refusal>; }}
                pub trait ViewType: Sized {{ fn from_reached(reached: Reached) -> Result<Self, Refusal>; }}
                pub trait AsNode {{ fn node(&self) -> &Node; }}
                pub enum Reached {{ Node(Node), Value(Vec<Value>) }}
                pub enum Arg<'a> {{ Node(&'a Node), Value(Vec<Value>) }}
                pub struct CallReply {{ pub result: Reached, pub updates: Vec<Vec<Value>> }}
                pub fn call(index: u32, _: Option<&Node>, args: &[Arg<'_>]) -> Result<CallReply, Refusal> {{
                    if index == 8 {{
                        assert!(matches!(&args[0], Arg::Value(v) if v == &vec![Value::U(7)]));
                        assert!(matches!(&args[1], Arg::Value(v) if v == &vec![Value::Rec(vec![Value::U(2), Value::S(-3)])]));
                        return Ok(CallReply {{ result: Reached::Value(vec![Value::B(true)]), updates: vec![vec![Value::U(12)], vec![Value::Rec(vec![Value::U(9), Value::S(-4)])]] }});
                    }}
                    if index == 5 {{ return Ok(CallReply {{ result: Reached::Node(Node), updates: vec![] }}); }}
                    Ok(CallReply {{ result: Reached::Value(vec![match index {{
                        3 | 7 => Value::Var(0, vec![Value::U(37)]),
                        6 => Value::Var(0, vec![]),
                        _ => Value::Var(1, vec![]),
                    }}]), updates: vec![] }})
                }}
                pub struct Seq<T>(T); pub struct Map<K,V>(K,V); pub struct Opt<T>(T);
                impl<T> ViewType for Seq<T> {{ fn from_reached(_: Reached) -> Result<Self, Refusal> {{ unimplemented!() }} }}
                pub struct Field<'a, T>(&'a Node, std::marker::PhantomData<T>);
                impl<'a, T> Field<'a, T> {{ pub fn new(n: &'a Node, _: u32) -> Self {{ Self(n, std::marker::PhantomData) }} }}
                pub fn get_many() {{}} pub fn set_many() {{}}
                pub struct Node;
                impl Node {{
                    pub fn get(&self, n: u32) -> Result<Vec<Value>, Refusal> {{ Ok(vec![Value::S(n as i64)]) }}
                    pub fn set(&self, _: u32, _: &[Value]) -> Result<(), Refusal> {{ Ok(()) }}
                    pub fn read(&self) -> Result<Vec<Value>, Refusal> {{ unimplemented!() }}
                    pub fn field(&self, _: u32) -> Result<Self, Refusal> {{ unimplemented!() }}
                    pub fn case(&self) -> Result<u32, Refusal> {{ unimplemented!() }}
                }}
                pub fn flatten(v: Value) -> Vec<Value> {{ vec![v] }}
                pub fn unflatten(v: Vec<Value>) -> Result<Value, Refusal> {{ Ok(v.into_iter().next().unwrap()) }}
                pub fn wire_refusal(r: Refusal) -> Refusal {{ r }}
            }}
            {code}
            fn main() {{
                use runtime::CopyValue;
                use internals::Value as V;
                use view::*;
                let value = Data {{ pair: Pair(7, -3), choice: Choice::Pair(9, true), array: [11, -12], option: Some(0.5), tuple: (false, "hello".into()) }};
                let wire = V::Rec(vec![V::Rec(vec![V::U(7), V::S(-3)]), V::Var(1, vec![V::U(9), V::B(true)]), V::Seq(vec![V::S(11), V::S(-12)]), V::Var(1, vec![V::F(0.5)]), V::Rec(vec![V::B(false), V::Str("hello".into())])]);
                assert_eq!(value.to_value(), wire);
                assert_eq!(Data::from_value(wire).unwrap(), value);
                for value in [Choice::Empty, Choice::Named {{ value: 'x' }}] {{ assert_eq!(Choice::from_value(value.to_value()).unwrap(), value); }}
                let value = Borrowed {{ text: Box::new("owned".into()), wide: (1 << 100) | 42 }};
                assert_eq!(Borrowed::from_value(value.to_value()).unwrap(), value);
                assert_eq!(<Owner as runtime::ViewType>::from_reached(runtime::Reached::Node(runtime::Node)).unwrap().second().unwrap(), 1, "declaration-order field number, not selected-field order");
                assert!(Data::from_value(V::None).is_err());
                assert_eq!(succeed().unwrap(), Ok(37));
                assert_eq!(fail().unwrap(), Err(GameError));
                assert_eq!(find().unwrap().unwrap().second().unwrap(), 1);
                assert!(missing().unwrap().is_none());
                assert_eq!(<Owner as runtime::ViewType>::from_reached(runtime::Reached::Node(runtime::Node)).unwrap().call_set_mode(5).unwrap(), Ok(37));
                let (mut n, mut pair) = (7, Pair(2, -3));
                assert!(mutate(&mut n, &mut pair).unwrap());
                assert_eq!((n, pair), (12, Pair(9, -4)));
                assert_eq!(DECLARED.len(), 12, "nine declarations plus three additional walked layouts");
            }}
        "#
        );
        let input = build.join("main.rs");
        let output = build.join("fixture");
        std::fs::write(&input, source).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["--edition=2024", "-Awarnings"])
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert!(
            std::process::Command::new(output)
                .status()
                .unwrap()
                .success()
        );
        std::fs::remove_dir_all(build).unwrap();
    }

    #[test]
    fn native_method_collision_names_do_not_depend_on_field_declarations() {
        let entries = [
            entry("horizon_client::Mode", "struct", "struct Mode { mode: u8 }"),
            entry("horizon_client::Mode::mode", "field", "mode: u8"),
            entry(
                "horizon_client::Mode::set_mode",
                "method",
                "fn set_mode(&mut self, mode: u8)",
            ),
        ];
        let all = [
            entry("horizon_client::Mode", "struct", "struct Mode { mode: u8 }"),
            entry(
                "horizon_client::Mode::mode_field",
                "method",
                "fn mode_field(&self) -> u8",
            ),
        ];
        let (_, counts) = crate::internals_table::table(&all, &[all[1].path.clone()]);
        assert_eq!(counts.renamed_methods[&all[1].path], "call_mode_field");
        let rule = Rule::new(&[entry(
            "horizon_client::Mode",
            "struct",
            "struct Mode { mode: u8, mode_field: u8 }",
        )]);
        assert_eq!(
            [
                descriptor_name("mode", Some("horizon_client::Mode"), &rule),
                method_name(&rule, "horizon_client::Mode", "mode_field"),
            ],
            ["call_mode_field", "call_call_mode_field"]
        );
        for paths in [
            vec![entries[2].path.clone()],
            vec![entries[2].path.clone(), entries[1].path.clone()],
        ] {
            let (_, counts) = crate::internals_table::table(&entries, &paths);
            assert!(counts.refused.is_empty());
            assert_eq!(counts.renamed_methods[&entries[2].path], "call_set_mode");
        }
    }

    #[test]
    fn mutable_copy_arguments_are_bound() {
        let entries = [
            entry(
                "horizon_client::Counter",
                "struct",
                "struct Counter { value: u8 }",
            ),
            entry(
                "horizon_client::Counter::mutate",
                "method",
                "fn mutate(&mut self, out: &mut u8)",
            ),
        ];
        generate(&entries, &[declaration(&entries[1])], &[], &[]).unwrap();
    }

    #[test]
    fn build_refuses_changed_missing_and_opaque_declarations() {
        let entries = [entry(
            "horizon_client::Data",
            "struct",
            "struct Data<T> { x: T }",
        )];
        let mut declared = declaration(&entries[0]);
        assert!(
            generate(&entries, &[declared], &[], &[])
                .unwrap_err()
                .contains("generic type")
        );
        declared = declaration(&entries[0]);
        declared.hash = "old".into();
        assert!(
            generate(&entries, &[declared], &[], &[])
                .unwrap_err()
                .contains("changed internal")
        );
        declared = declaration(&entries[0]);
        declared.path = "missing".into();
        assert!(
            generate(&entries, &[declared], &[], &[])
                .unwrap_err()
                .contains("missing")
        );
    }
}
