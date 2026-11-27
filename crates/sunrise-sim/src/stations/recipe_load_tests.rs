use crate::crafting::{Recipes, ResolvedSources};
use crate::enchant::Enchantments;
use crate::items::{ItemStack, Items};
use crate::stations::StationRecipes;
use serde_json::Value;
use serde_json::json;
use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use sunrise_world::loot::PackOverlay;
use sunrise_world::native_load_context::{
    DataLoadOrigin, LoadContext, LoadInputs, LoopContext, LoopSide,
};

fn lc() -> &'static LoopContext {
    crate::test_resources::server_loop_context()
}
thread_local! { static CALLS: Cell<usize> = const { Cell::new(0) }; }

fn empty_ingredient_recipes_retain_predicates_but_not_placement() {
    use crate::crafting::{Recipes, ResolvedSources};
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let mut items = Items::for_tests(Default::default());
    items
        .item_tag_order
        .insert("empty_fixture:empty".into(), vec![]);
    let inputs = [
        ("direct", json!([]), false, false),
        ("missing", json!("#empty_fixture:missing"), false, false),
        ("tag", json!("#empty_fixture:empty"), true, false),
        (
            "nested_tag",
            json!({"type":"neoforge:compound","children":["#empty_fixture:empty"]}),
            true,
            false,
        ),
        (
            "custom",
            json!({"type":"empty_fixture:predicate"}),
            true,
            true,
        ),
        (
            "nested",
            json!({"type":"neoforge:compound",
            "children":[{"type":"empty_fixture:predicate"}]}),
            true,
            true,
        ),
    ];
    let mut sources: Vec<_> = inputs
        .iter()
        .flat_map(|(id, ingredient, _, _)| {
            [
                (
                    format!("empty_fixture:{id}"),
                    json!({
                        "type":"minecraft:crafting_shaped", "pattern":["A"],
                        "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cook"),
                    json!({
                        "type":"minecraft:smelting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_cut"),
                    json!({
                        "type":"minecraft:stonecutting", "ingredient":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
                (
                    format!("empty_fixture:{id}_smith"),
                    json!({
                        "type":"minecraft:smithing_transform",
                        "base":"minecraft:stone", "addition":ingredient,
                        "result":{"id":"minecraft:stick"}
                    }),
                ),
            ]
        })
        .collect();
    sources.push((
        "empty_fixture:visible_neighbor".into(),
        json!({
            "type":"minecraft:crafting_shaped","pattern":["A"],
            "key":{"A":"minecraft:stone"},"result":{"id":"minecraft:stick"}
        }),
    ));
    for (name, file) in sources.iter().filter(|(name, _)| {
        name.starts_with("empty_fixture:direct") || name.starts_with("empty_fixture:missing")
    }) {
        let error = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![
                (name.clone(), file.clone()),
                sources.last().unwrap().clone(),
            ],
            &load,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(
            error.contains(name.strip_prefix("empty_fixture:").unwrap()),
            "{error}"
        );
        assert!(error.contains("invalid ingredient"), "{error}");
    }
    sources.retain(|(name, _)| {
        !name.starts_with("empty_fixture:direct") && !name.starts_with("empty_fixture:missing")
    });
    let resolved = ResolvedSources::resolve(sources, &items, &load).unwrap();
    let root = std::env::temp_dir();
    let recipes = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &resolved).unwrap();
    let stations = crate::stations::StationRecipes::from_sources(&items, &resolved);
    let mut accepted = ItemStack::new("minecraft:stone", 1);
    accepted.damage = 7;
    let book = crate::recipe_book::Book::build(&recipes, &items);
    let index = crate::recipe_book::RecipeIndex::from_recipes(&recipes);
    let mut learned = crate::recipe_book::Settings::default();
    assert_eq!(book.entries.len(), 1);
    assert_eq!(book.entries[0].id, "empty_fixture:visible_neighbor");
    assert_eq!(
        index.first_display("empty_fixture:visible_neighbor"),
        Some(0)
    );
    for (id, _, retained, matches) in inputs {
        let id = format!("empty_fixture:{id}");
        let entry = recipes.by_type["minecraft:crafting"]
            .iter()
            .find(|r| r.id == id);
        assert_eq!(entry.is_some(), retained, "{id}");
        assert!(!index.contains(&id));
        assert!(book.entry(&id).is_none());
        assert_eq!(
            learned.add_recipes(&index, std::slice::from_ref(&id)),
            (0, vec![])
        );
        assert!(!learned.knows(&id));
        assert_eq!(
            recipes.recipe_matches(&context, &id, &[Some(accepted.clone())], 1),
            matches
        );
        assert!(!recipes.recipe_matches(
            &context,
            &id,
            &[Some(ItemStack::new("minecraft:stone", 1))],
            1
        ));
        assert!(recipes.placement(&id).is_none_or(|p| p.impossible()));
        assert!(
            recipes
                .placement(&format!("{id}_cook"))
                .is_none_or(|p| p.impossible())
        );
        for (kind, suffix) in [
            ("minecraft:smelting", "_cook"),
            ("minecraft:stonecutting", "_cut"),
            ("minecraft:smithing", "_smith"),
        ] {
            assert_eq!(
                recipes.by_type[kind]
                    .iter()
                    .any(|r| r.id == format!("{id}{suffix}")),
                retained
            );
        }
        if let Some(entry) = entry {
            assert!(entry.ingredients[0].items.is_empty());
            if matches {
                assert!(
                    entry.ingredients[0].custom.is_some()
                        || entry.ingredients[0].children.is_some()
                );
            } else {
                assert!(entry.ingredients[0].custom.is_none());
            }
        }
    }
    assert_eq!(
        recipes
            .cook(&context, "minecraft:smelting", &accepted)
            .unwrap()
            .id,
        "empty_fixture:custom_cook"
    );
    assert!(stations.stonecutting.is_empty());
    assert!(stations.stonecutter(&context, &accepted).is_empty());
    assert!(!stations.smithing_accepts(&context, 1, &accepted));
    assert!(!recipes.cooking_accepts(&context, "minecraft:smelting", &accepted));
}

#[test]
fn empty_ingredient_load_diagnostics_distinguish_errors_and_warnings() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let mut items = Items::for_tests(Default::default());
    items.item_tag_order.insert("fixture:empty".into(), vec![]);
    for (ingredient, expected) in [
        (json!([]), log::Level::Error),
        (json!("#fixture:missing"), log::Level::Error),
        (json!("#fixture:empty"), log::Level::Warn),
        (json!({"type":"fixture:predicate"}), log::Level::Error),
    ] {
        crate::test_log::clear();
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            vec![(
                "fixture:recipe".into(),
                json!({"type":"minecraft:crafting_shaped",
                "pattern":["A"], "key":{"A":ingredient}, "result":{"id":"minecraft:stick"}}),
            )],
            &load,
        );
        let levels = crate::test_log::records();
        if expected == log::Level::Error {
            let error = recipes.err().unwrap().to_string();
            assert!(error.contains("data/fixture/recipe/recipe.json"), "{error}");
            assert!(
                error.contains("invalid ingredient")
                    || error.contains("ingredient kind fixture:predicate"),
                "{error}"
            );
            // One Error record, the refusal naming the file.
            let errors: Vec<_> = levels
                .iter()
                .filter(|(level, _)| *level == log::Level::Error)
                .collect();
            assert_eq!(errors.len(), 1, "{levels:?}");
            assert!(
                errors[0].1.contains("data/fixture/recipe/recipe.json"),
                "{levels:?}"
            );
        } else {
            assert_eq!(recipes.unwrap().crafting.len(), 1);
            assert!(levels.iter().any(|(level, _)| *level == log::Level::Warn));
            assert!(!levels.iter().any(|(level, _)| *level == log::Level::Error));
        }
    }
}

struct ListenerIngredient(&'static str, &'static [&'static str]);
#[mod_api::__subscriptions]
impl mod_api::Hooks for ListenerIngredient {
    fn ingredient(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::IngredientEvent<'_>,
    ) -> Option<mod_api::IngredientDefinition> {
        let kind = event.kind;
        if kind != self.0 {
            return None;
        }
        Some(mod_api::IngredientDefinition::new(
            self.1.iter().map(|id| id.to_string()).collect(),
            |_, stack| {
                CALLS.set(CALLS.get() + 1);
                stack.item_id() == "minecraft:stone" && stack.damage() == Some(7)
            },
        ))
    }
}

#[test]
fn panicking_ingredient_predicate_disables_only_its_definition_across_recipe_loads() {
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;

    struct Predicates;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Predicates {
        fn ingredient(
            &self,
            _: &dyn mod_api::DataLoadContext,
            event: &mut mod_api::IngredientEvent<'_>,
        ) -> Option<mod_api::IngredientDefinition> {
            if event.kind != "fixture:predicate" {
                return None;
            }
            let panics = matches!(
                event.data.get(&mod_api::JavaString::from("panics")),
                Some(mod_api::Tag::Byte(1))
            );
            Some(mod_api::IngredientDefinition::new(
                vec!["minecraft:stone".into()],
                move |_, stack| {
                    if panics && stack.count() == 3 {
                        CALLS.set(CALLS.get() + 1);
                        panic!("ingredient regression panic");
                    }
                    stack.count() == 2
                },
            ))
        }
    }

    crate::test_log::install();
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "predicate_owner",
            Box::new(Predicates),
        )]
        .into(),
        || {
            let data = crate::test_resources::server_load();
            let context = data.loop_context(LoopSide::Server, []);
            let items = Items::for_tests(Default::default());
            let load = |panics| {
                Recipes::from_sources(
                    &std::env::temp_dir(),
                    &items,
                    &PackOverlay::NONE,
                    [("fixture:panicking", panics), ("fixture:healthy", true)]
                        .map(|(recipe, panics)| {
                            (
                                recipe.into(),
                                json!({"type":"minecraft:crafting_shapeless",
                                "ingredients":[{"type":"fixture:predicate", "panics":panics}],
                                "result":{"id":"minecraft:stick"}}),
                            )
                        })
                        .into(),
                    &data,
                )
                .unwrap()
            };
            let one = [Some(ItemStack::new("minecraft:stone", 1))];
            let two = [Some(ItemStack::new("minecraft:stone", 2))];
            let three = [Some(ItemStack::new("minecraft:stone", 3))];
            let recipes = load(true);
            crate::test_log::clear();
            CALLS.set(0);
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &three, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            // The second recipe decoded exactly the same ingredient JSON into its own callback.
            assert!(recipes.recipe_matches(&context, "fixture:healthy", &two, 1));
            let recipes = load(true);
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &three, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            assert!(!recipes.recipe_matches(&context, "fixture:healthy", &one, 1));
            assert!(recipes.recipe_matches(&context, "fixture:healthy", &two, 1));
            let recipes = load(false);
            assert!(recipes.recipe_matches(&context, "fixture:panicking", &two, 1));
            assert_eq!(CALLS.get(), 1);
            let logs = crate::test_log::records();
            assert_eq!(
                logs.iter()
                    .filter(|(level, text)| *level == log::Level::Error
                        && text.contains("predicate_owner")
                        && text.contains("ingredient regression panic"))
                    .count(),
                1
            );
            assert_eq!(
                mod_api::__private::take_hook_panics(),
                [mod_api::__private::HookPanicReport {
                    mod_id: "predicate_owner",
                    hook: "ingredient_predicate",
                }]
            );
        },
    );
}

#[test]
fn unclaimed_custom_kind_refuses_load_but_an_empty_answer_only_warns() {
    crate::test_log::install();
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let items = Items::for_tests(Default::default());
    let recipe = || {
        vec![(
            "fixture:needs_mod".to_string(),
            json!({"type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"absent:kind"}], "result":{"id":"minecraft:stick"}}),
        )]
    };
    let load = || {
        LoadContext::new(
            DataLoadOrigin::ServerDataLoad,
            std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
            [],
        )
    };
    let run = |mods: Vec<mod_api::ModEntry>| {
        crate::test_log::clear();
        let mut loaded = None;
        mod_platform::testing::with_mods(mods.into(), || {
            loaded = Some(Recipes::from_sources(
                &std::env::temp_dir(),
                &items,
                &PackOverlay::NONE,
                recipe(),
                &load(),
            ));
        });
        let logs = crate::test_log::records();
        (loaded.unwrap(), logs)
    };
    // No listener: refuse the entire load, naming the file and kind once.
    let (recipes, logs) = run(vec![]);
    let error = recipes.err().unwrap().to_string();
    assert!(
        error.contains("data/fixture/recipe/needs_mod.json"),
        "{error}"
    );
    assert!(error.contains("ingredient kind absent:kind"), "{error}");
    // The refusal is logged once at Error, naming the file.
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert_eq!(logs[0].0, log::Level::Error);
    assert!(
        logs[0].1.contains("data/fixture/recipe/needs_mod.json"),
        "{logs:?}"
    );
    // An unrelated subscriber does not claim this kind.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "unrelated",
        Box::new(ListenerIngredient("other:kind", &[])),
    )]);
    let unrelated = recipes.err().unwrap().to_string();
    assert_eq!(unrelated, error);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].0, log::Level::Error);

    // A listener with an empty answer: kept, with a warning naming both.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "empty",
        Box::new(ListenerIngredient("absent:kind", &[])),
    )]);
    assert_eq!(recipes.unwrap().crafting.len(), 1);
    assert!(logs.iter().any(|(level, text)| *level == log::Level::Warn
        && text.contains("fixture:needs_mod")
        && text.contains("absent:kind")));
    assert!(!logs.iter().any(|(level, _)| *level == log::Level::Error));
    // A listener with items: loads as before, silently.
    let (recipes, logs) = run(vec![mod_api::ModEntry::new(
        "items",
        Box::new(ListenerIngredient("absent:kind", &["minecraft:stick"])),
    )]);
    assert_eq!(recipes.unwrap().crafting.len(), 1);
    assert!(logs.is_empty());
}

fn cooking_indexes_empty_predicate_descendants_once_in_registration_order() {
    use crate::crafting::Recipes;
    use serde_json::json;
    use sunrise_world::loot::PackOverlay;
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let items = Items::for_tests(Default::default());
    for (ingredient, damage, expected) in [
        (
            json!({"type":"neoforge:compound","children":[
            {"type":"empty_fixture:predicate"},"minecraft:dirt"]}),
            7,
            "fixture:a",
        ),
        (
            json!({"type":"neoforge:difference","base":"minecraft:stone",
            "subtracted":{"type":"empty_fixture:predicate"}}),
            7,
            "fixture:b",
        ),
    ] {
        let sources = vec![
            (
                "fixture:a".into(),
                json!({"type":"minecraft:smelting","ingredient":ingredient,
                "result":{"id":"minecraft:stick"}}),
            ),
            (
                "fixture:b".into(),
                json!({"type":"minecraft:smelting","ingredient":"minecraft:stone",
                "result":{"id":"minecraft:diamond"}}),
            ),
        ];
        let recipes = Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &PackOverlay::NONE,
            sources,
            &load,
        )
        .unwrap();
        let mut stack = ItemStack::new("minecraft:stone", 1);
        stack.damage = damage;
        CALLS.set(0);
        assert_eq!(
            recipes
                .cook(&context, "minecraft:smelting", &stack)
                .unwrap()
                .id,
            expected
        );
        assert_eq!(CALLS.get(), 1);
    }
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn empty_ingredient_recipes_retain_predicates_but_not_placement_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient("empty_fixture:predicate", &[])),
        )]
        .into(),
        empty_ingredient_recipes_retain_predicates_but_not_placement,
    );
}

// A mod listens to the `ingredient` hook and answers with no items.
#[test]
fn cooking_indexes_empty_predicate_descendants_once_in_registration_order_with_a_listener() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new(
            "empty",
            Box::new(ListenerIngredient("empty_fixture:predicate", &[])),
        )]
        .into(),
        cooking_indexes_empty_predicate_descendants_once_in_registration_order,
    );
}

// ---- station tables built from loaded recipes

#[test]
fn brewing_recipes_decode_once_by_type_preserving_potions_and_result_components() {
    let root = tempfile::tempdir().unwrap();
    let items = Items::for_tests(HashMap::new());
    let load = crate::test_resources::server_load();
    let recipe = serde_json::json!({
        "type": "minecraft:brewing",
        "input": {
            "item": "minecraft:potion",
            "potion_contents": {"potions": "minecraft:water"}
        },
        "reagent": {"item": "minecraft:blaze_powder"},
        "output": {
            "id": "minecraft:potion",
            "count": 1,
            "components": {
                "minecraft:potion_contents": {"potion": "minecraft:strength"},
                "minecraft:repair_cost": 3
            }
        }
    });
    let sources = ResolvedSources::resolve(
        vec![("example:custom_elixir".into(), recipe)],
        &items,
        &load,
    )
    .unwrap();
    let recipes =
        Recipes::from_resolved(root.path(), &items, &PackOverlay::NONE, &sources).unwrap();
    fs::write(root.path().join("items.json"), b"{}").unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let stations = crate::stations::StationData::from_sources(
        root.path(),
        &items,
        &PackOverlay::NONE,
        crate::banner::Banners::load(root.path(), &defaults),
        (&sources, &recipes),
    )
    .unwrap();
    assert_eq!(stations.brewing.recipes.len(), 1);
    assert_eq!(stations.brewing.recipes[0].id, "example:custom_elixir");
    assert!(stations.brewing.reagents.contains("minecraft:blaze_powder"));
    assert!(stations.brewing.inputs.contains("minecraft:potion"));
    let water = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:water");
    let awkward = ItemStack::new("minecraft:potion", 1).with_potion("minecraft:awkward");
    let reagent = ItemStack::new("minecraft:blaze_powder", 1);
    assert!(stations.brewing.find(&awkward, &reagent).is_none());
    let matched = stations.brewing.find(&water, &reagent).unwrap();
    assert_eq!(matched.output.potion(), Some("minecraft:strength"));
    assert_eq!(matched.output.data().map(|d| d.repair_cost), Some(3));
}

#[test]
fn recipe_rule_views_share_station_and_crafting_parses() {
    mod_platform::testing::with_recipe_types(&["example:machine"], || {
        let named = |name: &str, json: &str| {
            (
                name.to_owned(),
                serde_json::from_str::<Value>(json).unwrap(),
            )
        };
        let mut raw = vec![
            named(
                "cut",
                r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stick","count":2}}"#,
            ),
            named(
                "invalid",
                r#"{"type":"minecraft:stonecutting","ingredient":"minecraft:stone","result":{"id":"minecraft:stick","count":100}}"#,
            ),
            named(
                "smith",
                r#"{"type":"minecraft:smithing_transform","template":"minecraft:stick","base":"minecraft:stone","addition":"minecraft:iron_ingot","result":{"id":"minecraft:diamond"}}"#,
            ),
            named(
                "transmute",
                r#"{"type":"minecraft:crafting_transmute","input":"minecraft:stone","material":"minecraft:stick","material_count":{"min":2,"max":4},"add_material_count_to_result":true,"result":{"id":"minecraft:diamond","count":2,"components":{"minecraft:damage":0,"!minecraft:custom_name":{}}}}"#,
            ),
            named(
                "trim",
                r#"{"type":"minecraft:smithing_trim","template":"minecraft:stick","base":"minecraft:stone","addition":"minecraft:iron_ingot","pattern":"minecraft:coast"}"#,
            ),
            named("custom", r#"{"type":"example:machine","value":42}"#),
        ];
        let items = Items::for_tests(HashMap::new());
        let load = crate::test_resources::server_load();
        let root = std::env::temp_dir();
        let sources = ResolvedSources::resolve_collecting(raw.clone(), &items, &load).unwrap();
        let error = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &sources)
            .err()
            .unwrap()
            .to_string();
        assert!(
            error.contains("data/minecraft/recipe/invalid.json"),
            "{error}"
        );
        raw.remove(1);
        let sources = ResolvedSources::resolve(raw, &items, &load).unwrap();
        let recipes = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &sources).unwrap();
        let stations = crate::stations::StationRecipes::from_sources(&items, &sources);
        let cuts = &recipes.by_type["minecraft:stonecutting"];
        assert_eq!(cuts.len(), 1);
        let Some(crate::crafting::RecipeRule::Stonecutting { input, result }) = &cuts[0].rule
        else {
            panic!("missing stonecutting rule")
        };
        assert_eq!(input.order, ["minecraft:stone"]);
        assert_eq!(result.count, 2);
        assert_eq!(stations.stonecutting[0].result, *result);
        let smith = &serialized(&recipes, "minecraft:smithing_transform")[0];
        let Some(crate::crafting::RecipeRule::SmithingTransform { base, result, .. }) = &smith.rule
        else {
            panic!("missing smithing rule")
        };
        assert_eq!(base.order, ["minecraft:stone"]);
        assert_eq!(result.item.as_deref(), Some("minecraft:diamond"));
        assert!(
            smith.results.is_empty(),
            "smithing output inherits components"
        );
        let transmute = &serialized(&recipes, "minecraft:crafting_transmute")[0];
        let Some(crate::crafting::RecipeRule::Transmute {
            min,
            max,
            add_count,
            result,
            ..
        }) = &transmute.rule
        else {
            panic!("missing transmute rule")
        };
        assert_eq!((*min, *max, *add_count), (2, 4, true));
        assert_eq!(result.item.as_deref(), Some("minecraft:diamond"));
        let patch = result
            .components
            .as_ref()
            .expect("component patch retained");
        assert!(patch.get("minecraft:damage").is_some());
        assert!(patch.get("!minecraft:custom_name").is_some());
        let trim = &serialized(&recipes, "minecraft:smithing_trim")[0];
        let Some(crate::crafting::RecipeRule::SmithingTrim {
            pattern, addition, ..
        }) = &trim.rule
        else {
            panic!("missing smithing trim rule")
        };
        assert!(
            matches!(pattern, crate::crafting::RecipeTrimPattern::Reference(id) if id == "minecraft:coast")
        );
        assert_eq!(addition.order, ["minecraft:iron_ingot"]);
        assert!(trim.results.is_empty());
        assert!(
            transmute.results.is_empty(),
            "grid-dependent output is not fixed"
        );
        let grid = [
            Some(ItemStack::new("minecraft:stone", 1)),
            Some(ItemStack::new("minecraft:stick", 1)),
            Some(ItemStack::new("minecraft:stick", 1)),
            None,
        ];
        let output = recipes.craft(lc(), &grid, 2).expect("transmute matches");
        assert_eq!(output.item.as_ref(), "minecraft:diamond");
        assert_eq!(output.count, 4);
        let custom = &recipes.by_type["example:machine"][0];
        assert!(custom.rule.is_none());
        assert_eq!(custom.json["value"], 42);
    });
}

fn serialized<'a>(
    recipes: &'a crate::crafting::Recipes,
    serializer: &str,
) -> Vec<&'a crate::crafting::RecipeSnapshot> {
    recipes
        .by_type
        .values()
        .flat_map(|entries| entries.iter())
        .filter(|recipe| recipe.json["type"] == serializer)
        .collect()
}

// ---- every projection uses the same priority order
// The fixture helpers below are local copies of the priority file's; the
// originals stay in `crafting/priorities/tests.rs` with their own tests.
fn layer(pack: &str, bytes: &[u8]) -> sunrise_world::loot::RecipePriorityLayer {
    sunrise_world::loot::RecipePriorityLayer {
        pack: pack.into(),
        path: sunrise_mod_package::content::RECIPE_PRIORITIES_PATH,
        contents: Ok(bytes.to_vec()),
    }
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("sunrise-priorities-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("data/minecraft/recipe")).unwrap();
        Self(root)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn every_projection_uses_the_same_priority_order() {
    mod_platform::testing::with_recipe_types(&["test:crusher"], projections_follow_one_order);
}

fn projections_follow_one_order() {
    let root = Fixture::new("projections");
    let items = Items::for_tests(HashMap::new());
    let mut sources = Vec::new();
    let mut entries = serde_json::Map::new();
    for suffix in ["a", "b", "c"] {
        for kind in [
            "smelting",
            "blasting",
            "smoking",
            "campfire_cooking",
            "smithing_transform",
            "stonecutting",
            "crusher",
        ] {
            let id = format!("test:{kind}_{suffix}");
            let recipe = match kind {
                "smithing_transform" => {
                    json!({"type":"minecraft:smithing_transform", "base":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
                "stonecutting" => {
                    json!({"type":"minecraft:stonecutting", "ingredient":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
                "crusher" => json!({"type":"test:crusher", "output":suffix}),
                _ => {
                    json!({"type":format!("minecraft:{kind}"), "ingredient":"minecraft:stone", "result":{"id":"minecraft:diamond"}})
                }
            };
            entries.insert(id.clone(), json!(if suffix == "a" { -1 } else { 5 }));
            sources.push((id, recipe));
        }
    }
    let priority_layers = [layer(
        "projection",
        &serde_json::to_vec(&json!({"entries": entries})).unwrap(),
    )];
    let overlay = PackOverlay {
        priority_layers: &priority_layers,
        ..PackOverlay::default()
    };
    ResolvedSources::order_by_priority(&mut sources, &root.0, &overlay).unwrap();
    let resolved =
        ResolvedSources::resolve_collecting(sources, &items, &crate::test_resources::server_load())
            .unwrap()
            .checked()
            .unwrap();
    let recipes = Recipes::from_resolved(&root.0, &items, &overlay, &resolved).unwrap();
    let stations = StationRecipes::from_sources(&items, &resolved);
    let context = crate::test_resources::server_loop_context();
    let stone = ItemStack::new("minecraft:stone", 1);
    for kind in ["smelting", "blasting", "smoking", "campfire_cooking"] {
        assert_eq!(
            recipes
                .cook(context, &format!("minecraft:{kind}"), &stone)
                .unwrap()
                .id,
            format!("test:{kind}_b")
        );
    }
    assert_eq!(
        stations
            .smithing_recipe(context, &Enchantments::default(), None, Some(&stone), None)
            .unwrap()
            .0,
        "test:smithing_transform_b"
    );
    assert_eq!(
        stations
            .stonecutter(context, &stone)
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        [
            "test:stonecutting_b",
            "test:stonecutting_c",
            "test:stonecutting_a"
        ]
    );
    assert_eq!(
        recipes.by_type["test:crusher"]
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["test:crusher_b", "test:crusher_c", "test:crusher_a"]
    );
    // Smithing's serializers share one recipe type.
    for (kind, recipe_type) in [
        ("smelting", "minecraft:smelting"),
        ("blasting", "minecraft:blasting"),
        ("smoking", "minecraft:smoking"),
        ("campfire_cooking", "minecraft:campfire_cooking"),
        ("smithing_transform", "minecraft:smithing"),
        ("stonecutting", "minecraft:stonecutting"),
    ] {
        let ids: Vec<_> = recipes.by_type[recipe_type]
            .iter()
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(
            ids,
            [
                format!("test:{kind}_b"),
                format!("test:{kind}_c"),
                format!("test:{kind}_a")
            ]
        );
    }
}
