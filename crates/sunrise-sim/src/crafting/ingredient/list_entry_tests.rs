//! #1273: every entry of an ingredient list is an item id other than air; any
//! other entry fails the whole file, with the refusal naming it.
use super::*;
use serde_json::{Value, json};
use sunrise_world::loot::PackOverlay;
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};

fn load(ingredient: Value) -> Result<crate::crafting::Recipes, String> {
    load_with(&Items::for_tests(Default::default()), ingredient)
}

fn load_with(items: &Items, ingredient: Value) -> Result<crate::crafting::Recipes, String> {
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    );
    crate::crafting::Recipes::from_sources(
        &std::env::temp_dir(),
        items,
        &PackOverlay::NONE,
        vec![(
            "fixture:recipe".into(),
            json!({"type": "minecraft:crafting_shapeless",
                "ingredients": [ingredient], "result": {"id": "minecraft:stick"}}),
        )],
        &load,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn a_list_entry_that_is_not_an_item_id_fails_the_file() {
    for ingredient in [
        json!(["minecraft:stone", 5]),
        json!(["minecraft:stone", {}]),
        json!(["minecraft:stone", null]),
        json!(["minecraft:stone", "minecraft:air"]),
        json!(["minecraft:stone", "air"]),
        json!("minecraft:air"),
    ] {
        let Err(error) = load(ingredient.clone()) else {
            panic!("{ingredient} loaded");
        };
        assert!(error.contains("data/fixture/recipe/recipe.json"), "{error}");
        assert!(error.contains("invalid ingredient"), "{error}");
    }
}

#[test]
fn a_list_of_item_ids_still_loads_in_order() {
    let Ok(recipes) = load(json!(["minecraft:stone", "minecraft:dirt"])) else {
        panic!("a list of ids loads");
    };
    let Some((_, crate::crafting::Recipe::Shapeless { ingredients, .. })) =
        recipes.crafting.first()
    else {
        panic!("one shapeless recipe");
    };
    assert_eq!(ingredients[0].order, ["minecraft:stone", "minecraft:dirt"]);
}

/// Vanilla refuses air only among directly named ids: a tag that holds it
/// still loads.
#[test]
fn a_tag_holding_air_still_loads() {
    let mut items = Items::for_tests(Default::default());
    items.item_tag_order.insert(
        "fixture:with_air".into(),
        vec!["minecraft:stone".into(), "minecraft:air".into()],
    );
    assert!(load_with(&items, json!("#fixture:with_air")).is_ok());
}
