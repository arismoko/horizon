use super::{Build, Sources};
use crate::{
    catalog::Catalog,
    gameplay::{Borrow, GameplayPackage, RouteKind, Routing},
    host::HostCallee,
};
use mod_source::index::{Binding, Found, Index, ItemId};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
};
use syn::{FnArg, ImplItem, Item, Path as RustPath, Signature, TraitItem, Type};

struct Implementation {
    library: &'static str,
    module: Vec<String>,
    cfg: Vec<String>,
    item: syn::ItemImpl,
}

pub(super) fn check(root: &Path, routing: &Routing, build: &Build) -> Result<(), String> {
    if !routing.hooks.is_empty() {
        let canonical = Catalog::load(root).map_err(|error| error.to_string())?;
        for (name, route) in &routing.hooks {
            let mut hook = canonical
                .hooks
                .iter()
                .find(|hook| &hook.name == name)
                .cloned()
                .ok_or_else(|| format!("{name}: unused hook route"))?;
            hook.apply_gameplay_default();
            if hook.target != route.target || hook.receiver_type != route.receiver_type {
                return Err(format!(
                    "{name}: hook route contradicts the vocabulary declaration"
                ));
            }
        }
    }
    let areas = crate::host::load(root)?;
    let mut sources = Sources::new(root);
    let mut plans = Vec::new();
    for package in GameplayPackage::ALL {
        let library = package.domain().key;
        check_common_alias(root, package)?;
        let callees = crate::host::host_callees(&areas, package, routing)
            .map_err(|error| format!("{library}: {error}"))?;
        plans.push((package, callees));
    }
    let names = plans
        .iter()
        .flat_map(|(_, calls)| calls.iter().map(|call| call.method.as_str()))
        .collect::<HashSet<_>>();
    let impls = implementations(&mut sources, &names)?;
    for (package, calls) in plans {
        let library = package.domain().key;
        for call in calls {
            let mut pending = vec![build.clone()];
            while let Some(build) = pending.pop() {
                if package == GameplayPackage::Client
                    && build.evaluate("feature = \"client\"") == Some(false)
                {
                    continue;
                }
                let mut index = sources.binding_index(&build, true);
                let result = resolve_callee(&mut index, package, &call, &impls, &build);
                if let Some(builds) = build.split() {
                    pending.extend(builds);
                } else {
                    result.map_err(|why| {
                        format!(
                            "{library}: {}: {}::{}: {why}",
                            call.label, call.owner, call.method
                        )
                    })?;
                }
            }
        }
    }
    Ok(())
}

fn check_common_alias(root: &Path, package: GameplayPackage) -> Result<(), String> {
    if package == GameplayPackage::Common {
        return Ok(());
    }
    let domain = package.domain();
    let common = GameplayPackage::Common.domain();
    let manifest = crate::replace::manifest(&domain.package_path(root))?;
    let alias = package.common().trim_start_matches("::");
    let dependency = |manifest: &toml::Table| {
        manifest
            .get("dependencies")
            .and_then(toml::Value::as_table)
            .and_then(|deps| {
                deps.iter()
                    .find(|(name, _)| name.replace('-', "_") == alias)
            })
            .map(|(name, value)| (name.clone(), value.clone()))
    };
    let found = dependency(&manifest);
    let common_manifest = crate::replace::manifest(&common.package_path(root))?;
    let actual_library = common_manifest
        .get("lib")
        .and_then(|lib| lib.get("name"))
        .and_then(toml::Value::as_str);
    let mut relative_to = domain.package_path(root);
    let found = if found.as_ref().is_some_and(|(_, dependency)| {
        dependency.get("workspace").and_then(toml::Value::as_bool) == Some(true)
    }) {
        relative_to = root.to_path_buf();
        crate::replace::manifest(root)?
            .get("workspace")
            .and_then(toml::Value::as_table)
            .and_then(dependency)
    } else {
        found
    };
    if let Some((name, value)) = found
        && value.get("optional").and_then(toml::Value::as_bool) != Some(true)
        && value
            .get("package")
            .and_then(toml::Value::as_str)
            .unwrap_or(&name)
            == common.package
        && (name != common.package || actual_library.is_none_or(|name| name == alias))
        && let Some(path) = value.get("path").and_then(toml::Value::as_str)
        && relative_to.join(path).canonicalize().ok()
            == common.package_path(root).canonicalize().ok()
    {
        return Ok(());
    }
    Err(format!(
        "{}: Cargo alias {alias} must name the local {} package",
        domain.package, common.package
    ))
}

fn implementations(
    sources: &mut Sources<'_>,
    names: &HashSet<&str>,
) -> Result<Vec<Implementation>, String> {
    let mut impls = Vec::new();
    for package in GameplayPackage::ALL {
        let library = package.domain().key;
        let mut parsed = HashMap::new();
        for variant in sources.variants(library)? {
            let file = &variant.file;
            let parsed = match parsed.entry(&file.path) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    let text = fs::read_to_string(&file.path)
                        .map_err(|error| format!("{}: {error}", file.path.display()))?;
                    entry.insert(
                        syn::parse_file(&text)
                            .map_err(|error| format!("{}: {error}", file.path.display()))?,
                    )
                }
            };
            let mut items = parsed.items.as_slice();
            for &index in &variant.inline {
                let Item::Mod(module) = &items[index] else {
                    unreachable!("a recorded inline module");
                };
                items = &module.content.as_ref().expect("inline module").1;
            }
            for item in items {
                if let Item::Impl(item) = item
                    && (item.trait_.is_some() || item.items.iter().any(|item| {
                        matches!(item, ImplItem::Fn(method) if names.contains(method.sig.ident.to_string().as_str()))
                    }))
                {
                    impls.push(Implementation {
                        library,
                        module: file.module.split("::").skip(1).map(str::to_owned).collect(),
                        cfg: file.cfg.clone(),
                        item: item.clone(),
                    });
                }
            }
        }
    }
    Ok(impls)
}

fn words(path: &RustPath) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect()
}

fn type_id(index: &mut Index<'_>, scope: usize, path: &RustPath) -> Result<Option<ItemId>, String> {
    let mut path = path.clone();
    let mut scope = scope;
    let mut seen = HashSet::new();
    loop {
        let Binding::Found(Found::Item(id)) =
            index.resolve_type_binding(scope, &words(&path), path.leading_colon.is_some())?
        else {
            return Ok(None);
        };
        if let Item::Type(alias) = index.item(id)
            && let Type::Path(ty) = &*alias.ty
            && ty.qself.is_none()
        {
            if !seen.insert(id) {
                return Err("type alias cycle".into());
            }
            path = ty.path.clone();
            scope = id.module;
        } else {
            return Ok(Some(id));
        }
    }
}

fn signature_matches(sig: &Signature, check: &HostCallee) -> bool {
    let borrow = match sig.receiver() {
        None => Borrow::None,
        Some(receiver) if receiver.reference.is_some() && receiver.mutability.is_some() => {
            Borrow::Mutable
        }
        Some(receiver) if receiver.reference.is_some() => Borrow::Shared,
        Some(_) => return false,
    };
    borrow == check.borrow
        && sig
            .inputs
            .iter()
            .filter(|arg| matches!(arg, FnArg::Typed(_)))
            .count()
            == check.arity
}

fn resolve_callee(
    index: &mut Index<'_>,
    package: GameplayPackage,
    call: &HostCallee,
    impls: &[Implementation],
    build: &Build,
) -> Result<(), String> {
    let library = package.domain().key;
    let scope = index.module(library, &["game".into(), "host".into()])?;
    let path: RustPath = syn::parse_str(&call.owner).map_err(|error| error.to_string())?;
    let local = path
        .segments
        .first()
        .is_some_and(|part| part.ident == "crate");
    let expected = if local {
        library
    } else {
        GameplayPackage::Common.domain().key
    };
    if call.kind == RouteKind::Static {
        let mut qualified = words(&path);
        qualified.push(call.method.clone());
        let resolved = if path.leading_colon.is_some() {
            index.resolve_rooted_path(scope, &qualified)
        } else {
            index.resolve_path(scope, &qualified)
        };
        if let Ok(Found::Item(id)) = resolved
            && let Item::Fn(function) = index.item(id)
        {
            if index.modules[id.module].krate != expected {
                return Err(format!("static callee is defined outside {expected}"));
            }
            return if signature_matches(&function.sig, call) {
                Ok(())
            } else {
                Err("static callee has a different signature".into())
            };
        }
    }
    let owner = type_id(index, scope, &path)?.ok_or("callee owner is unresolved")?;
    let owning_library = index.modules[owner.module].krate.clone();
    if owning_library != expected {
        return Err(format!(
            "owner resolves to {owning_library}, expected {expected}"
        ));
    }
    if call.kind == RouteKind::Common {
        let receiver = syn::parse_str(&format!("{}::game::CommonGame", package.common()))
            .map_err(|error| error.to_string())?;
        if type_id(index, scope, &receiver)? != Some(owner) {
            return Err("common route does not name the type of the common field".into());
        }
    }
    let trait_route = matches!(call.kind, RouteKind::SharedTrait | RouteKind::SideOperation);
    let mut default_method = false;
    let receiver = if trait_route {
        let Item::Trait(trait_) = index.item(owner) else {
            return Err("owner is not a trait".into());
        };
        let Some(method) = trait_.items.iter().find_map(|item| match item {
            TraitItem::Fn(method)
                if method.sig.ident == call.method
                    && signature_matches(&method.sig, call)
                    && build.enabled(&method.attrs) =>
            {
                Some(method)
            }
            _ => None,
        }) else {
            return Err("trait does not declare the method with the requested receiver".into());
        };
        default_method = method.default.is_some();
        let path = syn::parse_str(&format!("crate::game::{}", package.game_type()))
            .map_err(|error| error.to_string())?;
        type_id(index, scope, &path)?.ok_or("nominal game receiver is unresolved")?
    } else {
        owner
    };
    let mut matched = 0;
    for candidate in impls {
        let item = &candidate.item;
        if item.trait_.is_some() != trait_route {
            continue;
        }
        let Type::Path(ty) = &*item.self_ty else {
            continue;
        };
        let method = item.items.iter().find_map(|part| match part {
            ImplItem::Fn(method) if method.sig.ident == call.method => Some(method),
            _ => None,
        });
        if (!trait_route && (method.is_none() || candidate.library != owning_library))
            || (trait_route && ![library, owning_library.as_str()].contains(&candidate.library))
        {
            continue;
        }
        if method.is_none() && !default_method {
            continue;
        }
        if candidate
            .cfg
            .iter()
            .any(|cfg| build.evaluate(cfg) == Some(false))
            || !build.enabled(&item.attrs)
        {
            continue;
        }
        let scope = index.module(candidate.library, &candidate.module)?;
        if let Some((negation, trait_path, _)) = &item.trait_
            && (negation.is_some() || type_id(index, scope, trait_path)? != Some(owner))
        {
            continue;
        }
        let blanket = trait_route
            && ty.path.get_ident().is_some_and(|ident| {
                item.generics
                    .type_params()
                    .any(|param| param.ident == *ident)
            });
        if !blanket && type_id(index, scope, &ty.path)? != Some(receiver) {
            continue;
        }
        if let Some(method) = method
            && (!signature_matches(&method.sig, call) || !build.enabled(&method.attrs))
        {
            continue;
        }
        matched += 1;
    }
    match matched {
        1 => Ok(()),
        0 if trait_route => Err("no matching trait implementation for the nominal receiver".into()),
        0 => Err("no matching inherent definition on this owner".into()),
        _ => Err("multiple definitions match the declared owner and receiver".into()),
    }
}
