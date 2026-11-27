use crate::{SourceLayout, catalog::BindingKind, gameplay::Routing};
use std::{collections::BTreeMap, fs, path::Path};
use syn::{ImplItem, Item, TraitItem, Type};

#[test]
fn native_hooks_live_in_their_catalog_area() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = Routing::load(&root).unwrap().catalog(&root).unwrap();
    let mut modules = BTreeMap::<_, Vec<Item>>::new();
    for file in SourceLayout::read(&root)
        .unwrap()
        .files(&root, false)
        .unwrap()
    {
        if file.module.contains("::native_hooks") {
            let source = fs::read_to_string(&file.path).unwrap();
            modules
                .entry(file.module)
                .or_default()
                .extend(syn::parse_file(&source).unwrap().items);
        }
    }
    let mut checked = 0;
    for hook in catalog.hooks {
        let Some(binding) = hook.gameplay_binding().unwrap() else {
            continue;
        };
        checked += 1;
        let module = format!("{}::{}", binding.package.domain().key, binding.module);
        let items = modules
            .get(&module)
            .unwrap_or_else(|| panic!("missing module {module}"));
        assert!(
                items.iter().any(|item| match (item, &binding.kind) {
                    (Item::Fn(function), BindingKind::Free | BindingKind::Borrowed(_)) => {
                        function.sig.ident == binding.function
                    }
                    (Item::Trait(item), BindingKind::Shared) => {
                        item.ident == "GameHooks" && item.items.iter().any(|member| {
                            matches!(member, TraitItem::Fn(method) if method.sig.ident == binding.function && method.default.is_some())
                        })
                    }
                    (Item::Impl(item), BindingKind::Nominal | BindingKind::Scoped(_)) => {
                        let owner = match &binding.kind {
                            BindingKind::Scoped(owner) => owner.ident.to_string(),
                            _ => binding.package.game_type().to_owned(),
                        };
                        item.trait_.is_none()
                            && matches!(&*item.self_ty, Type::Path(ty) if ty.path.segments.last().is_some_and(|segment| segment.ident == owner))
                            && item.items.iter().any(|member| {
                                matches!(member, ImplItem::Fn(method) if method.sig.ident == binding.function)
                            })
                    }
                    _ => false,
                }),
                "missing catalog target {}",
                hook.target,
            );
    }
    assert!(checked > 0);
}
