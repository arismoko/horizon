//! Dispatch-test fragments assembled at their original lexical scopes.
use crate::catalog::{Catalog, DECLARATIONS};
use quote::ToTokens;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
use syn::Item;

const DIRECTORY: &str = "crates/sunrise-mod-api-gen/dispatch-tests/src/fixtures";
/// Folders of one file per shared stand-in, read into the scope of the same name.
const STAND_IN_SCOPES: [&str; 2] = ["stubs", "api"];

fn key(item: &Item) -> String {
    let name = match item {
        Item::Struct(i) => &i.ident,
        Item::Enum(i) => &i.ident,
        Item::Trait(i) => &i.ident,
        Item::Type(i) => &i.ident,
        Item::Fn(i) => &i.sig.ident,
        Item::Mod(i) => &i.ident,
        Item::Const(i) => &i.ident,
        Item::Static(i) => &i.ident,
        Item::Impl(i) => {
            return format!(
                "impl {:?} {}",
                i.trait_
                    .as_ref()
                    .map(|(_, p, _)| p.to_token_stream().to_string()),
                i.self_ty.to_token_stream()
            );
        }
        _ => return item.to_token_stream().to_string(),
    };
    name.to_string()
}

fn collect(
    source: &str,
    shared: bool,
    output: &mut BTreeMap<&'static str, String>,
    seen: &mut BTreeSet<(String, String)>,
) -> Result<(), String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    for item in file.items {
        let (scope, items) = if let Item::Mod(module) = &item {
            let scope = match module.ident.to_string().as_str() {
                "root" => Some("root"),
                "stubs" => Some("stubs"),
                "crate_root" => Some("crate_root"),
                "api" => Some("api"),
                _ if shared => None,
                _ => return Err("unsupported fixture envelope".into()),
            };
            if let Some(scope) = scope {
                if !module.attrs.is_empty() || !matches!(module.vis, syn::Visibility::Inherited) {
                    return Err("fixture envelopes must be private and have no attributes".into());
                }
                (
                    scope,
                    module
                        .content
                        .as_ref()
                        .ok_or("fixture envelopes must be inline")?
                        .1
                        .clone(),
                )
            } else {
                ("root", vec![item])
            }
        } else {
            ("root", vec![item])
        };
        for item in items {
            push(scope, item, output, seen)?;
        }
    }
    Ok(())
}

/// The real API types a dispatch snapshots are `Clone`; so are their stand-ins, except those
/// that hold a borrow or a trait object.
fn stand_in_clone(item: &mut Item) {
    let text = item.to_token_stream().to_string();
    match item {
        Item::Struct(syn::ItemStruct { attrs, .. }) | Item::Enum(syn::ItemEnum { attrs, .. })
            if !text.contains("Clone") && !text.contains("& 'a mut") && !text.contains("dyn") =>
        {
            attrs.push(syn::parse_quote!(#[derive(Clone)]));
        }
        Item::Mod(module) => {
            for item in module.content.iter_mut().flat_map(|(_, items)| items) {
                stand_in_clone(item);
            }
        }
        _ => {}
    }
}

fn push(
    scope: &'static str,
    item: Item,
    output: &mut BTreeMap<&'static str, String>,
    seen: &mut BTreeSet<(String, String)>,
) -> Result<(), String> {
    if !seen.insert((scope.to_owned(), key(&item))) {
        return Err(format!("duplicate fixture item in {scope}: {}", key(&item)));
    }
    let mut item = item;
    if matches!(scope, "stubs" | "api") {
        stand_in_clone(&mut item);
    }
    output
        .entry(scope)
        .or_default()
        .push_str(&format!("{}\n", item.to_token_stream()));
    Ok(())
}

pub(crate) fn generate(root: &Path, out: &Path, catalog: &Catalog) -> Result<(), String> {
    let dir = root.join(DIRECTORY);
    crate::rerun_if_changed(&dir.display());
    let mut areas = sunrise_game_discovery::module_files(&root.join(DECLARATIONS))?;
    // A fixture can support a declared engine target as well as a declaration file.
    // Their boundaries need not match when declarations are split per hook.
    const NATIVE: &str = "sunrise_client::game::native_hooks::";
    for hook in &catalog.hooks {
        let split = hook.gameplay_binding()?;
        let relative = match &split {
            Some(binding) => binding.module.strip_prefix("game::native_hooks::"),
            None => hook.target.strip_prefix(NATIVE),
        };
        let area = if split.is_some() {
            relative.and_then(|path| path.split("::").next())
        } else {
            relative.and_then(|path| path.split_once("::").map(|(area, _)| area))
        };
        if let Some(area) = area {
            areas.push(area.into());
        }
    }
    let mut output = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for area in sunrise_game_discovery::module_files(&dir)? {
        if area != "shared" && !areas.contains(&area) {
            return Err(format!("fixture area {area} has no catalog declaration"));
        }
        let path = dir.join(format!("{area}.rs"));
        crate::rerun_if_changed(&path.display());
        collect(
            &fs::read_to_string(&path).map_err(|e| e.to_string())?,
            area == "shared",
            &mut output,
            &mut seen,
        )
        .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    // `fixtures/stubs/<Name>.rs` holds bare items of the `stubs` scope, without an envelope.
    for scope in STAND_IN_SCOPES {
        let folder = dir.join(scope);
        if !folder.is_dir() {
            continue;
        }
        crate::rerun_if_changed(&folder.display());
        for name in sunrise_game_discovery::module_files(&folder)? {
            let path = folder.join(format!("{name}.rs"));
            crate::rerun_if_changed(&path.display());
            let source = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            for item in syn::parse_file(&source)
                .map_err(|e| format!("{}: {e}", path.display()))?
                .items
            {
                push(scope, item, &mut output, &mut seen)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    for scope in ["root", "stubs", "api", "crate_root"] {
        fs::write(
            out.join(format!("dispatch_fixtures_{scope}.rs")),
            output.get(scope).cloned().unwrap_or_default(),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_fixture(root: &Path) -> Result<(), String> {
        generate(root, root, &Catalog::load(root)?)
    }

    #[test]
    fn scopes_discovery_and_rejection() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../target/dispatch-fixture-discovery-{}",
            std::process::id()
        ));
        let dir = root.join(DIRECTORY);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(root.join(DECLARATIONS)).unwrap();
        fs::write(root.join(DECLARATIONS).join("alpha.rs"), "").unwrap();
        fs::write(dir.join("shared.rs"), "struct Common;").unwrap();
        generate_fixture(&root).unwrap();
        fs::write(
            dir.join("alpha.rs"),
            "fn check() {} mod root { mod runtime {} } mod stubs { struct Stub; } mod api { pub struct Api; } mod crate_root { mod item {} }",
        )
        .unwrap();
        fs::write(root.join(DECLARATIONS).join("zeta.rs"), "").unwrap();
        fs::write(dir.join("zeta.rs"), "fn last() {}").unwrap();
        generate_fixture(&root).unwrap();
        let ordered = fs::read_to_string(root.join("dispatch_fixtures_root.rs")).unwrap();
        assert!(ordered.find("check").unwrap() < ordered.find("last").unwrap());
        for (scope, name) in [
            ("root", "check"),
            ("root", "runtime"),
            ("stubs", "Stub"),
            ("api", "Api"),
            ("crate_root", "item"),
        ] {
            let source =
                fs::read_to_string(root.join(format!("dispatch_fixtures_{scope}.rs"))).unwrap();
            assert!(
                syn::parse_file(&source)
                    .unwrap()
                    .items
                    .iter()
                    .any(|i| key(i) == name)
            );
        }
        fs::create_dir_all(dir.join("stubs")).unwrap();
        fs::create_dir_all(dir.join("api")).unwrap();
        fs::write(dir.join("stubs/Folded.rs"), "struct Folded; impl Folded {}").unwrap();
        fs::write(dir.join("api/Exposed.rs"), "pub struct Exposed;").unwrap();
        generate_fixture(&root).unwrap();
        for (scope, name) in [("stubs", "Folded"), ("api", "Exposed")] {
            let source =
                fs::read_to_string(root.join(format!("dispatch_fixtures_{scope}.rs"))).unwrap();
            assert!(
                syn::parse_file(&source)
                    .unwrap()
                    .items
                    .iter()
                    .any(|i| key(i) == name)
            );
        }
        fs::write(dir.join("api/Stub.rs"), "pub struct Stub;").unwrap();
        generate_fixture(&root).unwrap();
        fs::write(dir.join("stubs/Stub.rs"), "struct Stub;").unwrap();
        assert!(
            generate_fixture(&root)
                .unwrap_err()
                .contains("Stub.rs: duplicate fixture item in stubs: Stub")
        );
        fs::remove_dir_all(dir.join("stubs")).unwrap();
        fs::remove_dir_all(dir.join("api")).unwrap();
        fs::remove_file(dir.join("alpha.rs")).unwrap();
        generate_fixture(&root).unwrap();
        assert!(
            !fs::read_to_string(root.join("dispatch_fixtures_root.rs"))
                .unwrap()
                .contains("check")
        );
        for (source, error) in [
            ("struct Common;", "duplicate"),
            (
                "mod stubs { struct S; } mod stubs { struct S; }",
                "duplicate",
            ),
            ("mod api { struct A; enum A {} }", "duplicate"),
            ("mod other {}", "unsupported"),
            ("#[allow(dead_code)] mod stubs {}", "no attributes"),
            ("mod api;", "inline"),
            ("pub mod root {}", "private"),
            ("#[allow(dead_code)] mod root {}", "no attributes"),
            ("mod root;", "inline"),
            ("struct Same; mod root { struct Same; }", "duplicate"),
        ] {
            fs::write(dir.join("alpha.rs"), source).unwrap();
            assert!(generate_fixture(&root).unwrap_err().contains(error));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fixture_root_has_no_hand_kept_items() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("dispatch-tests/src");
        let source = fs::read_to_string(root.join("lib.rs")).unwrap();
        assert_eq!(
            source.trim().replace("\r\n", "\n"),
            "#[cfg(test)]\ninclude!(concat!(env!(\"OUT_DIR\"), \"/dispatch_fixtures_crate_root.rs\"));\n\n#[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    mod stubs {\n        include!(concat!(env!(\"OUT_DIR\"), \"/dispatch_fixtures_stubs.rs\"));\n    }\n    use generated as mod_api;\n    include!(concat!(env!(\"OUT_DIR\"), \"/dispatch_fixtures_root.rs\"));\n}"
        );
        for name in ["shared", "base"] {
            assert!(
                fs::read_to_string(root.join(format!("fixtures/{name}.rs")))
                    .unwrap()
                    .starts_with("//! ")
            );
        }
    }
}
