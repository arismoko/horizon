//! The staged game's opt-in internals table (#589 W): for declared `internals.json` items
//! that can cross to a wasm mod, the code the wasm runtime reaches them through
//! (`mod_points::internals::Table`). Generated into the staged copy only, as the crate root's
//! `__internals`; the working tree has none of it.
//!
//! - **Types.** Each walked type gets one node function: a step into a field, an element or an
//!   entry, a copy-typed field's read and write, its case, its length.
//! - **Copies.** Each listed copy type gets one encoder, and a decoder when it can be built (no
//!   borrowed or shared part); a field or argument of any other copy shape is converted inline.
//! - **Functions.** Each listed function whose receiver, arguments and return can all cross gets a
//!   thunk: copies are decoded, references come in as nodes, and a returned reference goes back
//!   as a node.
/// Whether a function argument is a closure (`impl Fn...`, `dyn Fn...`). A closure has no
/// thunk: the component model refuses a call back into the guest inside that guest's own
/// call, so a mod declares the game's non-closure helper for the operation instead.
fn closure_argument(sig: &syn::Signature) -> bool {
    fn closure_bound(bound: &syn::TypeParamBound) -> bool {
        let syn::TypeParamBound::Trait(bound) = bound else {
            return false;
        };
        bound.path.segments.last().is_some_and(|segment| {
            matches!(
                segment.ident.to_string().as_str(),
                "Fn" | "FnMut" | "FnOnce"
            )
        })
    }
    sig.inputs.iter().any(|arg| {
        let syn::FnArg::Typed(arg) = arg else {
            return false;
        };
        match &*arg.ty {
            syn::Type::ImplTrait(ty) => ty.bounds.iter().any(closure_bound),
            syn::Type::TraitObject(ty) => ty.bounds.iter().any(closure_bound),
            _ => false,
        }
    })
}

fn structural_key(shape: &crate::crossing::Shape) -> bool {
    use crate::crossing::{Scalar, Shape};
    match shape {
        Shape::Scalar(
            Scalar::Bool
            | Scalar::Char
            | Scalar::Signed(_)
            | Scalar::Unsigned(_)
            | Scalar::Wide(_)
            | Scalar::String,
        )
        | Shape::Unit => true,
        Shape::Tuple(parts) => parts.iter().all(structural_key),
        Shape::Option(inner) | Shape::Array(inner, _) => structural_key(inner),
        _ => false,
    }
}

use crate::crossing::{Collection, Crossing, Foreign, Rule, Scalar, Shape, Wrapper, bare};
use crate::internals::Internal;
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// The game's crate.
const GAME: &str = "horizon_client";

/// What the generator made, and what it left out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub types: usize,
    /// Distinct paths requested by the per-mod declaration files.
    pub declared: usize,
    /// Declared paths that cannot be exposed, with their reason.
    pub refused: BTreeMap<String, &'static str>,
    pub thunks: usize,
    /// Entries without a field/codec/thunk binding, keyed by path, kind and hash. Unlike
    /// `refused`, this retains the reason when another kind at the same path is accepted.
    pub unbound: BTreeMap<(String, &'static str, String), &'static str>,
    /// Native methods exposed under a collision-free guest name.
    pub renamed_methods: BTreeMap<String, String>,
    /// Map nodes without key access because native equality cannot be proven structural.
    pub refused_map_keys: BTreeSet<String>,
    /// Functions with a part that can't cross, or a form a thunk can't make (a generic, a `self`
    /// taken by value, a returned value that is no copy).
    pub skipped: usize,
    /// Layouts whose accessibility or field set cannot be proven from the manifest.
    pub skipped_types: BTreeMap<&'static str, usize>,
    /// Functions refused at each stage of thunk generation.
    pub skipped_functions: BTreeMap<&'static str, usize>,
}

/// A field owner or receiver exposes only declared fields. A type reached below a field,
/// or through a function's arguments/result, exposes its whole crossable layout.
#[derive(Clone, Default)]
pub(crate) struct Selection {
    pub(crate) full: HashSet<String>,
    pub(crate) owners: BTreeMap<String, BTreeSet<usize>>,
}

impl Selection {
    fn reach(&mut self, shape: &Shape, rule: &Rule) {
        if rule.crossing_of(shape) == Crossing::Opaque {
            return;
        }
        match shape {
            Shape::Named(path, _) => {
                if !self.full.insert(path.clone()) {
                    return;
                }
                let named = rule.named(path).expect("listed");
                for field in rule.fields(path) {
                    self.reach(&rule.shape(&named.module, Some(path), field.ty), rule);
                }
            }
            Shape::Ref { to, .. }
            | Shape::Wrapped(_, to)
            | Shape::Array(to, _)
            | Shape::Slice(to)
            | Shape::Option(to) => self.reach(to, rule),
            Shape::Tuple(parts) | Shape::Collection(_, parts) => {
                for part in parts {
                    self.reach(part, rule);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default)]
struct NodeDef {
    path: String,
    hash: String,
    function: TokenStream,
    raw_fields: Vec<u32>,
    raw_gets: Vec<u32>,
    raw_index: bool,
    refused_key: Option<String>,
}

fn raw_scalar(shape: &Shape) -> bool {
    matches!(
        shape,
        Shape::Scalar(
            Scalar::Bool
                | Scalar::Char
                | Scalar::Signed(_)
                | Scalar::Unsigned(_)
                | Scalar::Float(_)
                | Scalar::Wide(_)
        )
    )
}

struct Gen<'r> {
    rule: &'r Rule,
    guest: bool,
    /// Selection validates signatures without regenerating the receiver's entire graph.
    probing: bool,
    hashes: &'r HashMap<String, (&'r str, &'r str)>,
    selection: Selection,
    /// Each walked type's id, by its spelling.
    ids: HashMap<String, u32>,
    /// Each walked type, in id order: its spelling and node function.
    nodes: Vec<NodeDef>,
    /// Each listed copy type's codec, by path: already generated.
    codecs: HashMap<String, bool>,
    items: Vec<TokenStream>,
    skip_reason: &'static str,
}

/// A type's Rust spelling, from the game crate's root.
pub(crate) fn spell(shape: &Shape) -> Option<TokenStream> {
    Some(match shape {
        Shape::Scalar(scalar) => match scalar {
            Scalar::Bool => quote!(bool),
            Scalar::Char => quote!(char),
            Scalar::Signed(name)
            | Scalar::Unsigned(name)
            | Scalar::Float(name)
            | Scalar::Wide(name) => {
                let name = format_ident!("{name}");
                quote!(#name)
            }
            Scalar::Str => quote!(str),
            Scalar::String => quote!(::std::string::String),
        },
        Shape::Unit => quote!(()),
        Shape::Ref { mutable, to } => {
            let to = spell(to)?;
            if *mutable {
                quote!(&'static mut #to)
            } else {
                quote!(&'static #to)
            }
        }
        Shape::Tuple(parts) => {
            let parts = parts.iter().map(spell).collect::<Option<Vec<_>>>()?;
            quote!((#(#parts,)*))
        }
        Shape::Array(elem, n) => {
            let elem = spell(elem)?;
            quote!([#elem; #n])
        }
        Shape::Slice(elem) => {
            let elem = spell(elem)?;
            quote!([#elem])
        }
        Shape::Option(inner) => {
            let inner = spell(inner)?;
            quote!(::std::option::Option<#inner>)
        }
        Shape::Wrapped(wrapper, inner) => {
            let inner = spell(inner)?;
            match wrapper {
                Wrapper::Box => quote!(::std::boxed::Box<#inner>),
                Wrapper::Arc => quote!(::std::sync::Arc<#inner>),
                Wrapper::Rc => quote!(::std::rc::Rc<#inner>),
                Wrapper::Cow => quote!(::std::borrow::Cow<'static, #inner>),
            }
        }
        Shape::Collection(collection, args) => {
            let args = args.iter().map(spell).collect::<Option<Vec<_>>>()?;
            match collection {
                Collection::Vec => quote!(::std::vec::Vec<#(#args),*>),
                Collection::VecDeque => quote!(::std::collections::VecDeque<#(#args),*>),
                Collection::HashSet => quote!(::std::collections::HashSet<#(#args),*>),
                Collection::BTreeSet => quote!(::std::collections::BTreeSet<#(#args),*>),
                Collection::HashMap => quote!(::std::collections::HashMap<#(#args),*>),
                Collection::BTreeMap => quote!(::std::collections::BTreeMap<#(#args),*>),
            }
        }
        Shape::Named(path, lifetimes) => {
            let path = path_tokens(path);
            if *lifetimes == 0 {
                path
            } else {
                let lifetimes = (0..*lifetimes).map(|_| quote!('static));
                quote!(#path<#(#lifetimes),*>)
            }
        }
        Shape::Opaque(_) => return None,
    })
}

/// A listed path, from the game crate's root.
fn path_tokens(path: &str) -> TokenStream {
    let mut segments = path.split("::");
    let first = segments.next().unwrap_or_default();
    let head = if first == GAME {
        quote!(crate)
    } else {
        let first = format_ident!("{first}");
        quote!(::#first)
    };
    let rest = segments.map(|s| format_ident!("{s}"));
    quote!(#head #(::#rest)*)
}

/// Whether a value of `shape` can be built from a copy: no borrowed or shared part.
fn buildable(shape: &Shape, rule: &Rule) -> bool {
    match shape {
        Shape::Scalar(Scalar::Str) | Shape::Ref { .. } | Shape::Opaque(_) => false,
        Shape::Scalar(_) | Shape::Unit => true,
        Shape::Tuple(parts) => parts.iter().all(|p| buildable(p, rule)),
        Shape::Array(elem, _) | Shape::Option(elem) => buildable(elem, rule),
        Shape::Wrapped(Wrapper::Box, inner) => buildable(inner, rule),
        Shape::Wrapped(..) | Shape::Slice(_) | Shape::Collection(..) => false,
        Shape::Named(path, _) => {
            let Some(named) = rule.named(path) else {
                return false;
            };
            if named.foreign && named.sealed {
                return false;
            }
            rule.fields(path)
                .iter()
                .all(|f| buildable(&rule.shape(&named.module, Some(path), f.ty), rule))
        }
    }
}

/// The fields of the listed type at `path` a node steps into: none for flags, which are read
/// whole (their bits are no field of theirs).
pub(crate) fn node_fields<'r>(rule: &'r Rule, path: &str) -> Vec<crate::crossing::Field<'r>> {
    if crate::crossing::foreign(path) == Some(Foreign::Bits) {
        Vec::new()
    } else {
        rule.fields(path)
    }
}

fn codec_names(path: &str) -> (syn::Ident, syn::Ident) {
    let id = path.replace("::", "__");
    (format_ident!("enc__{id}"), format_ident!("dec__{id}"))
}

impl<'r> Gen<'r> {
    fn new(
        rule: &'r Rule,
        hashes: &'r HashMap<String, (&'r str, &'r str)>,
        selection: Selection,
    ) -> Self {
        Self {
            rule,
            guest: false,
            probing: false,
            hashes,
            selection,
            ids: HashMap::new(),
            nodes: Vec::new(),
            codecs: HashMap::new(),
            items: Vec::new(),
            skip_reason: "unknown",
        }
    }

    /// An expression of `Value` from `expr`, a reference to a copy of `shape`.
    fn encode(&mut self, shape: &Shape, expr: TokenStream) -> TokenStream {
        // Callers may pass a borrow: projections and method calls must bind to the whole
        // expression, not turn `&key` into `&key.field` or `&key.iter().collect()`.
        let expr = quote!((#expr));
        match shape {
            Shape::Scalar(scalar) => match scalar {
                Scalar::Bool => quote!(V::B(*#expr)),
                Scalar::Char => quote!(V::C(*#expr)),
                Scalar::Signed(_) => quote!(V::S(*#expr as i64)),
                Scalar::Unsigned(_) => quote!(V::U(*#expr as u64)),
                Scalar::Float(_) => quote!(V::F(*#expr as f64)),
                Scalar::Wide(_) => quote!({
                    let bits = *#expr as u128;
                    V::Rec(::std::vec![V::U((bits >> 64) as u64), V::U(bits as u64)])
                }),
                Scalar::Str | Scalar::String => {
                    quote!(V::Str(::std::string::ToString::to_string(&#expr[..])))
                }
            },
            Shape::Unit => quote!(V::None),
            Shape::Ref { to, .. } | Shape::Wrapped(_, to) => self.encode(to, quote!((&**#expr))),
            Shape::Tuple(parts) => {
                let parts: Vec<_> = parts
                    .iter()
                    .enumerate()
                    .map(|(i, p)| {
                        let i = syn::Index::from(i);
                        self.encode(p, quote!((&#expr.#i)))
                    })
                    .collect();
                quote!(V::Rec(::std::vec![#(#parts),*]))
            }
            Shape::Array(elem, _) => {
                let each = self.encode(elem, quote!(item));
                quote!(V::Seq(#expr.iter().map(|item| #each).collect()))
            }
            Shape::Option(inner) => {
                let some = self.encode(inner, quote!(item));
                quote!(match #expr { ::std::option::Option::Some(item) => V::Var(1, ::std::vec![#some]), ::std::option::Option::None => V::Var(0, ::std::vec::Vec::new()) })
            }
            Shape::Named(path, _) => {
                self.codec(path);
                let (enc, _) = codec_names(path);
                quote!(#enc(#expr))
            }
            Shape::Slice(_) | Shape::Collection(..) | Shape::Opaque(_) => {
                unreachable!("not a copy")
            }
        }
    }

    /// An expression of `shape` from `expr`, a `Value`, with `?` on a mismatch.
    fn decode(&mut self, shape: &Shape, expr: TokenStream) -> TokenStream {
        match shape {
            Shape::Scalar(scalar) => match scalar {
                Scalar::Bool => {
                    quote!(match #expr { V::B(b) => b, _ => return Err(I::changed("a bool")) })
                }
                Scalar::Char => {
                    quote!(match #expr { V::C(c) => c, _ => return Err(I::changed("a char")) })
                }
                Scalar::Signed(_) | Scalar::Unsigned(_) => quote!(I::int(#expr)?),
                Scalar::Float(name) => {
                    let name = format_ident!("{name}");
                    quote!(match #expr { V::F(f) => f as #name, _ => return Err(I::changed("a float")) })
                }
                Scalar::Wide(name) => {
                    let name = format_ident!("{name}");
                    quote!({
                        let [high, low] = I::fields::<_, 2>(I::rec(#expr)?)?;
                        ((u128::from(I::int::<u64>(high)?) << 64) | u128::from(I::int::<u64>(low)?)) as #name
                    })
                }
                Scalar::String => {
                    quote!(match #expr { V::Str(s) => s, _ => return Err(I::changed("a string")) })
                }
                Scalar::Str if self.guest => {
                    quote!(match #expr { V::Str(s) => s, _ => return Err(I::changed("a string")) })
                }
                Scalar::Str => unreachable!("not buildable"),
            },
            Shape::Unit => quote!(()),
            Shape::Ref { to: inner, .. } | Shape::Wrapped(_, inner) if self.guest => {
                let inner = self.decode(inner, expr);
                quote!(::std::boxed::Box::new(#inner))
            }
            Shape::Wrapped(Wrapper::Box, inner) => {
                let inner = self.decode(inner, expr);
                quote!(::std::boxed::Box::new(#inner))
            }
            Shape::Tuple(parts) => {
                let names: Vec<_> = (0..parts.len()).map(|i| format_ident!("f{i}")).collect();
                let n = parts.len();
                let parts: Vec<_> = parts
                    .iter()
                    .zip(&names)
                    .map(|(p, name)| self.decode(p, quote!(#name)))
                    .collect();
                quote!({ let [#(#names),*] = I::fields::<_, #n>(I::rec(#expr)?)?; (#(#parts,)*) })
            }
            Shape::Array(elem, n) => {
                let each = self.decode(elem, quote!(item));
                quote!({
                    let items = I::fields::<_, #n>(I::seq(#expr)?)?;
                    let mut out = ::std::vec::Vec::with_capacity(#n);
                    for item in items { out.push(#each); }
                    match <[_; #n]>::try_from(out) { Ok(a) => a, Err(_) => unreachable!() }
                })
            }
            Shape::Option(inner) => {
                let some = self.decode(inner, quote!(item));
                quote!(match #expr {
                    V::Var(0, _) => ::std::option::Option::None,
                    V::Var(1, items) => { let [item] = I::fields::<_, 1>(items)?; ::std::option::Option::Some(#some) }
                    _ => return Err(I::changed("an option")),
                })
            }
            Shape::Named(path, _) => {
                self.codec(path);
                let (_, dec) = codec_names(path);
                quote!(#dec(#expr)?)
            }
            _ => unreachable!("not buildable"),
        }
    }

    /// The codec of the listed copy type at `path`, once.
    fn codec(&mut self, path: &str) {
        if self.codecs.contains_key(path) {
            return;
        }
        let rule = self.rule;
        let build = self.guest || buildable(&Shape::Named(path.into(), 0), rule);
        self.codecs.insert(path.to_owned(), build);
        let named = rule.named(path).expect("listed");
        let ty = if self.guest {
            crate::internals_view::name(path).into_token_stream()
        } else {
            spell(&Shape::Named(path.into(), named.lifetimes)).expect("named")
        };
        let fields = rule.fields(path);
        let shapes: Vec<Shape> = fields
            .iter()
            .map(|f| rule.shape(&named.module, Some(path), f.ty))
            .collect();
        let (enc, dec) = codec_names(path);
        let path_ty = if self.guest {
            let name = crate::internals_view::name(path);
            quote!(#name)
        } else {
            path_tokens(path)
        };
        let (encode, decode) = match &named.item {
            syn::Item::Struct(item) => {
                let binds: Vec<_> = (0..fields.len()).map(|i| format_ident!("f{i}")).collect();
                let members: Vec<_> = fields.iter().map(|f| &f.member).collect();
                // Flags have no field to bind: their one field is their bits.
                let bits = !self.guest && crate::crossing::foreign(path) == Some(Foreign::Bits);
                let bind = if bits {
                    quote!(let f0 = &value.bits();)
                } else {
                    quote!(let #path_ty { #(#members: #binds,)* .. } = value;)
                };
                let encoded: Vec<_> = shapes
                    .iter()
                    .zip(&binds)
                    .map(|(s, b)| self.encode(s, quote!(#b)))
                    .collect();
                let encode = quote!({ #bind V::Rec(::std::vec![#(#encoded),*]) });
                let n = fields.len();
                let decoded: Vec<_> = shapes
                    .iter()
                    .zip(&binds)
                    .map(|(s, b)| {
                        if build {
                            self.decode(s, quote!(#b))
                        } else {
                            quote!()
                        }
                    })
                    .collect();
                let built = if bits {
                    quote!(#path_ty::from_bits_retain(#(#decoded)*))
                } else {
                    quote!(#path_ty { #(#members: #decoded),* })
                };
                let decode = quote!({
                    let [#(#binds),*] = I::fields::<_, #n>(I::rec(value)?)?;
                    Ok(#built)
                });
                let _ = item;
                (encode, decode)
            }
            syn::Item::Enum(item) => {
                let mut encode_arms = Vec::new();
                let mut decode_arms = Vec::new();
                for (case, variant) in item.variants.iter().enumerate() {
                    let case = case as u32;
                    let ident = &variant.ident;
                    let at: Vec<usize> = fields
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| f.case.map(|(c, _)| c) == Some(case))
                        .map(|(i, _)| i)
                        .collect();
                    let binds: Vec<_> = at.iter().map(|i| format_ident!("f{i}")).collect();
                    let members: Vec<_> = at.iter().map(|i| &fields[*i].member).collect();
                    let pattern = quote!(#path_ty::#ident { #(#members: #binds,)* .. });
                    let encoded: Vec<_> = at
                        .iter()
                        .zip(&binds)
                        .map(|(i, b)| self.encode(&shapes[*i], quote!(#b)))
                        .collect();
                    encode_arms
                        .push(quote!(#pattern => V::Var(#case, ::std::vec![#(#encoded),*]),));
                    if build {
                        let decoded: Vec<_> = at
                            .iter()
                            .zip(&binds)
                            .map(|(i, b)| self.decode(&shapes[*i], quote!(#b)))
                            .collect();
                        let n = at.len();
                        decode_arms.push(quote!(V::Var(#case, items) => {
                            let [#(#binds),*] = I::fields::<_, #n>(items)?;
                            Ok(#path_ty::#ident { #(#members: #decoded),* })
                        }));
                    }
                }
                (
                    // Another crate's `#[non_exhaustive]` enum needs a wildcard; every case of this
                    // build is listed, so it never matches.
                    quote!(match value { #(#encode_arms)* _ => V::None }),
                    quote!(match value { #(#decode_arms)* _ => Err(I::changed("a case")) }),
                )
            }
            _ => unreachable!("a struct or an enum"),
        };
        self.items.push(quote! {
            #[allow(unused_variables, unreachable_patterns, clippy::all)]
            fn #enc(value: &#ty) -> V { #encode }
        });
        if build {
            self.items.push(quote! {
                #[allow(unused_variables, clippy::all)]
                fn #dec(value: V) -> ::std::result::Result<#ty, I::Refusal> { #decode }
            });
        }
    }

    /// The id of the walked type `shape`, generating its node function once. Reference and
    /// wrapper layers are stepped through when a node is made, so `shape` is never one.
    fn node(&mut self, shape: &Shape) -> u32 {
        if self.probing {
            return 0;
        }
        let ty = spell(shape).expect("crossable").to_string();
        if let Some(id) = self.ids.get(&ty) {
            return *id;
        }
        let id = self.nodes.len() as u32;
        self.ids.insert(ty.clone(), id);
        let (path, hash) = match shape {
            Shape::Named(path, _) => self
                .hashes
                .get(path)
                .map(|(p, h)| (p.to_string(), h.to_string()))
                .unwrap_or((path.clone(), String::new())),
            _ => (ty, String::new()),
        };
        self.nodes.push(NodeDef {
            path,
            hash,
            ..NodeDef::default()
        });
        // Wrapper traversal must also be raw before a field/index can prove separation.
        fn raw(shape: &Shape) -> bool {
            match shape {
                Shape::Wrapped(Wrapper::Box, to) => raw(to),
                Shape::Ref { .. } | Shape::Wrapped(..) => false,
                _ => true,
            }
        }
        let fields = match shape {
            Shape::Named(path, _) => {
                let named = self.rule.named(path).expect("listed");
                node_fields(self.rule, path)
                    .iter()
                    .enumerate()
                    .filter_map(|(i, f)| {
                        (f.case.is_none() && raw(&self.rule.shape(&named.module, Some(path), f.ty)))
                            .then_some(i as u32)
                    })
                    .collect()
            }
            Shape::Tuple(parts) => parts
                .iter()
                .enumerate()
                .filter_map(|(i, p)| raw(p).then_some(i as u32))
                .collect(),
            _ => Vec::new(),
        };
        if let Shape::Collection(Collection::HashMap | Collection::BTreeMap, args) = shape
            && (!buildable(&args[0], self.rule) || !structural_key(&args[0]))
        {
            self.nodes[id as usize].refused_key =
                Some(spell(&args[0]).map_or_else(|| format!("{:?}", args[0]), |ty| ty.to_string()));
        }
        let raw_gets = match shape {
            Shape::Named(path, _) => {
                let named = self.rule.named(path).expect("listed");
                node_fields(self.rule, path)
                    .iter()
                    .enumerate()
                    .filter_map(|(i, field)| {
                        (field.case.is_none()
                            && raw_scalar(&self.rule.shape(&named.module, Some(path), field.ty)))
                        .then_some(i as u32)
                    })
                    .collect()
            }
            Shape::Tuple(parts) => parts
                .iter()
                .enumerate()
                .filter_map(|(i, part)| raw_scalar(part).then_some(i as u32))
                .collect(),
            _ => Vec::new(),
        };
        self.nodes[id as usize].raw_fields = fields;
        self.nodes[id as usize].raw_gets = raw_gets;
        self.nodes[id as usize].raw_index = match shape {
            Shape::Array(elem, _) | Shape::Slice(elem) => raw(elem),
            Shape::Collection(Collection::Vec, args) => raw(&args[0]),
            _ => false,
        };
        let body = self.node_body(shape);
        let name = format_ident!("node_{id}");
        self.items.push(quote! {
            #[allow(unused_variables, unreachable_patterns, unreachable_code, clippy::all)]
            unsafe fn #name(p: *mut (), w: bool, op: I::Op<'_>) -> ::std::result::Result<I::Reply, I::Refusal> {
                unsafe { #body }
            }
        });
        self.nodes[id as usize].function = quote!(#name);
        id
    }

    /// `Reply::Node` for the value of `shape` that `q` (a `*mut ()`) points to, stepping through
    /// references and wrappers: a node may be written only if `w` and every layer allows it, and
    /// a node that may not is never reached through a `&mut`.
    fn step(&mut self, shape: &Shape, q: TokenStream, w: TokenStream) -> TokenStream {
        match shape {
            Shape::Ref { mutable, to } => {
                let ty = spell(shape).expect("crossable");
                let next = if *mutable {
                    quote!(if w { &raw mut **(q as *mut #ty) as *mut () } else { &raw const **(q as *const #ty) as *mut () })
                } else {
                    quote!(&raw const **(q as *const #ty) as *mut ())
                };
                let w_next = if *mutable { quote!(w) } else { quote!(false) };
                let rest = self.step(to, quote!(q), w_next);
                quote!({ let (q, w): (*mut (), bool) = (#q, #w); let q = #next; #rest })
            }
            Shape::Wrapped(wrapper, to) => {
                let ty = spell(shape).expect("crossable wrapper");
                let project = match wrapper {
                    Wrapper::Box => {
                        quote!(if w { ::std::boxed::Box::as_mut_ptr(&mut *(q as *mut #ty)) as *mut () } else { ::std::boxed::Box::as_ptr(&*(q as *const #ty)) as *mut () })
                    }
                    Wrapper::Arc => {
                        quote!(::std::sync::Arc::as_ptr(&*(q as *const #ty)) as *mut ())
                    }
                    Wrapper::Rc => quote!(::std::rc::Rc::as_ptr(&*(q as *const #ty)) as *mut ()),
                    Wrapper::Cow => quote!((&*(q as *const #ty)).as_ref() as *const _ as *mut ()),
                };
                let w_next = if *wrapper == Wrapper::Box {
                    quote!(w)
                } else {
                    quote!(false)
                };
                let rest = self.step(to, quote!(q), w_next);
                quote!({ let (q, w): (*mut (), bool) = (#q, #w); let q = #project; #rest })
            }
            _ => {
                let id = self.node(shape);
                quote!(Ok(I::Reply::Node(#q, #id, #w)))
            }
        }
    }

    /// The arms for one part numbered `i`: `get` reaches it as `&T`, `set` as `&mut T`, and
    /// `ptr` as a `*mut ()` made through `&mut` only if `w`; each may `return` a refusal.
    fn part(
        &mut self,
        i: u32,
        shape: &Shape,
        get: TokenStream,
        set: TokenStream,
        ptr: TokenStream,
        raw_get: bool,
    ) -> TokenStream {
        match self.rule.crossing_of(shape) {
            Crossing::Opaque => quote!(),
            Crossing::Copy(_) => {
                let read = self.encode(shape, quote!(r));
                let get = if raw_get {
                    let ty = spell(shape).expect("scalar");
                    quote!({ let value = ::std::ptr::read_unaligned((#ptr).cast::<#ty>()); let r = &value; Ok(I::Reply::Value(#read)) })
                } else {
                    quote!({ let r = #get; Ok(I::Reply::Value(#read)) })
                };
                let write = if buildable(shape, self.rule) {
                    let value = self.decode(shape, quote!(value));
                    let shape_rust = spell(shape).expect("copy");
                    quote!(I::Op::Check(#i, value) => {
                        let value = value.clone();
                        let _: #shape_rust = #value;
                        Ok(I::Reply::Done)
                    }, I::Op::Set(#i, value) => {
                        if !w { return Err(I::not_here("a read-only node")); }
                        let value = #value;
                        let r = #set;
                        *r = value;
                        Ok(I::Reply::Done)
                    })
                } else {
                    quote!()
                };
                quote! {
                    I::Op::Get(#i) => #get
                    #write
                }
            }
            Crossing::Handle => {
                let step = self.step(shape, quote!(#ptr), quote!(w));
                quote!(I::Op::Field(#i) => #step,)
            }
        }
    }

    /// `get`, `set` and `ptr` for a part matched out of `this` by `pattern` (binding `field`).
    fn matched(
        rust: &TokenStream,
        pattern: TokenStream,
    ) -> (TokenStream, TokenStream, TokenStream) {
        let miss = quote!(_ => return Err(I::not_here("another case")));
        (
            quote!(match &*(p as *const #rust) { #pattern => field, #miss }),
            quote!(match &mut *(p as *mut #rust) { #pattern => field, #miss }),
            quote!(if w {
                match &mut *(p as *mut #rust) { #pattern => field as *mut _ as *mut (), #miss }
            } else {
                match &*(p as *const #rust) { #pattern => field as *const _ as *mut (), #miss }
            }),
        )
    }

    fn node_body(&mut self, shape: &Shape) -> TokenStream {
        let rule = self.rule;
        let rust = spell(shape).expect("crossable");
        // Reading a whole copy would also expose undeclared fields of a partial owner.
        let full = !matches!(shape, Shape::Named(path, _) if !self.selection.full.contains(path));
        let read = match rule.crossing_of(shape) {
            Crossing::Copy(_) if full => {
                let read = self.encode(shape, quote!(r));
                quote!(I::Op::Read => { let r = &*(p as *const #rust); Ok(I::Reply::Value(#read)) })
            }
            _ => quote!(),
        };
        let len =
            quote!(I::Op::Len => Ok(I::Reply::Value(V::U((&*(p as *const #rust)).len() as u64))),);
        let arms = match shape {
            Shape::Named(path, _) => {
                let named = rule.named(path).expect("listed");
                let path_ty = path_tokens(path);
                let mut arms = Vec::new();
                for (i, field) in node_fields(rule, path).iter().enumerate() {
                    if !full
                        && !self
                            .selection
                            .owners
                            .get(path)
                            .is_some_and(|fields| fields.contains(&i))
                    {
                        continue;
                    }
                    let part_shape = rule.shape(&named.module, Some(path), field.ty);
                    let member = &field.member;
                    let (get, set, ptr) = match field.case {
                        None => (
                            quote!(&(*(p as *const #rust)).#member),
                            quote!(&mut (*(p as *mut #rust)).#member),
                            quote!(if w { &raw mut (*(p as *mut #rust)).#member as *mut () } else { &raw const (*(p as *const #rust)).#member as *mut () }),
                        ),
                        Some((_, case)) => {
                            Self::matched(&rust, quote!(#path_ty::#case { #member: field, .. }))
                        }
                    };
                    arms.push(self.part(
                        i as u32,
                        &part_shape,
                        get,
                        set,
                        ptr,
                        field.case.is_none() && raw_scalar(&part_shape),
                    ));
                }
                let case = match &named.item {
                    syn::Item::Enum(item) => {
                        let cases = item.variants.iter().enumerate().map(|(i, v)| {
                            let ident = &v.ident;
                            let i = i as u64;
                            quote!(#path_ty::#ident { .. } => #i,)
                        });
                        quote!(I::Op::Case => Ok(I::Reply::Value(V::U(match &*(p as *const #rust) { #(#cases)* _ => u64::MAX }))),)
                    }
                    _ => quote!(),
                };
                quote!(#(#arms)* #case)
            }
            Shape::Tuple(parts) => {
                let mut arms = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let index = syn::Index::from(i);
                    arms.push(self.part(
                        i as u32,
                        part,
                        quote!(&(*(p as *const #rust)).#index),
                        quote!(&mut (*(p as *mut #rust)).#index),
                        quote!(if w { &raw mut (*(p as *mut #rust)).#index as *mut () } else { &raw const (*(p as *const #rust)).#index as *mut () }),
                        raw_scalar(part),
                    ));
                }
                quote!(#(#arms)*)
            }
            Shape::Option(inner) => {
                let (get, set, ptr) =
                    Self::matched(&rust, quote!(::std::option::Option::Some(field)));
                let some = self.part(0, inner, get, set, ptr, false);
                quote! {
                    I::Op::Case => Ok(I::Reply::Value(V::U((&*(p as *const #rust)).is_some() as u64))),
                    #some
                }
            }
            Shape::Array(elem, n) => {
                let index = self.raw_indexed(&rust, elem, Some(*n));
                quote!(#len #index)
            }
            Shape::Collection(collection, args) => match collection {
                Collection::Vec => {
                    let index = self.raw_indexed(&rust, &args[0], None);
                    quote!(#len #index)
                }
                Collection::VecDeque => {
                    let index = self.indexed(&rust, &args[0], quote!(get(i)), quote!(get_mut(i)));
                    quote!(#len #index)
                }
                Collection::HashMap | Collection::BTreeMap
                    if buildable(&args[0], rule) && structural_key(&args[0]) =>
                {
                    let key_ty = spell(&args[0]).expect("buildable key");
                    let key = self.decode(&args[0], quote!(key.clone()));
                    let entry = self.element(&args[1]);
                    let encoded = self.encode(&args[0], quote!(&key));
                    quote! {
                        #len
                        I::Op::CanonicalKey(key) => { let key: #key_ty = #key; Ok(I::Reply::Value(#encoded)) }
                        I::Op::Key(key) => {
                            let key = #key;
                            let item: ::std::option::Option<*mut ()> = if w {
                                (&mut *(p as *mut #rust)).get_mut(&key).map(|item| item as *mut _ as *mut ())
                            } else {
                                (&*(p as *const #rust)).get(&key).map(|item| item as *const _ as *mut ())
                            };
                            match item { Some(q) => #entry, None => Err(I::not_here("no such key")) }
                        }
                    }
                }
                _ => len,
            },
            _ => quote!(),
        };
        quote!(match op { #arms #read _ => Err(I::not_here("not on this node")) })
    }

    /// An element's reply, from `q`, a `*mut ()` to it: its node, so it can be written field by
    /// field, or a scalar's copy.
    fn element(&mut self, shape: &Shape) -> TokenStream {
        match self.rule.crossing_of(shape) {
            Crossing::Copy(_) if matches!(shape, Shape::Named(..) | Shape::Tuple(_)) => {
                self.step(shape, quote!(q), quote!(w))
            }
            Crossing::Copy(_) => {
                let rust = spell(shape).expect("crossable");
                let value = self.encode(shape, quote!(r));
                quote!({ let r = &*(q as *const #rust); Ok(I::Reply::Value(#value)) })
            }
            Crossing::Handle => self.step(shape, quote!(q), quote!(w)),
            Crossing::Opaque => quote!(Err(I::not_here("an element that can't cross"))),
        }
    }

    fn raw_indexed(
        &mut self,
        rust: &TokenStream,
        elem: &Shape,
        length: Option<usize>,
    ) -> TokenStream {
        let element = self.element(elem);
        let ty = spell(elem).expect("element");
        let len = length.map_or_else(|| quote!((&*(p as *const #rust)).len()), |n| quote!(#n));
        let ptr = if length.is_none() {
            quote!(if w { (*(p as *mut #rust)).as_mut_ptr() } else { (*(p as *const #rust)).as_ptr() as *mut #ty })
        } else {
            quote!(p.cast::<#ty>())
        };
        quote! {
            I::Op::Index(i) => {
                let i = usize::try_from(i).unwrap_or(usize::MAX);
                if i >= #len { return Err(I::not_here("an index past the end")); }
                let q = (#ptr).add(i).cast::<()>();
                #element
            }
        }
    }

    fn indexed(
        &mut self,
        rust: &TokenStream,
        elem: &Shape,
        get: TokenStream,
        get_mut: TokenStream,
    ) -> TokenStream {
        let element = self.element(elem);
        quote! {
            I::Op::Index(i) => {
                let i = usize::try_from(i).unwrap_or(usize::MAX);
                let item: ::std::option::Option<*mut ()> = if w {
                    (&mut *(p as *mut #rust)).#get_mut.map(|item| item as *mut _ as *mut ())
                } else {
                    (&*(p as *const #rust)).#get.map(|item| item as *const _ as *mut ())
                };
                match item { Some(q) => #element, None => Err(I::not_here("an index past the end")) }
            }
        }
    }

    /// A raw head record uses the exact generic layout emitted by `point_shape::raw_items`.
    /// References in that record borrow the live arguments; operations address their pointees,
    /// not the reference slots. The receiver is a separate root, never a full-layout opt-in.
    fn raw_place(
        &mut self,
        internal: &Internal,
        point: &crate::points::GamePoint,
    ) -> Option<Vec<TokenStream>> {
        let rule = self.rule;
        self.skip_reason = "raw record has an opaque parameter";
        let sig = signature(&internal.signature)?;
        let name = &point.name;
        let shape = point_shape::shape(quote!(name = #name, raw), &sig).ok()?;
        let (module, _) = point.listener.as_ref()?.rsplit_once("::")?;
        let owner = if internal.kind == "method" {
            let (owner, _) = bare(&internal.path)
                .rsplit_once("::")
                .map(|(o, n)| (o.to_owned(), n.to_owned()))?;
            let (_, name) = owner.rsplit_once("::")?;
            let ty: syn::Type = syn::parse_str(name).ok()?;
            match rule.shape(module, None, &ty) {
                Shape::Named(path, _) => Some(path),
                _ => return None,
            }
        } else {
            None
        };
        let receiver = if sig.receiver().is_some() {
            let path = owner.as_ref()?;
            let ty = Shape::Named(path.clone(), rule.named(path)?.lifetimes);
            if rule.crossing_of(&ty) == Crossing::Opaque {
                return None;
            }
            self.selection.owners.entry(path.clone()).or_default();
            let id = self.node(&ty);
            quote!(Some(#id))
        } else {
            quote!(None)
        };
        let mut fields = Vec::new();
        let mut args = Vec::new();
        for field in point_shape::raw_fields(&sig) {
            // API text parameters are the point's Cow, shared with its API listener.
            let ty = if shape.args.iter().any(|a| a.name == field.name)
                && matches!(field.ty, syn::Type::Reference(_))
            {
                syn::parse_quote!(::std::borrow::Cow<'static, str>)
            } else {
                field.ty
            };
            let ty = rule.shape(module, owner.as_deref(), &ty);
            args.push(spell(&ty)?);
            self.selection.reach(&ty, rule);
            fields.push((field.name, ty));
        }
        match shape.raw_outcome(&sig) {
            point_shape::RawOutcome::Cancel => {
                fields.push((format_ident!("cancel"), Shape::Scalar(Scalar::Bool)))
            }
            point_shape::RawOutcome::Result(ty) => {
                let ty = rule.shape(module, owner.as_deref(), &ty);
                args.push(spell(&ty)?);
                let ty = Shape::Option(Box::new(ty));
                self.selection.reach(&ty, rule);
                fields.push((format_ident!("result"), ty));
            }
            point_shape::RawOutcome::None => {}
        }
        let record = path_tokens(&format!("{module}::{}", shape.raw_record()));
        let rust = if fields.is_empty() {
            record
        } else {
            quote!(#record<'static, #(#args),*>)
        };
        let key = rust.to_string();
        let id = self.nodes.len() as u32;
        self.ids.insert(key.clone(), id);
        self.nodes.push(NodeDef {
            path: key,
            ..NodeDef::default()
        });
        let arms: Vec<_> = fields
            .iter()
            .enumerate()
            .map(|(i, (member, ty))| {
                self.part(
                    i as u32,
                    ty,
                    quote!(&*(*(p as *const #rust)).#member),
                    quote!(&mut *(*(p as *mut #rust)).#member),
                    quote!(&raw mut *(*(p as *mut #rust)).#member as *mut ()),
                    false,
                )
            })
            .collect();
        let node = format_ident!("node_{id}");
        self.items.push(quote! {
            #[allow(unused_variables, unused_unsafe, clippy::all)]
            unsafe fn #node(p: *mut (), w: bool, op: I::Op<'_>) -> ::std::result::Result<I::Reply, I::Refusal> {
                unsafe { match op { #(#arms)* _ => Err(I::not_here("not on this raw record")) } }
            }
        });
        self.nodes[id as usize].function = quote!(#node);
        let writable = sig.receiver().is_some_and(|r| r.mutability.is_some());
        let variant = shape.variant();
        let target = &point.target;
        let hash = &point.wire_shape;
        let signature_hash = &internal.hash;
        let mut places = vec![
            quote!(I::Place { place: ::mod_points::PointId::#variant as usize, target: #target, hash: #hash, signature_hash: #signature_hash, receiver: #receiver, receiver_writable: #writable, record: #id },),
        ];
        // A RETURN borrows the returned value directly, not the head's Option<R> slot.
        let returned = match &sig.output {
            syn::ReturnType::Default => Shape::Unit,
            syn::ReturnType::Type(_, ty) => rule.shape(module, owner.as_deref(), ty),
        };
        if rule.crossing_of(&returned) != Crossing::Opaque {
            self.selection.reach(&returned, rule);
            let rust = spell(&returned)?;
            let id = self.nodes.len() as u32;
            let node = format_ident!("node_{id}");
            let path = format!("{module}::{}ReturnPlace", shape.raw_record());
            self.nodes.push(NodeDef {
                path,
                function: quote!(#node),
                ..NodeDef::default()
            });
            let arm = self.part(
                0,
                &returned,
                quote!(&*(p as *const #rust)),
                quote!(&mut *(p as *mut #rust)),
                quote!(p),
                false,
            );
            self.items.push(quote! {
                #[allow(unused_variables, unused_unsafe, clippy::all)]
                unsafe fn #node(p: *mut (), w: bool, op: I::Op<'_>) -> ::std::result::Result<I::Reply, I::Refusal> {
                    unsafe { match op { #arm _ => Err(I::not_here("not on this return place")) } }
                }
            });
            let receiver = if shape.borrowed_return {
                quote!(None)
            } else {
                receiver
            };
            let variant = shape.return_variant();
            places.push(quote!(I::Place { place: ::mod_points::PointId::#variant as usize, target: #target, hash: #hash, signature_hash: #signature_hash, receiver: #receiver, receiver_writable: #writable, record: #id },));
        }
        Some(places)
    }

    /// The thunk of the listed function `internal`, if every part of it can cross.
    fn thunk(&mut self, internal: &Internal) -> Option<TokenStream> {
        let rule = self.rule;
        self.skip_reason = "signature, generic impl, or unsafe/async function";
        let sig = signature(&internal.signature)?;
        if sig.generics.params.len() != sig.generics.lifetimes().count()
            || sig.asyncness.is_some()
            || sig.unsafety.is_some()
            || sig.variadic.is_some()
        {
            return None;
        }
        self.skip_reason = "closure argument; declare a non-closure helper";
        if closure_argument(&sig) {
            return None;
        }
        self.skip_reason = "unresolved or opaque owner";
        let path = bare(&internal.path);
        let (_, name) = path.rsplit_once("::")?;
        let name = format_ident!("{name}");
        let (module, this) = function_scope(rule, internal)?;
        self.skip_reason = "static or bounded input lifetime";
        struct StaticInput(bool);
        impl<'ast> syn::visit::Visit<'ast> for StaticInput {
            fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
                self.0 |= lifetime.ident == "static";
            }
        }
        let mut static_input = StaticInput(false);
        for input in &sig.inputs {
            syn::visit::Visit::visit_fn_arg(&mut static_input, input);
        }
        if static_input.0
            || sig.generics.where_clause.is_some()
            || sig.generics.lifetimes().any(|l| !l.bounds.is_empty())
        {
            return None;
        }
        let mut writes = 0u64;
        let mut receiver = None;
        let mut inputs = sig.inputs.iter();
        if let Some(this) = &this {
            let Some(syn::FnArg::Receiver(r)) = inputs.next() else {
                // An associated function: called on the type, no receiver.
                let callee = Callee {
                    internal,
                    module: &module,
                    this: Some(this),
                    name: &name,
                    receiver: None,
                };
                return self.thunk_tail(&callee, sig.inputs.iter(), &sig.output, 0);
            };
            self.skip_reason = "by-value receiver";
            r.reference.as_ref()?;
            let shape = Shape::Named(this.clone(), rule.named(this)?.lifetimes);
            self.skip_reason = "receiver cannot cross";
            if rule.crossing_of(&shape) == Crossing::Opaque {
                return None;
            }
            self.selection.owners.entry(this.clone()).or_default();
            let ty = spell(&shape)?;
            if r.mutability.is_some() {
                writes |= 1;
                receiver = Some((
                    quote!(&mut *(this.ok_or(I::changed("a receiver"))? as *mut #ty),),
                    self.node(&shape),
                ));
            } else {
                receiver = Some((
                    quote!(&*(this.ok_or(I::changed("a receiver"))? as *const #ty),),
                    self.node(&shape),
                ));
            }
        }
        let callee = Callee {
            internal,
            module: &module,
            this: this.as_deref(),
            name: &name,
            receiver,
        };
        self.thunk_tail(&callee, inputs, &sig.output, writes)
    }

    fn thunk_tail<'a>(
        &mut self,
        callee: &Callee<'_>,
        inputs: impl Iterator<Item = &'a syn::FnArg>,
        output: &syn::ReturnType,
        mut writes: u64,
    ) -> Option<TokenStream> {
        let rule = self.rule;
        let Callee {
            internal,
            module,
            this,
            name,
            ..
        } = *callee;
        let mut nodes = Vec::new();
        let mut lets = Vec::new();
        let mut args = Vec::new();
        let mut updates = Vec::new();
        let mut borrowed_copy = false;
        // Named input lifetimes can be tied to a receiver's stored borrows. Spelling a listed
        // lifetime-bearing input with `'static` would also invent an outlives guarantee.
        struct InputLifetimes<'r> {
            rule: &'r Rule,
            module: &'r str,
            this: Option<&'r str>,
            forbidden: bool,
        }
        impl<'ast> syn::visit::Visit<'ast> for InputLifetimes<'_> {
            fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
                self.forbidden |= lifetime.ident != "_";
            }

            fn visit_type(&mut self, ty: &'ast syn::Type) {
                if matches!(self.rule.shape(self.module, self.this, ty), Shape::Named(_, n) if n > 0)
                {
                    self.forbidden = true;
                }
                syn::visit::visit_type(self, ty);
            }
        }
        self.skip_reason = "argument cannot cross or be built";
        for (i, input) in inputs.enumerate() {
            let syn::FnArg::Typed(input) = input else {
                return None;
            };
            let mut lifetimes = InputLifetimes {
                rule,
                module,
                this,
                forbidden: false,
            };
            syn::visit::Visit::visit_type(&mut lifetimes, &input.ty);
            if lifetimes.forbidden {
                self.skip_reason = "named input lifetime or lifetime-bearing input type";
                return None;
            }
            let shape = rule.shape(module, this, &input.ty);
            self.selection.reach(&shape, rule);
            let arg = format_ident!("a{i}");
            match (&shape, rule.crossing_of(&shape)) {
                (_, Crossing::Opaque) => return None,
                (Shape::Ref { mutable, to }, _) if rule.crossing_of(to) == Crossing::Handle => {
                    let ty = spell(to)?;
                    let id = self.node(to);
                    nodes.push(quote!(::std::option::Option::Some(#id)));
                    if *mutable {
                        writes |= 1 << (i + 1);
                        lets.push(quote!(let #arg = &mut *(I::node(#arg)? as *mut #ty);));
                    } else {
                        lets.push(quote!(let #arg = &*(I::node(#arg)? as *const #ty);));
                    }
                    args.push(quote!(#arg));
                }
                (Shape::Ref { mutable, to }, Crossing::Copy(_)) => {
                    borrowed_copy = true;
                    if !*mutable && matches!(**to, Shape::Scalar(Scalar::Str)) {
                        let value =
                            self.decode(&Shape::Scalar(Scalar::String), quote!(I::value(#arg)?));
                        lets.push(quote!(let #arg: ::std::string::String = #value;));
                        args.push(quote!(&#arg[..]));
                        nodes.push(quote!(::std::option::Option::None));
                    } else if buildable(to, rule) {
                        let value = self.decode(to, quote!(I::value(#arg)?));
                        let mutability = mutable.then(|| quote!(mut));
                        lets.push(quote!(let #mutability #arg = #value;));
                        args.push(quote!(&#mutability #arg));
                        if *mutable {
                            updates.push(self.encode(to, quote!((&#arg))));
                        }
                        nodes.push(quote!(::std::option::Option::None));
                    } else {
                        return None;
                    }
                }
                (_, Crossing::Copy(_)) if buildable(&shape, rule) => {
                    let value = self.decode(&shape, quote!(I::value(#arg)?));
                    lets.push(quote!(let #arg = #value;));
                    args.push(quote!(#arg));
                    nodes.push(quote!(::std::option::Option::None));
                }
                _ => return None,
            }
        }
        let n = args.len();
        let names: Vec<_> = (0..n).map(|i| format_ident!("a{i}")).collect();
        self.skip_reason = "result cannot cross";
        let result = match output {
            syn::ReturnType::Default => quote!(Ok(I::Reply::Done)),
            syn::ReturnType::Type(_, ty) => {
                let shape = rule.shape(module, this, ty);
                if borrowed_copy
                    && callee.receiver.is_none()
                    && rule.crossing_of(&shape) == Crossing::Handle
                {
                    struct Elision(bool);
                    impl<'ast> syn::visit::Visit<'ast> for Elision {
                        fn visit_type_reference(&mut self, r: &'ast syn::TypeReference) {
                            self.0 |= r.lifetime.as_ref().is_none_or(|l| l.ident == "_");
                            syn::visit::visit_type_reference(self, r);
                        }
                    }
                    let mut elision = Elision(false);
                    syn::visit::Visit::visit_type(&mut elision, ty);
                    if elision.0 {
                        self.skip_reason = "returned node may borrow a decoded copy";
                        return None;
                    }
                }
                if let Some(reference) = optional_reference(rule, module, this, ty) {
                    self.selection.reach(&reference, rule);
                    let node =
                        self.step(&reference, quote!(&raw mut value as *mut ()), quote!(true));
                    quote!(match result {
                        Some(mut value) => #node,
                        None => Ok(I::Reply::Value(V::Var(0, vec![]))),
                    })
                } else {
                    let success = result_success(rule, module, this, ty);
                    let shape = success.clone().unwrap_or(shape);
                    self.selection.reach(&shape, rule);
                    match (&shape, rule.crossing_of(&shape)) {
                        (_, Crossing::Opaque) => return None,
                        (_, Crossing::Copy(_)) => {
                            if success.is_some() {
                                let value = self.encode(&shape, quote!((&value)));
                                quote!(Ok(I::Reply::Value(match result {
                                    Ok(value) => V::Var(0, vec![#value]),
                                    Err(_) => V::Var(1, vec![]),
                                })))
                            } else {
                                let value = self.encode(&shape, quote!((&result)));
                                quote!(Ok(I::Reply::Value(#value)))
                            }
                        }
                        (Shape::Ref { .. }, Crossing::Handle) => {
                            let node =
                                self.step(&shape, quote!(&raw mut result as *mut ()), quote!(true));
                            quote!({ let mut result = result; #node })
                        }
                        _ => return None,
                    }
                }
            }
        };
        let function = match this {
            Some(this) => {
                let ty = path_tokens(this);
                quote!(#ty::#name)
            }
            None => {
                let owner = bare(&internal.path);
                path_tokens(&owner)
            }
        };
        let (receiver, receiver_ty) = match &callee.receiver {
            Some((receiver, id)) => (receiver.clone(), quote!(::std::option::Option::Some(#id))),
            None => (quote!(), quote!(::std::option::Option::None)),
        };
        let thunk = format_ident!("thunk_{}", self.items.len());
        let path = &internal.path;
        let hash = &internal.hash;
        self.items.push(quote! {
            #[allow(unused_variables, unused_mut, clippy::all)]
            unsafe fn #thunk(this: ::std::option::Option<*mut ()>, args: ::std::vec::Vec<I::Arg>) -> ::std::result::Result<I::CallReply, I::Refusal> {
                unsafe {
                    let [#(#names),*] = I::fields::<_, #n>(args)?;
                    #(#lets)*
                    let result = #function(#receiver #(#args),*);
                    let result: ::std::result::Result<I::Reply, I::Refusal> = #result;
                    Ok(I::CallReply { result: result?, updates: vec![#(#updates),*] })
                }
            }
        });
        Some(
            quote!(I::Thunk { path: #path, hash: #hash, receiver: #receiver_ty, args: &[#(#nodes),*], writes: #writes, call: #thunk },),
        )
    }
}

/// A function return may discard an opaque error while preserving its copyable success value.
/// This applies to written `Result<T>` aliases too, without ever naming their error type.
pub(crate) fn result_success(
    rule: &Rule,
    module: &str,
    this: Option<&str>,
    ty: &syn::Type,
) -> Option<Shape> {
    let syn::Type::Path(ty) = ty else {
        return None;
    };
    let segment = ty.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    if !(1..=2).contains(&args.args.len()) {
        return None;
    }
    let syn::GenericArgument::Type(success) = args.args.first()? else {
        return None;
    };
    let shape = rule.shape(module, this, success);
    matches!(rule.crossing_of(&shape), Crossing::Copy(_)).then_some(shape)
}

/// A returned optional reference to state reaches the same call-scoped node as a reference.
pub(crate) fn optional_reference(
    rule: &Rule,
    module: &str,
    this: Option<&str>,
    ty: &syn::Type,
) -> Option<Shape> {
    let Shape::Option(inner) = rule.shape(module, this, ty) else {
        return None;
    };
    matches!(&*inner, Shape::Ref { .. }).then_some(())?;
    (rule.crossing_of(&inner) == Crossing::Handle).then_some(*inner)
}

/// A function a thunk calls, and how.
struct Callee<'c> {
    internal: &'c Internal,
    /// The module its signature is written in.
    module: &'c str,
    /// Its receiver's type, for a method or an associated function.
    this: Option<&'c str>,
    name: &'c syn::Ident,
    /// The receiver argument and its node type, for a method.
    receiver: Option<(TokenStream, u32)>,
}

/// Resolve the receiver exactly where its method signature is written.
pub(crate) fn function_scope(rule: &Rule, internal: &Internal) -> Option<(String, Option<String>)> {
    let path = bare(&internal.path);
    let (owner, _) = path.rsplit_once("::")?;
    let (module, this) = match internal.kind {
        "method" => {
            let (module, ty) = owner.rsplit_once("::")?;
            let written: syn::Path = syn::parse_str(ty).ok()?;
            let ty = match rule.shape(
                module,
                None,
                &syn::Type::Path(syn::TypePath {
                    qself: None,
                    path: written,
                }),
            ) {
                Shape::Named(path, _) => path,
                _ => return None,
            };
            (module.to_owned(), Some(ty))
        }
        "fn" => (owner.to_owned(), None),
        _ => return None,
    };
    Some((module, this))
}

/// The manifest records impl bounds as well as the method signature. A bounded or type-generic
/// impl is not callable on every receiver, even when the method itself has no generics.
pub(crate) fn signature(written: &str) -> Option<syn::Signature> {
    use syn::parse::Parser;
    let parser = |input: syn::parse::ParseStream<'_>| {
        if input.peek(syn::Token![impl]) {
            input.parse::<syn::Token![impl]>()?;
            let generics: syn::Generics = input.parse()?;
            let _: syn::Type = input.parse()?;
            let bounds: Option<syn::WhereClause> = input.parse()?;
            if generics.params.len() != generics.lifetimes().count()
                || generics.lifetimes().any(|l| !l.bounds.is_empty())
                || bounds.is_some()
            {
                return Err(input.error("a generic or bounded impl"));
            }
            let content;
            syn::braced!(content in input);
            content.parse::<syn::Signature>()
        } else {
            input.parse::<syn::Signature>()
        }
    };
    parser.parse_str(written).ok()
}

/// Plain guest copies use the exact same value encoding as the table's native copies.
pub(crate) fn guest_codecs(rule: &Rule, shapes: &[Shape]) -> TokenStream {
    let hashes = HashMap::new();
    let mut generator = Gen::new(rule, &hashes, Selection::default());
    generator.guest = true;
    let mut impls = Vec::new();
    for shape in shapes {
        let ty = crate::internals_view::spell(shape, rule);
        let encode = generator.encode(shape, quote!(self));
        let decode = generator.decode(shape, quote!(value));
        impls.push(quote! {
            impl CopyValue for #ty {
                fn to_value(&self) -> V { #encode }
                fn from_value(value: V) -> Result<Self, Refusal> {
                    (|| -> Result<Self, I::Refusal> { Ok(#decode) })().map_err(wire_refusal)
                }
            }
            impl ViewType for #ty {
                fn from_reached(reached: Reached) -> Result<Self, Refusal> {
                    match reached {
                        Reached::Value(cells) => Self::from_value(unflatten(cells).map_err(wire_refusal)?),
                        Reached::Node(n) => Self::from_value(unflatten(n.read()?).map_err(wire_refusal)?),
                    }
                }
            }
        });
    }
    let items = generator.items;
    quote!(#(#items)* #(#impls)*)
}

fn native_only() -> Vec<&'static str> {
    include_str!("../native-only.txt")
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|path| !path.is_empty())
        .collect()
}

/// Shared layout reach and callable validation for the game's table and the mod's view.
pub(crate) struct Selected<'a> {
    pub(crate) selection: Selection,
    pub(crate) counts: Counts,
    pub(crate) accepted: Vec<&'a Internal>,
    pub(crate) raw: Vec<(&'a Internal, &'a crate::points::GamePoint)>,
    pub(crate) members: Vec<(&'a Internal, String, u32)>,
}

pub(crate) fn select<'a>(
    internals: &'a [Internal],
    declared: BTreeSet<&str>,
    points: &'a [crate::points::GamePoint],
    rule: &Rule,
    hashes: &HashMap<String, (&str, &str)>,
) -> Selected<'a> {
    let native_only = native_only();
    let mut counts = Counts {
        declared: declared.len(),
        ..Counts::default()
    };
    let mut selection = Selection::default();
    // Index field identities once: catalog-wide coverage selects thousands of fields.
    let fields: HashMap<_, _> = rule
        .paths()
        .flat_map(|owner| {
            node_fields(rule, owner)
                .into_iter()
                .enumerate()
                .map(move |(index, field)| {
                    let member = field.member.to_token_stream().to_string();
                    let path = match field.case {
                        Some((_, case)) => format!("{owner}::{case}::{member}"),
                        None => format!("{owner}::{member}"),
                    };
                    (path, (owner, index, field))
                })
        })
        .collect();
    let mut functions = Vec::new();
    let mut members = Vec::new();
    let mut exposed = HashSet::new();
    for path in declared {
        if native_only.contains(&path) {
            counts.refused.insert(path.into(), "native only");
            continue;
        }
        let entries: Vec<_> = internals.iter().filter(|i| i.path == path).collect();
        if !entries.iter().any(|i| i.cfg.is_empty()) {
            counts.refused.insert(
                path.into(),
                if entries.is_empty() {
                    "not in internals.json"
                } else {
                    "conditional item"
                },
            );
            continue;
        }
        // A field and a method can legitimately have the same manifest path.
        for internal in entries.into_iter().filter(|i| i.cfg.is_empty()) {
            let reason = match internal.kind {
                "method" | "fn" => {
                    functions.push(internal);
                    continue;
                }
                "struct" | "enum" => {
                    let ty = bare(path);
                    let Some(named) = rule.named(&ty) else {
                        counts.refused.insert(path.into(), "unresolved type layout");
                        continue;
                    };
                    let shape = Shape::Named(ty.clone(), named.lifetimes);
                    if rule.crossing_of(&shape) == Crossing::Opaque {
                        let reason = rule
                            .named(&ty)
                            .and_then(|n| n.opaque_reason)
                            .unwrap_or("opaque type");
                        *counts.skipped_types.entry(reason).or_default() += 1;
                        reason
                    } else {
                        selection.reach(&shape, rule);
                        exposed.insert(path.to_owned());
                        continue;
                    }
                }
                "field" => {
                    // An enum field's manifest path includes its case; numbering spans all cases.
                    let field = fields
                        .get(&bare(path))
                        .map(|(owner, index, field)| (*owner, *index, field));
                    if let Some((owner, index, field)) = field {
                        let named = rule.named(owner).expect("listed");
                        let shape = rule.shape(&named.module, Some(owner), field.ty);
                        let owner_shape = Shape::Named(owner.into(), named.lifetimes);
                        if rule.crossing_of(&owner_shape) == Crossing::Opaque {
                            named.opaque_reason.unwrap_or("opaque owner")
                        } else if rule.crossing_of(&shape) == Crossing::Opaque {
                            "field type cannot cross"
                        } else {
                            selection
                                .owners
                                .entry(owner.into())
                                .or_default()
                                .insert(index);
                            members.push((internal, owner.to_owned(), index as u32));
                            selection.reach(&shape, rule);
                            exposed.insert(path.to_owned());
                            continue;
                        }
                    } else {
                        "unresolved field owner"
                    }
                }
                _ => "unsupported item kind",
            };
            counts.unbound.insert(
                (internal.path.clone(), internal.kind, internal.hash.clone()),
                reason,
            );
            counts.refused.insert(path.into(), reason);
        }
    }
    // Validate functions before generating the final nodes: arguments/results may promote a
    // partial owner to a full layout. Discard each refused probe, including its nodes/codecs.
    let mut accepted = Vec::new();
    let mut raw = Vec::new();
    for internal in functions {
        let mut has_place = false;
        if let Some(point) = points.iter().find(|p| p.raw && p.target == internal.path) {
            let mut probe = Gen {
                probing: true,
                ..Gen::new(rule, hashes, selection.clone())
            };
            if probe.raw_place(internal, point).is_some() {
                selection = probe.selection;
                raw.push((internal, point));
                has_place = true;
                exposed.insert(internal.path.clone());
            }
        }
        let mut probe = Gen {
            probing: true,
            ..Gen::new(rule, hashes, selection.clone())
        };
        if probe.thunk(internal).is_some() {
            selection = probe.selection;
            if let Some((_, Some(owner))) = function_scope(rule, internal) {
                let path = bare(&internal.path);
                let func = path.rsplit("::").next().unwrap();
                let guest = crate::internals_view::method_name(rule, &owner, func);
                if guest != func {
                    counts.renamed_methods.insert(internal.path.clone(), guest);
                }
            }
            accepted.push(internal);
            exposed.insert(internal.path.clone());
        } else {
            counts.unbound.insert(
                (internal.path.clone(), internal.kind, internal.hash.clone()),
                probe.skip_reason,
            );
            if !has_place {
                *counts
                    .skipped_functions
                    .entry(probe.skip_reason)
                    .or_default() += 1;
                counts
                    .refused
                    .insert(internal.path.clone(), probe.skip_reason);
            }
        }
    }
    counts.refused.retain(|path, _| !exposed.contains(path));
    counts.skipped = counts.refused.len();
    Selected {
        selection,
        counts,
        accepted,
        raw,
        members,
    }
}

/// The `__internals` module for the union of declared manifest paths, and what it holds.
/// Missing or unsupported declarations are reported, not fatal: the loader refuses that mod.
pub fn table(internals: &[Internal], declared: &[String]) -> (String, Counts) {
    table_with_points(internals, declared, &[])
}

/// The table, including the declared raw injection places of the staged game.
pub fn table_with_points(
    internals: &[Internal],
    declared: &[String],
    points: &[crate::points::GamePoint],
) -> (String, Counts) {
    let hashes: HashMap<_, _> = internals
        .iter()
        .filter(|i| matches!(i.kind, "struct" | "enum") && i.cfg.is_empty())
        .map(|i| (bare(&i.path), (i.path.as_str(), i.hash.as_str())))
        .collect();
    let rule = Rule::new(internals);
    let declared: BTreeSet<_> = declared.iter().map(String::as_str).collect();
    let native_only = native_only();
    let Selected {
        selection,
        mut counts,
        accepted,
        raw,
        members,
    } = select(internals, declared, points, &rule, &hashes);
    let mut roots: BTreeSet<_> = selection.full.iter().cloned().collect();
    roots.extend(selection.owners.keys().cloned());
    let mut generator = Gen::new(&rule, &hashes, selection);
    for path in roots {
        generator.node(&Shape::Named(
            path.clone(),
            rule.named(&path).expect("listed").lifetimes,
        ));
    }
    let thunks: Vec<_> = accepted
        .into_iter()
        .map(|i| generator.thunk(i).expect("validated thunk"))
        .collect();
    let places: Vec<_> = raw
        .into_iter()
        .flat_map(|(internal, point)| {
            generator
                .raw_place(internal, point)
                .expect("validated raw place")
        })
        .collect();
    let members: Vec<_> = members
        .into_iter()
        .map(|(internal, owner, field)| {
            let id = generator.node(&Shape::Named(
                owner.clone(),
                rule.named(&owner).expect("listed").lifetimes,
            ));
            let (path, hash) = (&internal.path, &internal.hash);
            quote!(I::Member { path: #path, hash: #hash, owner: #id, field: #field },)
        })
        .collect();
    counts.refused_map_keys = generator
        .nodes
        .iter()
        .filter_map(|n| n.refused_key.clone())
        .collect();
    counts.types = generator.nodes.len();
    counts.thunks = thunks.len();
    let types = generator
        .nodes
        .iter()
        .map(|NodeDef { path, hash, function, raw_fields, raw_gets, raw_index, .. }| quote!(I::Type { path: #path, hash: #hash, node: #function, raw_fields: &[#(#raw_fields),*], raw_gets: &[#(#raw_gets),*], raw_index: #raw_index },));
    let items = &generator.items;
    let code = quote! {
        #![allow(unsafe_code, unused_unsafe, unused_parens, non_snake_case, dead_code)]
        use ::mod_points::internals as I;
        use I::Value as V;
        #(#items)*
        /// The game's opt-in internals table, generated by the staging pass.
        pub static TABLE: I::Table = I::Table {
            types: &[#(#types)*],
            thunks: &[#(#thunks)*],
            places: &[#(#places)*],
            members: &[#(#members)*],
            native_only: &[#(#native_only),*],
        };
    };
    (code.to_string(), counts)
}

#[cfg(test)]
pub(crate) mod tests {
    #[test]
    fn map_keys_require_structural_equality() {
        use crate::crossing::{Scalar, Shape};
        assert!(super::structural_key(&Shape::Option(Box::new(
            Shape::Scalar(Scalar::Unsigned("u32"))
        ))));
        assert!(!super::structural_key(&Shape::Named("CustomEq".into(), 0)));
    }

    use super::*;
    use crate::crossing::tests::internal;

    fn all_table(internals: &[Internal]) -> (String, Counts) {
        table(
            internals,
            &internals.iter().map(|i| i.path.clone()).collect::<Vec<_>>(),
        )
    }

    pub(crate) fn run_fixture(code: &str, declarations: &str, body: &str) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let build = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| root.join("target"), std::path::PathBuf::from)
            .join(format!(
                "internals-table-fixture-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
        std::fs::create_dir_all(&build).unwrap();
        let runtime = root
            .join("crates/horizon-mod-points/src/internals.rs")
            .canonicalize()
            .unwrap();
        let source = format!(
            r#"
            extern crate self as mod_points;
            #[path = {runtime:?}] pub mod internals;
            {declarations}
            mod __internals {{ {code} }}
            fn main() {{ use internals::{{Op, Reply, Value}}; {body} }}
        "#
        );
        let input = build.join("main.rs");
        let output = build.join(format!("fixture{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&input, source).unwrap();
        let compiled = std::process::Command::new("rustc")
            .args(["--edition=2024", "-Dwarnings"])
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
    fn mutable_copy_arguments_round_trip_through_generated_thunks() {
        let entries = [
            internal(
                "horizon_client::Pair",
                "struct",
                "struct Pair { x: u32, flag: bool }",
            ),
            internal(
                "horizon_client::mutate",
                "fn",
                "fn mutate(n: &mut u32, pair: &mut Pair) -> u32",
            ),
            internal(
                "horizon_client::borrow",
                "fn",
                "fn borrow(n: &mut u32) -> &mut u32",
            ),
            internal(
                "horizon_client::fail",
                "fn",
                "fn fail(n: &mut u32) -> Result<(), String>",
            ),
            internal("horizon_client::unit", "fn", "fn unit(n: &mut u32)"),
        ];
        let (code, counts) = all_table(&entries);
        assert_eq!(counts.thunks, 4);
        run_fixture(
            &code,
            r#"
            struct Pair { x: u32, flag: bool }
            fn mutate(n: &mut u32, pair: &mut Pair) -> u32 { *n += 5; pair.x += 7; pair.flag = !pair.flag; *n + pair.x }
            fn borrow(n: &mut u32) -> &mut u32 { *n += 1; n }
            fn fail(n: &mut u32) -> Result<(), String> { *n = 99; Err("failed".into()) }
            fn unit(n: &mut u32) { *n = 42; }
        "#,
            r#"
            use internals::{Arg, CallReply};
            let mut n = 0u32;
            let mut pair = Pair { x: 2, flag: false };
            let ty = __internals::TABLE.types.iter().find(|t| t.path.ends_with("Pair")).unwrap();
            unsafe { assert_eq!((ty.node)((&raw mut pair).cast(), true, Op::Read), Ok(Reply::Value(Value::Rec(vec![Value::U(2), Value::B(false)])))); }
            for thunk in __internals::TABLE.thunks {
                assert_eq!(thunk.writes, 0, "copy locals never write through nodes");
                assert!(thunk.args.iter().all(Option::is_none));
                unsafe { assert!((thunk.call)(None, vec![Arg::Node((&raw mut n).cast())]).is_err()); }
                let mut args = vec![Arg::Value(Value::U(7))];
                let (result, updates) = if thunk.path.ends_with("mutate") {
                    args.push(Arg::Value(Value::Rec(vec![Value::U(2), Value::B(false)])));
                    (Reply::Value(Value::U(21)), vec![Value::U(12), Value::Rec(vec![Value::U(9), Value::B(true)])])
                } else if thunk.path.ends_with("borrow") {
                    (Reply::Value(Value::U(8)), vec![Value::U(8)])
                } else if thunk.path.ends_with("fail") {
                    (Reply::Value(Value::Var(1, vec![])), vec![Value::U(99)])
                } else { (Reply::Done, vec![Value::U(42)]) };
                unsafe { assert_eq!((thunk.call)(None, args), Ok(CallReply { result, updates })); }
            }
        "#,
        );
    }

    #[test]
    fn returned_nodes_cannot_borrow_decoded_copy_locals() {
        let mut entries = vec![
            internal(
                "horizon_client::State",
                "struct",
                "struct State { values: Vec<u8> }",
            ),
            internal(
                "horizon_client::State::bytes",
                "method",
                "fn bytes(&self, n: &mut String) -> &Vec<u8>",
            ),
        ];
        entries.extend(
            [
                "fn bytes(n: &mut String) -> &Vec<u8>",
                "fn maybe(n: &mut String) -> Option<&Vec<u8>>",
                "fn shared(n: &String) -> &Vec<u8>",
                "fn named<'a>(n: &'a mut String) -> &'a Vec<u8>",
                "fn fixed(n: &mut String) -> &'static Vec<u8>",
            ]
            .map(|sig| {
                let name = signature(sig).unwrap().ident;
                internal(&format!("horizon_client::{name}"), "fn", sig)
            }),
        );
        let (code, counts) = all_table(&entries);
        assert_eq!(counts.thunks, 2);
        let skipped = &counts.skipped_functions;
        assert_eq!(skipped["returned node may borrow a decoded copy"], 3);
        assert_eq!(
            skipped["named input lifetime or lifetime-bearing input type"],
            1
        );
        run_fixture(
            &code,
            r#"
            struct State { values: Vec<u8> }
            impl State { fn bytes(&self, n: &mut String) -> &Vec<u8> { n.clear(); &self.values } }
            fn fixed(_: &mut String) -> &'static Vec<u8> { static EMPTY: Vec<u8> = Vec::new(); &EMPTY }
        "#,
            r#"
            let mut state = State { values: vec![42] };
            for thunk in __internals::TABLE.thunks {
                let receiver = thunk.receiver.map(|_| (&raw mut state).cast());
                let reply = unsafe { (thunk.call)(receiver, vec![internals::Arg::Value(Value::Str("copy".into()))]).unwrap() };
                assert_eq!(reply.updates.len(), 1);
                let Reply::Node(p, ty, w) = reply.result else { panic!("node") };
                assert_eq!(unsafe { (__internals::TABLE.types[ty as usize].node)(p, w, Op::Len) }, Ok(Reply::Value(Value::U(if receiver.is_some() { 1 } else { 0 }))));
            }
        "#,
        );
    }

    #[test]
    fn coverage_lists_refused_map_key_types() {
        let entries = [
            internal("horizon_client::Key", "struct", "struct Key { id: u32 }"),
            internal(
                "horizon_client::Bag",
                "struct",
                "struct Bag { values: std::collections::BTreeMap<Key, u32> }",
            ),
        ];
        let (_, counts) = all_table(&entries);
        assert!(
            counts
                .refused_map_keys
                .iter()
                .any(|key| key.contains("Key")),
            "{:?}",
            counts.refused_map_keys
        );
    }

    #[test]
    fn generated_option_map_keys_canonicalize_before_writes() {
        let entries = [
            internal(
                "horizon_client::Player",
                "struct",
                "struct Player { health: u32 }",
            ),
            internal(
                "horizon_client::Bag",
                "struct",
                "struct Bag { players: std::collections::BTreeMap<Option<u32>, Player> }",
            ),
        ];
        let (code, counts) = all_table(&entries);
        assert!(counts.refused_map_keys.is_empty());
        run_fixture(
            &code,
            "struct Player { health: u32 } struct Bag { players: std::collections::BTreeMap<Option<u32>, Player> }",
            r#"
            let mut bag = Bag { players: std::collections::BTreeMap::from([(None, Player { health: 1 })]) };
            let ty = __internals::TABLE.types.iter().find(|t| t.path.ends_with("Bag")).unwrap();
            unsafe {
                let Reply::Node(map, map_ty, true) = (ty.node)((&raw mut bag).cast(), true, Op::Field(0)).unwrap() else { panic!() };
                let map_node = __internals::TABLE.types[map_ty as usize].node;
                let Reply::Value(key1) = map_node(map, true, Op::CanonicalKey(&Value::Var(0, vec![]))).unwrap() else { panic!() };
                let Reply::Value(key2) = map_node(map, true, Op::CanonicalKey(&Value::Var(0, vec![Value::U(42)]))).unwrap() else { panic!() };
                assert_eq!(key1, key2);
                let Reply::Node(player, ty, true) = map_node(map, true, Op::Key(&key1)).unwrap() else { panic!() };
                let player_node = __internals::TABLE.types[ty as usize].node;
                assert_eq!(player_node(player, true, Op::Set(0, Value::U(17))), Ok(Reply::Done));
                let Reply::Node(player, ty, _) = map_node(map, false, Op::Key(&key2)).unwrap() else { panic!() };
                assert_eq!((__internals::TABLE.types[ty as usize].node)(player, false, Op::Get(0)), Ok(Reply::Value(Value::U(17))));
            }
        "#,
        );
    }

    #[test]
    fn generated_shared_enum_arguments() {
        let entries = [
            internal(
                "horizon_client::Large",
                "struct",
                "struct Large { data: [u64; 17] }",
            ),
            internal(
                "horizon_client::Choice",
                "enum",
                "enum Choice { Both { a: Large, b: Large } }",
            ),
            internal(
                "horizon_client::inspect",
                "fn",
                "fn inspect(a: &Large, b: &Large) -> u64",
            ),
        ];
        let (code, _) = all_table(&entries);
        run_fixture(
            &code,
            "struct Large { data: [u64; 17] } enum Choice { Both { a: Large, b: Large } } fn inspect(a: &Large, b: &Large) -> u64 { a.data[0] + b.data[0] }",
            r#"
            let mut choice = Choice::Both { a: Large { data: [2; 17] }, b: Large { data: [3; 17] } };
            let ty = __internals::TABLE.types.iter().find(|t| t.path.ends_with("Choice")).unwrap();
            unsafe {
                let Reply::Node(a, _, false) = (ty.node)((&raw mut choice).cast(), false, Op::Field(0)).unwrap() else { panic!() };
                let Reply::Node(b, _, false) = (ty.node)((&raw mut choice).cast(), false, Op::Field(1)).unwrap() else { panic!() };
                let thunk = __internals::TABLE.thunks.iter().find(|t| t.path.ends_with("inspect")).unwrap();
                assert_eq!((thunk.call)(None, vec![internals::Arg::Node(a), internals::Arg::Node(b)]).map(|r| r.result), Ok(Reply::Value(Value::U(5))));
            }
        "#,
        );
    }

    #[test]
    fn generated_collection_nodes_compile_and_read_the_actual_element() {
        let (code, _) = all_table(&[internal(
            "horizon_client::Bag",
            "struct",
            "struct Bag { items: Vec<u8> }",
        )]);
        run_fixture(
            &code,
            "struct Bag { items: Vec<u8> }",
            r#"
            let mut bag = Bag { items: vec![7] };
            let walk = __internals::TABLE.types.iter().find(|ty| ty.path.ends_with("Bag")).unwrap().node;
            unsafe {
                let Reply::Node(p, ty, writable) = walk(&raw mut bag as *mut (), false, Op::Field(0)).unwrap() else { panic!("not a node") };
                assert!(!writable);
                assert_eq!((__internals::TABLE.types[ty as usize].node)(p, writable, Op::Index(0)), Ok(Reply::Value(Value::U(7))));
            }
        "#,
        );
    }

    #[test]
    fn generated_raw_arrays_and_vecs_keep_disjoint_siblings_and_validate_writes() {
        let entries = [
            internal(
                "horizon_client::Pair",
                "struct",
                "struct Pair { value: u32, padding: [u8; 17] }",
            ),
            internal(
                "horizon_client::Choice",
                "enum",
                "enum Choice { Values(Vec<Pair>) }",
            ),
            internal(
                "horizon_client::Bag",
                "struct",
                "struct Bag { array: [Pair; 17], items: Vec<Pair>, choice: Choice, boxed: Box<[Pair; 17]>, shared: std::sync::Arc<Vec<Pair>> }",
            ),
            internal(
                "horizon_client::heal",
                "fn",
                "fn heal(a: &mut Pair, b: &mut Pair)",
            ),
        ];
        let (code, _) = all_table(&entries);
        run_fixture(
            &code,
            // The node-borrow test needs a handle-kind Pair, not a mutable copy local.
            "#[derive(Clone, Copy)] struct Pair { value: u32, padding: [u8; 17] } enum Choice { Values(Vec<Pair>) } struct Bag { array: [Pair; 17], items: Vec<Pair>, choice: Choice, boxed: Box<[Pair; 17]>, shared: std::sync::Arc<Vec<Pair>> } fn heal(a: &mut Pair, b: &mut Pair) { a.value = 20; b.value = 21; }",
            r#"
            let mut bag = Bag { array: [Pair { value: 1, padding: [0; 17] }; 17], items: vec![Pair { value: 2, padding: [0; 17] }, Pair { value: 3, padding: [0; 17] }], choice: Choice::Values(vec![Pair { value: 4, padding: [0; 17] }]), boxed: Box::new([Pair { value: 5, padding: [0; 17] }; 17]), shared: std::sync::Arc::new(vec![Pair { value: 6, padding: [0; 17] }]) };
            let table = &__internals::TABLE;
            let bag_ty = table.types.iter().find(|t| t.path.ends_with("Bag")).unwrap();
            assert_eq!(bag_ty.raw_fields, &[0, 1, 2, 3]);
            unsafe {
                for field in [0, 1, 3] {
                    let Reply::Node(p, ty, _) = (bag_ty.node)((&raw mut bag).cast(), true, Op::Field(field)).unwrap() else { panic!("collection") };
                    let collection = &table.types[ty as usize];
                    assert!(collection.raw_index);
                    let Reply::Node(left, ty, _) = (collection.node)(p, true, Op::Index(0)).unwrap() else { panic!("left") };
                    let Reply::Node(right, _, _) = (collection.node)(p, true, Op::Index(1)).unwrap() else { panic!("right") };
                    let pair = &table.types[ty as usize];
                    assert_eq!(pair.raw_gets, &[0]);
                    assert!((pair.node)(left, true, Op::Check(0, &Value::B(true))).is_err());
                    assert_eq!((pair.node)(left, true, Op::Check(0, &Value::U(7))), Ok(Reply::Done));
                    assert_eq!((pair.node)(left, true, Op::Set(0, Value::U(7))), Ok(Reply::Done));
                    assert_eq!((pair.node)(right, true, Op::Set(0, Value::U(8))), Ok(Reply::Done));
                    assert_eq!((pair.node)(left, true, Op::Get(0)), Ok(Reply::Value(Value::U(7))));
                    assert_eq!((pair.node)(right, true, Op::Get(0)), Ok(Reply::Value(Value::U(8))));
                    (table.thunks[0].call)(None, vec![internals::Arg::Node(left), internals::Arg::Node(right)]).unwrap();
                    assert_eq!((pair.node)(left, true, Op::Get(0)), Ok(Reply::Value(Value::U(20))));
                }
                let Reply::Node(p, ty, _) = (bag_ty.node)((&raw mut bag).cast(), true, Op::Field(2)).unwrap() else { panic!("enum") };
                assert!(table.types[ty as usize].raw_fields.is_empty());
                let Reply::Node(_, _, writable) = (bag_ty.node)((&raw mut bag).cast(), true, Op::Field(4)).unwrap() else { panic!("arc") };
                assert!(!writable);
                (table.types[ty as usize].node)(p, true, Op::Field(0)).unwrap();
            }
            assert_eq!((bag.array[0].value, bag.items[1].value), (20, 21));
            "#,
        );
    }

    #[test]
    fn optional_reference_returns_keep_the_referent_not_the_temporary_option() {
        let entries = [
            internal(
                "horizon_client::Bag",
                "struct",
                "struct Bag { values: Vec<u8> }",
            ),
            internal(
                "horizon_client::Bag::find",
                "method",
                "fn find(&mut self, present: bool) -> Option<&mut Bag>",
            ),
        ];
        let (code, counts) = all_table(&entries);
        assert_eq!(counts.thunks, 1);
        run_fixture(
            &code,
            "struct Bag { values: Vec<u8> } impl Bag { fn find(&mut self, present: bool) -> Option<&mut Self> { present.then_some(self) } }",
            r#"
            let _ = Op::Read;
            let mut bag = Bag { values: vec![7] };
            unsafe {
                let thunk = &__internals::TABLE.thunks[0];
                let ptr = &raw mut bag as *mut ();
                assert!(matches!((thunk.call)(Some(ptr), vec![internals::Arg::Value(Value::B(true))]).unwrap().result, Reply::Node(p, _, true) if p == ptr));
                assert_eq!((thunk.call)(Some(ptr), vec![internals::Arg::Value(Value::B(false))]).map(|r| r.result), Ok(Reply::Value(Value::Var(0, vec![]))));
            }
            "#,
        );
    }

    #[test]
    fn aggregate_map_keys_encode_borrowed_expressions() {
        let declaration = "struct Bag {
            tuple: std::collections::BTreeMap<(i32, u64, bool), u32>,
            array: std::collections::BTreeMap<[i32; 2], u32>,
            nested: std::collections::BTreeMap<Option<(u128, [i32; 1])>, u32>,
        }";
        let entry = internal("horizon_client::Bag", "struct", declaration);
        let (code, counts) = all_table(&[entry]);
        assert!(counts.refused_map_keys.is_empty());
        run_fixture(
            &code,
            declaration,
            r#"
            let mut bag = Bag {
                tuple: Default::default(), array: Default::default(), nested: Default::default(),
            };
            let keys = [
                Value::Rec(vec![Value::S(-7), Value::U(11), Value::B(true)]),
                Value::Seq(vec![Value::S(13), Value::S(-17)]),
                Value::Var(1, vec![Value::Rec(vec![
                    Value::Rec(vec![Value::U(1), Value::U(0)]),
                    Value::Seq(vec![Value::S(-19)]),
                ])]),
            ];
            let owner = __internals::TABLE.types.iter().find(|t| t.path == "horizon_client::Bag").unwrap();
            for (field, key) in keys.into_iter().enumerate() {
                unsafe {
                    let Reply::Node(ptr, ty, writable) = (owner.node)((&raw mut bag).cast(), true, Op::Field(field as u32)).unwrap() else { panic!() };
                    let node = __internals::TABLE.types[ty as usize].node;
                    assert_eq!(node(ptr, writable, Op::CanonicalKey(&key)), Ok(Reply::Value(key)));
                }
            }
            "#,
        );
    }

    #[test]
    fn result_returns_discard_opaque_errors_but_not_success_values() {
        let entries = [
            internal(
                "horizon_client::succeed",
                "fn",
                "fn succeed() -> Result<u32>",
            ),
            internal(
                "horizon_client::fail",
                "fn",
                "fn fail() -> std::result::Result<(), Box<dyn std::error::Error>>",
            ),
        ];
        let (code, counts) = all_table(&entries);
        assert_eq!(counts.thunks, 2);
        run_fixture(
            &code,
            "type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>; fn succeed() -> Result<u32> { Ok(37) } fn fail() -> Result<()> { Err(std::io::Error::other(\"opaque\").into()) }",
            r#"
            let _ = Op::Read;
            unsafe {
                for thunk in __internals::TABLE.thunks {
                    let expected = if thunk.path.ends_with("succeed") { Value::Var(0, vec![Value::U(37)]) } else { Value::Var(1, vec![]) };
                    assert_eq!((thunk.call)(None, vec![]).map(|r| r.result), Ok(Reply::Value(expected)));
                }
            }
            "#,
        );
    }

    #[test]
    fn native_only_targets_are_refused_even_when_the_signature_can_cross() {
        let path = "horizon_core::random::hash::seed_from_hash";
        let entry = internal(path, "fn", "fn seed_from_hash(name: &str) -> u64");
        let (code, counts) = table(&[entry], &[path.into()]);
        assert_eq!(counts.refused.get(path), Some(&"native only"));
        assert_eq!(counts.thunks, 0);
        assert_eq!(counts.types, 0);
        let module: syn::File = syn::parse_str(&code).unwrap();
        let table = module
            .items
            .iter()
            .find_map(|item| match item {
                syn::Item::Static(item) if item.ident == "TABLE" => Some(item),
                _ => None,
            })
            .unwrap();
        let syn::Expr::Struct(table) = &*table.expr else {
            panic!("a table")
        };
        let native_only = table
            .fields
            .iter()
            .find(|field| {
                field.member == syn::Member::Named(syn::parse_str("native_only").unwrap())
            })
            .unwrap();
        let syn::Expr::Reference(paths) = &native_only.expr else {
            panic!("static paths")
        };
        let syn::Expr::Array(paths) = &*paths.expr else {
            panic!("paths")
        };
        assert!(paths.elems.iter().any(|value| matches!(value, syn::Expr::Lit(syn::ExprLit {lit: syn::Lit::Str(value), ..}) if value.value() == path)));
    }

    #[test]
    fn undeclared_types_and_methods_are_absent() {
        let entries = [
            internal(
                "horizon_client::Hidden",
                "struct",
                "struct Hidden { n: u8 }",
            ),
            internal("horizon_client::skip", "fn", "fn skip()"),
        ];
        let (_, counts) = table(&entries, &[]);
        assert_eq!(counts.types, 0);
        assert_eq!(counts.thunks, 0);
    }

    #[test]
    fn declared_fields_are_partial_and_keep_global_ids() {
        let mut entries = [
            internal(
                "horizon_client::Owner",
                "struct",
                "struct Owner { first: u8, second: u8 }",
            ),
            internal("horizon_client::Owner::first", "field", "u8"),
            internal("horizon_client::Owner::second", "field", "u8"),
        ];
        entries[2].hash = "second-field".into();
        for first in [false, true] {
            let mut declared = vec!["horizon_client::Owner::second".into()];
            if first {
                declared.push("horizon_client::Owner::first".into());
            }
            let (code, counts) = table(&entries, &declared);
            assert_eq!(counts.types, 1);
            run_fixture(
                &code,
                "struct Owner { first: u8, second: u8 }",
                &format!(
                    r#"
                let mut owner = Owner {{ first: 3, second: 7 }};
                let member = __internals::TABLE.members.iter().find(|m| m.path == "horizon_client::Owner::second").unwrap();
                assert_eq!(member.field, 1);
                assert_eq!(member.hash, "second-field");
                let walk = __internals::TABLE.types[member.owner as usize].node;
                assert_eq!(owner.first, 3);
                let ptr = &raw mut owner as *mut ();
                unsafe {{
                    assert_eq!(walk(ptr, true, Op::Get(1)), Ok(Reply::Value(Value::U(7))));
                    assert_eq!(walk(ptr, true, Op::Get(0)).is_ok(), {first});
                    assert!(walk(ptr, true, Op::Read).is_err());
                    assert_eq!(walk(ptr, true, Op::Set(1, Value::U(9))), Ok(Reply::Done));
                    assert_eq!(owner.second, 9);
                }}
            "#
                ),
            );
        }
    }

    #[test]
    fn a_declared_field_reaches_its_nested_elements_but_not_its_siblings() {
        let entries = [
            internal(
                "horizon_client::Owner",
                "struct",
                "struct Owner { chosen: Nested, sibling: Other }",
            ),
            internal("horizon_client::Owner::chosen", "field", "Nested"),
            internal(
                "horizon_client::Nested",
                "struct",
                "struct Nested { children: Vec<Child> }",
            ),
            internal("horizon_client::Child", "struct", "struct Child { n: u8 }"),
            internal("horizon_client::Other", "struct", "struct Other { n: u8 }"),
        ];
        let (code, counts) = table(&entries, &["horizon_client::Owner::chosen".into()]);
        assert_eq!(counts.types, 4);
        assert!(!code.contains("Other"));
        run_fixture(
            &code,
            "struct Owner { chosen: Nested, sibling: Other } struct Nested { children: Vec<Child> } struct Child { n: u8 } struct Other { n: u8 }",
            r#"
            let mut owner = Owner { chosen: Nested { children: vec![Child { n: 7 }] }, sibling: Other { n: 3 } };
            assert_eq!(owner.sibling.n, 3);
            let root = __internals::TABLE.members[0].owner as usize;
            unsafe {
                let node = |p, ty: usize, op| (__internals::TABLE.types[ty].node)(p, true, op);
                let Reply::Node(p, ty, _) = node(&raw mut owner as *mut (), root, Op::Field(0)).unwrap() else { panic!("nested") };
                let Reply::Node(p, ty, _) = node(p, ty as usize, Op::Field(0)).unwrap() else { panic!("list") };
                let Reply::Node(p, ty, _) = node(p, ty as usize, Op::Index(0)).unwrap() else { panic!("child") };
                assert_eq!(node(p, ty as usize, Op::Get(0)), Ok(Reply::Value(Value::U(7))));
                assert!(node(&raw mut owner as *mut (), root, Op::Field(1)).is_err());
            }
        "#,
        );
    }

    #[test]
    fn declared_methods_reach_arguments_results_and_fields_transitively_but_not_receiver_siblings()
    {
        let entries = [
            internal(
                "horizon_client::Game",
                "struct",
                "struct Game { hidden: Hidden }",
            ),
            internal(
                "horizon_client::Hidden",
                "struct",
                "struct Hidden { n: u8 }",
            ),
            internal(
                "horizon_client::Arg",
                "struct",
                "struct Arg { children: Vec<Child> }",
            ),
            internal("horizon_client::Child", "struct", "struct Child { n: u8 }"),
            internal("horizon_client::Answer", "enum", "enum Answer { Yes, No }"),
            internal(
                "horizon_client::Game::inspect",
                "method",
                "fn inspect(&self, arg: &Arg) -> Answer",
            ),
            internal("horizon_client::Game::skip", "method", "fn skip(&self)"),
        ];
        let (code, counts) = table(&entries, &["horizon_client::Game::inspect".into()]);
        assert_eq!(counts.types, 5); // Game, Arg, Child, Vec<Child>, Answer.
        assert_eq!(counts.thunks, 1);
        assert!(!code.contains("Hidden"));
        assert!(!code.contains("Game :: skip"));
        assert!(code.contains("horizon_client::Arg"));
        assert!(code.contains("horizon_client::Answer"));
    }

    #[test]
    fn wide_integers_round_trip_as_high_and_low_leaves() {
        let entries = [internal(
            "horizon_client::Wide",
            "struct",
            "struct Wide { unsigned: u128, signed: i128 }",
        )];
        let (code, _) = all_table(&entries);
        run_fixture(
            &code,
            "struct Wide { unsigned: u128, signed: i128 }",
            r#"
            let mut value = Wide { unsigned: u128::MAX, signed: i128::MIN };
            let walk = __internals::TABLE.types[0].node;
            let ptr = &raw mut value as *mut ();
            let unsigned = Value::Rec(vec![Value::U(u64::MAX), Value::U(u64::MAX)]);
            let signed = Value::Rec(vec![Value::U(1 << 63), Value::U(0)]);
            unsafe {
                assert_eq!(walk(ptr, true, Op::Get(0)), Ok(Reply::Value(unsigned.clone())));
                assert_eq!(walk(ptr, true, Op::Get(1)), Ok(Reply::Value(signed.clone())));
                value.unsigned = 0;
                value.signed = 0;
                assert_eq!(walk(ptr, true, Op::Set(0, unsigned)), Ok(Reply::Done));
                assert_eq!(walk(ptr, true, Op::Set(1, signed)), Ok(Reply::Done));
            }
            assert_eq!(value.unsigned, u128::MAX);
            assert_eq!(value.signed, i128::MIN);
        "#,
        );
    }

    #[test]
    fn foreign_keys_read_and_flags_round_trip_through_the_generated_table() {
        let shape = |path: &str, kind: &'static str, signature: &str| {
            let mut entry = internal(path, kind, signature);
            entry.shape_only = true;
            entry
        };
        let entries = [
            internal(
                "horizon_client::Held",
                "struct",
                "struct Held { key : winit :: keyboard :: PhysicalKey , mods : winit :: keyboard :: ModifiersState , }",
            ),
            shape(
                "winit::keyboard::KeyCode",
                "enum",
                "# [non_exhaustive] enum KeyCode { KeyA , KeyB , }",
            ),
            shape(
                "winit::keyboard::NativeKeyCode",
                "enum",
                "enum NativeKeyCode { Unidentified , Xkb (u32) , }",
            ),
            shape(
                "winit::keyboard::PhysicalKey",
                "enum",
                "enum PhysicalKey { Code (KeyCode) , Unidentified (NativeKeyCode) , }",
            ),
            shape(
                "winit::keyboard::ModifiersState",
                "struct",
                "struct ModifiersState (u32) ;",
            ),
        ];
        let (code, _) = all_table(&entries);
        // The flags' field is private, as `bitflags!` makes it: only `bits` reaches it.
        run_fixture(
            &code,
            r#"
            extern crate self as winit;
            #[allow(dead_code)]
            pub mod keyboard {
                #[non_exhaustive] pub enum KeyCode { KeyA, KeyB }
                pub enum NativeKeyCode { Unidentified, Xkb(u32) }
                pub enum PhysicalKey { Code(KeyCode), Unidentified(NativeKeyCode) }
                struct Bits(u32);
                pub struct ModifiersState(Bits);
                impl ModifiersState {
                    pub fn bits(&self) -> u32 { self.0.0 }
                    pub fn from_bits_retain(bits: u32) -> Self { Self(Bits(bits)) }
                }
            }
            struct Held { key: winit::keyboard::PhysicalKey, mods: winit::keyboard::ModifiersState }
            "#,
            r#"
            use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
            let mut held = Held {
                key: PhysicalKey::Code(KeyCode::KeyB),
                mods: ModifiersState::from_bits_retain(0b100_100),
            };
            let walk = __internals::TABLE
                .types
                .iter()
                .find(|t| t.path == "horizon_client::Held")
                .unwrap()
                .node;
            let ptr = &raw mut held as *mut ();
            let key_b = Value::Var(0, vec![Value::Var(1, vec![])]);
            let control_shift = Value::Rec(vec![Value::U(0b100_100)]);
            let xkb = Value::Var(1, vec![Value::Var(1, vec![Value::U(38)])]);
            unsafe {
                assert_eq!(walk(ptr, false, Op::Get(0)), Ok(Reply::Value(key_b)));
                assert_eq!(walk(ptr, false, Op::Get(1)), Ok(Reply::Value(control_shift)));
                // A key holds a sealed `KeyCode`: read-only.
                assert!(walk(ptr, true, Op::Set(0, xkb)).is_err());
                assert_eq!(
                    walk(ptr, true, Op::Set(1, Value::Rec(vec![Value::U(0b100)]))),
                    Ok(Reply::Done)
                );
            }
            assert!(matches!(held.key, PhysicalKey::Code(KeyCode::KeyB)));
            assert_eq!(held.mods.bits(), 0b100);
        "#,
        );
    }

    #[test]
    fn winit_keys_are_read_only_and_the_other_listed_winit_types_are_built() {
        let (code, _) = all_table(&crate::crossing::foreign_internals());
        for (path, built) in [
            ("winit__keyboard__KeyCode", false),
            ("winit__keyboard__PhysicalKey", false),
            ("winit__keyboard__NativeKeyCode", true),
            ("winit__keyboard__KeyLocation", true),
            ("winit__keyboard__ModifiersState", true),
            ("winit__event__ElementState", true),
            ("winit__event__MouseButton", true),
        ] {
            assert!(code.contains(&format!("fn enc__{path}")), "{path}");
            assert_eq!(code.contains(&format!("fn dec__{path}")), built, "{path}");
        }
    }

    #[test]
    fn a_shared_field_and_method_path_opts_in_both_kinds() {
        let entries = [
            internal(
                "horizon_client::Owner",
                "struct",
                "struct Owner { same: u8 }",
            ),
            internal("horizon_client::Owner::same", "field", "u8"),
            internal(
                "horizon_client::Owner::same",
                "method",
                "fn same(&self) -> u8",
            ),
        ];
        let (code, counts) = table(&entries, &["horizon_client::Owner::same".into()]);
        assert_eq!(counts.types, 1);
        assert_eq!(counts.thunks, 1);
        assert_eq!(counts.skipped, 0);
        run_fixture(
            &code,
            "struct Owner { same: u8 } impl Owner { fn same(&self) -> u8 { self.same } }",
            r#"
            assert_eq!(__internals::TABLE.members.len(), 1);
            let mut owner = Owner { same: 7 };
            let member = &__internals::TABLE.members[0];
            let walk = __internals::TABLE.types[member.owner as usize].node;
            unsafe { assert_eq!(walk(&raw mut owner as *mut (), false, Op::Get(member.field)), Ok(Reply::Value(Value::U(7)))); }
        "#,
        );
    }

    #[test]
    fn refused_declarations_do_not_leave_nodes_or_codecs() {
        let entries = [
            internal(
                "horizon_client::Arg",
                "struct",
                "struct Arg { items: Vec<u8> }",
            ),
            internal("horizon_client::bad", "fn", "fn bad(arg: &Arg) -> Unknown"),
            internal(
                "horizon_client::Opaque",
                "struct",
                "struct Opaque<T> { x: T }",
            ),
        ];
        let (_, counts) = table(
            &entries,
            &[
                "missing".into(),
                "horizon_client::bad".into(),
                "horizon_client::Opaque".into(),
                "missing".into(),
            ],
        );
        assert_eq!(counts.declared, 3);
        assert_eq!(counts.skipped, 3);
        assert_eq!(counts.types, 0);
        assert_eq!(counts.refused["missing"], "not in internals.json");
        assert_eq!(counts.refused["horizon_client::Opaque"], "generic type");
    }

    #[test]
    fn method_signatures_accept_bare_and_impl_forms_but_not_bounded_impls() {
        for written in [
            "fn count(&self) -> usize",
            "impl Game<'_> { fn count(&self) -> usize }",
        ] {
            let (_, counts) = all_table(&[
                internal(
                    "horizon_client::Game",
                    "struct",
                    "struct Game<'a> { label: &'a str }",
                ),
                internal("horizon_client::Game<'_>::count", "method", written),
            ]);
            assert_eq!(counts.thunks, 1, "{written}");
        }
        assert!(signature("impl<T> Game<T> { fn count(&self) -> usize }").is_none());
        assert!(signature("impl<'a: 'static> Game<'a> { fn count(&self) -> usize }").is_none());
        assert!(signature("impl Game where Self: Sized { fn count(&self) -> usize }").is_none());
    }

    #[test]
    fn a_call_cannot_keep_a_decoded_borrow_beyond_its_frame() {
        let (_, counts) = all_table(&[
            internal(
                "horizon_client::take_static",
                "fn",
                "fn take_static(s: &'static str)",
            ),
            internal(
                "horizon_client::take_bounded",
                "fn",
                "fn take_bounded<'a: 'static>(s: &'a str)",
            ),
            internal("horizon_client::take", "fn", "fn take(s: &str)"),
        ]);
        assert_eq!(counts.thunks, 1);
        assert_eq!(
            counts.skipped_functions["static or bounded input lifetime"],
            2
        );
    }

    #[test]
    fn an_argument_cannot_supply_a_borrow_stored_in_the_receiver() {
        let (_, counts) = all_table(&[
            internal(
                "horizon_client::World",
                "struct",
                "struct World { rows: Vec<u8> }",
            ),
            internal(
                "horizon_client::Holder",
                "struct",
                "struct Holder<'a> { world: &'a World }",
            ),
            internal(
                "horizon_client::Foo",
                "struct",
                "struct Foo<'a> { world: &'a World }",
            ),
            internal(
                "horizon_client::Holder<'_>::replace",
                "method",
                "impl<'a> Holder<'a> { fn replace(&mut self, world: &'a World) }",
            ),
            internal("horizon_client::take", "fn", "fn take(foo: &mut Foo<'_>)"),
            internal("horizon_client::inspect", "fn", "fn inspect(world: &World)"),
        ]);
        assert_eq!(counts.thunks, 1);
        assert_eq!(
            counts.skipped_functions["named input lifetime or lifetime-bearing input type"],
            2
        );
    }

    #[test]
    fn conditional_layouts_and_api_owned_layouts_stay_opaque() {
        let (code, counts) = all_table(&[
            internal(
                "horizon_client::Conditional",
                "struct",
                "struct Conditional { #[cfg(feature = \"audio\")] sound: u8 }",
            ),
            internal(
                "horizon_client::Cases",
                "enum",
                "enum Cases { #[cfg(test)] Test, Normal }",
            ),
            internal(
                "horizon_client_core::External",
                "struct",
                "struct External { private: u8 }",
            ),
            internal(
                "mod_api::External",
                "struct",
                "struct External { private: u8 }",
            ),
            internal(
                "mod_points::External",
                "struct",
                "struct External { private: u8 }",
            ),
            internal(
                "horizon_client::Generic",
                "struct",
                "struct Generic<T> { value: T }",
            ),
        ]);
        assert_eq!(counts.types, 1, "{code}");
        assert_eq!(counts.skipped_types["conditional field or variant"], 2);
        assert_eq!(counts.skipped_types["lower-crate layout not widened"], 2);
        assert_eq!(counts.skipped_types["generic type"], 1);
    }

    #[test]
    fn reachable_lower_shapes_are_walked_and_sealed_ones_never_built() {
        let shape = |path: &str, kind: &'static str, signature: &str| {
            let mut entry = internal(path, kind, signature);
            entry.shape_only = true;
            entry
        };
        let (code, counts) = all_table(&[
            shape(
                "horizon_items::Stack",
                "struct",
                "struct Stack { count : u8 , }",
            ),
            shape(
                "horizon_items::Sealed",
                "struct",
                "# [non_exhaustive] struct Sealed { count : u8 , }",
            ),
            shape(
                "horizon_items::Kind",
                "enum",
                "enum Kind { # [non_exhaustive] One { n : u8 } , Two , }",
            ),
            shape("mod_api::Api", "struct", "struct Api { private : u8 , }"),
        ]);
        syn::parse_file(&code).unwrap();
        assert_eq!(counts.types, 3, "{code}");
        assert_eq!(counts.skipped_types["lower-crate layout not widened"], 1);
        assert!(code.contains("fn dec__horizon_items__Stack"));
        assert!(!code.contains("fn dec__horizon_items__Sealed"));
        assert!(!code.contains("fn dec__horizon_items__Kind"));
        // Read from another crate: patterns that tolerate fields and cases they don't list.
        assert!(code.contains("count : f0 , .."), "{code}");
    }

    #[test]
    fn import_facts_recover_crossable_functions_without_global_name_guesses() {
        let mut take = internal("horizon_client::b::take", "fn", "fn take(data: Alias)");
        take.refs
            .insert("Alias".into(), "horizon_client::a::Data".into());
        let (_, counts) = all_table(&[
            internal("horizon_client::a::Data", "struct", "struct Data { x: u8 }"),
            take,
        ]);
        assert_eq!(counts.thunks, 1);
    }

    #[test]
    fn unresolved_imports_and_unexpected_generics_do_not_borrow_another_types_layout() {
        let (_, counts) = all_table(&[
            internal("horizon_client::a::Cell", "struct", "struct Cell { x: u8 }"),
            internal("horizon_client::a::take", "fn", "fn take(c: Cell<u64>)"),
            internal("horizon_client::b::take", "fn", "fn take(c: Cell)"),
        ]);
        assert_eq!(counts.types, 1);
        assert_eq!(counts.thunks, 0);
        assert_eq!(counts.skipped, 2);
    }

    #[test]
    fn unsized_and_custom_hasher_fields_are_not_walked() {
        let (code, counts) = all_table(&[internal(
            "horizon_client::Fields",
            "struct",
            "struct Fields { slice: &'static [u8], map: std::collections::HashMap<String, u8, UnknownHasher>, tuple: (u8, Unknown) }",
        )]);
        assert_eq!(counts.types, 1);
        assert!(!code.contains("I :: Op :: Field"));
    }

    #[test]
    fn the_table_covers_what_crosses_and_parses() {
        let internals = [
            internal(
                "horizon_client::g::Pos",
                "struct",
                "struct Pos { x : i32 , y : i32 , z : i32 , }",
            ),
            internal(
                "horizon_client::g::Mode",
                "enum",
                "enum Mode { Survival , Custom (u8) , Named { name : String } , }",
            ),
            internal(
                "horizon_client::g::Player",
                "struct",
                "struct Player { pos : Pos , mode : Mode , tag : & 'static str , }",
            ),
            internal(
                "horizon_client::g::World",
                "struct",
                "struct World { players : Vec < Player > , spawn : Option < Pos > , by_name : std::collections::HashMap < String , Player > , lock : RefCell < u8 > , }",
            ),
            internal(
                "horizon_client::g::Game",
                "struct",
                "struct Game < 'a > { world : Box < World > , shared : std::sync::Arc < World > , res : & 'a World , }",
            ),
            internal(
                "horizon_client::g::Game<'_>::spawn",
                "method",
                "fn spawn (& mut self , at : Pos , name : & str) -> Option < & mut Player >",
            ),
            internal(
                "horizon_client::g::Game<'_>::count",
                "method",
                "fn count (& self) -> usize",
            ),
            internal(
                "horizon_client::g::Game<'_>::world",
                "method",
                "fn world (& self) -> & World",
            ),
            internal(
                "horizon_client::g::Game<'_>::closure",
                "method",
                "fn closure (& self , f : impl Fn ()) -> usize",
            ),
            internal(
                "horizon_client::g::Game<'_>::generic",
                "method",
                "fn generic < T > (& self , t : T)",
            ),
            internal(
                "horizon_client::g::Game<'_>::owned",
                "method",
                "fn owned (& self) -> Vec < Player >",
            ),
            internal(
                "horizon_client::g::helper",
                "fn",
                "fn helper (world : & mut World , pos : [i32 ; 3]) -> bool",
            ),
        ];
        let (code, counts) = all_table(&internals);
        syn::parse_file(&code).unwrap_or_else(|e| panic!("{e}\n{code}"));
        // Pos, Mode, Player, World, Game, and World's list and map; `Option<Pos>` is a copy.
        assert_eq!(counts.types, 7, "{code}");
        assert_eq!(counts.thunks, 4);
        assert_eq!(counts.skipped, 3);
        assert_eq!(
            counts.refused["horizon_client::g::Game<'_>::closure"],
            "closure argument; declare a non-closure helper"
        );
        // A borrowed field is read, never written; a shared one is walked read-only.
        assert!(code.contains("fn enc__horizon_client__g__Player"));
        assert!(!code.contains("fn dec__horizon_client__g__Player"));
        assert!(code.contains("fn dec__horizon_client__g__Pos"));
    }
}

#[cfg(test)]
mod catalog_coverage;
