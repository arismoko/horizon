//! Derive hook subscriptions from a trait implementation.
mod event;
mod wire;

/// Derive [`Event`](../mod_api/trait.Event.html) for a mod's own event:
/// `#[event(id = "your_mod:name", side = server | client, cancellable)]` on a
/// `#[non_exhaustive]` struct; a cancellable one has a `cancelled: bool` field.
#[proc_macro_derive(Event, attributes(event))]
pub fn derive_event(input: TokenStream) -> TokenStream {
    event::expand(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive the canonical owned-value encoding.
///
/// `#[wire(crate = ::mod_vocab::wire)]` selects the contract module for
/// lower-layer crates. The default is `::mod_api::wire`.
#[proc_macro_derive(Wire, attributes(wire))]
pub fn derive_wire(input: TokenStream) -> TokenStream {
    wire::expand(syn::parse_macro_input!(input as syn::DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    ImplItem, ImplItemFn, ItemImpl, Meta, Token, parse::Parser, parse_macro_input,
    punctuated::Punctuated,
};

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

/// A method's `#[listener(priority = …, receive_cancelled)]`, removed from the method.
struct ListenerOptions {
    priority: syn::Expr,
    /// The attribute's span, for the error when the hook has no cancel to receive.
    receive_cancelled: Option<proc_macro2::Span>,
}

/// Takes `method`'s `#[listener]` or `#[subscribe]` (named by `kind`), if it has one.
fn take_listener(method: &mut ImplItemFn, kind: &str) -> syn::Result<Option<ListenerOptions>> {
    let Some(index) = method
        .attrs
        .iter()
        .position(|attr| attr.path().is_ident(kind))
    else {
        return Ok(None);
    };
    let attr = method.attrs.remove(index);
    if let Some(again) = method.attrs.iter().find(|attr| attr.path().is_ident(kind)) {
        return Err(syn::Error::new_spanned(
            again,
            format!("a method takes one #[{kind}]"),
        ));
    }
    let mut options = ListenerOptions {
        priority: syn::parse_quote!(0),
        receive_cancelled: None,
    };
    if matches!(attr.meta, Meta::Path(_)) {
        return Ok(Some(options));
    }
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("priority") {
            options.priority = meta.value()?.parse()?;
            Ok(())
        } else if meta.path.is_ident("receive_cancelled") {
            options.receive_cancelled = Some(meta.path.get_ident().unwrap().span());
            Ok(())
        } else {
            Err(meta.error("expected `priority = <i32>` or `receive_cancelled`"))
        }
    })?;
    Ok(Some(options))
}

fn uninspectable(item: &ImplItem) -> bool {
    matches!(item, ImplItem::Macro(_) | ImplItem::Verbatim(_))
}

fn expand(mut implementation: ItemImpl, export: bool) -> TokenStream {
    let Some((_, trait_path, _)) = &implementation.trait_ else {
        return syn::Error::new_spanned(
            &implementation,
            "expected an impl of Hooks or a Part trait",
        )
        .into_compile_error()
        .into();
    };
    let name = &trait_path.segments.last().unwrap().ident;
    let part = match name.to_string().as_str() {
        "Hooks" => None,
        "BlockPart" => Some("block_subscribed"),
        "BlockEntityPart" => Some("block_entity_subscribed"),
        "ItemPart" => Some("item_subscribed"),
        "MenuPart" => Some("menu_subscribed"),
        "MobEffectPart" => Some("effect_subscribed"),
        "FluidPart" => Some("fluid_subscribed"),
        "EntityPart" => Some("entity_subscribed"),
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
            format!("uninspectable items in impl {trait_path} for {ty} are not supported by sunrise_part; write hook methods explicitly", trait_path = quote!(#trait_path), ty = quote!(#ty)),
        )
        .into_compile_error()
        .into();
    }
    let mut methods = Vec::new();
    let mut cfgs = Vec::new();
    let mut listeners = Vec::new();
    // Mod event listeners: plain methods, moved out of the trait impl.
    let mut subscribers = Vec::new();
    let mut inherent = Vec::new();
    let mut index = 0;
    while index < implementation.items.len() {
        let ImplItem::Fn(method) = &mut implementation.items[index] else {
            index += 1;
            continue;
        };
        match take_listener(method, "subscribe") {
            Ok(Some(_)) if part.is_some() => {
                return syn::Error::new_spanned(
                    &method.sig.ident,
                    "#[subscribe] listens to a mod event from a mod's `impl Hooks`, not a part",
                )
                .into_compile_error()
                .into();
            }
            Ok(Some(options)) => {
                subscribers.push((method.sig.ident.clone(), cfg_attributes(method), options));
                inherent.push(implementation.items.remove(index));
            }
            Ok(None) => index += 1,
            Err(error) => return error.into_compile_error().into(),
        }
    }
    if !subscribers.is_empty() {
        // `register` adds the listeners, then runs the mod's own `register`, if any.
        let own = implementation.items.iter().position(
            |item| matches!(item, ImplItem::Fn(method) if method.sig.ident == "register"),
        );
        let own = own.map(|position| {
            let ImplItem::Fn(mut method) = implementation.items.remove(position) else {
                unreachable!()
            };
            method.sig.ident = format_ident!("__sunrise_register");
            let cfgs = cfg_attributes(&method);
            let call = quote!(#(#cfgs)* Self::__sunrise_register(self, r, cx););
            inherent.push(ImplItem::Fn(method));
            call
        });
        let adds = subscribers.iter().map(|(method, cfgs, options)| {
            let priority = &options.priority;
            let add = if options.receive_cancelled.is_some() {
                format_ident!("add_listener_receive_cancelled")
            } else {
                format_ident!("add_listener")
            };
            // A refusal is recorded on the registrar and refuses the mod at startup.
            quote! { #(#cfgs)* let _ = r.#add(#priority, Self::#method); }
        });
        implementation.items.push(syn::parse_quote! {
            fn register(&self, r: &mut dyn mod_api::Registrar, cx: &mod_api::RegistrationContext) {
                let _ = (&r, cx);
                #(#adds)*
                #own
            }
        });
    }
    // A block part's plain declarations are not hooks, so their names subscribe nothing.
    const BLOCK_PART_PLAIN: [&str; 1] = ["explosion_resistance_is_pure"];
    for item in &mut implementation.items {
        if let ImplItem::Fn(method) = item {
            if name == "BlockPart"
                && BLOCK_PART_PLAIN.contains(&method.sig.ident.to_string().as_str())
            {
                continue;
            }
            match take_listener(method, "listener") {
                Ok(Some(_)) if part.is_some() => {
                    return syn::Error::new_spanned(
                        &method.sig.ident,
                        "#[listener] orders a trait mod's hooks; a part's are called for its own thing",
                    )
                    .into_compile_error()
                    .into();
                }
                Ok(Some(options)) => {
                    listeners.push((method.sig.ident.clone(), cfg_attributes(method), options))
                }
                Ok(None) => {}
                Err(error) => return error.into_compile_error().into(),
            }
            methods.push(method.sig.ident.clone());
            cfgs.push(cfg_attributes(method));
        }
    }
    let id = format_ident!("HookId");
    let ids = format_ident!("hook_ids");
    let subscribed_name = format_ident!("{}", part.unwrap_or("subscribed"));
    let subscribed = {
        syn::parse_quote! {
            fn #subscribed_name(&self) -> &'static [Option<mod_api::#id>] {
                &[#(#(#cfgs)* mod_api::#ids::#methods),*]
            }
        }
    };
    implementation.items.push(subscribed);
    if !listeners.is_empty() {
        let entries = listeners.iter().map(|(method, cfgs, options)| {
            let priority = &options.priority;
            let receive_cancelled = options.receive_cancelled.is_some();
            quote! { #(#cfgs)* mod_api::Listener { hook: mod_api::#ids::#method, priority: #priority, receive_cancelled: #receive_cancelled } }
        });
        // Only a hook a mod may cancel has a constant here, so the option fails to compile on
        // any other.
        let checks = listeners.iter().filter_map(|(method, cfgs, options)| {
            let at_option = syn::Ident::new(&method.to_string(), options.receive_cancelled?);
            Some(quote! { #(#cfgs)* const _: () = mod_api::cancel_hooks::#at_option; })
        });
        implementation.items.push(syn::parse_quote! {
            fn listeners(&self) -> &'static [mod_api::Listener] {
                #(#checks)*
                &[#(#entries),*]
            }
        });
    }
    let export = if export && part.is_none() {
        let ty = &implementation.self_ty;
        quote! { pub fn hooks() -> Box<dyn mod_api::Hooks> { Box::new(<#ty as Default>::default()) } }
    } else {
        quote!()
    };
    let inherent = if inherent.is_empty() {
        quote!()
    } else {
        let (generics, _, where_clause) = implementation.generics.split_for_impl();
        let ty = &implementation.self_ty;
        quote! { impl #generics #ty #where_clause { #(#inherent)* } }
    };
    quote!(#implementation #inherent #export).into()
}

/// Generate the defaults of a host area trait. Methods marked `#[decline_unavailable]`
/// do nothing or return `false`, according to their return type. Others panic if unimplemented.
/// The same declarations drive game forwarders in the build-time API generator.
/// A backend trait may carry `#[table_backend("path")]`: the game answers its reads from the
/// borrowed view at `path`, which implements the trait, and `Game` forwards to it. Methods can
/// opt out of generated conversion in either direction with `#[forward_api_inputs]` or
/// `#[forward_api_output]` when the implementation accepts or returns the API types directly.
/// `#[serialize_text]` validates Text arguments at a forwarding boundary that writes a packet
/// or a saved component; unmarked text arguments are converted only for in-process use.
/// Use `#[manual_handle]` for an inherent handle wrapper written by hand.
/// `#[goal_view]` marks a read a mob's goal tick answers from its own view for every holder.
#[proc_macro_attribute]
pub fn host_area(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    let mut area = parse_macro_input!(input as syn::ItemTrait);
    // `#[table_backend("path")]` and `#[server_only]` are read by the build-time generator alone.
    area.attrs.retain(|attr| {
        !attr.path().is_ident("table_backend") && !attr.path().is_ident("server_only")
    });
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
        let decline = method
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("decline_unavailable"));
        method.attrs.retain(|attr| {
            !attr.path().is_ident("static_call")
                && !attr.path().is_ident("service")
                && !attr.path().is_ident("read_stack")
                && !attr.path().is_ident("game_method")
                && !attr.path().is_ident("access")
                && !attr.path().is_ident("forward_api_inputs")
                && !attr.path().is_ident("forward_api_output")
                && !attr.path().is_ident("serialize_text")
                && !attr.path().is_ident("decline_unavailable")
                && !attr.path().is_ident("manual_handle")
                && !attr.path().is_ident("goal_view")
                && !attr.path().is_ident("server_only")
        });
        // A shared published read answers from the installed table, without a live Game.
        let published = match method
            .attrs
            .iter()
            .position(|attr| attr.path().is_ident("published"))
            .map(|index| method.attrs.remove(index).parse_args::<syn::Path>())
        {
            None => None,
            Some(Ok(path)) => Some(path),
            Some(Err(error)) => return error.to_compile_error().into(),
        };
        if let Some(path) = published {
            let args = method.sig.inputs.iter().filter_map(|arg| match arg {
                syn::FnArg::Typed(arg) => Some(arg.pat.clone()),
                syn::FnArg::Receiver(_) => None,
            });
            method.default = Some(syn::parse_quote!({ crate::__private::#path(#(#args),*) }));
            method.semi_token = None;
            continue;
        }
        method.default = Some(if decline {
            match &method.sig.output {
                syn::ReturnType::Default => syn::parse_quote!({}),
                syn::ReturnType::Type(_, ty)
                    if matches!(
                        ty.as_ref(),
                        syn::Type::Path(path) if path.qself.is_none() && path.path.is_ident("bool")
                    ) =>
                {
                    syn::parse_quote!({ false })
                }
                output => {
                    return syn::Error::new_spanned(
                        output,
                        "#[decline_unavailable] requires no return value or a bool return value",
                    )
                    .to_compile_error()
                    .into();
                }
            }
        } else {
            let operation = method.sig.ident.to_string();
            let label = format!("{union}::{operation}");
            syn::parse_quote!({ unimplemented!("{}", #label) })
        });
        method.semi_token = None;
    }
    quote!(#area).into()
}

/// Mark a trait mod and derive its hook subscriptions.
///
/// A hook method may carry `#[listener(priority = <i32>, receive_cancelled)]`, both optional:
/// mods run by descending priority (0 when omitted), then in dependency order, and a mod that
/// receives cancelled events is still called after an earlier mod cancels, with
/// `event.cancelled` to read and clear. `receive_cancelled` compiles only on a hook a mod may
/// cancel.
///
/// A method with `#[subscribe(priority = <i32>, receive_cancelled)]` (both optional) listens to
/// another mod's [`Event`](../mod_api/trait.Event.html) instead: the macro moves it out of the
/// trait impl and adds it in `register` with `Registrar::add_listener`, before the mod's own
/// `register` body runs.
#[proc_macro_attribute]
pub fn sunrise_mod(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), true)
}

/// Mark a part implementation and derive the scoped hooks it overrides.
///
/// Hook methods must be written explicitly, not generated inside the impl:
///
/// ```compile_fail
/// use sunrise_mod_api_macros::sunrise_part;
/// trait BlockPart { fn tick(&self) {} }
/// struct Block;
/// macro_rules! hook { () => { fn tick(&self) {} }; }
/// #[sunrise_part]
/// impl BlockPart for Block { hook!(); }
/// ```
#[proc_macro_attribute]
pub fn sunrise_part(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `#[sunrise_loop(name = "…", side = "server" | "client")]` on a published loop's entry function
/// (`[[native.loop]]`): a marker the staging pass lists in the prebuilt game's `loops.json`,
/// generating no code. The entry's body is one call of its generic loop function with the empty
/// slot, `self.<loop>(&mut ())`; a mod build installs a copy of the entry that calls it with the
/// mods' slots instead.
#[proc_macro_attribute]
pub fn sunrise_loop(attribute: TokenStream, input: TokenStream) -> TokenStream {
    let parser =
        syn::punctuated::Punctuated::<syn::MetaNameValue, syn::Token![,]>::parse_terminated;
    let pairs = parse_macro_input!(attribute with parser);
    let value = |key: &str| {
        pairs.iter().find_map(|pair| match &pair.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) if pair.path.is_ident(key) => Some(text.value()),
            _ => None,
        })
    };
    let named = value("name").is_some_and(|name| !name.is_empty());
    let sided = value("side").is_some_and(|side| side == "server" || side == "client");
    if !named || !sided || pairs.len() != 2 {
        return syn::Error::new_spanned(
            proc_macro2::TokenStream::from(input),
            "write #[sunrise_loop(name = \"…\", side = \"server\" | \"client\")]",
        )
        .to_compile_error()
        .into();
    }
    input
}

/// Derive subscriptions for internal test fixtures that construct their own mods.
#[doc(hidden)]
#[proc_macro_attribute]
pub fn __subscriptions(_attribute: TokenStream, input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as ItemImpl), false)
}

/// `if_installed!(host, other_mod, { other_mod::call(); })`: runs the block only where the optional
/// dependency `other_mod` is installed. A native mod is built with the Cargo feature `other_mod`
/// only when that dependency is installed.
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

/// Marks a function below the game crate as accepting native function replacements.
/// Game functions need no marker. This has no effect on a plain build: the stager
/// rewrites the function only in its staged copy. Use on free functions or impl methods,
/// without arguments.
#[proc_macro_attribute]
pub fn replaceable(args: TokenStream, item: TokenStream) -> TokenStream {
    if !args.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "replaceable takes no arguments",
        )
        .to_compile_error()
        .into();
    }
    if syn::parse::<syn::ItemFn>(item.clone()).is_err()
        && syn::parse::<syn::ImplItemFn>(item.clone()).is_err()
    {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "replaceable requires a free function or impl method",
        )
        .to_compile_error()
        .into();
    }
    item
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
