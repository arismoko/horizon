use super::*;

#[test]
fn weapon_component_recipes_decode_defaults_and_match_live_borrowed_values() {
    use serde_json::json;
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        crate::json_nbt::json_to_compound(
            json!({"minecraft:weapon":{"item_damage_per_attack":1,"extra":"ignored"}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let ingredient = |weapon| {
        json!({
            "type":"neoforge:components",
            "items":"minecraft:diamond_sword",
            "components":{"minecraft:weapon":weapon},
            "strict":true
        })
    };
    let recipe = |ingredient| {
        json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[ingredient],
            "result":{"id":"minecraft:diamond"}
        })
    };
    let load = load();
    let recipes = crate::crafting::Recipes::from_sources(
        &std::env::temp_dir(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![
            (
                "fixture:weapon_default".into(),
                recipe(ingredient(json!({}))),
            ),
            (
                "fixture:weapon_custom".into(),
                recipe(ingredient(json!({
                    "item_damage_per_attack":2.9,
                    "disable_blocking_for_seconds":1.25,
                    "unknown":[1,2]
                }))),
            ),
            (
                "fixture:weapon_json_zero".into(),
                // Weapon refuses negative zero before generic NBT conversion loses its sign.
                recipe(ingredient(json!({"disable_blocking_for_seconds":-0.0}))),
            ),
            (
                "fixture:weapon_removed".into(),
                recipe(json!({
                    "type":"neoforge:components",
                    "items":"minecraft:diamond_sword",
                    "components":{"!minecraft:weapon":{}}
                })),
            ),
            (
                "fixture:weapon_invalid".into(),
                recipe(ingredient(json!({
                    "disable_blocking_for_seconds":-1.0
                }))),
            ),
            (
                "fixture:weapon_wrong_type".into(),
                recipe(ingredient(json!({
                    "item_damage_per_attack":"1"
                }))),
            ),
        ],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 3);
    let context = load.loop_context(LoopSide::Server, []);
    let matches =
        |id, stack: &ItemStack| recipes.recipe_matches(&context, id, &[Some(stack.clone())], 1);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(matches("fixture:weapon_default", &sword));
    assert!(!matches("fixture:weapon_json_zero", &sword));
    assert!(!matches("fixture:weapon_custom", &sword));
    sword.data_mut().set_loot_component(
        "minecraft:weapon",
        &Some(Tag::compound([
            ("item_damage_per_attack", Tag::Long(4_294_967_298)),
            (
                "disable_blocking_for_seconds",
                Tag::Double(1.25f64.to_bits()),
            ),
            ("extra", Tag::Byte(9)),
        ])),
    );
    assert!(matches("fixture:weapon_custom", &sword));
    assert!(!matches("fixture:weapon_default", &sword));
    sword.data_mut().set_loot_component(
        "minecraft:weapon",
        &Some(Tag::compound([
            ("item_damage_per_attack", Tag::Byte(2)),
            ("disable_blocking_for_seconds", Tag::Float(1.5f32.to_bits())),
        ])),
    );
    assert!(!matches("fixture:weapon_custom", &sword));
    sword
        .data_mut()
        .set_loot_component("minecraft:weapon", &Some(Tag::Compound(Default::default())));
    assert!(matches("fixture:weapon_default", &sword));
    sword
        .data_mut()
        .set_loot_component("minecraft:custom_data", &Some(Tag::Int(7)));
    assert!(!matches("fixture:weapon_default", &sword));
    sword
        .data_mut()
        .set_loot_component("minecraft:weapon", &None);
    sword
        .data_mut()
        .set_loot_component("minecraft:custom_data", &None);
    sword.data_mut().set_loot_component(
        "!minecraft:weapon",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(!matches("fixture:weapon_default", &sword));
    assert!(matches("fixture:weapon_removed", &sword));
    assert!(!matches("fixture:weapon_invalid", &sword));
    assert!(!matches("fixture:weapon_wrong_type", &sword));
    let invalid = IngredientRule::Components {
        patch: [(
            "minecraft:weapon".into(),
            Tag::compound([(
                "disable_blocking_for_seconds",
                Tag::Float((-0.0f32).to_bits()),
            )]),
        )]
        .into_iter()
        .collect(),
        removed: Vec::new(),
        strict: false,
    };
    let error = super::components::validate(&invalid, &items).unwrap_err();
    assert!(error.starts_with("component minecraft:weapon: "));
}

#[test]
fn intangible_projectile_recipe_decodes_maps_and_matches_borrowed_units() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:arrow".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:intangible_projectile":{"default_field":false}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let root = tempfile::tempdir().unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:intangible_projectile".into(),
            serde_json::from_str(
                r#"{"type":"minecraft:smelting","ingredient":{"type":"neoforge:components","items":["minecraft:arrow","minecraft:stone"],"components":{"minecraft:intangible_projectile":{"ignored":[1,2]}},"strict":true},"result":{"id":"minecraft:diamond"},"experience":0.0,"cookingtime":20}"#,
            )
            .unwrap(),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.cooking.len(), 1);
    let ingredient = &recipes.cooking[0].ingredient;
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:intangible_projectile"),
        Some(&Tag::Compound(Default::default()))
    );
    let mut arrow = ItemStack::new("minecraft:arrow", 1);
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &arrow)
            .is_some()
    );
    let mut stone = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&context, &stone));
    stone.data_mut().set_other(
        "minecraft:intangible_projectile",
        Some(Tag::compound([(
            "different",
            Tag::String("ignored".into()),
        )])),
    );
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &stone)
            .is_some()
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stone)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stone
        .data_mut()
        .set_other("minecraft:intangible_projectile", Some(Tag::Byte(1)));
    assert!(!ingredient.test_in(&context, &stone));
    let absent = parse(
        r#"{"type":"neoforge:components","items":["minecraft:arrow","minecraft:stone"],"components":{"!minecraft:intangible_projectile":{}}}"#,
        &items,
    );
    assert!(!absent.test_in(&context, &arrow));
    let patch = Tag::compound([(
        "!minecraft:intangible_projectile",
        Tag::Compound(Default::default()),
    )]);
    (_, arrow.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(absent.test_in(&context, &arrow));
    assert!(!ingredient.test_in(&context, &arrow));
    assert!(
        recipes
            .cook(&context, "minecraft:smelting", &arrow)
            .is_none()
    );
    assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    let mut populated = ItemStack::new("minecraft:stone", 1);
    populated.data_mut().set_other(
        "minecraft:intangible_projectile",
        Some(Tag::Compound(Default::default())),
    );
    assert!(!absent.test_in(&context, &populated));
}

#[test]
fn intangible_projectile_invalid_payload_rejects_whole_recipe_with_component_error() {
    let items = Items::empty();
    let root = tempfile::tempdir().unwrap();
    let load = load();
    for value in [
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!(0),
        serde_json::json!("unit"),
        serde_json::json!([]),
    ] {
        let ingredient = serde_json::json!({
            "type":"neoforge:components",
            "items":"minecraft:arrow",
            "components":{"minecraft:intangible_projectile":value}
        });
        let error = components::validate_recipe_components(&ingredient, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:intangible_projectile:"),
            "{value}: {error}"
        );
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                (
                    "fixture:invalid_intangible_projectile".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless",
                        "ingredients":[ingredient],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
                (
                    "fixture:valid_companion".into(),
                    serde_json::json!({
                        "type":"minecraft:crafting_shapeless",
                        "ingredients":["minecraft:arrow"],
                        "result":{"id":"minecraft:diamond"}
                    }),
                ),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{value}");
        let context = load.loop_context(LoopSide::Server, []);
        let grid = [Some(ItemStack::new("minecraft:arrow", 1))];
        assert!(recipes.recipe_matches(&context, "fixture:valid_companion", &grid, 1));
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:invalid_intangible_projectile",
            &grid,
            1
        ));
        let indexed: Vec<_> = recipes
            .by_type
            .values()
            .flat_map(|recipes| recipes.iter())
            .map(|recipe| recipe.id.as_str())
            .collect();
        assert_eq!(indexed, ["fixture:valid_companion"], "{value}");
    }
}
#[test]
#[ignore = "requires prepared resources"]
fn components_removal_requires_absence_including_defaults() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"!minecraft:enchantments":{}}}"#,
        &items,
    );
    assert!(!ingredient.shown.as_ref().unwrap()[0].carries_component(
        items.get("minecraft:diamond_sword"),
        "minecraft:enchantments"
    ));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!ingredient.test_in(&context, &sword));
    let patch = Tag::compound([("!minecraft:enchantments", Tag::Compound(Default::default()))]);
    (_, sword.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(ingredient.test_in(&context, &sword));
    sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.data_mut().set_loot_component(
        "minecraft:enchantments",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(!ingredient.test_in(&context, &sword));
}
#[test]
#[ignore = "requires prepared resources"]
fn block_tag_logs_uses_block_items_and_rejects_hash() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:block_tag","tag":"minecraft:logs"}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    assert!(ingredient.test_in(&context, &ItemStack::new("minecraft:oak_log", 1)));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
    assert!(ingredient.is_simple());
    let mut alias_items = items.clone();
    alias_items.block_tag_order.insert(
        "fixture:wall_signs".into(),
        vec!["minecraft:oak_wall_sign".into()],
    );
    let wall = parse(
        r#"{"type":"neoforge:block_tag","tag":"fixture:wall_signs"}"#,
        &alias_items,
    );
    assert_eq!(wall.order, ["minecraft:oak_sign"]);
    assert!(
        Ingredient::parse(
            &serde_json::json!({"type":"neoforge:block_tag","tag":"#minecraft:logs"}),
            &items
        )
        .is_none()
    );
}
#[test]
fn custom_display_matches_base_but_shows_another_item() {
    let items = Items::empty();
    let ingredient = parse(
        r#"{"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    assert!(ingredient.test_in(&context, &ItemStack::new("minecraft:apple", 1)));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:diamond", 1)));
    assert!(ingredient.is_simple());
    assert_eq!(
        crate::recipe_book::Book::default().items_of(&ingredient),
        ["minecraft:diamond"]
    );
    assert_eq!(ingredient.order, ["minecraft:apple"]);
    let placement = crate::crafting::Placement::of(vec![Some(ingredient)], None);
    assert_eq!(placement.ingredients[0].order, ["minecraft:apple"]);
    // 26.3 TagSlotDisplay carries an item HolderSet, including the # prefix.
    let tags = Items::for_tests(std::collections::HashMap::from([(
        "minecraft:planks".into(),
        std::collections::HashSet::from(["minecraft:oak_planks".into()]),
    )]));
    let tagged = parse(
        r##"{"type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:tag","tag":"#minecraft:planks"}}"##,
        &tags,
    );
    assert_eq!(
        crate::recipe_book::Book::default().items_of(&tagged),
        ["minecraft:oak_planks"]
    );
}
#[test]
fn simplicity_follows_all_children_and_custom_display_base() {
    let items = Items::empty();
    for source in [
        r#"{"type":"neoforge:compound","children":["minecraft:apple","minecraft:stone"]}"#,
        r#"{"type":"neoforge:intersection","children":["minecraft:apple","minecraft:apple"]}"#,
        r#"{"type":"neoforge:difference","base":["minecraft:apple","minecraft:stone"],"subtracted":"minecraft:stone"}"#,
    ] {
        assert!(parse(source, &items).is_simple());
    }
    let source = r#"{"type":"neoforge:custom_display","base":{"type":"neoforge:components","items":"minecraft:apple","components":{}},"display":{"type":"minecraft:item","item":"minecraft:diamond"}}"#;
    assert!(!parse(source, &items).is_simple());
    let mut custom = Ingredient::of(vec!["minecraft:apple".into()]);
    custom.custom = Some(("fixture:custom".into(), Default::default()));
    assert!(!custom.is_simple());
}

#[test]
fn builtin_kinds_bypass_mod_resolution_and_honor_type_key_priority() {
    let load = load();
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}"#,
        r#"{"type":"neoforge:block_tag","tag":"minecraft:logs"}"#,
        r#"{"type":"fixture:wrong","neoforge:ingredient_type":"neoforge:custom_display","base":"minecraft:apple","display":{"type":"minecraft:item","item":"minecraft:diamond"}}"#,
    ] {
        let mut json: Value = serde_json::from_str(source).unwrap();
        let original = json.clone();
        assert!(!Ingredient::resolve(&mut json, &load));
        assert_eq!(json, original);
    }
}

#[test]
fn recipe_book_and_placement_use_component_ingredient_item_lists() {
    use crate::recipe_book::{Book, Category, Display, Entry};
    let items = Items::empty();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}"#,
        &items,
    );
    let entry = Entry {
        id: "fixture:components".into(),
        display_id: -1,
        category: Category::CraftingEquipment,
        group: None,
        display: Display::Shapeless(vec![ingredient.clone()]),
        result: ItemStack::new("minecraft:diamond", 1),
        variants: Vec::new(),
        stacks: Vec::new(),
    };
    let mut book = Book::default();
    let placement = crate::crafting::Placement::of(vec![Some(ingredient)], None);
    book.placements.insert(entry.id.clone(), placement.clone());
    let mut stock = crate::menu::place::Stacked::default();
    // An undamaged sword cannot satisfy the concrete recipe, but the book and
    // placement intentionally account only for its item holder.
    stock.account(&std::sync::Arc::from("minecraft:diamond_sword"), 1);
    assert!(book.can_craft(&entry, &stock));
    assert!(stock.can_craft(&placement.ingredients.iter().collect::<Vec<_>>()));
    assert_eq!(
        book.items_of(&placement.ingredients[0]),
        ["minecraft:diamond_sword"]
    );
}

#[test]
fn components_enchantable_decodes_positive_records_and_matches_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        horizon_core::nbt::Compound::from([(
            "minecraft:enchantable".into(),
            Tag::compound([
                ("value", Tag::Short(15)),
                ("ignored", Tag::String("prototype".into())),
            ]),
        )]),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{"value":15}}}"#,
        &items,
    );
    let normalized = horizon_command::item::decoded_component_value(
        "minecraft:enchantable",
        &Tag::compound([
            ("value", Tag::Double(15.9f64.to_bits())),
            ("ignored", Tag::String("not part of the record".into())),
        ]),
        &items.component_registry,
    )
    .unwrap();
    assert_eq!(normalized, Tag::compound([("value", Tag::Int(15))]));

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{"value":0}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{"value":-1}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{"value":"15"}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":15}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none(), "{source}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:enchantable"), "{error}");
    }

    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(
        ingredient.test_in(&context, &sword),
        "borrowed item default"
    );
    for value in [
        Tag::Byte(15),
        Tag::Short(15),
        Tag::Int(15),
        Tag::Long(4_294_967_311),
        Tag::Float(15.9f32.to_bits()),
        Tag::Double(15.9f64.to_bits()),
    ] {
        sword.data_mut().set_other(
            "minecraft:enchantable",
            Some(Tag::compound([
                ("value", value),
                ("ignored", Tag::String("live".into())),
            ])),
        );
        assert!(ingredient.test_in(&context, &sword));
    }
    sword.data_mut().set_other(
        "minecraft:enchantable",
        Some(Tag::compound([("value", Tag::Int(16))])),
    );
    assert!(!ingredient.test_in(&context, &sword));
    sword.data_mut().set_other(
        "minecraft:enchantable",
        Some(Tag::compound([("value", Tag::Int(0))])),
    );
    assert!(!ingredient.test_in(&context, &sword));

    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"!minecraft:enchantable":{}}}"#;
    let absence = parse(absence_source, &items);
    let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_other(
        "!minecraft:enchantable",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!ingredient.test_in(&context, &removed));
    let no_defaults = Items::empty();
    let bare = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(parse(absence_source, &no_defaults).test_in(&context, &bare));
    let positive_without_default = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:enchantable":{"value":15}}}"#,
        &no_defaults,
    );
    assert!(!positive_without_default.test_in(&context, &bare));
}

#[test]
fn item_model_component_literals_decode_validate_and_match() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:item_model":"unregistered/model"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"unregistered/model"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"minecraft:unregistered/model"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":":unregistered/model"}}"#,
    ] {
        let ingredient = parse(source, &items);
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .other("minecraft:item_model"),
            Some(&Tag::String("minecraft:unregistered/model".into())),
        );
        let mut stack = ItemStack::new("minecraft:stick", 1);
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        for model in [
            "unregistered/model",
            "minecraft:unregistered/model",
            ":unregistered/model",
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:item_model", &Some(Tag::String(model.into())));
            let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
                assert!(ingredient.test_in(&context, &stack));
            });
            assert_eq!(allocations.calls, 0, "{allocations:?}");
        }
        for model in [
            "minecraft:other",
            "fixture:unregistered/model",
            "Minecraft:unregistered/model",
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:item_model", &Some(Tag::String(model.into())));
            assert!(!ingredient.test_in(&context, &stack));
        }
    }
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"Minecraft:stick"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"minecraft:Stick"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"bad namespace:stick"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"..:stick"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":"a:b:c"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:item_model":5}}"#,
    ] {
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&value, &items).is_none());
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(value["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("component minecraft:item_model:"), "{error}");
    }
    for model in ["", ":", "minecraft:"] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:item_model",
                &Tag::String(model.into()),
                &items.component_registry,
            )
            .unwrap(),
            Tag::String("minecraft:".into()),
        );
    }
    for units in [vec![0xd800], vec![0xdc00], vec![0xd83d, 0xde00]] {
        let invalid_utf16 = Tag::String(horizon_core::nbt::JavaString(units));
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:item_model",
                &invalid_utf16,
                &items.component_registry,
            )
            .is_err()
        );
    }
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:stick","minecraft:iron_ingot"],"components":{"!minecraft:item_model":{}}}"#,
        &items,
    );
    let positive = parse(
        r#"{"type":"neoforge:components","items":["minecraft:stick","minecraft:iron_ingot"],"components":{"minecraft:item_model":"unregistered/model"}}"#,
        &items,
    );
    let mut removed = ItemStack::new("minecraft:stick", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:item_model",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!positive.test_in(&context, &removed));
    let mut populated = ItemStack::new("minecraft:stick", 1);
    populated.data_mut().set_loot_component(
        "minecraft:item_model",
        &Some(Tag::String("unregistered/model".into())),
    );
    assert!(!absence.test_in(&context, &populated));
    let no_default = ItemStack::new("minecraft:iron_ingot", 1);
    assert!(absence.test_in(&context, &no_default));
    assert!(!positive.test_in(&context, &no_default));
}

#[test]
fn enchantment_glint_override_decodes_boolean_literals_and_borrowed_numeric_values() {
    const COMPONENT: &str = "minecraft:enchantment_glint_override";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:apple".into(),
        std::collections::BTreeMap::from([(COMPONENT.into(), Tag::Short(256))]),
    )]));
    let enabled = parse(
        r#"{"type":"neoforge:components","items":"minecraft:apple","components":{"minecraft:enchantment_glint_override":true}}"#,
        &items,
    );
    let disabled = parse(
        r#"{"type":"neoforge:components","items":"minecraft:apple","components":{"minecraft:enchantment_glint_override":false}}"#,
        &items,
    );
    for (ingredient, expected) in [(&enabled, Tag::Byte(1)), (&disabled, Tag::Byte(0))] {
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .other(COMPONENT),
            Some(&expected),
        );
    }
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut candidate = ItemStack::new("minecraft:apple", 1);
    assert!(
        enabled.test_in(&context, &candidate),
        "inherited numeric default"
    );
    assert!(!disabled.test_in(&context, &candidate));

    // NbtOps.getBooleanValue uses Number.doubleValue, not Tag.asBoolean's byteValue.
    for (value, expected) in [
        (Tag::Byte(-1), true),
        (Tag::Short(256), true),
        (Tag::Int(256), true),
        (Tag::Long(4_294_967_296), true),
        (Tag::Float(0.5_f32.to_bits()), true),
        (Tag::Double(0.5_f64.to_bits()), true),
        (Tag::Float(f32::NAN.to_bits()), true),
        (Tag::Double(f64::NAN.to_bits()), true),
        (Tag::Double(f64::INFINITY.to_bits()), true),
        (Tag::Double(f64::from_bits(1).to_bits()), true),
        (Tag::Byte(0), false),
        (Tag::Short(0), false),
        (Tag::Int(0), false),
        (Tag::Long(0), false),
        (Tag::Float((-0.0_f32).to_bits()), false),
        (Tag::Double((-0.0_f64).to_bits()), false),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                COMPONENT,
                &value,
                &items.component_registry,
            )
            .unwrap(),
            Tag::Byte(i8::from(expected)),
        );
        candidate
            .data_mut()
            .set_loot_component(COMPONENT, &Some(value));
        assert_eq!(enabled.test_in(&context, &candidate), expected);
        assert_eq!(disabled.test_in(&context, &candidate), !expected);
    }
    let numeric = parse(
        r#"{"type":"neoforge:components","items":"minecraft:apple","components":{"minecraft:enchantment_glint_override":0.5}}"#,
        &items,
    );
    candidate
        .data_mut()
        .set_loot_component(COMPONENT, &Some(Tag::Int(256)));
    assert!(numeric.test_in(&context, &candidate));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(enabled.test_in(&context, std::hint::black_box(&candidate)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    candidate.data_mut().set_loot_component(COMPONENT, &None);
    assert!(
        enabled.test_in(&context, &candidate),
        "default is inherited again"
    );
    candidate.data_mut().set_loot_component(
        "!minecraft:enchantment_glint_override",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(
        !enabled.test_in(&context, &candidate),
        "removed, not inherited"
    );
    assert!(
        !disabled.test_in(&context, &candidate),
        "absent is not false"
    );

    let invalid: Value = serde_json::from_str(
        r#"{"type":"neoforge:components","items":"minecraft:apple","components":{"minecraft:enchantment_glint_override":"true"}}"#,
    )
    .unwrap();
    assert!(Ingredient::parse(&invalid, &items).is_none());
    let rule = IngredientRule::Components {
        patch: crate::json_nbt::json_to_compound(invalid["components"].as_object().unwrap())
            .unwrap(),
        removed: Vec::new(),
        strict: false,
    };
    assert_eq!(
        components::validate(&rule, &items).unwrap_err(),
        "component minecraft:enchantment_glint_override: Not a number",
    );
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:apple","minecraft:stick"],"components":{"!minecraft:enchantment_glint_override":{}}}"#,
        &items,
    );
    let mut removed = ItemStack::new("minecraft:apple", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:enchantment_glint_override",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!enabled.test_in(&context, &removed));
    assert!(!disabled.test_in(&context, &removed));
    let mut populated = ItemStack::new("minecraft:apple", 1);
    populated
        .data_mut()
        .set_loot_component(COMPONENT, &Some(Tag::Byte(0)));
    assert!(!absence.test_in(&context, &populated));
    assert!(absence.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
}

#[test]
fn unbreakable_unit_recipe_matches_presence_defaults_and_removal() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:unbreakable":{"ignored":"default"}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let source: Value = serde_json::from_str(
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:stone"],"components":{"minecraft:unbreakable":{"show_in_tooltip":false,"unknown":[1,2]}},"strict":true}],"result":{"id":"minecraft:diamond"}}"#,
    )
    .unwrap();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![("fixture:unbreakable".into(), source)],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let matches = |stack: ItemStack| {
        recipes.recipe_matches(
            &context,
            "fixture:unbreakable",
            &[Some(stack), None, None, None],
            2,
        )
    };
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(matches(sword.clone()));
    sword.data_mut().set_other(
        "!minecraft:unbreakable",
        Some(Tag::Compound(Default::default())),
    );
    assert!(!matches(sword.clone()));
    let removal = parse(
        r#"{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:stone"],"components":{"!minecraft:unbreakable":{}}}"#,
        &items,
    );
    assert!(removal.test_in(&context, &sword));
    assert!(!removal.test_in(&context, &ItemStack::new("minecraft:diamond_sword", 1)));
    let mut stone = ItemStack::new("minecraft:stone", 3);
    assert!(removal.test_in(&context, &stone));
    assert!(!matches(stone.clone()));
    stone.data_mut().set_other(
        "minecraft:unbreakable",
        Some(Tag::compound([(
            "unrecognized",
            Tag::String("live".into()),
        )])),
    );
    assert!(!removal.test_in(&context, &stone));
    assert!(matches(stone.clone()));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":{"ignored":false}}}"#,
        &items,
    );
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:unbreakable"),
        Some(&Tag::Compound(Default::default()))
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stone)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    stone
        .data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(1)));
    assert!(!matches(stone.clone()));
    stone.count = 0;
    assert!(!ingredient.test_in(&context, &stone));
}

#[test]
fn unbreakable_unit_recipe_rejects_non_maps_before_publication() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for payload in [r#"true"#, "false", "null", "0", r#""unit""#, "[]", "[{}]"] {
        let mut source: Value = serde_json::from_str(
            r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:unbreakable":true}}],"result":{"id":"minecraft:diamond"}}"#,
        )
        .unwrap();
        source["ingredients"][0]["components"]["minecraft:unbreakable"] =
            serde_json::from_str(payload).unwrap();
        assert!(Ingredient::parse(&source["ingredients"][0], &items).is_none());
        let error =
            components::validate_recipe_components(&source["ingredients"][0], &items).unwrap_err();
        assert!(
            error.contains("minecraft:unbreakable"),
            "{payload}: {error}"
        );
        let companion = serde_json::json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": ["minecraft:stone"],
            "result": {"id": "minecraft:diamond"}
        });
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid_unbreakable".into(), source),
                ("fixture:valid_companion".into(), companion),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{payload}");
        let context = load.loop_context(LoopSide::Server, []);
        let grid = [Some(ItemStack::new("minecraft:stone", 1)), None, None, None];
        assert!(
            recipes.recipe_matches(&context, "fixture:valid_companion", &grid, 2),
            "{payload}"
        );
        assert!(
            !recipes.recipe_matches(&context, "fixture:invalid_unbreakable", &grid, 2),
            "{payload}"
        );
        let indexed: Vec<_> = recipes
            .by_type
            .values()
            .flat_map(|recipes| recipes.iter())
            .map(|recipe| recipe.id.as_str())
            .collect();
        assert_eq!(indexed, ["fixture:valid_companion"], "{payload}");
    }
}

#[test]
fn glider_component_recipe_normalizes_maps_and_matches_borrowed_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:elytra".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:glider":{"ignored_default":false}})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let source = r#"{"type":"neoforge:components","items":"minecraft:elytra","components":{"minecraft:glider":{"ignored_patch":[1,2]}},"strict":true}"#;
    let ingredient = parse(source, &items);
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:glider".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[serde_json::from_str::<Value>(source).unwrap()],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let default = ItemStack::new("minecraft:elytra", 1);
    let mut explicit = default.clone();
    explicit.data_mut().set_other(
        "minecraft:glider",
        Some(Tag::compound([(
            "ignored_candidate",
            Tag::String("anything".into()),
        )])),
    );
    let mut removed = default.clone();
    (_, removed.extra) = crate::item_data::decode_components(Some(&Tag::compound([(
        "!minecraft:glider",
        Tag::Int(9),
    )])));
    let mut malformed = default.clone();
    malformed
        .data_mut()
        .set_other("minecraft:glider", Some(Tag::Byte(1)));
    for stack in [&default, &explicit] {
        assert!(recipes.recipe_matches(&context, "fixture:glider", &[Some(stack.clone())], 1));
    }
    for stack in [&removed, &malformed] {
        assert!(!recipes.recipe_matches(&context, "fixture:glider", &[Some(stack.clone())], 1));
    }
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:elytra","components":{"!minecraft:glider":123}}"#,
        &items,
    );
    assert!(!absent.test_in(&context, &default));
    assert!(!absent.test_in(&context, &explicit));
    assert!(absent.test_in(&context, &removed));
    let absent_without_default = parse(
        r#"{"type":"neoforge:components","items":"minecraft:elytra","components":{"!minecraft:glider":123}}"#,
        &Items::empty(),
    );
    assert!(absent_without_default.test_in(&context, &default));
    assert!(!absent_without_default.test_in(&context, &explicit));
    let without_default = parse(source, &Items::empty());
    assert!(!without_default.test_in(&context, &default));
    assert!(without_default.test_in(&context, &explicit));
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:glider"),
        Some(&Tag::Compound(Default::default()))
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(ingredient.test_in(&context, &default)));
            assert!(std::hint::black_box(
                ingredient.test_in(&context, &explicit)
            ));
            assert!(!std::hint::black_box(
                ingredient.test_in(&context, &removed)
            ));
            assert!(!std::hint::black_box(
                ingredient.test_in(&context, &malformed)
            ));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
}

#[test]
fn glider_component_malformed_payloads_name_component_and_omit_recipe() {
    let items = Items::empty();
    let root = tempfile::tempdir().unwrap();
    let load = load();
    for payload in [
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!(0),
        serde_json::json!("unit"),
        serde_json::json!([]),
    ] {
        let recipe = serde_json::json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{
                "type":"neoforge:components",
                "items":"minecraft:elytra",
                "components":{"minecraft:glider":payload}
            }],
            "result":{"id":"minecraft:diamond"}
        });
        let error =
            components::validate_recipe_components(&recipe["ingredients"][0], &items).unwrap_err();
        assert!(
            error.contains("component minecraft:glider:"),
            "{payload}: {error}"
        );
        assert!(Ingredient::parse(&recipe["ingredients"][0], &items).is_none());
        let companion = serde_json::json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": ["minecraft:elytra"],
            "result": {"id": "minecraft:diamond"}
        });
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid_glider".into(), recipe),
                ("fixture:valid_companion".into(), companion),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{payload}");
        let context = load.loop_context(LoopSide::Server, []);
        let grid = [Some(ItemStack::new("minecraft:elytra", 1))];
        assert!(
            recipes.recipe_matches(&context, "fixture:valid_companion", &grid, 1),
            "{payload}"
        );
        assert!(
            !recipes.recipe_matches(&context, "fixture:invalid_glider", &grid, 1),
            "{payload}"
        );
        let indexed: Vec<_> = recipes
            .by_type
            .values()
            .flat_map(|recipes| recipes.iter())
            .map(|recipe| recipe.id.as_str())
            .collect();
        assert_eq!(indexed, ["fixture:valid_companion"], "{payload}");
    }
}

#[test]
fn waxed_recipe_decodes_unit_maps_and_matches_borrowed_values() {
    let items = Items::empty();
    let source = r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{"ignored":{"nested":true}}}}"#;
    let ingredient = parse(source, &items);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:waxed"),
        Some(&Tag::compound([] as [(&str, Tag); 0]))
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:waxed".into(),
            serde_json::from_str(
                r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{"unknown":42}}}],"result":{"id":"minecraft:diamond"}}"#,
            )
            .unwrap(),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack.clone()), None, None, None], 2)
            .is_none()
    );
    for payload in [
        Tag::compound([] as [(&str, Tag); 0]),
        Tag::compound([("different", Tag::List(vec![Tag::Int(7)]))]),
    ] {
        stack.data_mut().set_other("minecraft:waxed", Some(payload));
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(std::hint::black_box(ingredient.test_in(&context, &stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        assert_eq!(
            recipes
                .craft(&context, &[Some(stack.clone()), None, None, None], 2)
                .unwrap()
                .item
                .as_ref(),
            "minecraft:diamond"
        );
    }
    stack
        .data_mut()
        .set_other("minecraft:waxed", Some(Tag::Byte(1)));
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack.clone()), None, None, None], 2)
            .is_none()
    );
    stack.data_mut().set_other("minecraft:waxed", None);
    stack.data_mut().set_other(
        "!minecraft:waxed",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!ingredient.test_in(&context, &stack));
    assert!(
        recipes
            .craft(&context, &[Some(stack), None, None, None], 2)
            .is_none()
    );
}

#[test]
fn waxed_defaults_and_removal_preserve_presence() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        horizon_core::nbt::Compound::from([(
            "minecraft:waxed".into(),
            Tag::compound([("ignored", Tag::Int(9))]),
        )]),
    )]));
    let present = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:waxed":{}},"strict":true}"#,
        &items,
    );
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:waxed":{}}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut stack = ItemStack::new("minecraft:stone", 1);
    assert!(present.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &stack));
    stack.data_mut().set_other(
        "!minecraft:waxed",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(!present.test_in(&context, &stack));
    assert!(absent.test_in(&context, &stack));
    let plain_absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:waxed":{}}}"#,
        &Items::empty(),
    );
    let mut populated = ItemStack::new("minecraft:stone", 1);
    assert!(plain_absent.test_in(&context, &populated));
    populated.data_mut().set_other(
        "minecraft:waxed",
        Some(Tag::compound([("ignored", Tag::Int(3))])),
    );
    assert!(!plain_absent.test_in(&context, &populated));
}

#[test]
fn waxed_malformed_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let root = tempfile::tempdir().unwrap();
    let load = load();
    for payload in [
        serde_json::json!(true),
        serde_json::json!(false),
        serde_json::json!(null),
        serde_json::json!(0),
        serde_json::json!("unit"),
        serde_json::json!([]),
    ] {
        let recipe = serde_json::json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{
                "type":"neoforge:components",
                "items":"minecraft:stone",
                "components":{"minecraft:waxed":payload}
            }],
            "result":{"id":"minecraft:diamond"}
        });
        let error =
            components::validate_recipe_components(&recipe["ingredients"][0], &items).unwrap_err();
        assert!(
            error.contains("component minecraft:waxed:"),
            "{payload}: {error}"
        );
        assert!(Ingredient::parse(&recipe["ingredients"][0], &items).is_none());
        let companion = serde_json::json!({
            "type": "minecraft:crafting_shapeless",
            "ingredients": ["minecraft:stone"],
            "result": {"id": "minecraft:diamond"}
        });
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid_waxed".into(), recipe),
                ("fixture:valid_companion".into(), companion),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{payload}");
        let context = load.loop_context(LoopSide::Server, []);
        let grid = [Some(ItemStack::new("minecraft:stone", 1))];
        assert!(
            recipes.recipe_matches(&context, "fixture:valid_companion", &grid, 1),
            "{payload}"
        );
        assert!(
            !recipes.recipe_matches(&context, "fixture:invalid_waxed", &grid, 1),
            "{payload}"
        );
        let indexed: Vec<_> = recipes
            .by_type
            .values()
            .flat_map(|recipes| recipes.iter())
            .map(|recipe| recipe.id.as_str())
            .collect();
        assert_eq!(indexed, ["fixture:valid_companion"], "{payload}");
    }
}

#[test]
fn tooltip_style_recipe_decodes_identifiers_and_matches_borrowed_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stone".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:tooltip_style":":style"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    for (value, valid) in [
        (serde_json::json!("style"), true),
        (serde_json::json!(":style"), true),
        (serde_json::json!("fixture:"), true),
        (serde_json::json!(""), true),
        (serde_json::json!(":"), true),
        (serde_json::json!("UPPER"), false),
        (serde_json::json!("..:style"), false),
        (serde_json::json!("a:b:c"), false),
        (serde_json::json!(3), false),
    ] {
        let source = serde_json::json!({
            "type":"minecraft:crafting_shapeless",
            "ingredients":[{"type":"neoforge:components","items":"minecraft:stone",
                "components":{"minecraft:tooltip_style":value}}],
            "result":{"id":"minecraft:diamond"}
        });
        if !valid {
            let error = components::validate_recipe_components(&source["ingredients"][0], &items)
                .unwrap_err();
            assert!(
                error.contains("component minecraft:tooltip_style:"),
                "{error}"
            );
        }
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![("fixture:tooltip_style".into(), source)],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), usize::from(valid), "{value}");
        if !valid {
            continue;
        }
        let crate::crafting::Recipe::Shapeless { ingredients, .. } = &recipes.crafting[0].1 else {
            panic!("expected shapeless recipe");
        };
        let ingredient = &ingredients[0];
        let mut stack = ItemStack::new("minecraft:stone", 1);
        assert_eq!(
            ingredient.test_in(&context, &stack),
            value == "style" || value == ":style"
        );
        stack.data_mut().set_other(
            "minecraft:tooltip_style",
            Some(crate::crafting::json_tag(&value)),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0);
        assert_eq!(allocations.bytes, 0);
        stack.data_mut().set_other(
            "minecraft:tooltip_style",
            Some(Tag::String("fixture:different".into())),
        );
        assert!(!ingredient.test_in(&context, &stack));
        let absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:tooltip_style":{}}}"#,
            &items,
        );
        assert!(!absent.test_in(&context, &stack));
        assert!(!absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
        let plain_absent = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:tooltip_style":{}}}"#,
            &Items::empty(),
        );
        let plain_positive = parse(
            r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"minecraft:tooltip_style":"style"}}"#,
            &Items::empty(),
        );
        let plain = ItemStack::new("minecraft:stone", 1);
        assert!(plain_absent.test_in(&context, &plain));
        assert!(!plain_positive.test_in(&context, &plain));
        stack
            .data_mut()
            .set_loot_component("minecraft:tooltip_style", &None);
        stack.data_mut().set_loot_component(
            "!minecraft:tooltip_style",
            &Some(Tag::Compound(Default::default())),
        );
        assert!(absent.test_in(&context, &stack));
        assert!(!ingredient.test_in(&context, &stack));
    }
}

#[test]
fn note_block_sound_component_recipe_literals_validate_and_match() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    for source in [
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":":unregistered/sound"}}],"result":{"id":"minecraft:diamond"}}"#,
    ] {
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:note_block_sound".into(),
                serde_json::from_str(source).unwrap(),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1, "{source}");
        let mut stack = ItemStack::new("minecraft:stick", 1);
        assert!(!recipes.recipe_matches(
            &context,
            "fixture:note_block_sound",
            &[Some(stack.clone())],
            1,
        ));
        for sound in [
            "unregistered/sound",
            "minecraft:unregistered/sound",
            ":unregistered/sound",
        ] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(recipes.recipe_matches(
                &context,
                "fixture:note_block_sound",
                &[Some(stack.clone())],
                1,
            ));
        }
        for sound in [
            "minecraft:other",
            "fixture:unregistered/sound",
            "Minecraft:unregistered/sound",
            "",
        ] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(!recipes.recipe_matches(
                &context,
                "fixture:note_block_sound",
                &[Some(stack.clone())],
                1,
            ));
        }
    }
    for source in [
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":false}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":null}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":42}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":{}}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"Minecraft:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:Sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"..:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"bad namespace:sound"}}],"result":{"id":"minecraft:diamond"}}"#,
        r#"{"type":"minecraft:crafting_shapeless","ingredients":[{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"a:b:c"}}],"result":{"id":"minecraft:diamond"}}"#,
    ] {
        let value: Value = serde_json::from_str(source).unwrap();
        let ingredient = &value["ingredients"][0];
        assert!(Ingredient::parse(ingredient, &items).is_none(), "{source}");
        let error = components::validate_recipe_components(ingredient, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:note_block_sound:"),
            "{error}"
        );
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![("fixture:invalid_note_block_sound".into(), value)],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty(), "{source}");
        assert!(
            recipes.by_type.values().all(|recipes| recipes.is_empty()),
            "{source}"
        );
    }
}

#[test]
fn note_block_sound_component_defaults_removal_and_strict_matching() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:note_block_sound":":unregistered/sound"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"unregistered/sound"},"strict":true}"#,
        &items,
    );
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .other("minecraft:note_block_sound"),
        Some(&Tag::String("minecraft:unregistered/sound".into())),
    );
    let mut stack = ItemStack::new("minecraft:stick", 1);
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(ingredient.test_in(&context, &stack));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for sound in [
        "unregistered/sound",
        "minecraft:unregistered/sound",
        ":unregistered/sound",
    ] {
        stack.data_mut().set_loot_component(
            "minecraft:note_block_sound",
            &Some(Tag::String(sound.into())),
        );
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            assert!(ingredient.test_in(&context, &stack));
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
    }
    stack
        .data_mut()
        .set_loot_component("minecraft:custom_name", &Some(Tag::String("Named".into())));
    assert!(!ingredient.test_in(&context, &stack));
    stack = ItemStack::new("minecraft:stick", 1);
    let removal = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"!minecraft:note_block_sound":{}}}"#,
        &items,
    );
    assert!(!removal.test_in(&context, &stack));
    let mut populated = ItemStack::new("minecraft:stick", 1);
    populated.data_mut().set_loot_component(
        "minecraft:note_block_sound",
        &Some(Tag::String("fixture:other".into())),
    );
    assert!(!removal.test_in(&context, &populated));
    let plain_removal = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"!minecraft:note_block_sound":{}}}"#,
        &Items::empty(),
    );
    assert!(plain_removal.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
    stack.data_mut().set_loot_component(
        "!minecraft:note_block_sound",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(!ingredient.test_in(&context, &stack));
    assert!(removal.test_in(&context, &stack));
    stack = ItemStack::new("minecraft:stick", 1);

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":""}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":":"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:note_block_sound":"minecraft:"}}"#,
    ] {
        let empty = parse(source, &items);
        assert!(!empty.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
        for sound in ["", ":", "minecraft:"] {
            stack.data_mut().set_loot_component(
                "minecraft:note_block_sound",
                &Some(Tag::String(sound.into())),
            );
            assert!(empty.test_in(&context, &stack));
        }
        for value in [
            Tag::Int(0),
            Tag::String("fixture:".into()),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
            Tag::String(horizon_core::nbt::JavaString(vec![0xdc00])),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd83d, 0xde00])),
        ] {
            stack
                .data_mut()
                .set_loot_component("minecraft:note_block_sound", &Some(value));
            assert!(!empty.test_in(&context, &stack));
        }
    }
}

#[test]
fn weapon_recipe_rejections_preserve_siblings_and_presence_strictness() {
    use serde_json::json;
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let source = |payload| {
        json!({"type":"neoforge:components","items":"minecraft:paper",
        "components":{"minecraft:weapon":payload},"strict":true})
    };
    let recipe = |ingredient| json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
    for payload in [
        json!(true),
        json!(false),
        json!(null),
        json!("weapon"),
        json!([]),
        json!({"item_damage_per_attack":"1"}),
        json!({"disable_blocking_for_seconds":false}),
        json!({"disable_blocking_for_seconds":-0.0}),
    ] {
        let invalid = source(payload);
        let error = components::validate_recipe_components(&invalid, &items).unwrap_err();
        assert!(error.contains("minecraft:weapon"), "{error}");
        let recipes = crate::crafting::Recipes::from_sources(
            &std::env::temp_dir(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![
                ("fixture:invalid".into(), recipe(invalid)),
                ("fixture:valid".into(), recipe(json!("minecraft:stick"))),
            ],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:stick", 1))], 1)
                .is_some()
        );
        assert!(
            recipes
                .craft(&context, &[Some(ItemStack::new("minecraft:paper", 1))], 1)
                .is_none()
        );
    }
    for value in [Tag::End, Tag::Byte(0), Tag::List(vec![])] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([("minecraft:weapon".into(), value)]),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:weapon"), "{error}");
    }
    let mut value = source(json!({"ignored":true}));
    let strict = Ingredient::parse(&value, &items).unwrap();
    value["strict"] = json!(false);
    let relaxed = Ingredient::parse(&value, &items).unwrap();
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:paper","components":{"!minecraft:weapon":{}}}"#,
        &items,
    );
    let mut paper = ItemStack::new("minecraft:paper", 1);
    assert!(!strict.test_in(&context, &paper));
    assert!(absent.test_in(&context, &paper));
    paper.data_mut().set_loot_component(
        "minecraft:weapon",
        &Some(Tag::compound([("unknown", Tag::Int(7))])),
    );
    assert!(strict.test_in(&context, &paper));
    assert!(!absent.test_in(&context, &paper));
    paper
        .data_mut()
        .set_other("minecraft:custom_data", Some(Tag::Int(1)));
    assert!(!strict.test_in(&context, &paper));
    assert!(relaxed.test_in(&context, &paper));
    paper
        .data_mut()
        .set_loot_component("minecraft:weapon", &None);
    paper.data_mut().set_loot_component(
        "!minecraft:weapon",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absent.test_in(&context, &paper));
    assert!(!relaxed.test_in(&context, &paper));
}
