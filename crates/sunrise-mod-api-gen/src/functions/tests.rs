use super::*;
use std::fs;

const GAME: &str = r#"use std::fmt;
pub struct Game<'a> {
    ticks: &'a u32,
}
pub struct Generic<T>(T);
impl<T> Generic<T> {
    pub fn generic_owner(&self) {}
}
pub trait Tick {
    fn tick(&self) -> u32;
}
impl Game<'_> {
    /// Kept.
    pub fn ticks(&self) -> u32 {
        *self.ticks
    }
    pub fn generic<T>(&self, t: T) -> T {
        t
    }
}
impl Tick for Game<'_> {
    fn tick(&self) -> u32 {
        1
    }
}
impl fmt::Display for Game<'_> {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}
#[cfg(unix)]
pub fn twice() {}
#[cfg(not(unix))]
pub fn twice() {}
#[cfg_attr(true, cfg_attr(not(false,), cfg(test,),),)]
mod inner {
    pub fn nested() -> u8 { 9 }
}
#[cfg(test,)]
mod tests {
    fn hidden() {}
}
mod inner {
    pub fn nested() -> u8 {
        2
    }
}
"#;

/// A game crate and a crate below it with one marked function.
fn fixture() -> tempfile::TempDir {
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
    write("crates/sunrise-client/src/lib.rs", "pub mod game;\n");
    write("crates/sunrise-client/src/game.rs", GAME);
    write(
        "crates/sunrise-low/Cargo.toml",
        "[package]\nname = 'sunrise-low'\n[lib]\nname = 'low'\n",
    );
    write(
        "crates/sunrise-low/src/lib.rs",
        "pub struct Clock;\nimpl Clock {\n    #[sunrise_mod_api_macros::replaceable]\n    pub fn advance(&self) {}\n}\npub fn unmarked() {}\n",
    );
    root
}

#[test]
fn method_digests_separate_impl_context_from_the_block() {
    let root = fixture();
    let file = root.path().join("crates/sunrise-client/src/game.rs");
    let indexed = |source| {
        fs::write(&file, source).unwrap();
        index(root.path(), &BTreeSet::new())
            .unwrap()
            .functions
            .remove("sunrise_client::game::Game::ticks")
            .unwrap()
    };
    let original = indexed(GAME.to_owned());
    let formatting = indexed(GAME.replace("*self.ticks", "/* a comment */ * self . ticks"));
    assert_ne!(formatting.sha256, original.sha256);
    assert_eq!(formatting.digests, original.digests);

    let unrelated = indexed(GAME.replace("        t\n", "        panic!()\n"));
    assert_eq!(unrelated.digests, original.digests);

    let new_context =
        indexed(GAME.replace("impl Game<'_> {", "#[allow(dead_code)] impl Game<'_> {"));
    assert_eq!(new_context.sha256, original.sha256);
    let original = original.digests.unwrap();
    let new_context = new_context.digests.unwrap();
    assert_ne!(new_context.declaration, original.declaration);
    assert_eq!(new_context.body, original.body);
}

#[test]
fn the_index_lists_each_function_with_its_text_or_its_refusal() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let listed: Vec<&str> = index.functions.keys().map(String::as_str).collect();
    assert_eq!(
        listed,
        [
            "sunrise_client::game::<Game as Tick>::tick",
            "sunrise_client::game::Game::ticks",
            "sunrise_client::game::inner::nested",
            "sunrise_low::Clock::advance",
        ]
    );
    let ticks = &index.functions["sunrise_client::game::Game::ticks"];
    assert_eq!(ticks.file, "crates/sunrise-client/src/game.rs");
    let [start, end] = ticks.span;
    assert_eq!(
        &GAME[start..end],
        "/// Kept.\n    pub fn ticks(&self) -> u32 {\n        *self.ticks\n    }"
    );
    assert_eq!(ticks.sha256, sha256(&GAME.as_bytes()[start..end]));
    let refused = |key: &str| index.refused[key].clone();
    assert_eq!(refused("sunrise_client::game::twice").code, Code::Ambiguous);
    let generic = refused("sunrise_client::game::Game::generic");
    assert_eq!(generic.code, Code::NotReplaceable);
    assert!(
        generic.why.contains("it is not replaceable"),
        "{}",
        generic.why
    );
    let generic_owner = refused("sunrise_client::game::Generic::generic_owner");
    assert_eq!(generic_owner.code, Code::NotReplaceable);
    assert!(generic_owner.why.contains("it is generic over types"));
    // The stager's own trait rule: a trait of another crate has no installer.
    let display = refused("sunrise_client::game::<Game as Display>::fmt");
    assert_eq!(
        display.why,
        "its trait implementation has no replacement installer (foreign trait)"
    );
    // Test modules and unmarked functions below the game crate are not listed at all.
    assert!(
        !index
            .functions
            .contains_key("sunrise_client::game::tests::hidden")
    );
    assert!(
        !index
            .refused
            .contains_key("sunrise_client::game::tests::hidden")
    );
    assert!(!index.functions.contains_key("sunrise_low::unmarked"));
    assert_eq!(index.crates["sunrise_low"].library, "low");
    let game = &index.crates["sunrise_client"].modules["sunrise_client::game"];
    assert_eq!(
        game,
        &[ModuleFile {
            path: "crates/sunrise-client/src/game.rs".into(),
            cfg: Vec::new(),
            sha256: sha256(GAME.as_bytes()),
            include_cfg: None,
        }]
    );
    assert_eq!(ticks.module, "sunrise_client::game");
    assert_eq!(
        index.functions["sunrise_client::game::inner::nested"].module,
        "sunrise_client::game"
    );
}

#[test]
fn restricted_reexports_in_another_module_refuse_a_target_by_name() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod inner {
    mod deep { pub fn f() {} }
    pub(crate) use deep::f;
}
mod public {
    mod deep { pub fn f() {} }
    pub use deep::f;
}
use public as local;
fn private_reexport() { inner::f(); }
fn public_reexport() { public::f(); }
fn local_import() { local::f(); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    let (code, why) = check("private_reexport").unwrap_err();
    assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
    assert!(
        why.contains("`inner::f`") && why.contains("`sunrise_client::game::inner`"),
        "{why}"
    );
    check("public_reexport").unwrap();
    check("local_import").unwrap();
}

#[test]
fn qualified_paths_follow_module_aliases_and_globs() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod inner {
    mod deep { pub fn f() {} pub fn private_glob() {} }
    pub(super) use self::deep::{f as alias, *};
}
mod public {
    mod deep { pub fn f() {} }
    pub use deep::*;
}
use self::inner as local;
mod child {
    use super::*;
    fn private_alias() { crate::game::inner::alias(); }
    fn private_glob() { super::local::private_glob(); }
    fn public_glob() { self::public::f(); }
}
fn block_import() { use crate::game::inner::alias; alias(); }
fn shadowed() { use public as inner; inner::f(); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::child::{name}"));
    for (name, path) in [
        ("private_alias", "crate::game::inner::alias"),
        ("private_glob", "super::local::private_glob"),
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains(path), "{why}");
    }
    check("public_glob").unwrap();
    let (code, why) = index
        .check(root.path(), "sunrise_client::game::block_import")
        .unwrap_err();
    assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
    assert!(why.contains("crate::game::inner::alias"), "{why}");
    index
        .check(root.path(), "sunrise_client::game::shadowed")
        .unwrap();
}

#[test]
fn only_unconditional_body_paths_can_prove_a_restricted_crossing() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod hooks {
    mod imp {
        pub fn f() {}
        macro_rules! m { () => { () }; }
        pub(crate) use m;
    }
    pub(super) use imp::{f, m};
}
fn active() { hooks::f(); }
fn gated_macro() {
    #[cfg(test)]
    hooks::m!();
}
fn gated_block() {
    #[cfg(test)]
    { hooks::f(); }
}
fn gated_local() {
    #[cfg(unix)]
    let _value = hooks::f();
}
fn gated_arm() {
    match 0 {
        #[cfg(feature = "optional")]
        0 => hooks::f(),
        _ => (),
    }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    assert_eq!(check("active").unwrap_err().0, "E_MIXIN_NOT_REPLACEABLE");
    for name in ["gated_block", "gated_local", "gated_arm", "gated_macro"] {
        check(name).unwrap();
    }
}

#[test]
fn a_cfg_selected_public_provider_stays_replaceable() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
#[cfg(any())]
mod provider {
    mod imp { pub fn helper() {} }
    pub(super) use imp::helper;
}
#[cfg(all())]
mod provider { pub fn helper() {} }
fn target() { provider::helper(); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    index
        .check(root.path(), "sunrise_client::game::target")
        .unwrap();
}

#[test]
fn restricted_imports_do_not_shadow_public_globs_in_another_namespace() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod types { pub type helper = (); }
mod values { pub fn helper() {} }
mod private_type {
    pub(crate) use super::types::helper;
    pub use super::values::*;
}
mod private_value {
    pub use super::types::*;
    pub(crate) use super::values::helper;
}
fn calls_public_value() { private_type::helper(); }
fn uses_public_type() { let _: private_value::helper = (); }
fn calls_private_value() { private_value::helper(); }
fn uses_private_type() { let _: private_type::helper = (); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    check("calls_public_value").unwrap();
    check("uses_public_type").unwrap();
    for (target, path) in [
        ("calls_private_value", "private_value::helper"),
        ("uses_private_type", "private_type::helper"),
    ] {
        let (code, why) = check(target).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains(path), "{why}");
    }
}

#[test]
fn a_cfg_gated_value_does_not_make_a_restricted_type_import_shadow_a_public_value() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod defs {
    pub type helper = ();
    #[cfg(any())]
    pub fn helper() {}
}
mod values { pub fn helper() {} }
mod provider {
    pub(crate) use super::defs::helper;
    pub use super::values::*;
}
fn calls_public_value() { provider::helper(); }
fn uses_private_type() { let _: provider::helper = (); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    check("calls_public_value").unwrap();
    let (code, why) = check("uses_private_type").unwrap_err();
    assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
    assert!(why.contains("provider::helper"), "{why}");
}

#[test]
fn a_bare_reference_follows_the_mirrors_import_to_its_provider() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod native_hooks {
    mod imp { pub fn queue() {} }
    pub(super) use imp::queue as restricted;
}
mod native_level_effects {
    use super::native_hooks::restricted as queue;
    fn by_alias() { queue(); }
}
mod glob_user {
    use super::native_hooks::*;
    fn by_glob() { restricted(); }
    fn shadowed_by_parameter(restricted: fn()) { restricted(); }
    fn shadowed_by_local() { let restricted = || {}; restricted(); }
    fn shadowed_by_item() { fn restricted() {} restricted(); }
}
mod opaque_glob_user {
    use super::native_hooks::*;
    use ::opaque::*;
    fn not_proven() { restricted(); }
}
pub mod public_hooks { pub fn queue() {} }
use self::public_hooks::queue as public_alias;
fn through_public_alias() { public_alias(); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    for (name, binding) in [
        ("native_level_effects::by_alias", "queue"),
        ("glob_user::by_glob", "restricted"),
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(
            why.contains(&format!("`{binding}`"))
                && why.contains("`sunrise_client::game::native_hooks`"),
            "{why}"
        );
    }
    for name in [
        "through_public_alias",
        "glob_user::shadowed_by_parameter",
        "glob_user::shadowed_by_local",
        "glob_user::shadowed_by_item",
        "opaque_glob_user::not_proven",
    ] {
        check(name).unwrap();
    }
}

#[test]
fn bare_type_paths_are_checked_in_signatures_and_bodies() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod types {
    mod imp { pub type Tag = (); }
    pub(super) use imp::Tag;
    pub type PublicTag = ();
}
mod copy {
    use super::types::*;
    fn signature(_: Option<Tag>) {}
    fn body() {
        let Tag = ();
        let _: Option<Tag> = None;
    }
    fn public(_: Option<PublicTag>) {
        let _: Option<PublicTag> = None;
    }
    fn local_type() {
        type Tag = ();
        let _: Option<Tag> = None;
    }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::copy::{name}"));
    for name in ["signature", "body"] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains("`Tag`"), "{why}");
    }
    check("public").unwrap();
    check("local_type").unwrap();
}

#[test]
fn qualified_projections_check_the_trait_prefix() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod types {
    mod imp {
        pub trait Tag { type Value; const VALUE: (); }
        impl Tag for () { type Value = (); const VALUE: () = (); }
    }
    pub(super) use imp::Tag;
    pub trait PublicTag { type Value; const VALUE: (); }
    impl PublicTag for () { type Value = (); const VALUE: () = (); }
}
mod copy {
    use super::types::*;
    fn restricted_type(_: <() as Tag>::Value) {}
    fn restricted_value() { let Tag = (); let _ = <() as Tag>::VALUE; }
    fn public(_: <() as PublicTag>::Value) { let _ = <() as PublicTag>::VALUE; }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::copy::{name}"));
    for name in ["restricted_type", "restricted_value"] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains("`Tag`"), "{why}");
    }
    check("public").unwrap();
}

#[test]
fn local_value_bindings_mask_imports_only_where_in_scope() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod hooks {
    mod imp { pub fn queue() {} }
    pub(super) use imp::queue;
    pub fn public() {}
}
mod copy {
    use super::hooks::*;
    fn before_binding() {
        let queue = { queue(); || {} };
        queue();
    }
    fn outside_block() {
        { let queue = || {}; queue(); }
        queue();
    }
    fn in_let_else() {
        let Some(queue) = None::<fn()> else { queue(); return; };
        queue();
    }
    fn outside_if_let() {
        if let Some(queue) = Some(|| {}) { queue(); } else { queue(); }
    }
    fn outside_match_arm() {
        match None::<fn()> {
            Some(queue) if { queue(); true } => queue(),
            _ => queue(),
        }
    }
    fn after_nested_item() {
        fn nested(queue: fn()) { queue(); }
        queue();
    }
    fn in_for_iterator() {
        for queue in { queue(); [|| {}] } { queue(); }
    }
    fn public_and_shadowed() {
        public();
        { let queue = || {}; queue(); }
        let closure = |queue: fn()| queue();
        closure(public);
        for queue in [public] { queue(); }
        if let Some(queue) = Some(|| true) && queue() { queue(); }
        while let Some(queue) = Some(|| {}) { queue(); break; }
        match Some(|| true) {
            Some(queue) if queue() => { queue(); }
            _ => (),
        }
    }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::copy::{name}"));
    for name in [
        "before_binding",
        "outside_block",
        "in_let_else",
        "outside_if_let",
        "outside_match_arm",
        "after_nested_item",
        "in_for_iterator",
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains("`queue`"), "{why}");
    }
    check("public_and_shadowed").unwrap();
}

#[test]
fn cfg_gates_on_nested_parameters_and_trait_items_do_not_refuse_the_copy() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/lib.rs"),
        "pub mod game; fn hidden() {}",
    )
    .unwrap();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod types {
    mod imp { pub type Tag = (); }
    pub(super) use imp::Tag;
}
mod copy {
    use super::types::*;
    fn disabled_parameter() {
        fn helper(#[cfg(any())] _: Option<Tag>) {}
        helper();
    }
    fn active_parameter() {
        fn helper(_: Option<Tag>) {}
        helper(None);
    }
    fn disabled_trait_item() {
        trait Local { #[cfg_attr(all(), cfg(any()))] fn helper() { crate::hidden(); } }
    }
    fn active_trait_item() {
        trait Local { fn helper() { crate::hidden(); } }
    }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::copy::{name}"));
    check("disabled_parameter").unwrap();
    check("disabled_trait_item").unwrap();
    for (name, path) in [
        ("active_parameter", "Tag"),
        ("active_trait_item", "crate::hidden"),
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains(&format!("`{path}`")), "{why}");
    }
}

#[test]
fn a_private_crate_root_call_refuses_the_target() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/lib.rs"),
        "pub mod game; fn hidden() {} pub(crate) fn restricted() {} pub fn exposed() {}",
    )
    .unwrap();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
fn private_call() { crate::hidden(); }
fn restricted_call() { super::restricted(); }
fn public_call() { crate::exposed(); }
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    for (name, path) in [
        ("private_call", "crate::hidden"),
        ("restricted_call", "super::restricted"),
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains(path), "{why}");
    }
    check("public_call").unwrap();
}

#[test]
fn cfg_gated_fields_and_variants_in_inline_modules_do_not_refuse_the_copy() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/lib.rs"),
        "pub mod game; const fn hidden() -> usize { 1 }",
    )
    .unwrap();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
fn gated_field() {
    mod local { pub struct S { #[cfg(any())] pub f: [u8; crate::hidden()] } }
}
fn gated_variant() {
    mod local { pub enum E { #[cfg(any())] A = crate::hidden() as isize } }
}
fn active_field() {
    mod local { pub struct S { pub f: [u8; crate::hidden()] } }
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    check("gated_field").unwrap();
    check("gated_variant").unwrap();
    let (code, why) = check("active_field").unwrap_err();
    assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
    assert!(why.contains("crate::hidden"), "{why}");
}

#[test]
fn inline_modules_check_crate_paths_without_using_the_enclosing_scope() {
    let root = fixture();
    fs::write(
        root.path().join("crates/sunrise-client/src/lib.rs"),
        r#"
pub mod game;
fn hidden() {}
pub fn exposed() {}
fn recursive(depth: u8) { if depth != 0 { crate::recursive(depth - 1); } }
"#,
    )
    .unwrap();
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        r#"
mod hooks {
    mod imp { pub fn exposed() {} }
    pub(super) use imp::exposed;
}
use self::hooks::*;
fn hidden_in_inline_module() {
    mod local { pub fn call() { crate::hidden(); } }
    local::call();
}
fn public_in_inline_module() {
    mod local {
        use crate::exposed;
        pub fn call() { exposed(); crate::exposed(); }
    }
    local::call();
}
fn restricted_after_inline_module() {
    mod local { pub fn call() { crate::exposed(); } }
    local::call();
    exposed();
}
"#,
    )
    .unwrap();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |name| index.check(root.path(), &format!("sunrise_client::game::{name}"));
    for (name, path) in [
        ("hidden_in_inline_module", "crate::hidden"),
        ("restricted_after_inline_module", "exposed"),
    ] {
        let (code, why) = check(name).unwrap_err();
        assert_eq!(code, "E_MIXIN_NOT_REPLACEABLE");
        assert!(why.contains(&format!("`{path}`")), "{why}");
    }
    check("public_in_inline_module").unwrap();
    index
        .check(root.path(), "sunrise_client::recursive")
        .unwrap();
}

#[test]
fn a_target_is_checked_by_its_key_and_its_text() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let check = |target: &str| index.check(root.path(), target);
    check("sunrise_client::game::Game<'_>::ticks").unwrap();
    check("sunrise_client::game::<Game<'_> as crate::game::Tick>::tick").unwrap();
    check("sunrise_low::Clock::advance").unwrap();
    assert_eq!(
        check("sunrise_client::game::Game<'_>::no_such").unwrap_err(),
        (
            "E_MIXIN_TARGET_MISSING",
            "no function `sunrise_client::game::Game::no_such` in this game build's function index (functions.json)".to_owned()
        )
    );
    // A crate the source does not have is not a crate to mark.
    assert_eq!(
        check("sunrise_nowhere::f").unwrap_err(),
        (
            "E_MIXIN_TARGET_MISSING",
            "no crate `sunrise_nowhere` in the game's source".to_owned()
        )
    );
    assert_eq!(
        check("sunrise_low::unmarked").unwrap_err(),
        (
            "E_MIXIN_NOT_REPLACEABLE",
            crate::replace::UNMARKED.to_owned()
        )
    );
    assert_eq!(
        check("sunrise_client::game::twice").unwrap_err().0,
        "E_MIXIN_TARGET_AMBIGUOUS"
    );
    // A source that is not the build's: refused by name, not searched.
    let game = root.path().join("crates/sunrise-client/src/game.rs");
    fs::write(&game, GAME.replace("*self.ticks", "*self.ticks + 1")).unwrap();
    let (code, why) = check("sunrise_client::game::Game::ticks").unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(
        why.contains("differ from this game build's functions.json"),
        "{why}"
    );
    fs::write(&game, "").unwrap();
    let (code, why) = check("sunrise_client::game::Game::ticks").unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(why.contains("has no bytes"), "{why}");
    // An index naming a file outside the source is refused before anything is read.
    let mut escaping = index.clone();
    escaping
        .functions
        .get_mut("sunrise_client::game::inner::nested")
        .unwrap()
        .file = "../outside.rs".into();
    let (code, why) = escaping
        .check(root.path(), "sunrise_client::game::inner::nested")
        .unwrap_err();
    assert_eq!(code, "E_MIXIN_SOURCE");
    assert!(why.contains("not a path inside the game's source"), "{why}");
}

#[test]
fn the_copy_reads_the_index_and_refuses_a_stale_source() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let targets = ["sunrise_client::game::Game<'_>::ticks".to_owned()];
    let generated = crate::patch::generate(
        root.path(),
        "sunrise_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap();
    assert_eq!(
        generated.patched[0].id,
        "sunrise_patch::game::<Game<'_> as __Fnr_Game_ticks>::ticks"
    );
    assert!(generated.libs[0].1.contains("*self.ticks"));
    fs::write(
        root.path().join("crates/sunrise-client/src/game.rs"),
        GAME.replace("*self.ticks", "0"),
    )
    .unwrap();
    let refused = crate::patch::generate(
        root.path(),
        "sunrise_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap_err();
    assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
}

#[test]
fn every_file_the_copy_reads_is_checked_not_only_the_span() {
    let root = fixture();
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let targets = ["sunrise_client::game::Game::ticks".to_owned()];
    // The target's span is unchanged; the crate root the mirror reads is not the build's.
    let lib = root.path().join("crates/sunrise-client/src/lib.rs");
    fs::write(&lib, "pub mod game;\npub fn added() {}\n").unwrap();
    let refused = crate::patch::generate(
        root.path(),
        "sunrise_patch",
        &targets,
        &Default::default(),
        &index,
    )
    .unwrap_err();
    assert_eq!(refused[0].code, "E_MIXIN_SOURCE");
    assert!(
        refused[0]
            .why
            .contains("lib.rs differs from this game build's functions.json"),
        "{}",
        refused[0].why
    );
}

#[test]
fn a_function_staging_gave_no_slot_is_refused_unless_a_loop_runs_it() {
    let root = fixture();
    let mut index = index(root.path(), &BTreeSet::new()).unwrap();
    let slots = BTreeSet::from(["sunrise_client::game::Game::ticks".to_owned()]);
    let generic = BTreeSet::from(["sunrise_client::game::inner::nested".to_owned()]);
    index.retain_slotted(&slots, &generic);
    let listed: Vec<&str> = index.functions.keys().map(String::as_str).collect();
    assert_eq!(
        listed,
        [
            "sunrise_client::game::Game::ticks",
            "sunrise_client::game::inner::nested"
        ]
    );
    assert_eq!(
        index.refused["sunrise_low::Clock::advance"].why,
        "the game build gives it no replacement slot"
    );
}

#[test]
fn a_target_key_drops_generics_spaces_and_the_traits_path() {
    assert_eq!(
        key(
            "sunrise_client::game::level_access::<GameLevel<'_> as crate::LevelAccess>::block_state"
        ),
        "sunrise_client::game::level_access::<GameLevel as LevelAccess>::block_state"
    );
    assert_eq!(
        key("sunrise_client::game::server_side::autosave_server::Game<'_>::next_autosave_interval"),
        "sunrise_client::game::server_side::autosave_server::Game::next_autosave_interval"
    );
}

#[test]
fn refusal_counts_group_templated_and_exact_reasons() {
    let mut index = FunctionIndex::default();
    let add = |index: &mut FunctionIndex, key: &str, why: &str| {
        index.refused.insert(
            key.to_owned(),
            Refused {
                code: Code::NotReplaceable,
                why: why.to_owned(),
            },
        );
    };
    add(
        &mut index,
        "a",
        "its trait implementation has no replacement installer (foreign trait)",
    );
    add(
        &mut index,
        "b",
        "its trait implementation has no replacement installer (foreign trait)",
    );
    add(
        &mut index,
        "c",
        "its trait implementation has no replacement installer: owner type name `Boat` is ambiguous in the crate: more than one module declares it, and the `impl`'s module neither declares it nor imports it from one of them",
    );
    add(
        &mut index,
        "d",
        "its body uses `with_entity!`, a macro defined outside its module's path",
    );
    add(
        &mut index,
        "e",
        "it names `super::persist::SpawnTree`, which crosses a non-public `use` in `sunrise_client::game::server_side::persist`",
    );
    add(
        &mut index,
        "f",
        "it names `crate::hidden`, which reaches the non-public crate-root function `hidden`",
    );
    assert_eq!(
        index.refusal_counts(),
        BTreeMap::from([
            ("it names a non-public crate-root function".to_owned(), 1),
            (
                "it names a path that crosses a non-public use".to_owned(),
                1
            ),
            (
                "its body uses a macro defined outside its module's path".to_owned(),
                1
            ),
            (
                "its trait implementation has no replacement installer (ambiguous trait owner)"
                    .to_owned(),
                1
            ),
            (
                "its trait implementation has no replacement installer (foreign trait)".to_owned(),
                2
            ),
        ])
    );
}

#[test]
fn a_method_below_the_game_crate_is_named_by_its_types_module() {
    let root = fixture();
    let low = root.path().join("crates/sunrise-low/src");
    fs::write(low.join("lib.rs"), "pub mod clock;\npub struct Clock;\n").unwrap();
    fs::write(
        low.join("clock.rs"),
        "use super::Clock;\nimpl Clock {\n    #[sunrise_mod_api_macros::replaceable]\n    pub fn advance(&self) {}\n}\n",
    )
    .unwrap();
    let named = "sunrise_low::Clock::advance";
    let by_file = "sunrise_low::clock::Clock::advance";
    let marked = crate::replace::marked(root.path(), crate::source::LEGACY_LAYOUT).unwrap();
    assert_eq!(
        marked
            .iter()
            .map(|(_, path)| path.as_str())
            .collect::<Vec<_>>(),
        [named]
    );
    let index = index(root.path(), &BTreeSet::new()).unwrap();
    let advance = &index.functions[named];
    assert_eq!(advance.module, "sunrise_low::clock");
    assert_eq!(advance.file, "crates/sunrise-low/src/clock.rs");
    assert_eq!(index.check(root.path(), named), Ok(()));
    let found = index
        .found(
            &mut Source::new(root.path(), crate::source::LEGACY_LAYOUT).unwrap(),
            named,
        )
        .unwrap();
    assert_eq!(found.module, "sunrise_low::clock");
    // The path of the file the `impl` is in is refused, naming the target.
    let (code, why) = index.check(root.path(), by_file).unwrap_err();
    assert_eq!(code, "E_MIXIN_TARGET_MISSING");
    assert!(why.contains(&format!("`{named}`")), "{why}");
    // A bridge looks it up by name the same way.
    let bridged = crate::patch::find(
        &mut Source::new(root.path(), crate::source::LEGACY_LAYOUT).unwrap(),
        named,
    )
    .unwrap();
    assert_eq!(bridged.module, "sunrise_low::clock");
    let (code, why) = crate::patch::find(
        &mut Source::new(root.path(), crate::source::LEGACY_LAYOUT).unwrap(),
        by_file,
    )
    .err()
    .expect("refused");
    assert_eq!(code, "E_MIXIN_TARGET_MISSING");
    assert!(why.contains(&format!("`{named}`")), "{why}");
    let copy = crate::patch::generate_for_main_sdk(
        root.path(),
        "fixture",
        &[named.into()],
        &Default::default(),
        &index,
    )
    .unwrap();
    assert_eq!(copy.patched.len(), 1);
    assert_eq!(copy.patched[0].target, named);
    // The staging pass gives it its slot under the same name.
    fs::write(
        root.path().join("Cargo.lock"),
        "version = 4

[[package]]
name = \"sunrise-client\"
version = \"0.1.0\"

[[package]]
name = \"sunrise-low\"
version = \"0.1.0\"
",
    )
    .unwrap();
    let staged = crate::replace::stage(root.path()).unwrap();
    assert!(staged.slots.contains(named), "{:?}", staged.slots);
}
