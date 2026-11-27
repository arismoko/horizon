use crate::catalog::{BindingKind, GameplayBinding};
use crate::{Fill, SourceLayout, dispatch, gameplay, needs_fill, rustfmt, source};
use quote::ToTokens;
use std::{collections::BTreeMap, fs, path::Path};
use syn::{Block, FnArg, ImplItem, Item, Signature, TraitItem, Type, spanned::Spanned};

pub(crate) fn fill(root: &Path, layout: SourceLayout) -> Result<(), String> {
    let catalog = gameplay::Routing::load(root)?.catalog(root)?;
    let emitted = dispatch::emit(
        catalog
            .hooks
            .iter()
            .filter(|hook| !dispatch::is_render(hook)),
        &catalog.draw_contexts,
        true,
    )?;
    let fills: BTreeMap<_, _> = emitted
        .fills
        .iter()
        .map(|fill| (&fill.native, fill))
        .collect();
    let mut modules = BTreeMap::<_, Vec<_>>::new();
    for hook in &catalog.hooks {
        if let Some(binding) = hook.gameplay_binding()?
            && let Some(fill) = fills.get(&binding.function)
        {
            let module = format!("{}::{}", binding.package.domain().key, binding.module);
            modules.entry(module).or_default().push((binding, *fill));
        }
    }
    let mut pending = BTreeMap::<_, Vec<_>>::new();
    for file in layout.files(root, false)? {
        let targets: Vec<_> = modules
            .iter()
            .filter_map(|(module, bindings)| {
                let path = module.strip_prefix(&file.module)?;
                (path.is_empty() || path.starts_with("::")).then_some((path, bindings))
            })
            .collect();
        if targets.is_empty() {
            continue;
        }
        let text = fs::read_to_string(&file.path).map_err(|error| error.to_string())?;
        let input = syn::parse_file(&text).map_err(|error| error.to_string())?;
        for (path, bindings) in targets {
            for items in in_module(&input.items, path) {
                for (binding, fill) in bindings {
                    pending
                        .entry(file.path.clone())
                        .or_default()
                        .extend(edits(&text, items, binding, fill)?);
                }
            }
        }
    }
    let mut outputs = Vec::new();
    for (path, mut changes) in pending
        .into_iter()
        .filter(|(_, changes)| !changes.is_empty())
    {
        changes.sort();
        changes.dedup();
        let mut text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        for (start, end, body) in changes.into_iter().rev() {
            text.replace_range(start..end, &body);
        }
        outputs.push((path, rustfmt(&text)?));
    }
    for (path, bytes) in outputs {
        fs::write(&path, bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

fn in_module<'a>(items: &'a [Item], relative: &str) -> Vec<&'a [Item]> {
    let mut scopes = vec![items];
    for name in relative.split("::").filter(|name| !name.is_empty()) {
        scopes = scopes
            .into_iter()
            .flat_map(|items| {
                items.iter().filter_map(|item| {
                    if let Item::Mod(item) = item
                        && item.ident == name
                        && !mod_source::modules::test_only(&item.attrs)
                    {
                        return item.content.as_ref().map(|(_, items)| items.as_slice());
                    }
                    None
                })
            })
            .collect();
    }
    scopes
}

fn bodies<'a>(item: &'a Item, binding: &GameplayBinding) -> Vec<(&'a Signature, &'a Block)> {
    match (item, &binding.kind) {
        (Item::Fn(function), BindingKind::Free) => {
            vec![(&function.sig, &function.block)]
        }
        (Item::Fn(function), BindingKind::Borrowed(expected))
            if matches!(function.sig.inputs.first(), Some(FnArg::Typed(receiver))
                if matches!(&*receiver.ty, Type::Reference(reference)
                    if reference.elem.to_token_stream().to_string() == expected.to_token_stream().to_string())) =>
        {
            vec![(&function.sig, &function.block)]
        }
        (Item::Trait(definition), BindingKind::Shared) if definition.ident == "GameHooks" => {
            definition
                .items
                .iter()
                .filter_map(|member| match member {
                    TraitItem::Fn(method) => Some((&method.sig, method.default.as_ref()?)),
                    _ => None,
                })
                .collect()
        }
        (Item::Impl(definition), BindingKind::Nominal | BindingKind::Scoped(_))
            if definition.trait_.is_none() =>
        {
            let owner = match &binding.kind {
                BindingKind::Scoped(owner) => owner.ident.to_string(),
                _ => binding.package.game_type().to_owned(),
            };
            if !matches!(&*definition.self_ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == owner))
            {
                return Vec::new();
            }
            definition
                .items
                .iter()
                .filter_map(|member| match member {
                    ImplItem::Fn(method) => Some((&method.sig, &method.block)),
                    _ => None,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn edits(
    text: &str,
    items: &[Item],
    binding: &GameplayBinding,
    fill: &Fill,
) -> Result<Vec<(usize, usize, String)>, String> {
    let lines: Vec<_> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let mut edits = Vec::new();
    for (sig, body) in items.iter().flat_map(|item| bodies(item, binding)) {
        if sig.ident != binding.function {
            continue;
        }
        let [statement] = body.stmts.as_slice() else {
            continue;
        };
        let syn::Stmt::Local(local) = statement else {
            continue;
        };
        if !matches!(local.pat, syn::Pat::Wild(_)) {
            continue;
        }
        let method = syn::ItemFn {
            attrs: Vec::new(),
            vis: syn::Visibility::Inherited,
            sig: sig.clone(),
            block: Box::new(body.clone()),
        };
        let native = binding
            .function
            .strip_prefix("native_")
            .expect("canonical binding name");
        if !needs_fill(&method, &format!("dispatch_{native}"), true)? {
            continue;
        }
        let mut call = fill.call.clone();
        if matches!(binding.kind, BindingKind::Borrowed(_))
            && let Some(FnArg::Typed(receiver)) = sig.inputs.first()
        {
            call = call.replace(
                "__fire_receiver",
                &receiver.pat.to_token_stream().to_string(),
            );
        }
        let span = statement.span();
        edits.push((
            source::offset(text, &lines, span.start()),
            source::offset(text, &lines, span.end()),
            call,
        ));
    }
    Ok(edits)
}

#[cfg(test)]
#[test]
fn selects_nominal_shared_and_free_placeholders_by_owner_and_module() {
    let text = r#"
mod nominal {
    impl ServerGame { fn native_test(&self, x: u32) { let _ = x; } }
    impl Decoy { fn native_test(&self, x: u32) { let _ = x; } }
}
mod shared {
    trait GameHooks { fn native_test(&self, x: u32) { let _ = x; } }
    trait Decoy { fn native_test(&self, x: u32) { let _ = x; } }
}
mod free {
    fn native_test(x: u32) { let _ = x; }
    impl Decoy { fn native_test(x: u32) { let _ = x; } }
}
mod filled {
    fn native_test(x: u32) { callback(x); }
}
mod borrowed {
    fn native_test(loan: &Level, x: u32) { let _ = (loan, x); }
    #[cfg(feature = "other")]
    fn native_test(loan: &Other, x: u32) { let _ = (loan, x); }
}
"#;
    let file = syn::parse_file(text).unwrap();
    for (name, kind) in [
        ("nominal", BindingKind::Nominal),
        ("shared", BindingKind::Shared),
        ("free", BindingKind::Free),
        ("filled", BindingKind::Free),
        ("borrowed", BindingKind::Borrowed(syn::parse_quote!(Level))),
    ] {
        let scopes = in_module(&file.items, name);
        let [items] = scopes.as_slice() else {
            panic!("expected one module")
        };
        let binding = GameplayBinding {
            package: crate::GameplayPackage::Server,
            module: name.into(),
            function: "native_test".into(),
            kind,
        };
        let fill = Fill {
            native: "native_test".into(),
            call: if name == "borrowed" {
                "fire_test(__fire_receiver, x);"
            } else {
                "fire_test(x);"
            }
            .into(),
        };
        let changes = edits(text, items, &binding, &fill).unwrap();
        assert_eq!(changes.len(), usize::from(name != "filled"));
        if let [(start, end, call)] = changes.as_slice() {
            let (statement, expected_call) = if name == "borrowed" {
                ("let _ = (loan, x);", "fire_test(loan, x);")
            } else {
                ("let _ = x;", "fire_test(x);")
            };
            assert_eq!(&text[*start..*end], statement);
            assert_eq!(call, expected_call);
        }
    }
}
