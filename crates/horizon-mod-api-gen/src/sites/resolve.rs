use super::*;
use crate::{
    crossing::{Rule, Shape},
    internals::Internal,
};
use quote::{ToTokens, quote};
use syn::{FnArg, Type};

fn named(shape: Shape) -> Result<String, Error> {
    match shape {
        Shape::Named(path, _) => Ok(path),
        Shape::Ref { to, .. } => named(*to),
        _ => Err(Cause::Missing("unresolved receiver type").into()),
    }
}

/// Resolve a receiver through the internals catalog's import and field type information.
fn receiver(
    expr: &Expr,
    module: &str,
    this: Option<&str>,
    rule: &Rule,
    sig: &syn::Signature,
) -> Result<String, Error> {
    match expr {
        Expr::Path(path) if path.path.is_ident("self") => this
            .map(str::to_owned)
            .ok_or_else(|| Cause::Missing("self outside a method").into()),
        Expr::Path(path) => {
            let ty = sig
                .inputs
                .iter()
                .find_map(|arg| match arg {
                    FnArg::Typed(arg)
                        if arg.pat.to_token_stream().to_string()
                            == path.to_token_stream().to_string() =>
                    {
                        Some(&*arg.ty)
                    }
                    _ => None,
                })
                .ok_or(Cause::Missing(
                    "unresolved receiver: local has no parameter type annotation",
                ))?;
            named(rule.shape(module, this, ty))
        }
        Expr::Field(field) => {
            let owner = receiver(&field.base, module, this, rule, sig)?;
            let field_ty = rule
                .fields(&owner)
                .into_iter()
                .find(|f| f.case.is_none() && f.member == field.member)
                .ok_or(Cause::Missing("unresolved receiver field"))?;
            let scope = &rule
                .named(&owner)
                .ok_or(Cause::Missing("unresolved receiver owner"))?
                .module;
            named(rule.shape(scope, Some(&owner), field_ty.ty))
        }
        _ => Err(Cause::Missing(
            "unresolved receiver: chained calls and computed receivers are not supported",
        )
        .into()),
    }
}

/// A resolved call, using the same catalog and type rule as the internals view.
pub(crate) struct Resolved<'a> {
    pub callee: &'a Internal,
    pub signature: syn::Signature,
    pub receiver: Option<&'a Expr>,
    pub receiver_type: proc_macro2::TokenStream,
    pub arguments: Vec<&'a Expr>,
    pub fields: Vec<syn::Ident>,
    pub types: Vec<Type>,
    pub output: proc_macro2::TokenStream,
}

/// Identity facts, independent of whether the call can be lowered into a repeatable record.
pub(crate) struct Callee<'a> {
    pub callee: &'a Internal,
    pub signature: Option<syn::Signature>,
    pub receiver: Option<&'a Expr>,
    pub arguments: Vec<&'a Expr>,
}

/// Resolve all runtime calls for per-callee ordinals, including non-record-safe calls.
pub(crate) fn callee<'a>(
    expr: &'a Expr,
    enclosing: &Internal,
    enclosing_sig: &syn::Signature,
    lexical: &super::analysis::Scope,
    entries: &'a [Internal],
    rule: &Rule,
) -> Result<Callee<'a>, Error> {
    lexical.check_target(expr)?;
    let (scope, this) = crate::internals_table::function_scope(rule, enclosing)
        .ok_or(Cause::Missing("unresolved enclosing receiver"))?;
    let (callee, recv, arguments) = match expr {
        Expr::MethodCall(call) => {
            let owner = receiver(&call.receiver, &scope, this.as_deref(), rule, enclosing_sig)?;
            let mut candidates = entries
                .iter()
                .filter(|e| {
                    e.kind == "method"
                        && e.cfg.is_empty()
                        && e.path.ends_with(&format!("::{}", call.method))
                })
                .filter(|e| {
                    crate::internals_table::function_scope(rule, e)
                        .is_some_and(|(_, ty)| ty.as_deref() == Some(&owner))
                });
            let callee = candidates
                .next()
                .ok_or(Cause::Missing("callee not in the internals catalog"))?;
            if candidates.next().is_some() {
                return Err(Cause::Ambiguous("ambiguous callee").into());
            }
            (
                callee,
                Some(&*call.receiver),
                call.args.iter().collect::<Vec<_>>(),
            )
        }
        Expr::Call(call) => {
            let Expr::Path(path) = &*call.func else {
                return Err(Cause::Missing("indirect call").into());
            };
            if path.qself.is_some() {
                return Err(Cause::Missing("qualified-self call is not supported").into());
            }
            let mut written = path.path.clone();
            if written.segments.first().is_some_and(|s| s.ident == "Self") {
                let owner = this
                    .as_ref()
                    .ok_or(Cause::Missing("Self outside a method"))?;
                let tail = written
                    .segments
                    .iter()
                    .skip(1)
                    .map(|s| s.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                written = syn::parse_str(&format!("{owner}::{tail}"))?;
            }
            let path = rule
                .resolve(&scope, &written)
                .ok_or(Cause::Missing("unresolved callee path"))?;
            let mut candidates = entries.iter().filter(|e| {
                if !e.cfg.is_empty() {
                    return false;
                }
                let canonical = match e.kind {
                    "fn" => plain_path(&e.path),
                    "method" => match crate::internals_table::function_scope(rule, e) {
                        Some((_, Some(owner))) => format!(
                            "{owner}::{}",
                            e.path.rsplit("::").next().expect("method name")
                        ),
                        _ => return false,
                    },
                    _ => return false,
                };
                canonical == path
            });
            let callee = candidates
                .next()
                .ok_or(Cause::Missing("callee not in the internals catalog"))?;
            if candidates.next().is_some() {
                return Err(Cause::Ambiguous("ambiguous callee").into());
            }
            (callee, None, call.args.iter().collect())
        }
        _ => unreachable!(),
    };
    let signature = crate::internals_table::signature(&callee.signature);
    let (mut recv, mut arguments) = (recv, arguments);
    if recv.is_none()
        && signature
            .as_ref()
            .is_some_and(|sig| sig.receiver().is_some())
    {
        recv = arguments.first().copied();
        if recv.is_some() {
            arguments.remove(0);
        }
    }
    Ok(Callee {
        callee,
        signature,
        receiver: recv,
        arguments,
    })
}

/// Resolve a selected call and refuse shapes that cannot have a repeatable raw continuation.
pub(crate) fn resolve<'a>(
    expr: &'a Expr,
    enclosing: &Internal,
    enclosing_sig: &syn::Signature,
    lexical: &super::analysis::Scope,
    entries: &'a [Internal],
    rule: &Rule,
) -> Result<Resolved<'a>, Error> {
    let Callee {
        callee,
        signature,
        receiver: recv,
        arguments,
    } = callee(expr, enclosing, enclosing_sig, lexical, entries, rule)?;
    let generic = match expr {
        Expr::MethodCall(call) => call.turbofish.is_some(),
        Expr::Call(call) => {
            matches!(&*call.func, Expr::Path(path) if path.path.segments.iter().any(|s| !s.arguments.is_empty()))
        }
        _ => false,
    };
    if generic {
        return Err(Cause::Signature("generic callee").into());
    }

    let sig = signature.ok_or(Cause::Signature("generic or bounded callee"))?;
    if sig.generics.params.len() != sig.generics.lifetimes().count()
        || sig.unsafety.is_some()
        || sig.asyncness.is_some()
        || sig.variadic.is_some()
    {
        return Err(Cause::Signature("generic, unsafe, async or variadic callee").into());
    }
    let (callee_module, callee_this) = crate::internals_table::function_scope(rule, callee)
        .ok_or(Cause::Missing("unresolved callee scope"))?;
    let mut types = Vec::new();
    let mut fields = Vec::new();
    for arg in &sig.inputs {
        if let FnArg::Typed(arg) = arg {
            let syn::Pat::Ident(pat) = &*arg.pat else {
                return Err(Cause::Record("destructured callee parameter").into());
            };
            let shape = rule.shape(&callee_module, callee_this.as_deref(), &arg.ty);
            // Moving an owned field out of &mut Args cannot implement a repeatable
            // original. Borrowing fields need a separate alias analysis, not guesses.
            if !matches!(
                shape,
                Shape::Scalar(
                    crate::crossing::Scalar::Bool
                        | crate::crossing::Scalar::Char
                        | crate::crossing::Scalar::Signed(_)
                        | crate::crossing::Scalar::Unsigned(_)
                        | crate::crossing::Scalar::Float(_)
                        | crate::crossing::Scalar::Wide(_)
                ) | Shape::Unit
            ) {
                return Err(Cause::Record(
                    "argument record requires a borrowing or non-Copy field",
                )
                .into());
            }
            types.push((*arg.ty).clone());
            fields.push(pat.ident.clone());
        }
    }
    if arguments.len() != fields.len() {
        return Err(Cause::Record("callee argument count differs").into());
    }
    super::analysis::arguments(
        &arguments,
        recv,
        sig.receiver()
            .is_some_and(|r| r.reference.is_some() && r.mutability.is_some()),
    )?;
    let output = match &sig.output {
        syn::ReturnType::Default => quote!(()),
        syn::ReturnType::Type(_, ty) => {
            let shape = rule.shape(&callee_module, callee_this.as_deref(), ty);
            if shape.may_borrow() {
                return Err(Cause::Signature("borrowed or unresolved call result").into());
            }
            crate::internals_table::spell(&shape).ok_or(Cause::Missing("unresolved call result"))?
        }
    };
    let recv_type = match (sig.receiver(), callee_this.as_ref()) {
        (Some(r), Some(owner)) if r.reference.is_some() => {
            let shape = rule.shape(&callee_module, Some(owner), &syn::parse_quote!(Self));
            let ty = crate::internals_table::spell(&shape)
                .ok_or(Cause::Missing("unresolved receiver spelling"))?
                .to_string()
                .replace("'static", "'g")
                .parse::<proc_macro2::TokenStream>()
                .map_err(|_| Cause::Signature("invalid native receiver type"))?;
            if r.mutability.is_some() {
                quote!(&mut #ty)
            } else {
                quote!(&#ty)
            }
        }
        (None, _) => quote!(()),
        _ => return Err(Cause::Signature("by-value receiver").into()),
    };

    Ok(Resolved {
        callee,
        signature: sig,
        receiver: recv,
        receiver_type: recv_type,
        arguments,
        fields,
        types,
        output,
    })
}

#[cfg(test)]
mod tests;
