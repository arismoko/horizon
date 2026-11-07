//! Derive hook subscriptions from a trait implementation.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    ImplItem, ImplItemFn, ItemImpl, Meta, Token, parse::Parser, parse_macro_input,
    punctuated::Punctuated,
};

/// The staged call site's typed boundary. Only generated source invokes this macro.
#[proc_macro]
pub fn site(input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(input with Punctuated::<syn::Expr, Token![,]>::parse_terminated);
    let args: Vec<_> = args.iter().collect();
    let [
        wrap,
        place,
        listener,
        recv_ty,
        record,
        output,
        receiver,
        args,
        original,
    ] = args.as_slice()
    else {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "expected fresh wrap name, site types, receiver, args and original",
        )
        .to_compile_error()
        .into();
    };
    quote! {{
        fn #wrap<'g>(__at: usize, __receiver: #recv_ty<'g, '_>, __args: &mut #record,
            __original: &mut dyn FnMut(#recv_ty<'g, '_>, &mut #record) -> #output) -> #output {
            ::mod_points::__private::wrap(::mod_points::SiteId::#place, __at, |__entry| {
                match __entry {
                    Some((__pointer, __next)) => {
                        // SAFETY: registration checks this exact site's listener type.
                        let __listener: #listener = unsafe { ::std::mem::transmute(__pointer) };
                        __listener(__receiver, __args, &mut |__recv, __args| #wrap(__next, __recv, __args, __original))
                    }
                    None => __original(__receiver, __args),
                }
            })
        }
        #wrap(0, #receiver, &mut #args, &mut #original)
    }}.into()
}

/// Keep only attributes that can gate a method, including nested `cfg_attr`s.
fn cfg_meta(meta: &Meta) -> Option<Meta> {
    if meta.path().is_ident("cfg") {
        return Some(meta.clone());
    }
    let Meta::List(list) = meta else { return None };
    if !list.path.is_ident("cfg_attr") {
        return None;
    }
    let args = Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .ok()?;
    let mut args = args.iter();
    let condition = args.next()?;
    let gated: Vec<_> = args.filter_map(cfg_meta).collect();
    if gated.is_empty() {
        None
    } else {
        Some(syn::parse_quote!(cfg_attr(#condition, #(#gated),*)))
    }
}

fn cfg_attributes(method: &ImplItemFn) -> Vec<syn::Attribute> {
    method
        .attrs
        .iter()
        .filter_map(|attr| cfg_meta(&attr.meta))
        .map(|meta| syn::parse_quote!(#[#meta]))
        .collect()
}

fn uninspectable(item: &ImplItem) -> bool {
    matches!(item, ImplItem::Macro(_) | ImplItem::Verbatim(_))
}

fn expand(mut implementation: ItemImpl, export: bool) -> TokenStream {
    let Some((_, trait_path, _)) = &implementation.trait_ else {
        return syn::Error::new_spanned(
            &implementation,
            "expected an impl of Hooks or RenderHooks",
        )
        .into_compile_error()
        .into();
    };
    let name = &trait_path.segments.last().unwrap().ident;
    let (render, part) = match name.to_string().as_str() {
        "Hooks" => (false, None),
        "RenderHooks" => (true, None),
        "BlockPart" => (false, Some("block_subscribed")),
        "BlockEntityPart" => (false, Some("block_entity_subscribed")),
        "ItemPart" => (false, Some("item_subscribed")),
        "MenuPart" => (false, Some("menu_subscribed")),
        // Entity parts carry no scoped hook yet (#1077): `register_goals` is
        // not one, so its subscriptions stay empty whatever it overrides.
        "EntityPart" => (false, Some("entity_subscribed_empty")),
        _ => {
            return syn::Error::new_spanned(name, "expected a hook or part trait")
                .into_compile_error()
                .into();
        }
    };
    if part.is_some()
        && let Some(item) = implementation.items.iter().find(|item| uninspectable(item))
    {
        let ty = &implementation.self_ty;
        return syn::Error::new_spanned(
            item,
            format!("uninspectable items in impl {trait_path} for {ty} are not supported by horizon_part; write hook methods explicitly", trait_path = quote!(#trait_path), ty = quote!(#ty)),
        )
        .into_compile_error()
        .into();
    }
    let mut methods = Vec::new();
    let mut cfgs = Vec::new();
    for item in &implementation.items {
        if let ImplItem::Fn(method) = item {
            methods.push(&method.sig.ident);
            cfgs.push(cfg_attributes(method));
        }
    }
    let id = if render {
        format_ident!("RenderHookId")
    } else {
        format_ident!("HookId")
    };
    let ids = if render {
        format_ident!("render_hook_ids")
    } else {
        format_ident!("hook_ids")
    };
    let subscribed_name = format_ident!("{}", part.unwrap_or("subscribed"));
    let subscribed = if subscribed_name == "entity_subscribed_empty" {
        // Entity parts (arismoko/horizon#1077): no scoped hook exists for them
        // yet, so `register_goals` cannot be mistaken for one.
        syn::parse_quote! {
            fn entity_subscribed(&self) -> &'static [Option<mod_api::HookId>] {
                &[]
            }
        }
    } else {
        syn::parse_quote! {
            fn #subscribed_name(&self) -> &'static [Option<mod_api::#id>] {
                &[#(#(#cfgs)* mod_api::#ids::#methods),*]
            }
        }
    };
    implementation.items.push(subscribed);
    let export = if export && !render && part.is_none() {
        let ty = &implementation.self_ty;
        quote! {
            pub fn hooks() -> Box<dyn mod_api::Hooks> { Box::new(<#ty as Default>::default()) }
            /// The mod's hooks for the guest glue in `mod_api::wasm`, when built to `mod.wasm`.
            #[cfg(target_arch = "wasm32")]
            #[unsafe(no_mangle)]
            fn __horizon_mod_hooks() -> Box<dyn mod_api::Hooks> { hooks() }
        }
    } else {
        quote!()
    };
    quote!(#implementation #export).into()
}

/// Generate the panicking defaults of a host area from each method's access declaration.
/// The same declarations drive game forwarders in the build-time API generator.
#[proc_macro_attribute]
pub fn host_area(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    let mut area = parse_macro_input!(input as syn::ItemTrait);
    let name = area.ident.to_string();
    let union = [
        "ClientBackendMut",
        "BackendMut",
        "ClientBackend",
        "ServerHost",
        "ClientHost",
        "Backend",
        "Host",
    ]
    .into_iter()
    .find(|suffix| name.ends_with(suffix))
    .unwrap_or("Host");
    let union = if union == "ClientBackend" {
        "Backend"
    } else {
        union
    };
    for member in &mut area.items {
        let syn::TraitItem::Fn(method) = member else {
            continue;
        };
        let Some(index) = method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("access"))
        else {
            return syn::Error::new_spanned(&method.sig, "host method needs #[access(Level)]")
                .to_compile_error()
                .into();
        };
        let attr = method.attrs.remove(index);
        let level = match attr.parse_args::<syn::Ident>() {
            Ok(level) => level,
            Err(error) => return error.to_compile_error().into(),
        };
        method.attrs.retain(|attr| {
            !attr.path().is_ident("static_call") && !attr.path().is_ident("service")
        });
        let operation = method.sig.ident.to_string();
        let label = format!("{union}::{operation}");
        method.default = Some(syn::parse_quote!({
            crate::phase::unprovided(#label, #operation, crate::phase::LoadAccess::#level)
        }));
        method.semi_token = None;
    }
    quote!(#area).into()
}

/// Mark a trait mod or its renderer and derive its hook subscriptions.
#[proc_macro_attribute]
pub fn horizon_mod(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), true)
}

/// Mark a part implementation and derive the scoped hooks it overrides.
///
/// Hook methods must be written explicitly, not generated inside the impl:
///
/// ```compile_fail
/// use horizon_mod_api_macros::horizon_part;
/// trait BlockPart { fn tick(&self) {} }
/// struct Block;
/// macro_rules! hook { () => { fn tick(&self) {} }; }
/// #[horizon_part]
/// impl BlockPart for Block { hook!(); }
/// ```
#[proc_macro_attribute]
pub fn horizon_part(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `#[point(side = "server" | "client" | "both", name = "…", raw, free)]` on a function: an
/// injection point (#589), a slot the launcher fills at load for the mods that listen there. The
/// staging pass that builds the prebuilt game (and the launcher's source builds) puts it on every
/// function [`point_shape::eligible`] admits; the source tree carries none. Its head checks the slot
/// (one atomic load and a branch while nothing listens), then hands each API function listener the
/// parameters with API types (and, on a `Game` method with a `side`, the side's host), and each raw
/// listener (`raw`: the pass declared its record and listener types beside the function) the
/// receiver and a `&mut` to every parameter; each may change them. Its return does the same with
/// the value returned; a value that borrows reaches only raw listeners, as itself. `free` marks a
/// free function, whose body moves to a sibling function rather than an associated one.
#[proc_macro_attribute]
pub fn point(attribute: TokenStream, input: TokenStream) -> TokenStream {
    let mut function = parse_macro_input!(input as ImplItemFn);
    if let Some(inline) = function.attrs.iter().find(|attr| {
        attr.path().is_ident("inline")
            && attr
                .parse_args::<syn::Ident>()
                .is_ok_and(|arg| arg == "always")
    }) {
        return syn::Error::new_spanned(
            inline,
            "a point keeps its function out of line: drop #[inline(always)]",
        )
        .to_compile_error()
        .into();
    }
    let shape = match point_shape::shape(attribute.into(), &function.sig) {
        Ok(shape) => shape,
        Err(e) => return e.to_compile_error().into(),
    };
    // The body moves to an inlined inner function, so the return listeners run however it
    // returns; the outer function takes each parameter by a plain, mutable name.
    let inner = syn::Ident::new(
        &format!(
            "__point_{}",
            function.sig.ident.to_string().trim_start_matches("r#")
        ),
        function.sig.ident.span(),
    );
    let mut outer = function.clone();
    let mut call = Vec::new();
    let mut passed = Vec::new();
    // Where each typed parameter lives while the listeners run: in the API record, or a local.
    let mut places = std::collections::HashMap::new();
    // For each native-width parameter: the raw listener's local, and its write-back.
    let mut widened = Vec::new();
    for (i, input) in outer.sig.inputs.iter_mut().enumerate() {
        let syn::FnArg::Typed(typed) = input else {
            continue;
        };
        let original = match &*typed.pat {
            syn::Pat::Ident(pat) => Some(pat.ident.clone()),
            _ => None,
        };
        let plain = format_ident!("__p{i}");
        *typed.pat = syn::parse_quote!(mut #plain);
        match original.and_then(|name| shape.args.iter().find(|arg| arg.name == name)) {
            Some(arg) => {
                let name = &arg.name;
                passed.push(plain);
                match point_shape::wide(&arg.ty) {
                    // A native width crosses the record as 64 bits, and a raw listener gets it
                    // back in its own type, in a local written back after it.
                    Some(wide) => {
                        let (ty, local) = (&arg.ty, format_ident!("__w{i}"));
                        places.insert(i, quote!(#local));
                        widened.push((
                            quote!(let mut #local = __args.#name as #ty;),
                            quote!(__args.#name = #local as #wide;),
                        ));
                        call.push(quote!(__args.#name as #ty));
                    }
                    None => {
                        places.insert(i, quote!(__args.#name));
                        call.push(match arg.ty {
                            syn::Type::Reference(_) => quote!(&*__args.#name),
                            _ => quote!(__args.#name),
                        });
                    }
                }
            }
            None => {
                places.insert(i, quote!(#plain));
                call.push(quote!(#plain));
            }
        }
    }
    // The inner function's type and const parameters, named at the call: one only in the return
    // type can't be inferred through it.
    let generics: Vec<_> = function
        .sig
        .generics
        .params
        .iter()
        .filter_map(|param| match param {
            syn::GenericParam::Type(ty) => Some(&ty.ident),
            syn::GenericParam::Const(c) => Some(&c.ident),
            syn::GenericParam::Lifetime(_) => None,
        })
        .collect();
    let turbofish = if generics.is_empty() {
        quote!()
    } else {
        quote!(::<#(#generics),*>)
    };
    let (receiver, callee) = match (shape.receiver, shape.free) {
        (point_shape::Receiver::None, true) => (quote!(), quote!(#inner)),
        (point_shape::Receiver::None, false) => (quote!(), quote!(Self::#inner)),
        (point_shape::Receiver::Shared, _) => (quote!(self,), quote!(self.#inner)),
        (point_shape::Receiver::Mut, _) => (quote!(&mut *self,), quote!(self.#inner)),
    };
    let erased_receiver = match shape.receiver {
        point_shape::Receiver::None => quote!(::std::ptr::null_mut()),
        point_shape::Receiver::Shared => quote!(self as *const Self as *mut ()),
        point_shape::Receiver::Mut => quote!(&mut *self as *mut Self as *mut ()),
    };
    // What the API listeners get before their record: the shape's host, if it has one.
    let host = match (shape.receiver, shape.side) {
        (_, None) => quote!(),
        (point_shape::Receiver::Mut, Some(point_shape::Side::Both)) => {
            quote!(crate::game::points::host_side(&mut *self),)
        }
        (point_shape::Receiver::Mut, Some(_)) => quote!(&mut *self,),
        (_, Some(_)) => quote!(self,),
    };
    let record = shape.record();
    let outcome = shape.raw_outcome(&function.sig);
    // A raw `result` is the API record's when the API carries the value, else a slot of its own.
    let api_result = matches!(shape.returns, point_shape::Returns::Value(_));
    // A native-width value crosses the record as 64 bits.
    let (widen, narrow) = match &shape.returns {
        point_shape::Returns::Value(ty) => match point_shape::wide(ty) {
            Some(wide) => (quote!(as #wide), quote!(as #ty)),
            None => (quote!(), quote!()),
        },
        _ => (quote!(), quote!()),
    };
    let result_place = match (&shape.returns, api_result) {
        (point_shape::Returns::Value(ty), true) if point_shape::wide(ty).is_some() => {
            let wide = point_shape::wide(ty);
            widened.push((
                quote!(let mut __w_result = __args.result.map(|__value| __value as #ty);),
                quote!(__args.result = __w_result.map(|__value| __value as #wide);),
            ));
            quote!(__w_result)
        }
        (_, true) => quote!(__args.result),
        (_, false) => quote!(__result_slot),
    };
    let (unpack, repack): (Vec<_>, Vec<_>) = widened.into_iter().unzip();
    // The raw record, built afresh for each firing from the places the parameters live.
    let raw_record = {
        let ty = shape.raw_record();
        let fields = point_shape::raw_fields(&function.sig)
            .into_iter()
            .map(|field| {
                let (name, place) = (field.name, &places[&field.input]);
                quote!(#name: &mut #place)
            });
        let outcome = match outcome {
            point_shape::RawOutcome::Cancel => quote!(cancel: &mut __args.cancel,),
            point_shape::RawOutcome::Result(_) => quote!(result: &mut #result_place,),
            point_shape::RawOutcome::None => quote!(),
        };
        quote!(#ty { #(#fields,)* #outcome })
    };
    let fire = |variant: &syn::Ident, method: &syn::Ident, api: bool| {
        let name = method.to_string();
        let api = api.then(|| {
            quote! {
                mod_points::dispatch::#method(#host &mut __args);
            }
        });
        let raw = shape.raw.then(|| {
            let listener = shape.raw_listener();
            quote! {
                #(#unpack)*
                mod_points::__private::fire_raw(
                    mod_points::PointId::#variant,
                    #name,
                    &mut |__kind| match __kind {
                        mod_points::__private::RawKind::Typed(__function) => {
                            // SAFETY: registration guarantees this point's listener type.
                            unsafe {
                                ::std::mem::transmute::<*const (), #listener>(__function)(
                                    #receiver &mut #raw_record,
                                )
                            }
                        }
                        mod_points::__private::RawKind::Indexed(__function, __index) => {
                            unsafe { __function(mod_points::PointId::#variant as usize, __index, #erased_receiver, &mut #raw_record as *mut _ as *mut ()) };
                        }
                        mod_points::__private::RawKind::Erased(__function) => {
                            // SAFETY: this slot supplies its exact receiver and raw record types.
                            unsafe {
                                __function(
                                    mod_points::PointId::#variant as usize,
                                    #erased_receiver,
                                    &mut #raw_record as *mut _ as *mut (),
                                )
                            }
                        }
                    },
                );
                #(#repack)*
            }
        });
        quote!(#api #raw)
    };
    let listened = |variant: &syn::Ident| quote!(mod_points::__private::POINTS[mod_points::PointId::#variant as usize].load(::std::sync::atomic::Ordering::Acquire));
    let (head_variant, head_method) = (
        shape.variant(),
        syn::Ident::new(&shape.name, proc_macro2::Span::call_site()),
    );
    let head_listened = listened(&head_variant);
    let head_fire = fire(&head_variant, &head_method, true);
    let result_slot = match &outcome {
        point_shape::RawOutcome::Result(_) if shape.raw && !api_result => {
            quote!(let mut __result_slot = None;)
        }
        _ => quote!(),
    };
    let stop = match (&shape.returns, &outcome) {
        (point_shape::Returns::Unit, _) => quote!(if __args.cancel {
            return;
        }),
        (point_shape::Returns::UnitResult, _) => quote!(if __args.cancel {
            return Ok(());
        }),
        (point_shape::Returns::Value(_), _) => {
            quote!(if let Some(__result) = __args.result.take() {
                return __result #narrow;
            })
        }
        (_, point_shape::RawOutcome::Result(_)) if shape.raw => {
            quote!(if let Some(__result) = __result_slot.take() {
                return __result;
            })
        }
        _ => quote!(),
    };
    let returned = if shape.at_return() {
        let (variant, method) = (
            shape.return_variant(),
            format_ident!("{}_return", shape.name),
        );
        let return_listened = listened(&variant);
        let name = method.to_string();
        let api = match (&shape.returns, shape.has_return()) {
            (_, false) => quote!(),
            (point_shape::Returns::Value(_), true) => quote! {
                __args.result = Some(__result #widen);
                mod_points::dispatch::#method(#host &mut __args);
                let __result = __args.result.map_or(__result, |__value| __value #narrow);
            },
            (_, true) => quote! {
                mod_points::dispatch::#method(#host &mut __args);
            },
        };
        let raw = shape.raw.then(|| {
            let listener = shape.raw_return_listener();
            let (receiver, erased_receiver) = if shape.borrowed_return {
                (quote!(), quote!(::std::ptr::null_mut()))
            } else {
                (receiver.clone(), erased_receiver.clone())
            };
            let value = match &function.sig.output {
                syn::ReturnType::Type(..) => quote!(&mut __result),
                syn::ReturnType::Default => quote!(),
            };
            quote! {
                #[allow(unused_mut)]
                let mut __result = __result;
                mod_points::__private::fire_raw(
                    mod_points::PointId::#variant,
                    #name,
                    &mut |__kind| match __kind {
                        mod_points::__private::RawKind::Typed(__function) => {
                            // SAFETY: registration guarantees this place's return listener type.
                            unsafe {
                                ::std::mem::transmute::<*const (), #listener>(__function)(
                                    #receiver #value
                                )
                            }
                        }
                        mod_points::__private::RawKind::Indexed(__function, __index) => {
                            unsafe { __function(mod_points::PointId::#variant as usize, __index, #erased_receiver, &mut __result as *mut _ as *mut ()) };
                        }
                        mod_points::__private::RawKind::Erased(__function) => {
                            // SAFETY: this slot supplies its exact receiver and return types.
                            unsafe {
                                __function(
                                    mod_points::PointId::#variant as usize,
                                    #erased_receiver,
                                    &mut __result as *mut _ as *mut (),
                                )
                            }
                        }
                    },
                );
            }
        });
        quote! {
            if #return_listened {
                #api
                #raw
                return __result;
            }
        }
    } else {
        quote!()
    };
    outer.block = syn::parse_quote!({
        #[allow(unused_mut)]
        let mut __args = mod_points::#record::__new(#(#passed),*);
        #result_slot
        if #head_listened {
            #head_fire
            #stop
        }
        let __result = #callee #turbofish(#(#call),*);
        #returned
        __result
    });
    // The outer function keeps the function's own `#[inline]`: a small accessor in a lower crate
    // (`Chunk::get`) stays inlinable into its callers, slot check and all.
    function.sig.ident = inner;
    function.vis = syn::Visibility::Inherited;
    function
        .attrs
        .retain(|attr| !attr.path().is_ident("doc") && !attr.path().is_ident("inline"));
    function.attrs.push(syn::parse_quote!(#[inline(always)]));
    quote!(#outer #function).into()
}

/// Derive subscriptions for internal test fixtures that construct their own mods.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn __subscriptions(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `if_installed!(host, other_mod, { other_mod::call(); })`: runs the block only where the optional
/// dependency `other_mod` is installed. A native mod is built with the Cargo feature `other_mod`
/// only when it is; a wasm mod is built with it and asks the game at the call, since an absent
/// library's functions trap. One guard for both builds.
#[proc_macro]
pub fn if_installed(input: TokenStream) -> TokenStream {
    let IfInstalled { host, id, block } = parse_macro_input!(input as IfInstalled);
    let name = id.to_string();
    quote! {
        {
            // A `let` takes the attribute where an expression statement can't.
            #[cfg(feature = #name)]
            let () = if ::mod_api::prelude::LevelsHost::is_loaded(&*#host, #name) #block;
            #[cfg(not(feature = #name))]
            let _ = &#host;
        }
    }
    .into()
}

struct IfInstalled {
    host: syn::Expr,
    id: syn::Ident,
    block: syn::Block,
}

impl syn::parse::Parse for IfInstalled {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let host = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let id = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        Ok(Self {
            host,
            id,
            block: input.parse()?,
        })
    }
}

/// Declares how a plain-data type crosses the generated wasm interface.
///
/// `#[wasm(via = Form)]` on a type or its re-export checks `From<&Type> for Form`
/// and `TryFrom<Form> for Type`. `#[wasm(host_constructor = method)]` on an owned
/// tuple struct makes incoming host lifts call that checked constructor with the
/// lifted fields instead of constructing the tuple directly. It returns `Result<Self, E>`
/// with a displayable refusal. Guest lifts remain structural: the host owns validation.
#[proc_macro_attribute]
pub fn wasm(attribute: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as syn::Item);
    let rule = parse_macro_input!(attribute as WasmRule);
    if let WasmRule::HostConstructor(constructor) = rule {
        let syn::Item::Struct(record) = &item else {
            return syn::Error::new_spanned(
                item,
                "host_constructor requires an owned tuple struct",
            )
            .to_compile_error()
            .into();
        };
        let syn::Fields::Unnamed(fields) = &record.fields else {
            return syn::Error::new_spanned(
                item,
                "host_constructor requires an owned tuple struct",
            )
            .to_compile_error()
            .into();
        };
        if !record.generics.params.is_empty() {
            return syn::Error::new_spanned(
                item,
                "host_constructor requires an owned tuple struct",
            )
            .to_compile_error()
            .into();
        }
        let ty = &record.ident;
        let params: Vec<_> = fields
            .unnamed
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let name = format_ident!("p{i}");
                let ty = &field.ty;
                quote!(#name: #ty)
            })
            .collect();
        let args: Vec<_> = (0..params.len()).map(|i| format_ident!("p{i}")).collect();
        return quote! {
            #item
            const _: () = {
                fn checked_constructor(#(#params),*) -> ::core::result::Result<#ty, ::std::string::String> {
                    #ty::#constructor(#(#args),*).map_err(|error| error.to_string())
                }
                let _ = #ty::#constructor;
                let _ = checked_constructor;
            };
        }
        .into();
    }
    let WasmRule::Via(via) = rule else {
        unreachable!()
    };
    let ty = match &item {
        syn::Item::Struct(item) => item.ident.clone(),
        syn::Item::Enum(item) => item.ident.clone(),
        syn::Item::Use(item) => match last_use_name(&item.tree) {
            Some(name) => name,
            None => {
                return syn::Error::new_spanned(item, "#[wasm] exports one type")
                    .to_compile_error()
                    .into();
            }
        },
        item => {
            return syn::Error::new_spanned(item, "#[wasm] goes on a type or its `pub use`")
                .to_compile_error()
                .into();
        }
    };
    quote! {
        #item
        const _: () = {
            fn crosses_as_its_form()
            where
                #via: for<'a> ::core::convert::From<&'a #ty>,
                #ty: ::core::convert::TryFrom<#via>,
            {
            }
        };
    }
    .into()
}

/// The single conversion rule on a type declaration.
enum WasmRule {
    Via(syn::Type),
    HostConstructor(syn::Ident),
}

impl syn::parse::Parse for WasmRule {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let key: syn::Ident = input.parse()?;
        input.parse::<syn::Token![=]>()?;
        match key.to_string().as_str() {
            "via" => Ok(Self::Via(input.parse()?)),
            "host_constructor" => Ok(Self::HostConstructor(input.parse()?)),
            _ => Err(syn::Error::new(
                key.span(),
                "expected via or host_constructor",
            )),
        }
    }
}

/// The name a `use` tree of one path brings in.
fn last_use_name(tree: &syn::UseTree) -> Option<syn::Ident> {
    match tree {
        syn::UseTree::Path(path) => last_use_name(&path.tree),
        syn::UseTree::Name(name) => Some(name.ident.clone()),
        syn::UseTree::Rename(rename) => Some(rename.rename.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_items_are_uninspectable() {
        let item = ImplItem::Verbatim(quote!(future syntax));
        assert!(uninspectable(&item));
    }
}
