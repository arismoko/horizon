//! Tests for owner lookup, trait discovery, visibility, cfg handling, and per-slot evidence.
use crate::{
    functions,
    replace::{self, OwnerTypes, ResolvedItem, TraitSlotEvidence, WorkspaceTraits},
    staging,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (rel, text) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
}

fn minimal_workspace(root: &Path, lib_rs: &str, game_rs: &str, extra: &[(&str, &str)]) {
    write_files(
        root,
        &[
            (
                "crates/sunrise-client/Cargo.toml",
                "[package]\nname = 'sunrise-client'\n[dependencies]\n",
            ),
            ("crates/sunrise-client/src/lib.rs", lib_rs),
            ("crates/sunrise-client/src/entry.rs", ""),
            ("crates/sunrise-client/src/game.rs", game_rs),
            (
                "Cargo.lock",
                "version = 4\n\n[[package]]\nname = \"sunrise-client\"\nversion = \"0.1.0\"\n",
            ),
            ("crates/sunrise-mod-api-gen/internals.txt", ""),
            (crate::crossing::CROSSING_FOREIGN_FILE, "[]"),
        ],
    );
    write_files(root, extra);
}

fn run_stage(root: &Path) -> (staging::Staged, functions::FunctionIndex) {
    let build = staging::Build {
        commit: "c".repeat(40),
    };
    let staged = staging::stage(
        root,
        &root.join("internals.json"),
        staging::Mode::Build,
        &build,
    )
    .unwrap();
    let text = fs::read_to_string(root.join(functions::FILE)).unwrap();
    let (_, index): (staging::Build, functions::FunctionIndex) =
        mod_surface::build_data::read(functions::FILE, &text).unwrap();
    (staged, index)
}

fn rustc_check(source: &Path, out_dir: &Path, extra_args: &[&str]) {
    let result = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--emit=metadata",
            "--out-dir",
        ])
        .arg(out_dir)
        .args(extra_args)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "rustc failed on {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&result.stderr)
    );
}

fn rustc_rlib(source: &Path, out_dir: &Path, crate_name: &str, extra_args: &[&str]) -> PathBuf {
    let result = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=rlib",
            "--crate-name",
            crate_name,
            "--out-dir",
        ])
        .arg(out_dir)
        .args(extra_args)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "rustc rlib failed on {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&result.stderr)
    );
    out_dir.join(format!("lib{crate_name}.rlib"))
}

fn rustc_run_bin(source: &Path, out_dir: &Path, extra_args: &[&str]) {
    let bin = out_dir.join(format!("driver{}", std::env::consts::EXE_SUFFIX));
    let result = Command::new("rustc")
        .args(["--edition=2024", "-o"])
        .arg(&bin)
        .args(extra_args)
        .arg(source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "rustc bin failed on {}:\n{}",
        source.display(),
        String::from_utf8_lossy(&result.stderr)
    );
    let run = Command::new(&bin).output().unwrap();
    assert!(
        run.status.success(),
        "driver failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

fn refusal_of(modules: &[(&str, &str)], impl_mod: &str, impl_src: &str) -> Option<String> {
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
            syn::Item::Impl(block) => Some(block),
            _ => None,
        })
        .unwrap();
    replace::trait_refusal(impl_mod, block, &types, &WorkspaceTraits::empty())
}

fn binding_of(
    modules: &[(&str, &str)],
    impl_mod: &str,
    impl_src: &str,
    method: &str,
) -> Result<TraitSlotEvidence, String> {
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
            syn::Item::Impl(block) => Some(block),
            _ => None,
        })
        .unwrap();
    let ident = syn::Ident::new(method, proc_macro2::Span::call_site());
    replace::resolve_trait_binding(impl_mod, block, &ident, &types, &WorkspaceTraits::empty())
}

#[test]
#[ignore = "daily"]
fn cfg_gated_local_and_foreign_pathbuf_refused_and_compiles_both_cfgs() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let game_rs = r#"
mod subject {
    #[cfg(feature = "local_defs")]
    pub struct PathBuf;
    #[cfg(not(feature = "local_defs"))]
    use std::path::PathBuf;
    pub trait Entity { fn f(&self); }
    impl Entity for PathBuf { fn f(&self) {} }
}
"#;
    minimal_workspace(root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (staged, index) = run_stage(root);
    let key = "sunrise_client::game::subject::<PathBuf as Entity>::f";
    assert!(!staged.replace.slots.contains(key));
    assert_eq!(
        index.refused.get(key).map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (conditional trait owner)")
    );
    let lib = root.join("crates/sunrise-client/src/lib.rs");
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    rustc_check(&lib, &out, &["--crate-name=sunrise_client"]);
    rustc_check(
        &lib,
        &out,
        &[
            "--crate-name=sunrise_client",
            "--cfg",
            "feature=\"local_defs\"",
        ],
    );
}

#[test]
#[ignore = "daily"]
fn foreign_type_alias_and_test_only_prelude_string_refused_and_compile() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let game_rs = r#"
mod other {
    pub struct PathBuf;
    pub struct String;
}
mod alias_subject {
    type PathBuf = std::path::PathBuf;
    pub trait Entity { fn f(&self); }
    impl Entity for PathBuf { fn f(&self) {} }
}
mod prelude_subject {
    #[cfg(test)]
    use std::string::String;
    pub trait Entity { fn f(&self); }
    impl Entity for String { fn f(&self) {} }
}
"#;
    minimal_workspace(root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (staged, index) = run_stage(root);
    let alias_key = "sunrise_client::game::alias_subject::<PathBuf as Entity>::f";
    let prelude_key = "sunrise_client::game::prelude_subject::<String as Entity>::f";
    assert!(!staged.replace.slots.contains(alias_key));
    assert!(!staged.replace.slots.contains(prelude_key));
    assert_eq!(
        index.refused.get(alias_key).map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (type alias trait owner)")
    );
    assert_eq!(
        index.refused.get(prelude_key).map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (foreign trait owner)")
    );
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    rustc_check(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );
}

#[test]
#[ignore = "daily"]
fn bare_crate_name_requires_extern_crate_self_and_compiles_with_same_named_extern() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    let ext_src = dir.path().join("ext.rs");
    fs::write(&ext_src, "pub mod same { pub struct Boat; }\n").unwrap();
    let ext_rlib = rustc_rlib(&ext_src, &out, "sunrise_client", &["-C", "metadata=ext"]);

    // Without `extern crate self as sunrise_client;`, the bare crate name is external and refused.
    let root = dir.path().join("no-self");
    minimal_workspace(
        &root,
        "pub mod game; mod entry;\n",
        r#"
pub mod same { pub struct Boat; }
mod subject {
    use sunrise_client::same::Boat;
    pub trait Entity { fn f(&self); }
    impl Entity for Boat { fn f(&self) {} }
}
"#,
        &[],
    );
    let (staged, index) = run_stage(&root);
    let key = "sunrise_client::game::subject::<Boat as Entity>::f";
    assert!(!staged.replace.slots.contains(key));
    assert_eq!(
        index.refused.get(key).map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (imported trait owner)")
    );
    rustc_check(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &[
            "--crate-name=sunrise_client",
            "-C",
            "metadata=local",
            "--extern",
            &format!("sunrise_client={}", ext_rlib.display()),
        ],
    );

    // With `extern crate self as sunrise_client;`, the path is proven local and compiles.
    let root_self = dir.path().join("with-self");
    minimal_workspace(
        &root_self,
        "extern crate self as sunrise_client;\npub mod game; mod entry;\n",
        r#"
pub mod same { pub struct Boat; }
mod subject {
    use sunrise_client::game::same::Boat;
    pub trait Entity { fn f(&self); }
    impl Entity for Boat { fn f(&self) {} }
}
"#,
        &[],
    );
    let (staged_self, index_self) = run_stage(&root_self);
    assert!(staged_self.replace.slots.contains(key));
    assert_eq!(
        index_self.trait_slots.get(key),
        Some(&TraitSlotEvidence {
            owner: ResolvedItem {
                module: "sunrise_client::game::same".into(),
                item: "Boat".into(),
            },
            trait_decl: ResolvedItem {
                module: "sunrise_client::game::subject".into(),
                item: "Entity".into(),
            },
            installer_module: "sunrise_client::game::subject".into(),
            installer: "__fnr_as_Entity_f".into(),
            reader: Some("__fnr_read_slot_0".into()),
            cell: Some(0),
        })
    );
    rustc_check(
        &root_self.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );
}

#[test]
#[ignore = "daily"]
fn final_declaration_visibility_handles_hidden_and_public_globs_and_private_string() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();

    // Hidden + public globbed Boat becomes ambiguous after staging and emits no installer.
    let game_rs = r#"
mod a { struct Boat; }
mod b { pub struct Boat; }
mod subject {
    use super::a::*;
    use super::b::*;
    pub trait Entity { fn f(&self); }
    impl Entity for Boat { fn f(&self) {} }
}
"#;
    let key = "sunrise_client::game::subject::<Boat as Entity>::f";
    let root_replace_only = dir.path().join("hidden-and-public-replace-only");
    minimal_workspace(
        &root_replace_only,
        "pub mod game; mod entry;\n",
        game_rs,
        &[],
    );
    let lib_ro = root_replace_only.join("crates/sunrise-client/src/lib.rs");
    rustc_check(&lib_ro, &out, &["--crate-name=sunrise_client"]);
    let replace_only = replace::stage(&root_replace_only).unwrap();
    assert!(!replace_only.slots.contains(key));
    assert!(
        !fs::read_to_string(root_replace_only.join("crates/sunrise-client/src/game.rs"))
            .unwrap()
            .contains("__fnr_")
    );
    rustc_check(&lib_ro, &out, &["--crate-name=sunrise_client"]);

    let root = dir.path().join("hidden-and-public");
    minimal_workspace(&root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (staged, index) = run_stage(&root);
    assert!(!staged.replace.slots.contains(key));
    assert_eq!(
        index.refused.get(key).map(|r| r.why.as_str()),
        Some(
            "its trait implementation has no replacement installer (ambiguous trait owner after staging)"
        )
    );
    assert!(
        !fs::read_to_string(root.join("crates/sunrise-client/src/game.rs"))
            .unwrap()
            .contains("__fnr_")
    );

    // Hidden + public globbed trait: refused as ambiguous trait after staging.
    assert_eq!(
        refusal_of(
            &[
                ("k::a", "trait Entity { fn f(&self); }"),
                ("k::b", "pub trait Entity { fn f(&self); }"),
            ],
            "k::subject",
            "use super::a::*; use super::b::*; pub struct Boat; impl Entity for Boat { fn f(&self) {} }",
        )
        .as_deref(),
        Some("its trait implementation has no replacement installer (ambiguous trait after staging)")
    );

    // Private glob String shadows prelude after widening: every impl in `subject` is refused and compiles.
    let root_priv = dir.path().join("private-string");
    let game_priv = r#"
mod other { struct String; }
pub mod subject {
    use super::other::*;
    pub struct Local;
    pub trait Entity { fn f(&self); }
    impl Entity for String { fn f(&self) {} }
    impl Entity for Local { fn f(&self) {} }
    impl Local { pub fn inherent(&self) {} }
}
"#;
    minimal_workspace(&root_priv, "pub mod game; mod entry;\n", game_priv, &[]);
    let lib_priv = root_priv.join("crates/sunrise-client/src/lib.rs");
    rustc_check(&lib_priv, &out, &["--crate-name=sunrise_client"]);
    let (staged_priv, index_priv) = run_stage(&root_priv);
    let str_key = "sunrise_client::game::subject::<String as Entity>::f";
    let local_trait_key = "sunrise_client::game::subject::<Local as Entity>::f";
    let local_inherent_key = "sunrise_client::game::subject::Local::inherent";
    for refused_key in [str_key, local_trait_key, local_inherent_key] {
        assert!(
            !staged_priv.replace.slots.contains(refused_key),
            "{refused_key}"
        );
        assert!(
            index_priv.refused.get(refused_key).is_some_and(|r| r
                .why
                .contains("widened glob shadows prelude or external name")),
            "{refused_key}: {:?}",
            index_priv.refused.get(refused_key)
        );
    }
    assert!(
        !fs::read_to_string(root_priv.join("crates/sunrise-client/src/game.rs"))
            .unwrap()
            .contains("__fnr_")
    );
    rustc_check(&lib_priv, &out, &["--crate-name=sunrise_client"]);

    // Foreign glob + hidden local glob item: widening may collide with the foreign glob.
    let root_foreign_glob = dir.path().join("foreign-glob-shadow");
    let game_foreign_glob = r#"
mod other { struct Secret; }
pub mod subject {
    use std::fmt::*;
    use super::other::*;
    pub struct Local;
    pub trait Entity { fn f(&self); }
    impl Entity for Local { fn f(&self) {} }
}
"#;
    minimal_workspace(
        &root_foreign_glob,
        "pub mod game; mod entry;\n",
        game_foreign_glob,
        &[],
    );
    let (staged_fg, index_fg) = run_stage(&root_foreign_glob);
    assert!(!staged_fg.replace.slots.contains(local_trait_key));
    assert!(index_fg.refused.get(local_trait_key).is_some_and(|r| {
        r.why
            .contains("widened glob shadows prelude or external name")
    }));
    rustc_check(
        &root_foreign_glob.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );

    // Positive controls: non-prelude hidden glob item and locally shadowed prelude name keep slots.
    let root_ok = dir.path().join("non-prelude-hidden-glob");
    let game_ok = r#"
mod other { struct Helper; struct String; }
pub mod subject {
    use super::other::*;
    pub struct Local;
    pub struct String;
    pub trait Entity { fn f(&self); }
    impl Entity for Local { fn f(&self) {} }
    impl Entity for String { fn f(&self) {} }
}
"#;
    minimal_workspace(&root_ok, "pub mod game; mod entry;\n", game_ok, &[]);
    let (staged_ok, _) = run_stage(&root_ok);
    assert!(staged_ok.replace.slots.contains(local_trait_key));
    assert!(staged_ok.replace.slots.contains(str_key));
    rustc_check(
        &root_ok.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );
}

#[test]
#[ignore = "daily"]
fn namespace_and_definition_identity_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();

    // Local function imported as `Boat` does not mask a glob-imported `Boat` type.
    let root_fn = dir.path().join("local-fn-value");
    let src_fn = r#"
mod defs { pub struct Boat; }
mod subject {
    fn helper() {}
    use self::helper as Boat;
    use super::defs::*;
    pub trait Entity { fn f(&self); }
    impl Entity for Boat { fn f(&self) {} }
}
"#;
    minimal_workspace(&root_fn, "pub mod game; mod entry;\n", src_fn, &[]);
    let (staged_fn, index_fn) = run_stage(&root_fn);
    let key = "sunrise_client::game::subject::<Boat as Entity>::f";
    assert!(staged_fn.replace.slots.contains(key));
    assert_eq!(
        index_fn.trait_slots.get(key).map(|e| &e.owner),
        Some(&ResolvedItem {
            module: "sunrise_client::game::defs".into(),
            item: "Boat".into(),
        })
    );
    rustc_check(
        &root_fn.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );

    // Foreign `std::mem::drop as Boat` alongside a glob is refused fail-closed.
    let root_drop = dir.path().join("foreign-drop-value");
    let src_drop = r#"
mod defs { pub struct Boat; }
mod subject {
    use std::mem::drop as Boat;
    use super::defs::*;
    pub trait Entity { fn f(&self); }
    impl Entity for Boat { fn f(&self) {} }
}
"#;
    minimal_workspace(&root_drop, "pub mod game; mod entry;\n", src_drop, &[]);
    let (staged_drop, index_drop) = run_stage(&root_drop);
    assert!(!staged_drop.replace.slots.contains(key));
    assert_eq!(
        index_drop.refused.get(key).map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (glob import in owner path)")
    );
    rustc_check(
        &root_drop.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );

    // Module vs struct glob collision is refused (`glob import in owner path`).
    assert_eq!(
        refusal_of(
            &[
                ("k::a", "pub mod boats;"),
                ("k::a::boats", "pub struct Boat;"),
                ("k::b", "pub struct boats;"),
            ],
            "k::subject",
            "use super::a::*; use super::b::*; use boats::Boat; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
        )
        .as_deref(),
        Some("its trait implementation has no replacement installer (glob import in owner path)")
    );

    // Module vs enum variant and struct vs enum variant glob collisions are refused.
    assert_eq!(
        refusal_of(
            &[
                ("k::a", "pub mod boats;"),
                ("k::a::boats", "pub struct Boat;"),
                ("k::b", "pub enum E { boats } pub use E::*;"),
            ],
            "k::subject",
            "use super::a::*; use super::b::*; use boats::Boat; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
        )
        .as_deref(),
        Some("its trait implementation has no replacement installer (glob import in owner path)")
    );
    assert!(
        refusal_of(
            &[
                ("k::a", "pub struct Boat;"),
                ("k::b", "pub enum E { Boat } pub use E::*;"),
            ],
            "k::subject",
            "use super::a::*; use super::b::*; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
        )
        .unwrap()
        .contains("ambiguous"),
    );

    // Distinct structs from one module re-exported under one spelling are ambiguous.
    let dup = refusal_of(
        &[
            ("k::defs", "pub struct A; pub struct B;"),
            ("k::x", "pub use super::defs::A as Boat;"),
            ("k::y", "pub use super::defs::B as Boat;"),
        ],
        "k::subject",
        "use super::x::*; use super::y::*; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
    )
    .unwrap();
    assert!(dup.contains("ambiguous"), "{dup}");
}

#[test]
#[ignore = "daily"]
fn owner_controls_and_compiled_dispatch_with_same_spelled_decoy_owner() {
    // Controls: explicit missing path refused, local/explicit precedence, nested pass-through parent.
    assert_eq!(
        refusal_of(
            &[("k::unrelated", "pub struct Boat;")],
            "k::subject",
            "use super::missing::Boat; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
        )
        .as_deref(),
        Some("its trait implementation has no replacement installer (imported trait owner)")
    );
    assert_eq!(
        binding_of(
            &[("k::other", "pub struct Boat;")],
            "k::subject",
            "struct Boat; use super::other::*; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
            "f",
        )
        .unwrap()
        .owner,
        ResolvedItem {
            module: "k::subject".into(),
            item: "Boat".into(),
        }
    );
    assert_eq!(
        binding_of(
            &[
                ("k::game", "mod boats; pub mod outer;"),
                ("k::game::boats", "pub(in crate::game) struct Boat;"),
                (
                    "k::game::outer",
                    "pub(in crate::game::outer) use super::*; pub mod inner;",
                ),
                (
                    "k::game::outer::inner",
                    "pub(in crate::game::outer::inner) use super::*; pub mod entity;",
                ),
            ],
            "k::game::outer::inner::entity",
            "use super::boats::Boat; trait Entity { fn f(&self); } impl Entity for Boat { fn f(&self) {} }",
            "f",
        )
        .unwrap()
        .owner,
        ResolvedItem {
            module: "k::game::boats".into(),
            item: "Boat".into(),
        }
    );

    // Compiled dispatch check: installing on `boats::Boat` dispatches only for `boats::Boat`.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    minimal_workspace(
        root,
        "pub mod game; mod entry;\n",
        r#"
pub mod boats { pub struct Boat(pub u32); }
pub mod decoy { pub struct Boat(pub u32); }
#[path = "game/server_side.rs"]
pub(in crate::game) mod server_side;
pub(in crate::game) use server_side::entity;
"#,
        &[
            (
                "crates/sunrise-client/src/game/server_side.rs",
                "pub(in crate::game::server_side) use super::*;\n#[path = \"entity.rs\"]\npub(in crate::game) mod entity;\n",
            ),
            (
                "crates/sunrise-client/src/game/entity.rs",
                r#"
use super::boats::Boat;
pub(in crate::game) trait Entity { fn bounds(&self) -> u32; }
impl Entity for Boat { fn bounds(&self) -> u32 { self.0 } }
impl Entity for super::decoy::Boat { fn bounds(&self) -> u32 { self.0 + 100 } }
"#,
            ),
        ],
    );
    let (staged, index) = run_stage(root);
    let key = "sunrise_client::game::server_side::entity::<Boat as Entity>::bounds";
    assert!(staged.replace.slots.contains(key));
    assert_eq!(
        index.trait_slots.get(key),
        Some(&TraitSlotEvidence {
            owner: ResolvedItem {
                module: "sunrise_client::game::boats".into(),
                item: "Boat".into(),
            },
            trait_decl: ResolvedItem {
                module: "sunrise_client::game::server_side::entity".into(),
                item: "Entity".into(),
            },
            installer_module: "sunrise_client::game::server_side::entity".into(),
            installer: "__fnr_as_Entity_bounds".into(),
            reader: Some("__fnr_read_slot_0".into()),
            cell: Some(0),
        })
    );
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    let client_rlib = rustc_rlib(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        "sunrise_client",
        &[],
    );
    let driver = dir.path().join("driver.rs");
    fs::write(
        &driver,
        r#"
use sunrise_client::game::boats::Boat as RealBoat;
use sunrise_client::game::decoy::Boat as DecoyBoat;
use sunrise_client::game::server_side::entity::Entity;

fn patched(boat: &RealBoat) -> u32 { boat.0 + 500 }

fn main() {
    let real = RealBoat(7);
    let decoy = DecoyBoat(7);
    assert_eq!(real.bounds(), 7);
    assert_eq!(decoy.bounds(), 107);
    assert!(RealBoat::__fnr_as_Entity_bounds(patched));
    assert_eq!(real.bounds(), 507);
    assert_eq!(decoy.bounds(), 107);
}
"#,
    )
    .unwrap();
    rustc_run_bin(
        &driver,
        &out,
        &[
            "--extern",
            &format!("sunrise_client={}", client_rlib.display()),
        ],
    );
}

#[test]
#[ignore = "daily"]
fn unrelated_restricted_cfg_or_block_local_traits_never_admit_foreign_traits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let game_rs = r#"
pub struct Host;
mod side {
    pub(in crate::game::side) trait Write {}
    pub(self) trait SelfWrite {}
    #[cfg(any())]
    pub(in crate::game) trait DisabledWrite {}
    #[cfg(feature = "off")]
    pub(in crate::game) trait FeatureOffWrite {}
    pub(in crate::game::side) unsafe trait UnsafeWrite {}
    pub fn helper() {
        pub(self) trait BlockLocal {}
        trait IndentedPrivate {}
        trait BodyOnly { fn m(&self); }
        struct LocalHost;
        impl BodyOnly for LocalHost { fn m(&self) {} }
    }
    mod indented {
        trait IndentedModPrivate {}
    }
}
mod plain_foreign {
    pub struct Host;
    pub use std::fmt::Write;
    impl Write for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_self_write {
    pub struct Host;
    pub use std::fmt::Write as SelfWrite;
    impl SelfWrite for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_disabled {
    pub struct Host;
    pub use std::fmt::Write as DisabledWrite;
    impl DisabledWrite for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_feature_off {
    pub struct Host;
    pub use std::fmt::Write as FeatureOffWrite;
    impl FeatureOffWrite for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_unsafe {
    pub struct Host;
    pub use std::fmt::Write as UnsafeWrite;
    impl UnsafeWrite for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_block_local {
    pub struct Host;
    pub use std::fmt::Write as BlockLocal;
    impl BlockLocal for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_indented {
    pub struct Host;
    pub use std::fmt::Write as IndentedPrivate;
    impl IndentedPrivate for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod foreign_vs_indented_mod {
    pub struct Host;
    pub use std::fmt::Write as IndentedModPrivate;
    impl IndentedModPrivate for Host {
        fn write_str(&mut self, _: &str) -> std::fmt::Result { Ok(()) }
    }
}
mod local_unsafe_and_cfg {
    pub struct Host;
    pub unsafe trait LocalUnsafe { fn u(&self); }
    pub trait LocalCfg { fn c(&self); }
    unsafe impl LocalUnsafe for Host { fn u(&self) {} }
    #[cfg(feature = "x")]
    impl LocalCfg for Host { fn c(&self) {} }
}
"#;
    minimal_workspace(root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (staged, index) = run_stage(root);
    assert!(
        staged.replace.trait_slots.is_empty(),
        "expected no trait slots, got {:?}",
        staged.replace.trait_slots
    );
    for m in [
        "plain_foreign::<Host as Write>::write_str",
        "foreign_vs_self_write::<Host as SelfWrite>::write_str",
        "foreign_vs_disabled::<Host as DisabledWrite>::write_str",
        "foreign_vs_feature_off::<Host as FeatureOffWrite>::write_str",
        "foreign_vs_unsafe::<Host as UnsafeWrite>::write_str",
        "foreign_vs_block_local::<Host as BlockLocal>::write_str",
        "foreign_vs_indented::<Host as IndentedPrivate>::write_str",
        "foreign_vs_indented_mod::<Host as IndentedModPrivate>::write_str",
    ] {
        let key = format!("sunrise_client::game::{m}");
        assert_eq!(
            index.refused.get(&key).map(|r| r.why.as_str()),
            Some("its trait implementation has no replacement installer (foreign trait)"),
            "{key}"
        );
    }
    assert_eq!(
        index
            .refused
            .get("sunrise_client::game::local_unsafe_and_cfg::<Host as LocalUnsafe>::u")
            .map(|r| r.why.as_str()),
        Some("its trait implementation has no replacement installer (unsafe trait impl)")
    );
    assert_eq!(
        index
            .refused
            .get("sunrise_client::game::local_unsafe_and_cfg::<Host as LocalCfg>::c")
            .map(|r| r.why.as_str()),
        Some("its `impl` is compiled only under a `cfg`")
    );
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    rustc_check(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );
}

#[test]
#[ignore = "daily"]
fn duplicate_foreign_format_with_block_local_trait_stages_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let game_rs = r#"
mod display { pub use std::fmt::Display as Format; }
mod debug { pub use std::fmt::Debug as Format; }
fn helper() {
    pub(self) trait Format {}
}
pub struct Host;
impl display::Format for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("display")
    }
}
impl debug::Format for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("debug")
    }
}
"#;
    minimal_workspace(root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (staged, index) = run_stage(root);
    assert!(staged.replace.trait_slots.is_empty());
    let key = "sunrise_client::game::<Host as Format>::fmt";
    assert!(index.refused.contains_key(key));
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    rustc_check(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &["--crate-name=sunrise_client"],
    );
}

#[test]
fn non_ancestor_restricted_visibility_is_refused() {
    assert_eq!(
        refusal_of(
            &[(
                "k::game::side",
                "pub(in crate::game::other) trait Bad { fn f(&self); }"
            )],
            "k::game::other",
            "pub struct Host; impl super::side::Bad for Host { fn f(&self) {} }",
        )
        .as_deref(),
        Some("its trait implementation has no replacement installer (inaccessible trait)")
    );
}

#[test]
#[ignore = "daily"]
fn valid_visibilities_formatting_aliases_and_cross_crate_reexport_bind_and_compile() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let extra = [
        (
            "crates/sunrise-low/Cargo.toml",
            "[package]\nname = 'sunrise-low'\n",
        ),
        (
            "crates/sunrise-low/src/lib.rs",
            "pub trait CrossCrate { fn ping(&self) -> u32; }\n",
        ),
        (
            "crates/sunrise-client/Cargo.toml",
            "[package]\nname = 'sunrise-client'\n[dependencies]\nsunrise-low = { path = '../sunrise-low' }\n",
        ),
    ];
    let game_rs = r#"
pub use sunrise_low::CrossCrate;
pub(in crate::game) trait InGame { fn a(&self) -> u32; }
pub ( in crate::game ) trait SpacedVis { fn b(&self) -> u32; }
#[allow(dead_code)] trait SameLineAttr { fn c(&self) -> u32; }
pub trait
NewlineTrait { fn d(&self) -> u32; }
pub(self) trait SelfVis { fn e(&self) -> u32; }
trait PrivateTrait { fn f(&self) -> u32; }
mod inline { pub(super) trait InlineTrait { fn g(&self) -> u32; } }
mod defs { pub trait RealTrait { fn h(&self) -> u32; } }
pub struct Host;

impl InGame for Host { fn a(&self) -> u32 { 1 } }
impl SpacedVis for Host { fn b(&self) -> u32 { 2 } }
impl SameLineAttr for Host { fn c(&self) -> u32 { 3 } }
impl NewlineTrait for Host { fn d(&self) -> u32 { 4 } }
impl SelfVis for Host { fn e(&self) -> u32 { 5 } }
impl PrivateTrait for Host { fn f(&self) -> u32 { 6 } }
impl inline::InlineTrait for Host { fn g(&self) -> u32 { 7 } }
impl CrossCrate for Host { fn ping(&self) -> u32 { 8 } }

mod side {
    pub(in crate::game::side) trait SideTrait { fn s(&self) -> u32; }
    pub mod child {
        use super::super::Host;
        use super::SideTrait;
        impl SideTrait for Host { fn s(&self) -> u32 { 9 } }
    }
}

mod aliased {
    use super::Host;
    use super::defs::RealTrait as LocalAlias;
    impl LocalAlias for Host { fn h(&self) -> u32 { 10 } }
}

mod self_import_dep {
    use sunrise_low::{self, CrossCrate as _};
    pub struct Host2;
    impl sunrise_low::CrossCrate for Host2 { fn ping(&self) -> u32 { 11 } }
}
"#;
    minimal_workspace(root, "pub mod game; mod entry;\n", game_rs, &extra);
    let (staged, index) = run_stage(root);
    let expected: BTreeSet<&str> = BTreeSet::from([
        "sunrise_client::game::<Host as InGame>::a",
        "sunrise_client::game::<Host as SpacedVis>::b",
        "sunrise_client::game::<Host as SameLineAttr>::c",
        "sunrise_client::game::<Host as NewlineTrait>::d",
        "sunrise_client::game::<Host as SelfVis>::e",
        "sunrise_client::game::<Host as PrivateTrait>::f",
        "sunrise_client::game::<Host as InlineTrait>::g",
        "sunrise_client::game::<Host as CrossCrate>::ping",
        "sunrise_client::game::side::child::<Host as SideTrait>::s",
        "sunrise_client::game::aliased::<Host as LocalAlias>::h",
        "sunrise_client::game::self_import_dep::<Host2 as CrossCrate>::ping",
    ]);
    for key in &expected {
        assert!(staged.replace.slots.contains(*key), "missing slot {key}");
        assert!(index.functions.contains_key(*key), "missing index {key}");
        assert!(
            index.trait_slots.contains_key(*key),
            "missing evidence {key}"
        );
    }
    // Verify alias evidence points to the canonical trait declaration (`defs::RealTrait`).
    let alias_ev = index
        .trait_slots
        .get("sunrise_client::game::aliased::<Host as LocalAlias>::h")
        .unwrap();
    assert!(alias_ev.same_binding(&TraitSlotEvidence {
        owner: ResolvedItem {
            module: "sunrise_client::game".into(),
            item: "Host".into(),
        },
        trait_decl: ResolvedItem {
            module: "sunrise_client::game::defs".into(),
            item: "RealTrait".into(),
        },
        installer_module: "sunrise_client::game::aliased".into(),
        installer: "__fnr_as_LocalAlias_h".into(),
        reader: None,
        cell: None,
    }));
    assert_eq!(
        alias_ev.reader.as_deref(),
        alias_ev
            .cell
            .map(|c| format!("__fnr_read_slot_{c}"))
            .as_deref()
    );
    assert!(alias_ev.cell.is_some());
    // Verify cross-crate trait evidence points to `sunrise_low::CrossCrate`.
    let cross_ev = index
        .trait_slots
        .get("sunrise_client::game::<Host as CrossCrate>::ping")
        .unwrap();
    assert!(cross_ev.same_binding(&TraitSlotEvidence {
        owner: ResolvedItem {
            module: "sunrise_client::game".into(),
            item: "Host".into(),
        },
        trait_decl: ResolvedItem {
            module: "sunrise_low".into(),
            item: "CrossCrate".into(),
        },
        installer_module: "sunrise_client::game".into(),
        installer: "__fnr_as_CrossCrate_ping".into(),
        reader: None,
        cell: None,
    }));
    assert_eq!(
        cross_ev.reader.as_deref(),
        cross_ev
            .cell
            .map(|c| format!("__fnr_read_slot_{c}"))
            .as_deref()
    );
    assert!(cross_ev.cell.is_some());

    // Compile the staged crates with rustc to prove every emitted installer is valid Rust.
    let out = dir.path().join("rustc-out");
    fs::create_dir_all(&out).unwrap();
    let low_rlib = rustc_rlib(
        &root.join("crates/sunrise-low/src/lib.rs"),
        &out,
        "sunrise_low",
        &[],
    );
    rustc_check(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        &[
            "--crate-name=sunrise_client",
            "--extern",
            &format!("sunrise_low={}", low_rlib.display()),
        ],
    );
}

#[test]
#[ignore = "daily"]
fn per_slot_reader_and_cell_identity_matches_staged_methods_and_installers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("paired");
    let game_rs = r#"
pub mod real {
    pub struct Boat;
    pub trait Entity { fn f(&self) -> u32; fn g(&self) -> u32; }
}
pub mod decoy {
    pub struct Boat;
    pub trait Entity { fn f(&self) -> u32; fn g(&self) -> u32; }
}
pub mod subject {
    use super::real::{Boat, Entity};
    impl Entity for Boat {
        fn f(&self) -> u32 { 1 }
        fn g(&self) -> u32 { 2 }
    }
}
"#;
    minimal_workspace(&root, "pub mod game; mod entry;\n", game_rs, &[]);
    let (_, index) = run_stage(&root);
    let kf = "sunrise_client::game::subject::<Boat as Entity>::f";
    let kg = "sunrise_client::game::subject::<Boat as Entity>::g";
    let ev_f = &index.trait_slots[kf];
    let ev_g = &index.trait_slots[kg];
    assert_eq!(index.functions[kf].trait_binding.as_ref(), Some(ev_f));
    assert_eq!(index.functions[kg].trait_binding.as_ref(), Some(ev_g));
    assert_eq!(ev_f.cell, Some(0));
    assert_eq!(ev_f.reader.as_deref(), Some("__fnr_read_slot_0"));
    assert_eq!(ev_f.installer, "__fnr_as_Entity_f");
    assert_eq!(ev_g.cell, Some(1));
    assert_eq!(ev_g.reader.as_deref(), Some("__fnr_read_slot_1"));
    assert_eq!(ev_g.installer, "__fnr_as_Entity_g");

    // Verify per-method body reader call and installer cell in the parsed staged file.
    let staged_src = fs::read_to_string(root.join("crates/sunrise-client/src/game.rs")).unwrap();
    let parsed = syn::parse_file(&staged_src).unwrap();
    let subject_items = parsed
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Mod(m) if m.ident == "subject" => {
                m.content.as_ref().map(|(_, items)| items.as_slice())
            }
            _ => None,
        })
        .expect("subject module");
    let mut method_bodies = std::collections::BTreeMap::new();
    let mut generated_fns = std::collections::BTreeMap::new();
    for item in subject_items {
        if let syn::Item::Impl(block) = item {
            for member in &block.items {
                if let syn::ImplItem::Fn(method) = member {
                    let body = quote::ToTokens::to_token_stream(&method.block).to_string();
                    if block.trait_.is_some() {
                        method_bodies.insert(method.sig.ident.to_string(), body);
                    } else {
                        generated_fns.insert(method.sig.ident.to_string(), body);
                    }
                }
            }
        }
    }
    for (method, ev) in [("f", ev_f), ("g", ev_g)] {
        let reader = ev.reader.as_deref().unwrap();
        let cell = ev.cell.unwrap();
        let body = &method_bodies[method];
        assert!(
            body.contains(&format!("Self :: {reader} ()")),
            "{method} body: {body}"
        );
        let reader_body = &generated_fns[reader];
        let installer_body = &generated_fns[ev.installer.as_str()];
        let cell_tok = format!("__REPLACE [{cell}usize]");
        assert!(reader_body.contains(&cell_tok), "{reader}: {reader_body}");
        assert!(
            installer_body.contains(&cell_tok),
            "{}: {installer_body}",
            ev.installer
        );
    }

    // Compiled dispatch confirms installing `f` affects only `f`, never `g`.
    let out = dir.path().join("out");
    fs::create_dir_all(&out).unwrap();
    let rlib = rustc_rlib(
        &root.join("crates/sunrise-client/src/lib.rs"),
        &out,
        "sunrise_client",
        &[],
    );
    let driver = root.join("driver.rs");
    fs::write(
        &driver,
        "use sunrise_client::game::real::{Boat, Entity}; fn patched(_: &Boat) -> u32 { 11 } fn main() { assert_eq!([Boat.f(), Boat.g()], [1, 2]); assert!(Boat::__fnr_as_Entity_f(patched)); assert_eq!([Boat.f(), Boat.g()], [11, 2]); }",
    )
    .unwrap();
    rustc_run_bin(
        &driver,
        &out,
        &["--extern", &format!("sunrise_client={}", rlib.display())],
    );
}
