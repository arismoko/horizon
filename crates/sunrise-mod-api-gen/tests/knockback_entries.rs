//! Run the real entry bodies against a controlled living-knockback hook boundary.
//! No point instrumentation or game resources: only world/context services are doubles.
use quote::quote;
use std::{fs, path::Path, process::Command};

#[test]
#[ignore = "daily"]
fn retained_knockback_entries_apply_hook_edits_cancel_and_delegate_once() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut modules = proc_macro2::TokenStream::new();
    for (module, path) in [
        ("entries", "crates/sunrise-sim/src/mobs/knockback.rs"),
        (
            "player_entries",
            "crates/sunrise-gameplay-common/src/game/mobs/creatures/knockback.rs",
        ),
        (
            "server_entries",
            "crates/sunrise-gameplay/src/game/mobs/creatures/knockback_server.rs",
        ),
    ] {
        let source = fs::read_to_string(root.join(path)).unwrap();
        let mut file = syn::parse_file(&source).unwrap();
        // Let the fixture call both entry forms; leave signatures and bodies intact.
        for item in &mut file.items {
            // A free helper's restricted path names a module the fixture doesn't have.
            if let syn::Item::Fn(function) = item {
                function.vis = syn::parse_quote!(pub(crate));
            }
            if let syn::Item::Impl(owner) = item {
                for item in &mut owner.items {
                    if let syn::ImplItem::Fn(function) = item {
                        function.vis = syn::parse_quote!(pub(crate));
                        // This dependency-free fixture omits the no-op staging marker.
                        function.attrs.retain(|attr| {
                            !attr
                                .path()
                                .segments
                                .last()
                                .is_some_and(|s| s.ident == "replaceable")
                        });
                    }
                }
            }
        }
        let module = syn::Ident::new(module, proc_macro2::Span::call_site());
        modules.extend(quote!(mod #module { #file }));
    }
    // The real movement write both entries end in (`Knock::apply`), not a second copy of it.
    let health = fs::read_to_string(root.join("crates/sunrise-sim/src/health.rs")).unwrap();
    let knock_apply = syn::parse_file(&health)
        .unwrap()
        .items
        .into_iter()
        .find(|item| {
            matches!(item, syn::Item::Impl(owner)
                if owner.trait_.is_none()
                    && matches!(&*owner.self_ty, syn::Type::Path(path) if path.path.is_ident("Knock")))
        })
        .expect("health.rs has `impl Knock`");
    modules.extend(quote!(mod knock_apply { use crate::health::Knock; #knock_apply }));
    let directory = tempfile::tempdir_in(root.join("target")).unwrap();
    let source = directory.path().join("main.rs");
    let binary = directory.path().join("knockback");
    fs::write(
        &source,
        format!(
            "{}\n{}\n{}",
            include_str!("knockback_fixture/preamble.rs.txt"),
            modules,
            include_str!("knockback_fixture/main.rs.txt")
        ),
    )
    .unwrap();
    let output = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=knockback_fixture"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
