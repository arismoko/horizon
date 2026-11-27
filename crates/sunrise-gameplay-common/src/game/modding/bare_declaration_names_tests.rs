//! #3120: a bare name is the declaring mod's own, so a block entity type built with
//! `.blocks(&["press"])` holds the block `press` the same mod declares.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use mod_api::Side;
use mod_api::{__private::game::RegistrationContextNew, BlocksEvent, Hooks, Registrar};
use sunrise_mod_package::enabled::EnabledMods;
use sunrise_sim::mod_content::{
    BLOCK_ENTITY_TYPES, DeclaredContent, accept_declared, resource_root,
};

use crate::game::api_values::TryIntoEngine;

struct Press;
#[mod_api::__subscriptions]
impl Hooks for Press {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.block_entity_type("press").blocks(&["press"]).build();
    }
    fn blocks(&self, _: &mod_api::RegistrationContext, event: &mut BlocksEvent<'_>) {
        event
            .register(mod_api::Block::new("press", "minecraft:stone"))
            .unwrap();
    }
}

/// The least of a prepared resource folder the content overlay reads.
fn base(root: &Path) {
    fs::create_dir_all(root.join("assets/minecraft/blockstates")).unwrap();
    for (file, text) in [
        (
            "facts.json",
            r#"{"states":[["minecraft:stone"]],"defaults":{"minecraft:stone":0}}"#,
        ),
        (
            "gameplay-facts.json",
            r#"{"states":[{"item":"minecraft:stone"}]}"#,
        ),
        (
            "registries.json",
            r#"{"minecraft:item":{"entries":{}},"minecraft:block":{"entries":{}},
                "minecraft:block_entity_type":{"entries":{"minecraft:sign":{"protocol_id":8}}}}"#,
        ),
        ("items.json", "{}"),
        ("assets/minecraft/blockstates/stone.json", "{}"),
        ("sunrise-resources.json", "{}"),
    ] {
        fs::write(root.join(file), text).unwrap();
    }
}

#[test]
fn a_bare_block_name_binds_the_type_to_the_mods_own_block() {
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("press_mod", Box::new(Press))].into();
    let (blocks, types) = mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        let _registration = mod_platform::__private::RegistrationGuard::open();
        let context = mod_api::RegistrationContext::new();
        let mut blocks = Vec::new();
        crate::game::native_hooks::native_blocks(&context, &mut |listen| {
            assert!(listen(&mut |block| {
                blocks.push(
                    block
                        .answer
                        .try_into_engine(&())
                        .unwrap()
                        .validate_local()
                        .unwrap(),
                );
                Ok(())
            }));
        });
        assert!(mod_api::__private::take_declaration_refusals(&context).is_empty());
        (
            blocks,
            crate::game::api_values::block_entity_declarations()
                .map(|(_, kind)| kind)
                .collect(),
        )
    });
    let mut content = Vec::new();
    let batches = BTreeMap::from([(
        "press_mod",
        DeclaredContent {
            blocks,
            types,
            ..Default::default()
        },
    )]);
    let mut errors = Vec::new();
    accept_declared(&mut content, &batches, &[], &BTreeSet::new(), &mut errors).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    let scratch = std::env::temp_dir().join(format!(
        "sunrise-bare-declaration-names-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    let base_root = scratch.join("base");
    base(&base_root);
    let inventory = EnabledMods::new([("press_mod".to_owned(), Side::Both)]);
    let root = resource_root(
        &base_root,
        Path::new(""),
        &content,
        &scratch.join("cache"),
        &inventory,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(root.join(BLOCK_ENTITY_TYPES)).unwrap(),
        r#"{"press_mod:press":"press_mod:press"}"#
    );
    fs::remove_dir_all(scratch).unwrap();
}
