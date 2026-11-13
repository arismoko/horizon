use super::*;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
use std::cell::{Cell, RefCell};
thread_local! { static LIVE: Cell<bool> = const { Cell::new(true) }; static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) }; }
fn matching(
    _: &horizon_world::native_load_context::LoopContext,
    kind: &str,
    _: &horizon_core::nbt::Compound,
    stack: &ItemStack,
    accept: &mut bool,
) {
    if kind == "empty_fixture:predicate" {
        CALLS.with_borrow_mut(|calls| calls.push(kind.to_owned()));
        *accept = stack.item.as_ref() == "minecraft:stone" && stack.damage == 7;
    }
    if kind.starts_with("placement_guard:") {
        CALLS.with_borrow_mut(|calls| calls.push(kind.to_owned()));
        *accept = false;
        if kind == "placement_guard:changes_context" {
            LIVE.set(false);
        }
    }
}
fn install() {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        assert!(crate::native_stack_hooks::install(
            crate::native_stack_hooks::StackHooks {
                ingredient_test: matching,
                enchantments: |_, _, _| {},
                attributes: |_, _| {}
            }
        ));
    });
}
#[test]
fn a_changed_context_interrupts_a_compound_before_the_next_custom_leaf() {
    install();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    let context = load.loop_context(LoopSide::Server, []);
    let child = |kind: &str| {
        let mut value = Ingredient::of(vec!["fixture:item".into()]);
        value.custom = Some((kind.into(), Default::default()));
        value
    };
    let mut ingredient = Ingredient::of(vec!["fixture:item".into()]);
    ingredient.children = Some((
        Shape::Compound,
        vec![
            child("placement_guard:changes_context"),
            child("placement_guard:must_not_run"),
        ],
    ));
    assert_eq!(
        ingredient.test_in_guarded(&context, &ItemStack::new("fixture:item", 1), &|| LIVE.get()),
        None
    );
    CALLS.with_borrow(|calls| assert_eq!(calls, &["placement_guard:changes_context"]));
}

#[test]
fn empty_ingredient_recipes_retain_predicates_but_not_placement() {
    use crate::crafting::{Recipes, ResolvedSources};
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    install();
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
    let resolved = ResolvedSources::resolve(sources, &load);
    let root = std::env::temp_dir();
    let recipes = Recipes::from_resolved(&root, &items, &PackOverlay::NONE, &resolved).unwrap();
    let stations = crate::stations::StationRecipes::from_sources(&root, &items, &resolved);
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
        let entry = recipes.by_type["minecraft:crafting_shaped"]
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
            ("minecraft:smithing_transform", "_smith"),
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
#[ignore = "isolated logger fixture; run this selector alone"]
fn empty_ingredient_load_diagnostics_distinguish_errors_and_warnings() {
    struct Capture(std::sync::Mutex<Vec<(log::Level, String)>>);
    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            if record.target().contains("crafting::ingredient") {
                self.0
                    .lock()
                    .unwrap()
                    .push((record.level(), record.args().to_string()));
            }
        }
        fn flush(&self) {}
    }
    static LOG: Capture = Capture(std::sync::Mutex::new(Vec::new()));
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Warn);
    use crate::crafting::Recipes;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
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
        (json!({"type":"fixture:predicate"}), log::Level::Warn),
    ] {
        LOG.0.lock().unwrap().clear();
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
        )
        .unwrap();
        let levels = LOG.0.lock().unwrap();
        assert!(levels.iter().any(|(level, _)| *level == expected));
        if ingredient == json!("#fixture:missing") {
            assert!(
                levels
                    .iter()
                    .any(|(_, message)| message.contains("#fixture:missing"))
            );
        }
        let opposite = if expected == log::Level::Warn {
            log::Level::Error
        } else {
            log::Level::Warn
        };
        assert!(!levels.iter().any(|(level, _)| *level == opposite));
        assert_eq!(!recipes.crafting.is_empty(), expected == log::Level::Warn);
    }
}

#[test]
fn cooking_indexes_empty_predicate_descendants_once_in_registration_order() {
    use crate::crafting::Recipes;
    use horizon_world::loot::PackOverlay;
    use serde_json::json;
    install();
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
        CALLS.with_borrow_mut(Vec::clear);
        assert_eq!(
            recipes
                .cook(&context, "minecraft:smelting", &stack)
                .unwrap()
                .id,
            expected
        );
        CALLS.with_borrow(|calls| assert_eq!(calls.len(), 1));
    }
}
