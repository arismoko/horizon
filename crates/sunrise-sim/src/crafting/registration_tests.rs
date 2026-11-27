//! Serializer/type separation and fatal resolution after conditions.
use super::*;
#[test]
fn unregistered_serializer_refuses_whole_load_after_false_conditions() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("data/minecraft/recipe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("valid.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":3}}"#).unwrap();
    std::fs::write(folder.join("known_bad.json"), r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":100}}"#).unwrap();
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer"}"#,
    )
    .unwrap();
    let items = Items::for_tests(Default::default());
    let load = crate::test_resources::server_load();
    let error = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load)
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        "2 recipes failed to load:\n  recipes: data/minecraft/recipe/known_bad.json refused: invalid result: {\"count\":100,\"id\":\"minecraft:stick\"}\n  recipe data/minecraft/recipe/unknown.json, unregistered recipe serializer absent:serializer"
    );
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer","neoforge:conditions":[{"type":"neoforge:not","value":{"type":"neoforge:always"}}]}"#,
    )
    .unwrap();
    // With the serializer skipped by its false condition, the malformed sibling still
    // refuses the whole load, naming its file.
    let error = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("known_bad.json") && error.contains("invalid result"),
        "{error}"
    );
    std::fs::remove_file(folder.join("known_bad.json")).unwrap();
    let recipes = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load).unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].id,
        "minecraft:valid"
    );
    assert_eq!(
        recipes.by_type["minecraft:crafting"][0].json["type"],
        "minecraft:crafting_shapeless"
    );
    std::fs::write(
        folder.join("unknown.json"),
        r#"{"type":"absent:serializer","neoforge:conditions":[{"type":"absent:condition"}]}"#,
    )
    .unwrap();
    let error = Recipes::load(root.path(), &items, &PackOverlay::NONE, &load)
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("unknown.json") && error.contains("absent:condition"),
        "{error}"
    );
    assert!(
        !error.contains("unregistered recipe serializer"),
        "conditions must run before serializer lookup: {error}"
    );
}
struct Machine;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Machine {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_recipe_type("machine:grinding");
        r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
    }
}
#[test]
fn read_only_machine_serializer_reports_separate_type_without_crafting() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("machine", Box::new(Machine))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
                "machine",
                Box::new(Machine),
            )])
            .unwrap();
            let root = tempfile::tempdir().unwrap();
            let recipes = Recipes::from_sources(
                root.path(),
                &Items::empty(),
                &PackOverlay::NONE,
                vec![(
                    "machine:iron".into(),
                    serde_json::json!({"type":"machine:grinder_json","value":42}),
                )],
                &crate::test_resources::server_load(),
            )
            .unwrap();
            let machine = &recipes.by_type["machine:grinding"];
            assert_eq!(machine.len(), 1);
            assert_eq!(machine[0].kind, "machine:grinding");
            assert_eq!(machine[0].json["type"], "machine:grinder_json");
            assert_eq!(machine[0].json["value"], 42);
            assert!(recipes.crafting.is_empty() && recipes.cooking.is_empty());
            assert!(!recipes.by_type.contains_key("machine:grinder_json"));
            assert!(
                mod_kinds::recipe_type("wrong", "machine:grinding")
                    .unwrap_err()
                    .contains("machine:grinding")
            );
            assert!(
                recipes
                    .find(
                        crate::test_resources::server_loop_context(),
                        &[None, None, None, None],
                        2
                    )
                    .is_none()
            );
        },
    );
}

#[test]
fn unknown_serializer_refuses_before_unclaimed_ingredient_is_dropped() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let file: Value = serde_json::from_str(
            r#"{"type":"minecraft:future_serializer","ingredient":{"type":"absent:ingredient"}}"#,
        )
        .unwrap();
        let sources = vec![("minecraft:future".into(), file)];
        let context = crate::test_resources::server_load();
        let error = ResolvedSources::resolve(sources.clone(), &Items::empty(), &context)
            .err()
            .unwrap()
            .to_string();
        assert_eq!(
            error,
            "1 recipe failed to load:\n  recipe data/minecraft/recipe/future.json, unregistered recipe serializer minecraft:future_serializer"
        );
        let root = tempfile::tempdir().unwrap();
        let error = Recipes::from_sources(
            root.path(),
            &Items::empty(),
            &PackOverlay::NONE,
            sources,
            &context,
        )
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            "1 recipe failed to load:\n  recipe data/minecraft/recipe/future.json, unregistered recipe serializer minecraft:future_serializer"
        );
    });
}

const STICKS: &str =
    r#","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick","count":3}}"#;

/// A resource root whose `data/minecraft/recipe` holds `files`.
fn recipe_root(files: &[(&str, String)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("data/minecraft/recipe");
    std::fs::create_dir_all(&folder).unwrap();
    for (name, text) in files {
        std::fs::write(folder.join(format!("{name}.json")), text).unwrap();
    }
    root
}

fn load_error(root: &tempfile::TempDir) -> String {
    Recipes::load(
        root.path(),
        &Items::for_tests(Default::default()),
        &PackOverlay::NONE,
        &crate::test_resources::server_load(),
    )
    .err()
    .expect("the load is refused")
    .to_string()
}

#[test]
fn a_type_without_a_namespace_is_minecrafts() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        // `:path` too: the default is `minecraft`, never the file's namespace.
        let root = recipe_root(&[
            ("bare", format!(r#"{{"type":"crafting_shapeless"{STICKS}"#)),
            (
                "colon",
                format!(r#"{{"type":":crafting_shapeless"{STICKS}"#),
            ),
        ]);
        let recipes = Recipes::load(
            root.path(),
            &Items::for_tests(Default::default()),
            &PackOverlay::NONE,
            &crate::test_resources::server_load(),
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 2);
        assert_eq!(recipes.by_type["minecraft:crafting"].len(), 2);
        let grid = [Some(ItemStack::new("minecraft:stone", 1)), None, None, None];
        let (_, output) = recipes
            .find(crate::test_resources::server_loop_context(), &grid, 2)
            .unwrap();
        assert_eq!(output, ItemStack::new("minecraft:stick", 3));
    });
}

#[test]
fn an_unknown_bare_type_is_unregistered_not_malformed() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = recipe_root(&[("future", r#"{"type":"future_serializer"}"#.into())]);
        let error = load_error(&root);
        assert!(
            error.contains("data/minecraft/recipe/future.json"),
            "{error}"
        );
        assert!(error.contains("unregistered recipe serializer"), "{error}");
        assert!(error.contains("minecraft:future_serializer"), "{error}");
        assert!(!error.contains("malformed"), "{error}");
    });
}

#[test]
fn a_malformed_type_is_refused_by_name_with_every_other_bad_recipe() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = recipe_root(&[
            ("upper", r#"{"type":"Crafting_shapeless"}"#.into()),
            ("missing", r#"{"ingredients":[]}"#.into()),
            ("number", r#"{"type":3}"#.into()),
            ("array", "[]".into()),
            ("unknown", r#"{"type":"absent:serializer"}"#.into()),
        ]);
        let error = load_error(&root);
        assert!(error.starts_with("5 recipes failed to load:"), "{error}");
        for (file, category) in [
            ("upper", "malformed recipe serializer id"),
            ("missing", "missing or non-string type"),
            ("number", "missing or non-string type"),
            ("array", "not a JSON object"),
            ("unknown", "unregistered recipe serializer"),
        ] {
            let line = error
                .lines()
                .find(|line| line.contains(&format!("data/minecraft/recipe/{file}.json")))
                .unwrap_or_else(|| panic!("{file} is not named: {error}"));
            assert!(line.contains(category), "{line}");
        }
        assert!(error.contains("Crafting_shapeless"), "{error}");
    });
}

#[test]
fn a_failed_load_names_every_bad_recipe() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let sources = vec![
            (
                "minecraft:b".into(),
                serde_json::json!({"type":"absent:two"}),
            ),
            (
                "minecraft:a".into(),
                serde_json::json!({"type":"absent:one"}),
            ),
            (
                "ns:c".into(),
                serde_json::json!({"type":"minecraft:crafting_special_repairitem"}),
            ),
        ];
        let error = ResolvedSources::resolve(
            sources,
            &Items::empty(),
            &crate::test_resources::server_load(),
        )
        .err()
        .unwrap()
        .to_string();
        assert_eq!(
            error,
            "2 recipes failed to load:\n  recipe data/minecraft/recipe/a.json, unregistered recipe serializer absent:one\n  recipe data/minecraft/recipe/b.json, unregistered recipe serializer absent:two"
        );
    });
}

#[test]
fn a_failed_load_reports_resolution_and_validation_failures_together() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let sources = vec![
            (
                "minecraft:a".into(),
                serde_json::json!({"type":"absent:serializer"}),
            ),
            (
                "minecraft:b".into(),
                serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[{"type":"absent:kind"}],"result":{"id":"minecraft:stick"}}),
            ),
            (
                "minecraft:c".into(),
                serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[],"result":{"id":"minecraft:stick"}}),
            ),
            (
                "minecraft:d".into(),
                serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":["minecraft:stone"],"result":{"id":"minecraft:stick"}}),
            ),
        ];
        let root = tempfile::tempdir().unwrap();
        let error = Recipes::from_sources(
            root.path(),
            &Items::for_tests(Default::default()),
            &PackOverlay::NONE,
            sources,
            &crate::test_resources::server_load(),
        )
        .err()
        .unwrap()
        .to_string();
        let lines: Vec<_> = error.lines().collect();
        assert_eq!(lines[0], "3 recipes failed to load:", "{error}");
        for (line, file) in lines[1..].iter().zip(["a", "b", "c"]) {
            assert!(
                line.contains(&format!("data/minecraft/recipe/{file}.json")),
                "{error}"
            );
        }
        assert!(!error.contains("d.json"), "{error}");
    });
}

#[test]
fn collected_sources_and_table_build_report_every_stage_together() {
    mod_platform::testing::with_mods(Vec::new().into(), || {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("data/minecraft/recipe");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("a.json"), r#"{"type":"absent:serializer"}"#).unwrap();
        std::fs::write(
            folder.join("b.json"),
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[],"result":{"id":"minecraft:stick"}}"#,
        )
        .unwrap();
        let context = crate::test_resources::server_load();
        let items = Items::for_tests(Default::default());
        let sources =
            ResolvedSources::load_collecting(root.path(), &items, &PackOverlay::NONE, &context)
                .unwrap();
        assert!(!sources.is_complete());
        let error = Recipes::from_resolved(root.path(), &items, &PackOverlay::NONE, &sources)
            .err()
            .unwrap()
            .to_string();
        assert!(error.starts_with("2 recipes failed to load:"), "{error}");
        assert!(
            error.find("a.json").unwrap() < error.find("b.json").unwrap(),
            "{error}"
        );
        assert!(ResolvedSources::load(root.path(), &items, &PackOverlay::NONE, &context).is_err());
    });
}
