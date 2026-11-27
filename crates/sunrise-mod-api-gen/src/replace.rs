//! Function replacement: how a patched copy of a game function is installed in place of the
//! original, with the game prebuilt. The staging pass ([`stage`]) writes one check at the head of
//! every replaceable function of a staged source copy: a load of the function's slot in its
//! crate's `__REPLACE` array and, when a copy is installed there, a direct call of it. Beside the
//! function it writes the slot's typed reader (`__fnr_read_slot_<index>`) and installer
//! (`__fnr_<name>`), which a mixin build's patch crate calls with its copy
//! ([`crate::patch`]). A function's check is part of its body, so every inlined copy of the
//! function carries it.
//!
//! Replaceable: every free function and inherent method of the game module that [`eligible`]
//! admits, and each function marked with `#[sunrise_mod_api_macros::replaceable]` in the crates below it. The source tree
//! carries none of this: only a staged copy does, and everything the pass writes stays on the
//! line it annotates, so the staged source's line numbers are the pinned source's.
use crate::source::{self};
use mod_source::modules::ModuleFile;
use quote::quote;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use syn::{FnArg, ImplItem, Item, Pat, Type};

/// Emit the typed reader for one replacement cell, shared by staging and integration fixtures.
///
/// The cell must only hold null or function pointers of `ty`, as enforced by the
/// corresponding typed installer. The load is Acquire, pairing with the
/// installer's Release: a thread that sees the copy also sees everything written before it was
/// installed.
pub fn reader(
    reader: &syn::Ident,
    cell: usize,
    ty: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    quote! {
        #[doc(hidden)]
        #[inline(always)]
        #[allow(unsafe_code, dead_code, clippy::all)]
        pub fn #reader() -> ::std::option::Option<#ty> {
            let installed = crate::__REPLACE[#cell].load(::std::sync::atomic::Ordering::Acquire);
            if installed.is_null() {
                ::std::option::Option::None
            } else {
                // SAFETY: only this function's typed installer,
                // of this type, are stored here.
                ::std::option::Option::Some(unsafe { ::std::mem::transmute::<*mut (), #ty>(installed) })
            }
        }
    }
}

/// Whether a function with this signature and these attributes can take a replacement: its
/// slot holds one function pointer, so one compiled copy must stand for every call.
pub fn eligible(attrs: &[syn::Attribute], sig: &syn::Signature) -> bool {
    signature_exclusion(attrs, sig).is_none()
}

pub(crate) enum CandidateExclusion {
    Signature(&'static str),
    GenericImpl,
}

pub(crate) fn candidate_exclusion(
    attrs: &[syn::Attribute],
    sig: &syn::Signature,
    owner: Option<&syn::ItemImpl>,
    loop_generic: bool,
) -> Option<CandidateExclusion> {
    let mut unslotted;
    let sig = if loop_generic {
        unslotted = sig.clone();
        unslotted.generics = syn::Generics::default();
        &unslotted
    } else {
        sig
    };
    if let Some(reason) = signature_exclusion(attrs, sig) {
        return Some(CandidateExclusion::Signature(reason));
    }
    if !loop_generic && owner.is_some_and(|block| !lifetimes_only(&block.generics)) {
        return Some(CandidateExclusion::GenericImpl);
    }
    None
}

pub(crate) fn signature_exclusion(
    attrs: &[syn::Attribute],
    sig: &syn::Signature,
) -> Option<&'static str> {
    struct Opaque(bool);
    impl<'ast> syn::visit::Visit<'ast> for Opaque {
        fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
            self.0 = true;
        }
    }
    let mut opaque = Opaque(false);
    syn::visit::Visit::visit_signature(&mut opaque, sig);
    let rules = [
        (opaque.0, "impl Trait"),
        (sig.constness.is_some(), "const"),
        (sig.asyncness.is_some(), "async"),
        (sig.unsafety.is_some(), "unsafe"),
        (sig.abi.is_some(), "extern ABI"),
        (sig.variadic.is_some(), "variadic"),
        (!lifetimes_only(&sig.generics), "function type/const generics"),
        (matches!(&sig.output, syn::ReturnType::Type(_, ty) if matches!(**ty, Type::Never(_))), "never return"),
        (sig.inputs.iter().any(|input| matches!(input, FnArg::Receiver(r) if r.reference.is_none() || r.colon_token.is_some())), "by-value/typed receiver"),
        (sig.inputs.iter().any(|input| match input { FnArg::Receiver(r) => !r.attrs.is_empty(), FnArg::Typed(t) => !t.attrs.is_empty() }), "parameter attributes"),
    ];
    rules
        .into_iter()
        .find_map(|(applies, name)| applies.then_some(name))
        .or_else(|| {
            attrs.iter().find_map(|attr| {
                let path = attr.path();
                if path.is_ident("cfg") || path.is_ident("cfg_attr") {
                    Some("conditional function")
                } else if path.is_ident("test") {
                    Some("test function")
                } else if path.is_ident("track_caller") {
                    Some("track_caller")
                } else if path.is_ident("inline")
                    && attr
                        .parse_args::<syn::Ident>()
                        .is_ok_and(|arg| arg == "always")
                {
                    Some("inline(always)")
                } else {
                    None
                }
            })
        })
}

/// Whether `generics` declares lifetimes only: a type or const parameter means one function per
/// instance, which one slot can't replace.
pub fn lifetimes_only(generics: &syn::Generics) -> bool {
    generics
        .params
        .iter()
        .all(|param| matches!(param, syn::GenericParam::Lifetime(_)))
}

/// The installer of the function `name`: `__fnr_<name>`, in a free function's module or on an
/// inherent method's type; `__fnr_as_<Trait>_<name>` on the type for a method of its
/// `impl Trait`.
pub fn installer(trait_name: Option<&syn::Ident>, name: &syn::Ident) -> syn::Ident {
    match trait_name {
        Some(trait_name) => {
            quote::format_ident!("__fnr_as_{trait_name}_{}", source::unraw(name))
        }
        None => quote::format_ident!("__fnr_{}", source::unraw(name)),
    }
}

pub(crate) use crate::replace_resolve::module_widened_glob_shadow_refusal;
use crate::replace_resolve::{AMBIGUOUS_OWNER, ResolvedTraitImpl, resolve_trait_impl};
pub use crate::replace_resolve::{OwnerTypes, ResolvedItem, TraitSlotEvidence, WorkspaceTraits};

/// The trait of `block` when its methods can be replaced: a workspace trait implemented safely
/// for a single-segment type declared in the impl's own crate.
pub fn replaceable_trait<'a>(
    module: &str,
    block: &'a syn::ItemImpl,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Option<&'a syn::Ident> {
    let (_, path, _) = block.trait_.as_ref()?;
    resolve_trait_impl(module, block, types, workspace)
        .ok()
        .map(|_| &path.segments.last().expect("a trait path has a name").ident)
}

pub(crate) fn resolve_trait_binding(
    module: &str,
    block: &syn::ItemImpl,
    method: &syn::Ident,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Result<TraitSlotEvidence, String> {
    let (_, path, _) = block
        .trait_
        .as_ref()
        .ok_or_else(|| "not a trait implementation".to_owned())?;
    let last = &path.segments.last().expect("a trait path has a name").ident;
    match resolve_trait_impl(module, block, types, workspace) {
        Ok(ResolvedTraitImpl { owner, trait_decl }) => Ok(TraitSlotEvidence {
            owner,
            trait_decl,
            installer_module: module.to_owned(),
            installer: installer(Some(last), method).to_string(),
            reader: None,
            cell: None,
        }),
        Err(reason) => Err(format_trait_refusal(block, reason)),
    }
}

fn trait_exclusion(
    module: &str,
    block: &syn::ItemImpl,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Option<&'static str> {
    block.trait_.as_ref()?;
    resolve_trait_impl(module, block, types, workspace).err()
}

fn format_trait_refusal(block: &syn::ItemImpl, reason: &'static str) -> String {
    if reason == AMBIGUOUS_OWNER {
        format!(
            "its trait implementation has no replacement installer: owner type name `{}` is ambiguous in the crate: more than one module declares it, and the `impl`'s module neither declares it nor imports it from one of them",
            quote::ToTokens::to_token_stream(&block.self_ty)
        )
    } else {
        format!("its trait implementation has no replacement installer ({reason})")
    }
}

impl OwnerTypes {
    /// What `files`, a crate's, say about its types.
    pub(crate) fn of(files: &[ModuleFile]) -> Result<Self, String> {
        let mut types = Self::default();
        for file in files {
            let text = fs::read_to_string(&file.path)
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            let parsed =
                syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.path.display()))?;
            types.declare(&file.module, &parsed.items);
        }
        Ok(types)
    }
}

/// What the files of `krate`, a crate below the game crate, say about its types: the files the
/// staging pass reads.
pub(crate) fn crate_owner_types(root: &Path, krate: &str) -> Result<OwnerTypes, String> {
    OwnerTypes::of(&source::crate_files(root, krate)?)
}

/// The module a method of a crate below the game crate is named under in a target: the module
/// that declares its `impl`'s type, the path `internals.txt` gives the type, in whichever file of
/// the crate the `impl` is. Moving an `impl` between the files of a module keeps its methods'
/// targets. A type written as a path, or a name that leads to no single type of the crate,
/// keeps the `impl`'s own module (`module`).
pub(crate) fn owner_module(types: &OwnerTypes, module: &str, self_ty: &Type) -> String {
    let Type::Path(ty) = self_ty else {
        return module.to_owned();
    };
    match &ty.path.segments.iter().collect::<Vec<_>>()[..] {
        [name] if ty.qself.is_none() => types
            .declaring_module(module, &source::unraw(&name.ident))
            .unwrap_or_else(|| module.to_owned()),
        _ => module.to_owned(),
    }
}

/// Why a mixin's target in the trait implementation `block` has no installer, for the refusal;
/// `None` when it has one.
#[cfg(test)]
pub(crate) fn trait_refusal(
    module: &str,
    block: &syn::ItemImpl,
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
) -> Option<String> {
    let reason = trait_exclusion(module, block, types, workspace)?;
    Some(format_trait_refusal(block, reason))
}

/// `sig` with each type it names through `Self::` (an associated type of the trait) written
/// `<Self as Trait>::`: the method's pointer type is written outside the trait's `impl`, where
/// the short form doesn't resolve.
pub fn qualified(sig: &syn::Signature, trait_path: &syn::Path) -> syn::Signature {
    struct Qualify<'a>(&'a syn::Path);
    impl syn::visit_mut::VisitMut for Qualify<'_> {
        fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
            if ty.qself.is_none()
                && ty.path.segments.len() > 1
                && ty.path.segments[0].ident == "Self"
            {
                let rest = ty.path.segments.iter().skip(1);
                let trait_path = self.0;
                *ty = syn::parse_quote!(<Self as #trait_path>::#(#rest)::*);
            }
            syn::visit_mut::visit_type_path_mut(self, ty);
        }
    }
    let mut sig = sig.clone();
    syn::visit_mut::VisitMut::visit_signature_mut(&mut Qualify(trait_path), &mut sig);
    sig
}

/// Whether `sig` still names something through `Self::` (an associated constant of the trait, in
/// an array length): that has no form outside the trait's `impl`.
pub(crate) fn names_associated(sig: &syn::Signature) -> bool {
    struct Find(bool);
    impl<'ast> syn::visit::Visit<'ast> for Find {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            self.0 |= path.segments.len() > 1 && path.segments[0].ident == "Self";
            syn::visit::visit_path(self, path);
        }
    }
    let mut find = Find(false);
    syn::visit::Visit::visit_signature(&mut find, sig);
    find.0
}

/// Elided lifetimes of a method's return borrow from its receiver: named.
struct ElideTo(syn::Lifetime);

impl syn::visit_mut::VisitMut for ElideTo {
    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if reference.lifetime.is_none() {
            reference.lifetime = Some(self.0.clone());
        }
        syn::visit_mut::visit_type_reference_mut(self, reference);
    }
    fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
        if lifetime.ident == "_" {
            *lifetime = self.0.clone();
        }
    }
    // A function pointer in the return elides on its own; a trait object's explicit `'_` bound
    // (`Box<dyn Read + '_>`) borrows from the receiver while trait-bound arguments (`dyn Fn(&T)`)
    // elide on their own.
    fn visit_type_bare_fn_mut(&mut self, _: &mut syn::TypeBareFn) {}
    fn visit_type_trait_object_mut(&mut self, object: &mut syn::TypeTraitObject) {
        for bound in &mut object.bounds {
            if let syn::TypeParamBound::Lifetime(lifetime) = bound {
                self.visit_lifetime_mut(lifetime);
            }
        }
    }
}

/// The function-pointer type of `sig`, as written inside its `impl`: the receiver as the first
/// parameter (`&Self` or `&mut Self`), the function's own lifetimes bound for any call, and a
/// return that borrows from the receiver as the method's elision says.
pub fn pointer_type(sig: &syn::Signature) -> proc_macro2::TokenStream {
    // The function's own lifetimes are renamed: a `for<>` binder may not shadow them where
    // they are in scope.
    struct Rename(Vec<(syn::Ident, syn::Lifetime)>);
    impl syn::visit_mut::VisitMut for Rename {
        fn visit_lifetime_mut(&mut self, lifetime: &mut syn::Lifetime) {
            if let Some((_, fresh)) = self.0.iter().find(|(name, _)| *name == lifetime.ident) {
                *lifetime = fresh.clone();
            }
        }
    }
    let mut rename = Rename(
        sig.generics
            .lifetimes()
            .enumerate()
            .map(|(index, param)| {
                (
                    param.lifetime.ident.clone(),
                    syn::Lifetime::new(&format!("'__fnr_l{index}"), proc_macro2::Span::call_site()),
                )
            })
            .collect(),
    );
    let mut lifetimes: Vec<syn::Lifetime> =
        rename.0.iter().map(|(_, fresh)| fresh.clone()).collect();
    let mut output = sig.output.clone();
    syn::visit_mut::VisitMut::visit_return_type_mut(&mut rename, &mut output);
    let first = sig.receiver().map(|written| {
        let named = written.reference.as_ref().and_then(|(_, lifetime)| {
            lifetime.clone().map(|mut lifetime| {
                syn::visit_mut::VisitMut::visit_lifetime_mut(&mut rename, &mut lifetime);
                lifetime
            })
        });
        let lifetime = named.unwrap_or_else(|| {
            let fresh = syn::Lifetime::new("'__fnr_s", proc_macro2::Span::call_site());
            lifetimes.push(fresh.clone());
            fresh
        });
        syn::visit_mut::VisitMut::visit_return_type_mut(
            &mut ElideTo(lifetime.clone()),
            &mut output,
        );
        let mutability = &written.mutability;
        quote!(&#lifetime #mutability Self,)
    });
    let types = sig.inputs.iter().filter_map(|input| match input {
        FnArg::Typed(typed) => {
            let mut ty = (*typed.ty).clone();
            syn::visit_mut::VisitMut::visit_type_mut(&mut rename, &mut ty);
            Some(ty)
        }
        FnArg::Receiver(_) => None,
    });
    let bound = if lifetimes.is_empty() {
        quote!()
    } else {
        quote!(for<#(#lifetimes),*>)
    };
    quote!(#bound fn(#first #(#types),*) #output)
}

/// A replaceable function, as found in a parsed file.
struct Found<'a> {
    module: String,
    /// The module its target names it under: `module`, or for a method of a crate below the
    /// game crate the module that declares its type ([`owner_module`]).
    named_in: String,
    attrs: &'a [syn::Attribute],
    sig: &'a syn::Signature,
    body: &'a syn::Block,
    /// The `impl` it is a method of.
    owner: Option<&'a syn::ItemImpl>,
    /// That `impl`'s trait, when it is one's.
    trait_name: Option<&'a syn::Ident>,
    /// Resolved owner and trait declarations for trait-method slots.
    trait_binding: Option<TraitSlotEvidence>,
    /// Where the item that holds it ends: the `impl` block, or the function.
    end: proc_macro2::LineColumn,
}

impl Found<'_> {
    /// Its path, as a mixin's `target` names it.
    fn target(&self) -> String {
        match (self.owner, self.trait_name) {
            (Some(block), Some(trait_name)) => format!(
                "{}::<{} as {trait_name}>::{}",
                self.named_in,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (Some(block), None) => format!(
                "{}::{}::{}",
                self.named_in,
                source::owner(&block.self_ty),
                self.sig.ident
            ),
            (None, _) => format!("{}::{}", self.module, self.sig.ident),
        }
    }
}

/// The replaceable functions among `items`: free functions, inherent methods and methods of
/// workspace traits (`workspace`) that every build compiles, inline modules included.
fn functions<'a>(
    module: &str,
    items: &'a [Item],
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
    out: &mut Vec<Found<'a>>,
) {
    for item in items {
        match item {
            Item::Mod(inline) if !mod_source::modules::gated(&inline.attrs) => {
                if let Some((_, inner)) = &inline.content
                    && !mod_source::modules::test_only(&inline.attrs)
                {
                    functions(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        types,
                        workspace,
                        out,
                    );
                }
            }
            Item::Fn(function)
                if !mod_source::modules::gated(&function.attrs)
                    && candidate_exclusion(&function.attrs, &function.sig, None, false)
                        .is_none() =>
            {
                out.push(Found {
                    module: module.to_owned(),
                    named_in: module.to_owned(),
                    attrs: &function.attrs,
                    sig: &function.sig,
                    body: &function.block,
                    owner: None,
                    trait_name: None,
                    trait_binding: None,
                    end: function.block.brace_token.span.close().end(),
                });
            }
            Item::Impl(block)
                if !mod_source::modules::gated(&block.attrs)
                    && matches!(&*block.self_ty, Type::Path(ty) if ty.qself.is_none()) =>
            {
                if !lifetimes_only(&block.generics) {
                    continue;
                }
                let (trait_name, resolved_trait) = match &block.trait_ {
                    Some((_, path, _)) => {
                        let Ok(resolved) = resolve_trait_impl(module, block, types, workspace)
                        else {
                            continue;
                        };
                        let last = &path.segments.last().expect("a trait path has a name").ident;
                        (Some(last), Some(resolved))
                    }
                    None => {
                        if module_widened_glob_shadow_refusal(module, types, workspace).is_some() {
                            continue;
                        }
                        (None, None)
                    }
                };
                for member in &block.items {
                    if let ImplItem::Fn(method) = member
                        && candidate_exclusion(&method.attrs, &method.sig, Some(block), false)
                            .is_none()
                        && !block.trait_.as_ref().is_some_and(|(_, path, _)| {
                            names_associated(&qualified(&method.sig, path))
                        })
                    {
                        let trait_binding =
                            resolved_trait.as_ref().map(|resolved| TraitSlotEvidence {
                                owner: resolved.owner.clone(),
                                trait_decl: resolved.trait_decl.clone(),
                                installer_module: module.to_owned(),
                                installer: installer(trait_name, &method.sig.ident).to_string(),
                                reader: None,
                                cell: None,
                            });
                        out.push(Found {
                            module: module.to_owned(),
                            named_in: module.to_owned(),
                            attrs: &method.attrs,
                            sig: &method.sig,
                            body: &method.block,
                            owner: Some(block),
                            trait_name,
                            trait_binding,
                            end: block.brace_token.span.close().end(),
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

/// Module-scope function definitions, partitioned by their first exclusion rule. Local functions
/// inside bodies and macro expansions are not source-level replacement candidates.
fn inventory(
    module: &str,
    items: &[Item],
    types: &OwnerTypes,
    workspace: &WorkspaceTraits,
    inherited: Option<&'static str>,
    counts: &mut BTreeMap<&'static str, usize>,
) {
    fn record(
        attrs: &[syn::Attribute],
        sig: &syn::Signature,
        inherited: Option<&'static str>,
        counts: &mut BTreeMap<&'static str, usize>,
    ) {
        let reason = inherited
            .or_else(|| signature_exclusion(attrs, sig))
            .unwrap_or("eligible");
        *counts.entry(reason).or_default() += 1;
    }
    for item in items {
        match item {
            Item::Mod(inline) => {
                if let Some((_, inner)) = &inline.content {
                    let reason = inherited.or_else(|| {
                        (mod_source::modules::gated(&inline.attrs)
                            || mod_source::modules::test_only(&inline.attrs))
                        .then_some("conditional/test module")
                    });
                    inventory(
                        &format!("{module}::{}", inline.ident),
                        inner,
                        types,
                        workspace,
                        reason,
                        counts,
                    );
                }
            }
            Item::Fn(function) => record(&function.attrs, &function.sig, inherited, counts),
            Item::Impl(block) => {
                let reason = inherited.or_else(|| {
                    if mod_source::modules::gated(&block.attrs) {
                        Some("conditional impl")
                    } else if !lifetimes_only(&block.generics) {
                        Some("impl type/const generics")
                    } else if !matches!(&*block.self_ty, Type::Path(ty) if ty.qself.is_none()) {
                        Some("non-path/qualified impl owner")
                    } else if block.trait_.is_some() {
                        trait_exclusion(module, block, types, workspace)
                    } else {
                        module_widened_glob_shadow_refusal(module, types, workspace)
                    }
                });
                for member in &block.items {
                    if let ImplItem::Fn(method) = member {
                        let reason = reason.or_else(|| {
                            block
                                .trait_
                                .as_ref()
                                .filter(|(_, path, _)| {
                                    names_associated(&qualified(&method.sig, path))
                                })
                                .map(|_| "associated constant signature")
                        });
                        record(&method.attrs, &method.sig, reason, counts);
                    }
                }
            }
            Item::Trait(item) => {
                for member in &item.items {
                    if let syn::TraitItem::Fn(method) = member
                        && method.default.is_some()
                    {
                        record(
                            &method.attrs,
                            &method.sig,
                            Some("trait default body"),
                            counts,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Marked lower-crate functions and their source locations.
pub(crate) fn marked(
    root: &Path,
    layout: source::SourceLayout,
) -> Result<Vec<(String, String)>, String> {
    marked_by(root, layout, true)
}

/// [`marked`], with each method under the module of the file its `impl` is in rather than its
/// type's: where the item is, for a pass that reads it there.
pub(crate) fn marked_items(
    root: &Path,
    layout: source::SourceLayout,
) -> Result<Vec<(String, String)>, String> {
    marked_by(root, layout, false)
}

fn marked_by(
    root: &Path,
    layout: source::SourceLayout,
    by_type: bool,
) -> Result<Vec<(String, String)>, String> {
    fn scan(
        module: &str,
        items: &[Item],
        types: Option<&OwnerTypes>,
        out: &mut Vec<(String, String)>,
    ) -> Result<(), String> {
        for item in items {
            match item {
                Item::Fn(f) if source::marker(&f.attrs, "replaceable").is_some() => {
                    let path = format!("{module}::{}", f.sig.ident);
                    if let Some(reason) = signature_exclusion(&f.attrs, &f.sig) {
                        return Err(format!(
                            "#[sunrise_mod_api_macros::replaceable] on {path}: {reason}"
                        ));
                    }
                    out.push((
                        format!("#[sunrise_mod_api_macros::replaceable] on {path}"),
                        path,
                    ));
                }
                Item::Impl(block) => {
                    for member in &block.items {
                        if let ImplItem::Fn(f) = member
                            && source::marker(&f.attrs, "replaceable").is_some()
                        {
                            let path = source::plain_path(&format!(
                                "{}::{}::{}",
                                types.map_or_else(
                                    || module.to_owned(),
                                    |types| owner_module(types, module, &block.self_ty)
                                ),
                                source::owner(&block.self_ty),
                                f.sig.ident
                            ));
                            let reason =
                                match candidate_exclusion(&f.attrs, &f.sig, Some(block), false) {
                                    Some(CandidateExclusion::Signature(reason)) => Some(reason),
                                    Some(CandidateExclusion::GenericImpl) => {
                                        Some("impl type/const generics")
                                    }
                                    None => None,
                                }
                                .or_else(|| {
                                    mod_source::modules::gated(&block.attrs)
                                        .then_some("conditional impl")
                                });
                            if let Some(reason) = reason {
                                return Err(format!(
                                    "#[sunrise_mod_api_macros::replaceable] on {path}: {reason}"
                                ));
                            }
                            out.push((
                                format!("#[sunrise_mod_api_macros::replaceable] on {path}"),
                                path,
                            ));
                        }
                    }
                }
                Item::Mod(m) => {
                    if let Some((_, items)) = &m.content {
                        scan(&format!("{module}::{}", m.ident), items, types, out)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let crates = root.join("crates");
    if !crates.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(&crates).map_err(|e| e.to_string())? {
        let package = entry.map_err(|e| e.to_string())?.path();
        if !package.join("Cargo.toml").exists() {
            continue;
        }
        let name = package
            .file_name()
            .unwrap()
            .to_string_lossy()
            .replace('-', "_");
        if layout.domain(&name).is_some() {
            continue;
        }
        let manifest: toml::Value = toml::from_str(
            &fs::read_to_string(package.join("Cargo.toml")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let lib = package.join(
            manifest
                .get("lib")
                .and_then(|v| v.get("path"))
                .and_then(toml::Value::as_str)
                .unwrap_or("src/lib.rs"),
        );
        if !lib.exists() {
            continue;
        }
        let files = source::module_files(
            root,
            name.clone(),
            lib.clone(),
            lib.parent().unwrap().to_owned(),
            true,
        )?;
        let mut marking = Vec::new();
        for file in &files {
            let text = fs::read_to_string(&file.path).map_err(|e| e.to_string())?;
            if text.contains("replaceable") {
                let parsed =
                    syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.path.display()))?;
                marking.push((&file.module, parsed.items));
            }
        }
        if marking.is_empty() {
            continue;
        }
        // A method is named by its type's module, which may be another file's.
        let types = by_type.then(|| OwnerTypes::of(&files)).transpose()?;
        for (module, items) in &marking {
            scan(module, items, types.as_ref(), &mut out)?;
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// The refusal of a target below the game crate without the marker.
pub const UNMARKED: &str = "a function below the game crate takes a replacement only when it carries #[sunrise_mod_api_macros::replaceable]; ask for it in an issue or pull request on the game";

/// What [`stage`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Functions given a replacement slot.
    pub functions: usize,
    /// Files rewritten.
    pub files: usize,
    /// Crates given a slot array.
    pub crates: usize,
    /// The functions given a slot, by their key in the function index.
    pub slots: BTreeSet<String>,
    /// Per-slot resolved owner, trait, and installer module for every trait-method slot.
    pub trait_slots: BTreeMap<String, TraitSlotEvidence>,
}

/// The library name of the package at `package`: its `[lib]` name, or its own with underscores.
pub(crate) fn library_name(package: &Path) -> Result<String, String> {
    let manifest = package.join("Cargo.toml");
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let cargo: toml::Table =
        toml::from_str(&text).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let name = |table: &str| {
        cargo
            .get(table)
            .and_then(|table| table.get("name"))
            .and_then(toml::Value::as_str)
    };
    name("lib")
        .or(name("package"))
        .map(|name| name.replace('-', "_"))
        .ok_or_else(|| format!("{}: no package name", manifest.display()))
}

/// One crate's files in the staged copy.
struct Crate {
    /// Its library's name, as the game crate names it.
    name: String,
    /// Its library root, where its slot array goes.
    lib: PathBuf,
    files: Vec<ModuleFile>,
    /// Whether only its marked functions are replaceable.
    marked_only: bool,
    /// Its package folder.
    package: PathBuf,
}

/// The staging pass: in the staged copy at `root`, in place, gives every replaceable function its
/// head check, reader and installer, and each crate that has one its slot array. Run on a copy,
/// never on a working tree.
pub fn stage(root: &Path) -> Result<Staged, String> {
    stage_with_context(root, source::SourceLayout::read(root)?, None, None)
}

pub(crate) fn stage_with_context(
    root: &Path,
    layout: source::SourceLayout,
    mut pristine_owners: Option<BTreeMap<String, OwnerTypes>>,
    workspace: Option<&WorkspaceTraits>,
) -> Result<Staged, String> {
    let owned_workspace;
    let workspace = match workspace {
        Some(workspace) => workspace,
        None => {
            owned_workspace = WorkspaceTraits::load(root)?;
            &owned_workspace
        }
    };
    let list = marked(root, layout)?;
    let mut used = vec![false; list.len()];
    let game = layout.runtime.package_path(root);
    let mut crates = Vec::new();
    for domain in layout.domains {
        let mut files = domain.files(root, true)?;
        if crate::GameplayPackage::from_library(domain.key) == Some(crate::GameplayPackage::Client)
        {
            // Client builds can opt in to slots in modules gated only by the client feature.
            files.extend(
                domain
                    .files(root, false)?
                    .into_iter()
                    .filter(|file| file.cfg == ["feature = \"client\""] && !file.included),
            );
        }
        crates.push(Crate {
            name: domain.key.into(),
            lib: domain.library(root)?,
            files,
            marked_only: false,
            package: domain.package_path(root),
        });
    }
    let lower: BTreeSet<&str> = list
        .iter()
        .map(|(_, path)| path.split("::").next().unwrap_or_default())
        .collect();
    for name in lower {
        if layout.domain(name).is_some() {
            return Err(format!(
                "{name}: every function of the game crate's game module is replaceable already"
            ));
        }
        let (package, lib) = source::crate_root(root, name)?;
        // The game crate links every marked crate; a dependency back on the game would cycle.
        if built_on(&package, &game)? {
            return Err(format!(
                "{name}: it is built on sunrise-client, so the game crate can't link its replacement slots; a crate with #[sunrise_mod_api_macros::replaceable] must sit below the game crate"
            ));
        }
        crates.push(Crate {
            name: library_name(&package)?,
            lib,
            files: source::crate_files(root, name)?,
            marked_only: true,
            package,
        });
    }
    let mut staged = Staged::default();
    let mut runtime_slots = Vec::new();
    for krate in &crates {
        let mut count = 0;
        let mut cells = 0;
        let mut trait_methods = 0;
        let mut counts = BTreeMap::new();
        // Each file's text as the pass leaves it, the crate root's included.
        let mut texts: BTreeMap<&Path, String> = BTreeMap::new();
        let mut parsed = Vec::new();
        let reused_pristine = pristine_owners
            .as_mut()
            .and_then(|map| map.remove(&krate.name));
        let has_pristine = reused_pristine.is_some();
        let mut types = reused_pristine.unwrap_or_default();
        if !has_pristine && !krate.marked_only && krate.lib.is_file() {
            let text = fs::read_to_string(&krate.lib)
                .map_err(|e| format!("{}: {e}", krate.lib.display()))?;
            let items = syn::parse_file(&text)
                .map_err(|e| format!("{}: {e}", krate.lib.display()))?
                .items;
            types.declare(&krate.name, &items);
        }
        for file in &krate.files {
            let text = fs::read_to_string(&file.path)
                .map_err(|e| format!("{}: {e}", file.path.display()))?;
            if !file.cfg.is_empty() && !text.contains("replaceable") {
                continue;
            }
            let items = syn::parse_file(&text)
                .map_err(|e| format!("{}: {e}", file.path.display()))?
                .items;
            if has_pristine {
                types.declare_missing_inline(&file.module, &items);
            } else {
                types.declare(&file.module, &items);
            }
            parsed.push((file, text, items));
        }
        for (file, text, items) in &parsed {
            if !krate.marked_only && file.cfg.is_empty() {
                inventory(&file.module, items, &types, workspace, None, &mut counts);
            }
            let mut found = Vec::new();
            functions(&file.module, items, &types, workspace, &mut found);
            if !file.cfg.is_empty() {
                found.retain(|function| source::marker(function.attrs, "replaceable").is_some());
                *counts.entry("eligible").or_default() += found.len();
            }
            if krate.marked_only {
                for found in &mut found {
                    if let Some(block) = found.owner {
                        found.named_in = owner_module(&types, &found.module, &block.self_ty);
                    }
                }
                found.retain(|found| {
                    let target = source::plain_path(&found.target());
                    list.iter()
                        .position(|(_, path)| *path == target)
                        .inspect(|&index| used[index] = true)
                        .is_some()
                });
            }
            let Some(text) = rewrite(&file.path, text, &mut found, &mut cells)? else {
                continue;
            };
            for function in &found {
                let key = crate::functions::key(&function.target());
                if let Some(binding) = &function.trait_binding {
                    staged.trait_slots.insert(key.clone(), binding.clone());
                }
                staged.slots.insert(key);
            }
            trait_methods += found
                .iter()
                .filter(|function| function.trait_name.is_some())
                .count();
            count += found.len();
            staged.files += 1;
            texts.insert(&file.path, text);
        }
        eprintln!(
            "replacement slots: {}: total={count}, trait_methods={trait_methods}",
            krate.lib.display()
        );
        if !krate.marked_only {
            debug_assert_eq!(counts.get("eligible").copied().unwrap_or_default(), count);
            eprintln!(
                "replacement exclusions (first rule, module-scope definitions): {}",
                serde_json::to_string(&counts).expect("static string counts serialize")
            );
        }
        if count == 0 {
            continue;
        }
        if layout.is_split() {
            let package = krate
                .package
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let client = client_slot_package(package);
            runtime_slots.push((krate.name.as_str(), client));
        }
        let root_text = match texts.remove(krate.lib.as_path()) {
            Some(text) => text,
            None => fs::read_to_string(&krate.lib)
                .map_err(|e| format!("{}: {e}", krate.lib.display()))?,
        };
        // The generated readers cast a slot's pointer back to its function's type: the staged
        // crate allows that cast by name where the source forbids unsafe code outright.
        let mut root_text =
            root_text.replacen("#![forbid(unsafe_code)]", "#![deny(unsafe_code)]", 1);
        root_text.push_str(&format!(
            "\n/// The replacement of each replaceable function of this crate, or null: see `sunrise_mod_api_gen::replace`.\n#[doc(hidden)]\npub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; {cells}] = [const {{ ::std::sync::atomic::AtomicPtr::new(::std::ptr::null_mut()) }}; {cells}];\n"
        ));
        // The host must export lower-crate replacement slots even if the game never calls
        // them directly. An exported reference keeps them linked without work at startup.
        let linked: Vec<_> = crates
            .iter()
            .filter(|other| krate.package == game && other.marked_only)
            .map(|other| format!("&::{}::__REPLACE", other.name))
            .collect();
        if !linked.is_empty() {
            root_text.push_str(&format!(
                "#[doc(hidden)]\npub static __FNR_LOWER_SLOTS: [&[::std::sync::atomic::AtomicPtr<()>]; {}] = [{}];\n",
                linked.len(),
                linked.join(", ")
            ));
        }
        texts.insert(&krate.lib, root_text);
        for (path, text) in texts {
            fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        staged.functions += count;
        staged.crates += 1;
    }
    if let Some(((origin, path), _)) = list.iter().zip(&used).find(|(_, used)| !**used) {
        return Err(format!(
            "{origin}: {path}: no free function or inherent method every build compiles that can be replaced"
        ));
    }
    if !runtime_slots.is_empty() {
        let shell = layout.runtime.library(root)?;
        let mut text = fs::read_to_string(&shell).map_err(|error| error.to_string())?;
        text.push_str(&runtime_slot_anchor(&runtime_slots));
        fs::write(&shell, text).map_err(|error| error.to_string())?;
        if root.join("crates/sunrise-mod-host/Cargo.toml").is_file() {
            let (_, host) = source::crate_root(root, "sunrise_mod_host")?;
            let mut text = fs::read_to_string(&host).map_err(|error| error.to_string())?;
            text.push_str("\n#[doc(hidden)] pub use sunrise_client::__FNR_RUNTIME_SLOTS;\n");
            fs::write(&host, text).map_err(|error| error.to_string())?;
        }
    }
    let below: Vec<&Path> = crates
        .iter()
        .filter(|krate| krate.marked_only)
        .map(|krate| krate.package.as_path())
        .collect();
    link_dependencies(root, &game, &below, layout.is_split())?;
    Ok(staged)
}

fn client_slot_package(name: &str) -> bool {
    matches!(
        name,
        "sunrise-client-gameplay"
            | "sunrise-client-lib"
            | "sunrise-client-render"
            | "sunrise-mod-render"
    )
}

#[cfg(test)]
#[test]
fn client_feature_modules_stage_only_marked_methods() {
    let tree = tempfile::tempdir().unwrap();
    let root = tree.path();
    const CLIENT: source::GameDomain = crate::GAMEPLAY_LAYOUT.domains[2];
    let package = CLIENT.package_path(root);
    fs::create_dir_all(package.join("src/game")).unwrap();
    fs::write(
        package.join("Cargo.toml"),
        "[package]\nname = 'sunrise-client-gameplay'\n",
    )
    .unwrap();
    fs::write(package.join("src/lib.rs"), "pub mod game;\n").unwrap();
    fs::write(
        package.join("src/game.rs"),
        r#"
        #[cfg(feature = "client")] mod particles;
        #[cfg(feature = "extra")] mod extra;
        pub struct ClientGame;
        "#,
    )
    .unwrap();
    for name in ["particles", "extra"] {
        fs::write(
            package.join(format!("src/game/{name}.rs")),
            format!(
                "use super::ClientGame;
                 impl ClientGame {{
                     #[sunrise_mod_api_macros::replaceable]
                     fn {name}(&mut self) {{}}
                     fn {name}_helper(&mut self) {{}}
                 }}"
            ),
        )
        .unwrap();
    }
    let layout = source::SourceLayout {
        runtime: CLIENT,
        domains: &[CLIENT],
        entry: None,
    };
    let staged = stage_with_context(root, layout, None, None).unwrap();
    assert_eq!(
        staged.slots,
        ["sunrise_client_gameplay::game::particles::ClientGame::particles".to_owned()].into()
    );
}

fn runtime_slot_anchor(slots: &[(&str, bool)]) -> String {
    let entries = slots
        .iter()
        .map(|(name, client)| {
            let gate = if *client {
                "#[cfg(feature = \"client\")] "
            } else {
                ""
            };
            format!("{gate}&::{name}::__REPLACE,")
        })
        .collect::<String>();
    format!(
        "\n#[doc(hidden)]\n#[used]\npub static __FNR_RUNTIME_SLOTS: &[&[::std::sync::atomic::AtomicPtr<()>]] = &[{entries}];\n"
    )
}

#[cfg(test)]
#[test]
fn the_split_sdk_anchor_only_names_client_slots_in_client_builds() {
    let code = runtime_slot_anchor(&[
        ("sunrise_gameplay_common", false),
        ("sunrise_gameplay", false),
        ("sunrise_client_gameplay", true),
        ("sunrise_sim", false),
    ]);
    syn::parse_file(&code).unwrap();
    assert!(code.contains("&::sunrise_gameplay_common::__REPLACE,&::sunrise_gameplay::__REPLACE,"));
    assert!(code.contains("#[cfg(feature = \"client\")] &::sunrise_client_gameplay::__REPLACE,&::sunrise_sim::__REPLACE,"));
}

/// `path` with its `..` and `.` resolved, as written: the staged copy has no links to follow.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A package's manifest, parsed.
pub(crate) fn manifest(package: &Path) -> Result<toml::Table, String> {
    let path = package.join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Whether `package` is built on `game` through its path dependencies (normal and build ones).
fn built_on(package: &Path, game: &Path) -> Result<bool, String> {
    let game = lexical(game);
    let mut pending = vec![lexical(package)];
    let mut seen = BTreeSet::new();
    while let Some(next) = pending.pop() {
        if !seen.insert(next.clone()) {
            continue;
        }
        let cargo = manifest(&next)?;
        for kind in ["dependencies", "build-dependencies"] {
            let Some(table) = cargo.get(kind).and_then(toml::Value::as_table) else {
                continue;
            };
            for dependency in table.values() {
                if let Some(path) = dependency.get("path").and_then(toml::Value::as_str) {
                    let folder = lexical(&next.join(path));
                    if folder == game {
                        return Ok(true);
                    }
                    pending.push(folder);
                }
            }
        }
    }
    Ok(false)
}

/// Give the staged game crate a path dependency on each crate with replacement slots, in its manifest and its
/// lock entry: its source manifest needn't name a crate it reaches only through slots.
fn link_dependencies(
    root: &Path,
    game: &Path,
    linked: &[&Path],
    split: bool,
) -> Result<(), String> {
    if linked.is_empty() {
        return Ok(());
    }
    let mut cargo = manifest(game)?;
    let game_name = cargo
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| format!("{}: no package name", game.display()))?
        .to_owned();
    let dependencies = cargo
        .entry("dependencies")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("[dependencies] is not a table")?;
    let mut added = Vec::new();
    let mut client = Vec::new();
    for package in linked {
        let name = manifest(package)?
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("{}: no package name", package.display()))?
            .to_owned();
        // Kept under whatever name the game crate already gives it.
        let present = dependencies.iter().any(|(key, dependency)| {
            *key == name
                || dependency.get("package").and_then(toml::Value::as_str) == Some(name.as_str())
        });
        if present {
            continue;
        }
        let folder = package
            .file_name()
            .ok_or_else(|| format!("{}: no folder name", package.display()))?
            .to_string_lossy()
            .into_owned();
        let mut dependency = toml::toml! { path = (format!("../{folder}")) };
        if split && client_slot_package(&name) {
            dependency.insert("optional".into(), true.into());
            client.push(toml::Value::String(format!("dep:{name}")));
        }
        dependencies.insert(name.clone(), dependency.into());
        added.push(name);
    }
    if added.is_empty() {
        return Ok(());
    }
    if !client.is_empty() {
        cargo
            .get_mut("features")
            .and_then(toml::Value::as_table_mut)
            .and_then(|features| features.get_mut("client"))
            .and_then(toml::Value::as_array_mut)
            .ok_or("the split shell has no client feature")?
            .extend(client);
    }
    let lock_path = root.join("Cargo.lock");
    // A Windows checkout has CRLF line endings; Cargo reads either, and the entries match on LF.
    let lock = fs::read_to_string(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?
        .replace("\r\n", "\n");
    let added: Vec<&str> = added.iter().map(String::as_str).collect();
    let lock = locked_with(&lock, &game_name, &added)
        .ok_or_else(|| format!("Cargo.lock has no entry for {game_name}"))?;
    let path = game.join("Cargo.toml");
    fs::write(&path, toml::to_string(&cargo).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    fs::write(&lock_path, lock).map_err(|e| format!("{}: {e}", lock_path.display()))
}

fn locked_with(lock: &str, name: &str, added: &[&str]) -> Option<String> {
    let header = format!("[[package]]\nname = \"{name}\"\n");
    let start = lock.find(&header)?;
    let end = lock[start + header.len()..]
        .find("\n\n")
        .map_or(lock.len(), |at| start + header.len() + at);
    let entry = &lock[start..end];
    let mut lines: Vec<String> = entry.lines().map(str::to_owned).collect();
    let mut dependencies: Vec<String> = Vec::new();
    let open = lines.iter().position(|line| line == "dependencies = [");
    if let Some(open) = open {
        let close = open + lines[open..].iter().position(|line| line == "]")?;
        dependencies = lines.drain(open + 1..close).collect();
    }
    for name in added {
        let quoted = format!(" \"{name}\",");
        if !dependencies.contains(&quoted) {
            dependencies.push(quoted);
        }
    }
    dependencies.sort();
    let open = match open {
        Some(open) => open,
        None => {
            // `dependencies` follows `version` and `source`, before `checksum`.
            let at = lines
                .iter()
                .position(|line| line.starts_with("checksum"))
                .unwrap_or(lines.len());
            lines.insert(at, "dependencies = [".to_owned());
            lines.insert(at + 1, "]".to_owned());
            at
        }
    };
    for (offset, dependency) in dependencies.into_iter().enumerate() {
        lines.insert(open + 1 + offset, dependency);
    }
    Some(format!(
        "{}{}{}",
        &lock[..start],
        lines.join("\n"),
        &lock[end..]
    ))
}

/// `text` with the head check, reader and installer of each of `found`, numbering their cells
/// from `cells` and answering `replaced!(f)` from `f`'s slot. None when there is nothing to stage.
fn rewrite(
    path: &Path,
    text: &str,
    found: &mut [Found<'_>],
    cells: &mut usize,
) -> Result<Option<String>, String> {
    if found.is_empty() {
        return Ok(None);
    }
    let lines: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let at = |position| source::offset(text, &lines, position);
    // Replacements of byte ranges (an insertion is an empty range), applied in order.
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    let mut names = BTreeSet::new();
    let mut free = BTreeMap::new();
    for function in found {
        let slot = *cells;
        *cells += 1;
        let name = source::unraw(&function.sig.ident);
        let installer = installer(function.trait_name, &function.sig.ident);
        let reader = quote::format_ident!("__fnr_read_slot_{slot}");
        if let Some(binding) = &mut function.trait_binding {
            binding.reader = Some(reader.to_string());
            binding.cell = Some(slot);
        }
        if function.owner.is_none() {
            free.insert(name.clone(), reader.to_string());
        }
        let scope = match function.owner {
            Some(block) => format!("{}::{installer}", source::owner(&block.self_ty)),
            None => name.clone(),
        };
        if !names.insert((function.module.clone(), scope)) {
            return Err(format!(
                "{}: {} is declared twice in one module: one of them can't be replaced",
                path.display(),
                function.target()
            ));
        }
        // The copy is called with the function's own arguments: a parameter bound by a pattern
        // is taken by a plain name first, and bound as written after the check.
        let mut arguments = Vec::new();
        let mut rebind = String::new();
        for (index, input) in function.sig.inputs.iter().enumerate() {
            match input {
                FnArg::Receiver(_) => arguments.push("self".to_owned()),
                FnArg::Typed(typed) => match &*typed.pat {
                    Pat::Ident(pat) if pat.by_ref.is_none() && pat.subpat.is_none() => {
                        arguments.push(pat.ident.to_string());
                    }
                    pat => {
                        let plain = format!("__fnr_p{index}");
                        let span = syn::spanned::Spanned::span(pat);
                        let (start, end) = (at(span.start()), at(span.end()));
                        if !matches!(pat, Pat::Wild(_)) {
                            rebind.push_str(&format!(" let {} = {plain};", &text[start..end]));
                        }
                        edits.push((start, end, plain.clone()));
                        arguments.push(plain);
                    }
                },
            }
        }
        let call = if function.owner.is_some() {
            format!("Self::{reader}()")
        } else {
            format!("{reader}()")
        };
        let open = at(function.body.brace_token.span.open().end());
        edits.push((
            open,
            open,
            format!(
                " if let ::std::option::Option::Some(__fnr) = {call} {{ ::core::hint::cold_path(); return __fnr({}); }}{rebind}",
                arguments.join(", ")
            ),
        ));
        let ty = match function.owner.and_then(|block| block.trait_.as_ref()) {
            Some((_, path, _)) if function.trait_name.is_some() => {
                pointer_type(&qualified(function.sig, path))
            }
            _ => pointer_type(function.sig),
        };
        let entry = self::reader(&reader, slot, &ty);
        let items = quote! {
            #entry
            #[doc(hidden)]
            #[inline]
            #[allow(dead_code, non_snake_case, clippy::all)]
            pub fn #installer(replacement: #ty) -> bool {
                crate::__REPLACE[#slot]
                    .compare_exchange(
                        ::std::ptr::null_mut(),
                        replacement as *mut (),
                        ::std::sync::atomic::Ordering::Release,
                        ::std::sync::atomic::Ordering::Relaxed,
                    )
                    .is_ok()
            }
        };
        let items = match function.owner {
            Some(block) => {
                let (generics, _, clause) = block.generics.split_for_impl();
                let ty = &block.self_ty;
                quote!(impl #generics #ty #clause { #items })
            }
            None => items,
        };
        let end = at(function.end);
        edits.push((end, end, format!(" {items}")));
    }
    // The placeholder macro has no callers after its invocations are rewritten. Remove its
    // declaration too, keeping line endings so source locations still match the pinned tree.
    let parsed = syn::parse_file(text).map_err(|e| format!("{}: {e}", path.display()))?;
    for item in parsed.items {
        if let Item::Macro(item) = item
            && item.mac.path.is_ident("macro_rules")
            && item.ident.as_ref().is_some_and(|name| name == "replaced")
        {
            let span = syn::spanned::Spanned::span(&item);
            let (start, end) = (at(span.start()), at(span.end()));
            edits.push((
                start,
                end,
                text[start..end].chars().filter(|c| *c == '\n').collect(),
            ));
        }
    }
    // `replaced!(f)`: whether a copy of the free function `f` of this module is installed.
    let mut from = 0;
    while let Some(start) = text[from..].find("replaced!(").map(|i| i + from) {
        let inner = start + "replaced!(".len();
        let Some(close) = text[inner..].find(')').map(|i| i + inner) else {
            break;
        };
        from = close;
        let name = text[inner..close].trim();
        let Some(reader) = free.get(name) else {
            return Err(format!(
                "{}: replaced!({name}): no replaceable free function of that name in this file",
                path.display()
            ));
        };
        edits.push((start, close + 1, format!("{reader}().is_some()")));
    }
    // At one offset, a replacement of a range before an insertion there, each in pushed order.
    edits.sort_by_key(|(start, end, _)| (*start, *end == *start));
    let mut out =
        String::with_capacity(text.len() + edits.iter().map(|e| e.2.len()).sum::<usize>());
    let mut from = 0;
    for (start, end, replacement) in edits {
        if start < from {
            return Err(format!("{}: overlapping staging edits", path.display()));
        }
        out.push_str(&text[from..start]);
        out.push_str(&replacement);
        from = end;
    }
    out.push_str(&text[from..]);
    debug_assert_eq!(out.matches('\n').count(), text.matches('\n').count());
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(module: &str, file: &syn::File, types: &OwnerTypes) -> Vec<String> {
        let mut found = Vec::new();
        functions(
            module,
            &file.items,
            types,
            &WorkspaceTraits::empty(),
            &mut found,
        );
        found.iter().map(Found::target).collect()
    }

    fn staged(source: &str) -> String {
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &OwnerTypes::default(),
            &WorkspaceTraits::empty(),
            &mut found,
        );
        let mut count = 0;
        rewrite(Path::new("k.rs"), source, &mut found, &mut count)
            .unwrap()
            .unwrap_or_else(|| source.to_owned())
    }

    fn eligible_fn(source: &str) -> bool {
        let function: syn::ItemFn = syn::parse_str(source).unwrap();
        eligible(&function.attrs, &function.sig)
    }

    #[test]
    fn hook_functions_are_replaceable_and_single_instance_rules_refuse() {
        assert!(eligible_fn("fn native_tick(&mut self) {}"));
        assert!(eligible_fn(
            "fn borrow<'a>(&'a self, key: &str) -> &'a str { key }"
        ));
        for refused in [
            "const fn f() {}",
            "async fn f() {}",
            "unsafe fn f() {}",
            "fn f<T>(value: T) {}",
            "fn f<const N: usize>() {}",
            "fn f(value: impl Clone) {}",
            "fn f() -> impl Clone {}",
            "fn f(self) {}",
            "fn f(self: Box<Self>) {}",
            "fn f() -> ! { loop {} }",
            "#[inline(always)] fn f() {}",
            "#[track_caller] fn f() {}",
            "#[cfg(unix)] fn f() {}",
        ] {
            assert!(!eligible_fn(refused), "{refused}");
        }
    }

    #[test]
    fn a_method_checks_its_slot_first_and_forwards_its_own_arguments() {
        let out = staged(
            "impl Game<'_> {\n    fn next(&mut self, (a, b): (u32, u32), mut c: u8, _: bool) -> u32 {\n        a + b\n    }\n}\n",
        );
        assert!(
            out.contains(
                "fn next(&mut self, __fnr_p1: (u32, u32), mut c: u8, __fnr_p3: bool) -> u32 { if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { ::core::hint::cold_path(); return __fnr(self, __fnr_p1, c, __fnr_p3); } let (a, b) = __fnr_p1;\n"
            ),
            "{out}"
        );
        // The reader and installer sit in an `impl` of the same type, after the method's own.
        let file = syn::parse_file(&out).unwrap();
        let Item::Impl(added) = &file.items[1] else {
            panic!("{out}")
        };
        assert_eq!(source::owner(&added.self_ty), "Game<'_>");
        let names: Vec<_> = added
            .items
            .iter()
            .map(|item| match item {
                ImplItem::Fn(function) => function.sig.ident.to_string(),
                _ => panic!("{out}"),
            })
            .collect();
        assert_eq!(names, ["__fnr_read_slot_0", "__fnr_next"]);
        assert_eq!(out.lines().count(), 5);
    }

    #[test]
    fn the_pointer_type_binds_the_functions_lifetimes_and_the_receivers_borrow() {
        let method: syn::ImplItemFn =
            syn::parse_str("fn name<'a>(&self, key: &'a str, other: &Self) -> &str { key }")
                .unwrap();
        assert_eq!(
            pointer_type(&method.sig).to_string(),
            quote!(
                for<'__fnr_l0, '__fnr_s> fn(&'__fnr_s Self, &'__fnr_l0 str, &Self) -> &'__fnr_s str
            )
            .to_string()
        );
        let free: syn::ItemFn = syn::parse_str("fn interval(rate: f32) -> i64 { 0 }").unwrap();
        assert_eq!(
            pointer_type(&free.sig).to_string(),
            quote!(fn(f32) -> i64).to_string()
        );
        let trait_obj: syn::ImplItemFn = syn::parse_str(
            "fn open(&self, id: &str) -> Result<Box<dyn Read + '_>, Err> { todo!() }",
        )
        .unwrap();
        assert_eq!(
            pointer_type(&trait_obj.sig).to_string(),
            quote!(for<'__fnr_s> fn(&'__fnr_s Self, &str) -> Result<Box<dyn Read + '__fnr_s>, Err>)
                .to_string()
        );
        let fn_trait_obj: syn::ImplItemFn = syn::parse_str(
            "fn mapper(&self, a: &str) -> Box<dyn Fn(&str) -> &str + '_> { todo!() }",
        )
        .unwrap();
        assert_eq!(
            pointer_type(&fn_trait_obj.sig).to_string(),
            quote!(for<'__fnr_s> fn(&'__fnr_s Self, &str) -> Box<dyn Fn(&str) -> &str + '__fnr_s>)
                .to_string()
        );
    }

    #[test]
    fn inventory_partitions_definitions_by_the_actual_eligibility_rules() {
        let file = syn::parse_file("fn free() {} const fn constant() {} fn generic<T>() {} struct T; trait Owned { fn required(&self); fn default(&self) {} } impl Owned for T { fn required(&self) {} } #[cfg(test)] mod tests { fn helper() {} } impl T { fn consume(self) {} }").unwrap();
        let workspace = WorkspaceTraits::empty();
        let mut types = OwnerTypes::default();
        types.declare("k", &file.items);
        let mut counts = BTreeMap::new();
        inventory("k", &file.items, &types, &workspace, None, &mut counts);
        assert_eq!(counts.values().sum::<usize>(), 7);
        assert_eq!(
            counts,
            BTreeMap::from([
                ("eligible", 2),
                ("const", 1),
                ("function type/const generics", 1),
                ("trait default body", 1),
                ("conditional/test module", 1),
                ("by-value/typed receiver", 1),
            ])
        );
        let mut found = Vec::new();
        functions("k", &file.items, &types, &workspace, &mut found);
        assert_eq!(found.len(), counts["eligible"]);
    }

    #[test]
    #[ignore = "daily"]
    fn trait_installers_compile_with_warnings_denied() {
        let source = "pub struct Example; pub trait GameTrait { fn value(&self) -> u32; } impl GameTrait for Example { fn value(&self) -> u32 { 7 } }";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &{
                let mut types = OwnerTypes::default();
                types.declare("k", &file.items);
                types
            },
            &WorkspaceTraits::empty(),
            &mut found,
        );
        let mut count = 0;
        let mut out = rewrite(Path::new("k.rs"), source, &mut found, &mut count)
            .unwrap()
            .unwrap();
        assert_eq!(count, 1);
        out.push_str(" pub static __REPLACE: [std::sync::atomic::AtomicPtr<()>; 1] = [std::sync::atomic::AtomicPtr::new(std::ptr::null_mut())];");
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("fnr-trait-lint-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("fixture.rs");
        fs::write(&input, out).unwrap();
        let result = std::process::Command::new("rustc")
            .args([
                "--edition=2024",
                "--crate-type=lib",
                "--emit=metadata",
                "-Dwarnings",
            ])
            .arg(&input)
            .arg("--out-dir")
            .arg(&dir)
            .output()
            .unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    fn reader_names_do_not_collide_with_get_function_installers() {
        let out = staged(
            "struct Game; impl Game { fn projectile(&self) {} fn get_projectile(&self) {} }",
        );
        let file = syn::parse_file(&out).unwrap();
        let mut names = BTreeSet::new();
        for item in file.items {
            if let Item::Impl(block) = item {
                for item in block.items {
                    if let ImplItem::Fn(method) = item {
                        assert!(
                            names.insert(method.sig.ident.to_string()),
                            "duplicate method: {}",
                            method.sig.ident
                        );
                    }
                }
            }
        }
        assert!(names.contains("__fnr_get_projectile"));
        assert!(names.contains("__fnr_read_slot_0"));
    }

    #[test]
    fn slots_are_numbered_per_crate_and_replaced_reads_the_slot() {
        let source = "macro_rules! replaced { ($name:ident) => { false }; }\npub fn world_drawn(view: &mut View) { let _ = view; }\npub fn hooked() -> bool { replaced!(world_drawn) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &OwnerTypes::default(),
            &WorkspaceTraits::empty(),
            &mut found,
        );
        let mut count = 7;
        let out = rewrite(Path::new("k.rs"), source, &mut found, &mut count)
            .unwrap()
            .unwrap();
        assert_eq!(count, 9);
        assert!(!out.contains("macro_rules!"));
        assert_eq!(source.lines().count(), out.lines().count());
        assert!(out.contains("crate :: __REPLACE [7usize]"), "{out}");
        assert!(out.contains("crate :: __REPLACE [8usize]"), "{out}");
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = __fnr_read_slot_8() { ::core::hint::cold_path(); return __fnr(); } __fnr_read_slot_7().is_some() }"),
            "{out}"
        );
        syn::parse_file(&out).unwrap();
    }

    #[test]
    fn replaced_of_a_function_the_file_does_not_stage_is_refused() {
        let source = "pub fn hooked() -> bool { replaced!(elsewhere) }\n";
        let file = syn::parse_file(source).unwrap();
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &OwnerTypes::default(),
            &WorkspaceTraits::empty(),
            &mut found,
        );
        let error = rewrite(Path::new("k.rs"), source, &mut found, &mut 0).unwrap_err();
        assert!(error.contains("replaced!(elsewhere)"), "{error}");
    }

    #[test]
    fn other_crates_traits_generic_impls_and_test_modules_get_no_slot() {
        let file = syn::parse_file(
            "impl Clone for A { fn clone(&self) -> Self { A } }\nimpl<T> B<T> { fn f(&self) {} }\nimpl<const N: usize> C<N> { fn f(&self) {} }\nimpl B<u8> { fn concrete(&self) {} fn generic<T>(&self) {} }\n#[cfg(test)] mod tests { fn t() {} }\nmod inner { fn kept() {} }\nfn free() {}\n",
        )
        .unwrap();
        assert_eq!(
            targets("k", &file, &OwnerTypes::default()),
            ["k::B<u8>::concrete", "k::inner::kept", "k::free"]
        );
    }

    #[test]
    fn a_method_of_the_games_own_trait_gets_a_slot_on_its_type() {
        let source = "use engine::Style; trait Goal { type Kind; const N: usize; fn start(&mut self, mob: &Mob) -> bool; fn kind(&self) -> Self::Kind; fn sized(&self) -> [u8; Self::N]; } struct Wander; struct Raw; mod other { struct Style; }\nimpl Goal for Style { fn start(&mut self) {} } impl Goal for engine::Raw { fn start(&mut self) {} } impl Goal for Wander { fn start(&mut self, mob: &Mob) -> bool { true } fn kind(&self) -> Self::Kind { Kind } fn sized(&self) -> [u8; Self::N] { [0; Self::N] } }\nimpl Goal<u8> for Wander { fn start(&mut self) {} }\nunsafe impl Goal for Raw { fn start(&mut self) {} }\nimpl Goal for Vec3 { fn start(&mut self) {} }\n";
        let file = syn::parse_file(source).unwrap();
        let mut types = OwnerTypes::default();
        types.declare("k", &file.items);
        // A signature naming the trait's associated constant, a trait written with arguments, an
        // unsafe impl and another crate's type (imported under a name the crate also declares,
        // written by path, or not declared here at all) are left as the game wrote them.
        assert_eq!(
            targets("k", &file, &types),
            ["k::<Wander as Goal>::start", "k::<Wander as Goal>::kind"]
        );
        let mut found = Vec::new();
        functions(
            "k",
            &file.items,
            &types,
            &WorkspaceTraits::empty(),
            &mut found,
        );
        let out = rewrite(Path::new("k.rs"), source, &mut found, &mut 0)
            .unwrap()
            .unwrap();
        assert!(
            out.contains("{ if let ::std::option::Option::Some(__fnr) = Self::__fnr_read_slot_0() { ::core::hint::cold_path(); return __fnr(self, mob); } true }"),
            "{out}"
        );
        let staged = syn::parse_file(&out).unwrap();
        let Some(Item::Impl(added)) = staged.items.get(8) else {
            panic!("{out}")
        };
        assert!(added.trait_.is_none());
        assert!(out.contains("pub fn __fnr_as_Goal_start"), "{out}");
        // The trait's associated type is named through the trait outside its `impl`.
        assert!(out.contains("-> < Self as Goal > :: Kind"), "{out}");
    }

    #[test]
    fn a_trait_impl_requires_an_in_scope_owner_declaration() {
        let workspace = WorkspaceTraits::empty();
        let declares = syn::parse_file("pub struct GameLevel;").unwrap();
        let imported = syn::parse_file(
            "use crate::levels::GameLevel; trait LevelAccess { fn block_state(&self) -> u32; } impl LevelAccess for GameLevel { fn block_state(&self) -> u32 { 0 } }",
        )
        .unwrap();
        let unimported = syn::parse_file(
            "trait LevelAccess { fn block_state(&self) -> u32; } impl LevelAccess for GameLevel { fn block_state(&self) -> u32 { 0 } }",
        )
        .unwrap();
        let mut types = OwnerTypes::default();
        types.declare("k::levels", &declares.items);
        types.declare("k::level_access", &imported.items);
        assert_eq!(
            targets("k::level_access", &imported, &types),
            ["k::level_access::<GameLevel as LevelAccess>::block_state"]
        );
        // Without an import, an unrelated `GameLevel` in another module does not prove locality.
        let mut no_use = OwnerTypes::default();
        no_use.declare("k::levels", &declares.items);
        no_use.declare("k::level_access", &unimported.items);
        assert!(targets("k::level_access", &unimported, &no_use).is_empty());
        let Item::Impl(block) = &unimported.items[1] else {
            panic!("an impl")
        };
        assert_eq!(
            trait_refusal("k::level_access", block, &no_use, &workspace).unwrap(),
            "its trait implementation has no replacement installer (foreign trait owner)"
        );
    }

    /// The targets of `impl LevelAccess for GameLevel` in `k::game::level_access`, a child of
    /// `k::game`, whose file starts with `import`; each of `others` is a module and its file.
    fn level_access_targets(others: &[(&str, &str)], import: &str) -> Vec<String> {
        let mut types = OwnerTypes::default();
        for (module, text) in others {
            types.declare(module, &syn::parse_file(text).unwrap().items);
        }
        let file = syn::parse_file(&format!(
            "{import} trait LevelAccess {{ fn block_state(&self) -> u32; }} impl LevelAccess for GameLevel {{ fn block_state(&self) -> u32 {{ 0 }} }}"
        ))
        .unwrap();
        types.declare("k::game::level_access", &file.items);
        targets("k::game::level_access", &file, &types)
    }

    const KEPT: [&str; 1] = ["k::game::level_access::<GameLevel as LevelAccess>::block_state"];

    /// The game's own shape: `GameLevel` in the `game` module and in another one, the `impl`
    /// in a child of `game` that takes its parent's names.
    #[test]
    fn a_use_of_the_declaring_module_settles_a_name_two_modules_declare() {
        let twice = [
            ("k::game", "struct GameLevel;"),
            ("k::game::natural_spawning", "pub struct GameLevel;"),
        ];
        for import in [
            "use super::*;",
            "use super::GameLevel;",
            "use crate::game::{GameLevel, Other};",
            "use crate::game::natural_spawning::GameLevel;",
        ] {
            assert_eq!(level_access_targets(&twice, import), KEPT, "{import}");
        }
        // Renamed from a name that is no type there, a name its module does not hold, or brought
        // in by two globs that both declare it: nothing says which.
        for import in [
            "use super::Level as GameLevel;",
            "use crate::elsewhere::GameLevel;",
            "use super::*; use super::natural_spawning::*;",
        ] {
            assert!(level_access_targets(&twice, import).is_empty(), "{import}");
        }
        // One module reached through two globs is one answer.
        let through = [
            (
                "k::game",
                "struct GameLevel; mod other { struct GameLevel; }",
            ),
            ("k::game::prelude", "pub(in crate::game) use super::*;"),
        ];
        assert_eq!(
            level_access_targets(&through, "use super::*; use super::prelude::*;"),
            KEPT
        );
    }

    /// A name the parent takes from another crate reaches the child through its glob: the
    /// crate's own type of that name, declared elsewhere, is not what the `impl` names.
    #[test]
    fn a_foreign_name_inherited_through_a_glob_is_refused() {
        for parent in [
            "use engine::GameLevel;",
            "use engine::Level as GameLevel;",
            "use engine::*;",
        ] {
            let crate_ = [("k::game", parent), ("k::levels", "pub struct GameLevel;")];
            assert!(
                level_access_targets(&crate_, "use super::*;").is_empty(),
                "{parent}"
            );
        }
        // The same glob beside the parent's own type: the type is the parent's.
        let own = [("k::game", "use engine::*; struct GameLevel;")];
        assert_eq!(level_access_targets(&own, "use super::*;"), KEPT);
        // A glob of an enum's variants, or of a child module, brings in no other crate's name.
        let variants = [
            (
                "k::game",
                "enum Kind { A } use Kind::*; mod child { pub struct GameLevel; } use child::*;",
            ),
            ("k::levels", "struct GameLevel;"),
        ];
        assert_eq!(level_access_targets(&variants, "use super::*;"), KEPT);
    }

    /// A renamed import is the type it renames: followed when that is a type of the crate,
    /// refused when it is another crate's or leads nowhere, and never dropped for a type that
    /// happens to have the new name.
    #[test]
    fn a_renamed_import_is_followed_to_the_type_it_renames() {
        let own = [
            ("k::a", "pub struct Level;"),
            ("k::b", "pub struct GameLevel;"),
        ];
        assert_eq!(
            level_access_targets(&own, "use crate::a::Level as GameLevel;"),
            KEPT
        );
        for (a, import) in [
            // `k::a` takes `Level` from another crate, by name and by glob.
            (
                "pub use engine::Level;",
                "use crate::a::Level as GameLevel;",
            ),
            ("pub use engine::*;", "use crate::a::Level as GameLevel;"),
            // Renamed straight from another crate, in the parent.
            ("", "use engine::Level as GameLevel;"),
        ] {
            let crate_ = [("k::a", a), ("k::b", "pub struct GameLevel;")];
            assert!(
                level_access_targets(&crate_, import).is_empty(),
                "{a} {import}"
            );
        }
        // Inherited through the parent's glob, the renamed name is still not `k::b`'s type.
        let inherited = [
            ("k::game", "use engine::Level as GameLevel;"),
            ("k::b", "pub struct GameLevel;"),
        ];
        assert!(level_access_targets(&inherited, "use super::*;").is_empty());
    }

    /// `mod_api`'s `PartRegistrar` shape: the type in a child module, re-exported by its parent
    /// through a path written from the child's bare name, and the `impl`s in sibling files that
    /// take the parent's names.
    #[test]
    fn a_parents_re_export_of_its_childs_type_reaches_a_sibling() {
        for re_export in [
            "pub use shared::GameLevel;",
            "pub use shared::*;",
            "pub use self::shared::GameLevel;",
            "pub use shared::{GameLevel as Level, GameLevel};",
        ] {
            let crate_ = [
                ("k::game::shared", "pub struct GameLevel;"),
                ("k::game", re_export),
                ("k::other", "struct GameLevel;"),
            ];
            assert_eq!(
                level_access_targets(&crate_, "use super::*;"),
                KEPT,
                "{re_export}"
            );
        }
        // Re-exported under the name: the sibling's `GameLevel` is the child's `Shared`, a type of
        // the crate, whatever else is called `GameLevel`.
        let renamed = [
            ("k::game::shared", "pub struct Shared;"),
            ("k::game", "pub use shared::Shared as GameLevel;"),
            ("k::other", "struct GameLevel;"),
        ];
        assert_eq!(level_access_targets(&renamed, "use super::*;"), KEPT);
        // A child the parent does not have is another crate.
        let foreign = [
            ("k::game", "pub use engine_shared::GameLevel;"),
            ("k::other", "struct GameLevel;"),
        ];
        assert!(level_access_targets(&foreign, "use super::*;").is_empty());
    }

    /// A module's `use`s are its own, whichever of its files writes them.
    #[test]
    fn a_use_in_a_sibling_file_of_the_module_settles_the_name() {
        let crate_ = [
            ("k::a", "pub struct GameLevel;"),
            ("k::b", "pub struct GameLevel;"),
            ("k::game::level_access", "use crate::a::GameLevel;"),
        ];
        assert_eq!(level_access_targets(&crate_, ""), KEPT);
    }

    #[test]
    fn two_modules_types_of_one_name_each_keep_their_own_impls_slot() {
        let file =
            syn::parse_file("trait Goal { fn start(&mut self); } struct State; impl Goal for State { fn start(&mut self) {} }")
                .unwrap();
        let mut types = OwnerTypes::default();
        for module in ["k::a", "k::b"] {
            types.declare(module, &file.items);
        }
        for module in ["k::a", "k::b"] {
            assert_eq!(
                targets(module, &file, &types),
                [format!("{module}::<State as Goal>::start")]
            );
        }
    }

    #[test]
    fn reparented_side_module_resolves_pass_through_parent_owner_modules() {
        let mut types = OwnerTypes::default();
        types.declare(
            "k::game",
            &syn::parse_file(
                "mod boats; mod fireworks; mod minecarts; pub(in crate::game) mod server_side;\n\
                 pub(in crate::game) use server_side::{entity, general};\n\
                 pub(in crate::game) use boats::Boat as ExportedBoat;\n\
                 pub(in crate::game) trait Entity { fn bounds(&self); }\n\
                 pub(in crate::game) trait EntityRules { fn gravity(&self); }",
            )
            .unwrap()
            .items,
        );
        types.declare(
            "k::game::boats",
            &syn::parse_file("pub(in crate::game) struct Boat;")
                .unwrap()
                .items,
        );
        types.declare(
            "k::game::fireworks",
            &syn::parse_file("pub(in crate::game) struct Rocket;")
                .unwrap()
                .items,
        );
        types.declare(
            "k::game::minecarts",
            &syn::parse_file("pub(in crate::game) struct Minecart;")
                .unwrap()
                .items,
        );
        types.declare(
            "k::game::server_side",
            &syn::parse_file(
                "pub(in crate::game::server_side) use super::*;\n\
                 pub(in crate::game) mod entity;\n\
                 pub(in crate::game) mod general;\n\
                 pub(in crate::game) mod via_glob;",
            )
            .unwrap()
            .items,
        );
        types.declare(
            "k::game::server_side::general",
            &syn::parse_file("pub(in crate::game) struct GeneralRow;")
                .unwrap()
                .items,
        );
        let entity = syn::parse_file(
            "use super::{Entity, EntityRules, boats::Boat, fireworks::Rocket, minecarts::Minecart};\n\
             impl Entity for Boat { fn bounds(&self) {} }\n\
             impl EntityRules for Boat { fn gravity(&self) {} }\n\
             impl Entity for Rocket { fn bounds(&self) {} }\n\
             impl Entity for Minecart { fn bounds(&self) {} }\n",
        )
        .unwrap();
        types.declare("k::game::server_side::entity", &entity.items);
        assert_eq!(
            targets("k::game::server_side::entity", &entity, &types),
            [
                "k::game::server_side::entity::<Boat as Entity>::bounds",
                "k::game::server_side::entity::<Boat as EntityRules>::gravity",
                "k::game::server_side::entity::<Rocket as Entity>::bounds",
                "k::game::server_side::entity::<Minecart as Entity>::bounds",
            ]
        );
        // Unmoved module reaches moved `general` through `game`'s re-export, and module aliases work.
        let unmoved = syn::parse_file(
            "use super::{Entity, general::GeneralRow};\n\
             use super::boats::{self as b};\n\
             use b::Boat;\n\
             impl Entity for GeneralRow { fn bounds(&self) {} }\n\
             impl Entity for Boat { fn bounds(&self) {} }\n",
        )
        .unwrap();
        types.declare("k::game::unmoved", &unmoved.items);
        assert_eq!(
            targets("k::game::unmoved", &unmoved, &types),
            [
                "k::game::unmoved::<GeneralRow as Entity>::bounds",
                "k::game::unmoved::<Boat as Entity>::bounds",
            ]
        );
        // Child using `use super::*;` reaches an owner re-exported in `game` through `server_side`.
        let via_glob = syn::parse_file(
            "use super::*;\nimpl Entity for ExportedBoat { fn bounds(&self) {} }\n",
        )
        .unwrap();
        types.declare("k::game::server_side::via_glob", &via_glob.items);
        assert_eq!(
            targets("k::game::server_side::via_glob", &via_glob, &types),
            ["k::game::server_side::via_glob::<ExportedBoat as Entity>::bounds"]
        );
    }

    #[test]
    fn pass_through_parent_checks_foreign_visibility_cfg_and_refuses_other_globs() {
        let workspace = WorkspaceTraits::empty();
        let run = |modules: &[(&str, &str)], impl_mod: &str, impl_src: &str| {
            let mut types = OwnerTypes::default();
            for (module, src) in modules {
                types.declare(module, &syn::parse_file(src).unwrap().items);
            }
            let file = syn::parse_file(impl_src).unwrap();
            types.declare(impl_mod, &file.items);
            let block = file
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Impl(block) => Some(block),
                    _ => None,
                })
                .unwrap();
            trait_exclusion(impl_mod, block, &types, &workspace)
        };
        let impl_boat = "trait Entity { fn bounds(&self); } use super::boats::Boat; impl Entity for Boat { fn bounds(&self) {} }";
        for (label, modules, expected) in [
            (
                "foreign module re-export",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "use engine::boats;"),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("imported trait owner"),
            ),
            (
                "local module re-exporting foreign type",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "mod boats;"),
                    ("k::game::boats", "pub use engine::Boat;"),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("imported trait owner"),
            ),
            (
                "foreign glob in parent",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "use engine::*;"),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("glob import in owner path"),
            ),
            (
                "foreign glob beside parent glob",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "mod boats;"),
                    ("k::game::boats", "pub struct Boat;"),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; use engine::*; mod entity;",
                    ),
                ],
                Some("glob import in owner path"),
            ),
            (
                "unresolved missing module never falls back to unique local",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", ""),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("imported trait owner"),
            ),
            (
                "cfg-gated child module",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "#[cfg(feature = \"x\")] mod boats;"),
                    ("k::game::boats", "pub struct Boat;"),
                    (
                        "k::game::server_side",
                        "pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("conditional trait owner"),
            ),
            (
                "cfg-gated parent glob",
                vec![
                    ("k::unrelated", "pub struct Boat;"),
                    ("k::game", "mod boats;"),
                    ("k::game::boats", "pub struct Boat;"),
                    (
                        "k::game::server_side",
                        "#[cfg(feature = \"x\")] pub(in crate::game::server_side) use super::*; mod entity;",
                    ),
                ],
                Some("glob import in owner path"),
            ),
        ] {
            assert_eq!(
                run(&modules, "k::game::server_side::entity", impl_boat),
                expected,
                "{label}"
            );
        }
        // Visibility: restricted or public `server_side` glob is not a pass-through parent for outsiders.
        let restricted = [
            ("k::unrelated", "pub struct Boat;"),
            ("k::game", "mod boats; pub mod server_side;"),
            ("k::game::boats", "pub struct Boat;"),
            (
                "k::game::server_side",
                "pub(in crate::game::server_side) use super::*; mod entity;",
            ),
        ];
        assert_eq!(
            run(
                &restricted,
                "k::game::client_side::outsider",
                "trait Entity { fn bounds(&self); } use crate::game::server_side::boats::Boat; impl Entity for Boat { fn bounds(&self) {} }",
            ),
            Some("imported trait owner")
        );
        let pub_glob_private_mod = [
            ("k::unrelated", "pub struct Boat;"),
            ("k::game", "mod boats; pub mod server_side;"),
            ("k::game::boats", "pub struct Boat;"),
            ("k::game::server_side", "pub use super::*; mod entity;"),
        ];
        assert_eq!(
            run(
                &pub_glob_private_mod,
                "k::outside",
                "trait Entity { fn bounds(&self); } use crate::game::server_side::boats::Boat; impl Entity for Boat { fn bounds(&self) {} }",
            ),
            Some("glob import in owner path")
        );
        // Non-pass-through globs in intermediate owner paths are refused, never followed.
        let non_super_globs = [
            ("k::game", "pub mod a; pub mod b;"),
            ("k::game::a", "pub mod boats;"),
            ("k::game::a::boats", "pub struct Boat;"),
            ("k::game::b", "pub mod boats;"),
            ("k::game::b::boats", "pub struct Boat;"),
            (
                "k::game::server_side",
                "pub(in crate::game::server_side) use super::a::*; pub(in crate::game::server_side) use super::b::*; mod entity;",
            ),
        ];
        assert_eq!(
            run(&non_super_globs, "k::game::server_side::entity", impl_boat),
            Some("glob import in owner path")
        );
        let second_glob_disables_passthrough = [
            ("k::game", "mod boats; pub mod prelude;"),
            ("k::game::boats", "pub struct Boat;"),
            ("k::game::prelude", "pub(in crate::game) use super::boats;"),
            (
                "k::game::server_side",
                "pub(in crate::game::server_side) use super::*; pub(in crate::game::server_side) use super::prelude::*; mod entity;",
            ),
        ];
        assert_eq!(
            run(
                &second_glob_disables_passthrough,
                "k::game::server_side::entity",
                impl_boat
            ),
            Some("glob import in owner path")
        );
        // A `#[cfg(test)]` foreign import does not shadow the production local import.
        let with_test_import = [
            ("k::game", "mod boats;"),
            ("k::game::boats", "pub struct Boat;"),
            (
                "k::game::server_side",
                "pub(in crate::game::server_side) use super::*; mod entity;",
            ),
        ];
        assert_eq!(
            run(
                &with_test_import,
                "k::game::server_side::entity",
                "trait Entity { fn bounds(&self); } #[cfg(test)] use engine::Boat; use super::boats::Boat; impl Entity for Boat { fn bounds(&self) {} }",
            ),
            None
        );
    }

    #[test]
    fn the_pass_gives_each_crate_one_array_and_refuses_an_ineligible_marked_function() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname = 'sunrise-client'\n",
        );
        write(
            "crates/sunrise-client/src/lib.rs",
            "#![forbid(unsafe_code)]\nmod game;\n",
        );
        write(
            "crates/sunrise-client/src/game.rs",
            "impl Game<'_> { fn native_tick(&mut self) {} }\nfn free(a: u8) -> u8 { a }\n",
        );
        write(
            "crates/sunrise-low/Cargo.toml",
            "[package]\nname = 'sunrise-low'\n",
        );
        write(
            "crates/sunrise-low/src/lib.rs",
            "#[sunrise_mod_api_macros::replaceable] pub fn drawn() {}\npub fn unlisted() {}\n",
        );
        write(
            "Cargo.lock",
            "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"sunrise-low\"\nversion = \"0.1.0\"\n",
        );

        let staged = stage(root.path()).unwrap();
        assert_eq!(
            staged,
            Staged {
                functions: 3,
                files: 2,
                crates: 2,
                slots: BTreeSet::from([
                    "sunrise_client::game::Game::native_tick".to_owned(),
                    "sunrise_client::game::free".to_owned(),
                    "sunrise_low::drawn".to_owned(),
                ]),
                trait_slots: BTreeMap::new(),
            }
        );
        let lib = fs::read_to_string(root.path().join("crates/sunrise-client/src/lib.rs")).unwrap();
        assert!(lib.starts_with("#![deny(unsafe_code)]\n"), "{lib}");
        assert!(lib.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 2]"));
        let low = fs::read_to_string(root.path().join("crates/sunrise-low/src/lib.rs")).unwrap();
        assert!(
            low.contains("__fnr_drawn") && !low.contains("__fnr_unlisted"),
            "{low}"
        );
        assert!(low.contains("pub static __REPLACE: [::std::sync::atomic::AtomicPtr<()>; 1]"));
        syn::parse_file(&low).unwrap();
        // The staged game links the marked crate, so it must declare the dependency.
        assert!(lib.contains("= [&::sunrise_low::__REPLACE];"), "{lib}");
        let client: toml::Table = toml::from_str(
            &fs::read_to_string(root.path().join("crates/sunrise-client/Cargo.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            client["dependencies"]["sunrise-low"]["path"].as_str(),
            Some("../sunrise-low")
        );
        let lock = fs::read_to_string(root.path().join("Cargo.lock")).unwrap();
        assert!(
            lock.contains("name = \"sunrise-client\"\nversion = \"0.1.0\"\ndependencies = [\n \"sunrise-low\",\n]"),
            "{lock}"
        );

        write(
            "crates/sunrise-low/src/lib.rs",
            "#[sunrise_mod_api_macros::replaceable] pub const fn missing() {}\n",
        );
        let error = stage(root.path()).unwrap_err();
        assert!(
            error.contains("#[sunrise_mod_api_macros::replaceable] on sunrise_low::missing"),
            "{error}"
        );
    }

    #[test]
    fn a_marked_crate_built_on_the_game_crate_is_refused_by_name() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname = 'sunrise-client'\n",
        );
        write("crates/sunrise-client/src/lib.rs", "mod game;\n");
        write("crates/sunrise-client/src/game.rs", "");
        // Through a crate in between, as a cycle usually hides.
        write(
            "crates/sunrise-high/Cargo.toml",
            "[package]\nname = 'sunrise-high'\n[dependencies]\nsunrise-mid = { path = '../sunrise-mid' }\n",
        );
        write(
            "crates/sunrise-mid/Cargo.toml",
            "[package]\nname = 'sunrise-mid'\n[build-dependencies]\nsunrise-client = { path = '../sunrise-client' }\n",
        );
        write(
            "crates/sunrise-high/src/lib.rs",
            "#[sunrise_mod_api_macros::replaceable] pub fn drawn() {}\n",
        );
        let error = stage(root.path()).unwrap_err();
        assert!(
            error.starts_with("sunrise_high: it is built on sunrise-client"),
            "{error}"
        );
        // Left untouched: the refusal comes before any edit.
        assert_eq!(
            fs::read_to_string(root.path().join("crates/sunrise-high/src/lib.rs")).unwrap(),
            "#[sunrise_mod_api_macros::replaceable] pub fn drawn() {}\n"
        );
    }

    #[test]
    fn stages_method_returning_trait_object_with_elided_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let write = |path: &str, text: &str| {
            let path = root.path().join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        };
        write(
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname = 'sunrise-client'\n",
        );
        write("crates/sunrise-client/src/lib.rs", "mod game;\n");
        write(
            "crates/sunrise-client/src/game.rs",
            "impl Game<'_> {\n\
             fn open_resource(&self, id: &str) -> Result<Option<Box<dyn std::io::Read + '_>>, ()> { let _ = id; Ok(None) }\n\
             fn mapper(&self, _tag: &str) -> Box<dyn Fn(&str) -> &str + '_> { Box::new(|s| s) }\n\
             }\n",
        );

        let staged = stage(root.path()).unwrap();
        assert_eq!(staged.functions, 2);
        let rewritten =
            fs::read_to_string(root.path().join("crates/sunrise-client/src/game.rs")).unwrap();
        let open_expected = "for < '__fnr_s > fn (& '__fnr_s Self , & str) -> Result < Option < Box < dyn std :: io :: Read + '__fnr_s > > , () >";
        assert!(rewritten.contains(open_expected), "{rewritten}");
        let mapper_expected = "for < '__fnr_s > fn (& '__fnr_s Self , & str) -> Box < dyn Fn (& str) -> & str + '__fnr_s >";
        assert!(rewritten.contains(mapper_expected), "{rewritten}");
    }
}
