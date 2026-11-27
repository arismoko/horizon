use super::*;
use serde_json::json;

/// The table has no items, so every id a recipe names directly is unknown;
/// tags are not checked.
fn unknown(file: Value) -> Option<String> {
    unknown_item(&file, &Items::empty())
}

#[test]
fn an_unknown_result_item_is_named() {
    let file = json!({"type":"minecraft:crafting_shapeless","ingredients":["#minecraft:planks"],
        "result":{"id":"missing:thing"}});
    assert_eq!(unknown(file).as_deref(), Some("unknown item missing:thing"));
}

#[test]
fn an_unknown_ingredient_item_is_named() {
    let file = json!({"type":"minecraft:smelting","ingredient":"missing:ore",
        "result":{"id":"minecraft:stick"}});
    assert_eq!(unknown(file).as_deref(), Some("unknown item missing:ore"));
    let shaped = json!({"type":"minecraft:crafting_shaped","pattern":["a"],
        "key":{"a":["#minecraft:planks","gone:plank"]},"result":{"id":"minecraft:stick"}});
    assert_eq!(unknown(shaped).as_deref(), Some("unknown item gone:plank"));
}

#[test]
fn a_brewing_input_and_reagent_are_checked() {
    let file = json!({"type":"minecraft:brewing","input":{"item":"missing:bottle"}});
    assert_eq!(
        unknown(file).as_deref(),
        Some("unknown item missing:bottle")
    );
    let file = json!({"type":"minecraft:brewing","reagent":{"item":"missing:dust"}});
    assert_eq!(unknown(file).as_deref(), Some("unknown item missing:dust"));
}

#[test]
fn air_is_not_an_item_of_a_recipe() {
    let result =
        json!({"type":"minecraft:crafting_shapeless","ingredients":[],"result":"minecraft:air"});
    assert_eq!(
        unknown(result).as_deref(),
        Some("unknown item minecraft:air")
    );
    let ingredient = json!({"type":"minecraft:smelting","ingredient":"air","result":{"id":"x:y"}});
    assert_eq!(unknown(ingredient).as_deref(), Some("unknown item air"));
}

#[test]
fn a_tag_names_no_item() {
    let file = json!({"type":"minecraft:stonecutting","ingredient":"#missing:tag"});
    assert_eq!(unknown(file), None);
}

#[test]
fn nested_neoforge_kinds_are_checked() {
    for ingredient in [
        json!({"type":"neoforge:compound","children":["#a:b","nested:one"]}),
        json!({"type":"neoforge:intersection","children":["nested:one"]}),
        json!({"type":"neoforge:difference","base":"nested:one","subtracted":"#a:b"}),
        json!({"type":"neoforge:custom_display","base":"nested:one","display":{}}),
        json!({"type":"neoforge:components","items":"nested:one"}),
    ] {
        let file = json!({"type":"minecraft:stonecutting","ingredient":ingredient});
        assert_eq!(unknown(file).as_deref(), Some("unknown item nested:one"));
    }
}

#[test]
fn a_mod_recipe_type_stays_opaque() {
    let file = json!({"type":"mod:custom","ingredient":"missing:ore","result":{"id":"missing:x"}});
    assert_eq!(unknown(file), None);
}
