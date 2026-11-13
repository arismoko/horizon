use super::*;
use horizon_core::nbt::Tag;
use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};

fn load() -> LoadContext {
    LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        std::sync::Arc::new(LoadInputs::new(Default::default(), Default::default())),
        [],
    )
}
fn parse(source: &str, items: &Items) -> Ingredient {
    Ingredient::parse(&serde_json::from_str(source).unwrap(), items).unwrap()
}
fn resources() -> Items {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("prepared resources"),
    );
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    Items::catalog(&root, &horizon_world::loot::PackOverlay::NONE, &defaults).unwrap()
}
#[test]
fn dye_components_compile_recipes_match_live_values_and_preserve_defaults() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:red_dye".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:dye":"red"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:red_dye","components":{"minecraft:dye":"red"},"strict":true}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:dye".into(),
            serde_json::json!({
                "type":"minecraft:crafting_shapeless",
                "ingredients":[{
                    "type":"neoforge:components", "items":"minecraft:red_dye",
                    "components":{"minecraft:dye":"red"}, "strict":true
                }],
                "result":{"id":"minecraft:diamond"}
            }),
        )],
        &load,
    )
    .unwrap();
    assert_eq!(recipes.crafting.len(), 1);
    let mut stack = ItemStack::new("minecraft:red_dye", 1);
    assert!(
        ingredient.test_in(&context, &stack),
        "prototype dye default"
    );
    assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_some());
    stack
        .data_mut()
        .set_loot_component("minecraft:dye", &Some(Tag::String("red".into())));
    assert_eq!(
        stack.data().unwrap().other("minecraft:dye"),
        Some(&Tag::String("red".into()))
    );
    assert!(ingredient.test_in(&context, &stack));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    for value in [
        Tag::String("blue".into()),
        Tag::String("RED".into()),
        Tag::Int(14),
    ] {
        stack
            .data_mut()
            .set_loot_component("minecraft:dye", &Some(value));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
    }
    let removal = Tag::compound([("!minecraft:dye", Tag::Compound(Default::default()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
    assert!(!ingredient.test_in(&context, &stack));
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:red_dye","components":{"!minecraft:dye":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, &stack));
    assert!(!absent.test_in(&context, &ItemStack::new("minecraft:red_dye", 1)));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:blue_dye", 1)));
}

#[test]
fn malformed_dye_payloads_reject_whole_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("RED"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:red"),
        serde_json::json!(14),
        serde_json::json!(true),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:dye":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("component minecraft:dye:"), "{error}");
        let ingredient = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:red_dye", "components":patch
        });
        assert!(Ingredient::parse(&ingredient, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_dye".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[ingredient],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn shulker_color_components_compile_match_and_distinguish_absence_from_defaults() {
    // DataComponents.SHULKER_COLOR uses DyeColor.CODEC, not the mob's uncolored sentinel.
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:shulker_spawn_egg".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:shulker/color":"purple"})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    // Literal oracle names; do not derive expected acceptance from the decoder palette.
    for color in [
        "white",
        "orange",
        "magenta",
        "light_blue",
        "yellow",
        "lime",
        "pink",
        "gray",
        "light_gray",
        "cyan",
        "purple",
        "blue",
        "brown",
        "green",
        "red",
        "black",
    ] {
        let source = serde_json::json!({
            "type":"neoforge:components",
            "items":["minecraft:shulker_spawn_egg", "minecraft:stone"],
            "components":{"minecraft:shulker/color":color},
            "strict":true
        });
        let ingredient = Ingredient::parse(&source, &items).unwrap();
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:shulker_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert_eq!(recipes.crafting.len(), 1);
        let mut stack = ItemStack::new("minecraft:shulker_spawn_egg", 1);
        assert_eq!(ingredient.test_in(&context, &stack), color == "purple");
        assert_eq!(
            recipes.craft(&context, &[Some(stack.clone())], 1).is_some(),
            color == "purple"
        );
        let mut no_default = ItemStack::new("minecraft:stone", 1);
        assert!(!ingredient.test_in(&context, &no_default));
        assert!(
            recipes
                .craft(&context, &[Some(no_default.clone())], 1)
                .is_none()
        );
        let value = Tag::String(color.into());
        // Exact lowercase names are already canonical: no ordinal or namespace normalization.
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:shulker/color",
                &value,
                &Default::default()
            )
            .unwrap(),
            value
        );
        for candidate in [&mut stack, &mut no_default] {
            candidate
                .data_mut()
                .set_loot_component("minecraft:shulker/color", &Some(value.clone()));
            assert_eq!(
                candidate.data().unwrap().other("minecraft:shulker/color"),
                Some(&value)
            );
            assert!(ingredient.test_in(&context, candidate));
            assert!(
                recipes
                    .craft(&context, &[Some(candidate.clone())], 1)
                    .is_some()
            );
        }
        let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
            for _ in 0..100 {
                assert!(ingredient.test_in(&context, std::hint::black_box(&stack)));
            }
        });
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        assert_eq!(allocations.bytes, 0);
        stack
            .data_mut()
            .set_loot_component("minecraft:repair_cost", &Some(Tag::Int(1)));
        assert!(
            !ingredient.test_in(&context, &stack),
            "strict rejects extra components"
        );
        let mut loose_source = source.clone();
        loose_source["strict"] = serde_json::json!(false);
        assert!(
            Ingredient::parse(&loose_source, &items)
                .unwrap()
                .test_in(&context, &stack)
        );
        for invalid in [Tag::String("PURPLE".into()), Tag::Int(-1), Tag::Int(10)] {
            no_default
                .data_mut()
                .set_loot_component("minecraft:shulker/color", &Some(invalid));
            assert!(!ingredient.test_in(&context, &no_default));
            assert!(
                recipes
                    .craft(&context, &[Some(no_default.clone())], 1)
                    .is_none()
            );
        }
        let removal = Tag::compound([(
            "!minecraft:shulker/color",
            Tag::Compound(Default::default()),
        )]);
        (_, stack.extra) = crate::item_data::decode_components(Some(&removal));
        assert!(!ingredient.test_in(&context, &stack));
        assert!(recipes.craft(&context, &[Some(stack.clone())], 1).is_none());
        let absent = parse(
            r#"{"type":"neoforge:components","items":["minecraft:shulker_spawn_egg","minecraft:stone"],"components":{"!minecraft:shulker/color":{}},"strict":true}"#,
            &items,
        );
        assert!(absent.test_in(&context, &stack));
        assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
        assert!(!absent.test_in(&context, &ItemStack::new("minecraft:shulker_spawn_egg", 1)));
    }
}

#[test]
fn malformed_shulker_color_payloads_omit_recipes_with_component_errors() {
    let items = Items::empty();
    let load = load();
    let root = tempfile::tempdir().unwrap();
    for value in [
        serde_json::json!("PURPLE"),
        serde_json::json!("silver"),
        serde_json::json!("minecraft:purple"),
        serde_json::json!(" purple"),
        serde_json::json!(""),
        serde_json::json!(-1),
        serde_json::json!(0),
        serde_json::json!(10),
        serde_json::json!(15),
        serde_json::json!(16),
        serde_json::json!(true),
        serde_json::json!({}),
        serde_json::json!([]),
    ] {
        let patch = serde_json::json!({"minecraft:shulker/color":value});
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(patch.as_object().unwrap()).unwrap(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:shulker/color:"),
            "{error}"
        );
        let source = serde_json::json!({
            "type":"neoforge:components", "items":"minecraft:shulker_spawn_egg",
            "components":patch
        });
        assert!(Ingredient::parse(&source, &items).is_none());
        let recipes = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_shulker_color".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless", "ingredients":[source],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert!(recipes.crafting.is_empty());
        assert!(recipes.by_type.values().all(|recipes| recipes.is_empty()));
    }
    for value in [
        Tag::End,
        Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        Tag::String(horizon_core::nbt::JavaString(vec![
            112, 117, 114, 112, 108, 101, 0xd800,
        ])),
        Tag::Byte(-1),
        Tag::Short(10),
        Tag::Long(10),
        Tag::Float(10.0f32.to_bits()),
        Tag::Double(10.0f64.to_bits()),
    ] {
        assert!(
            horizon_command::item::decoded_component_value(
                "minecraft:shulker/color",
                &value,
                &Default::default()
            )
            .is_err()
        );
    }
}

#[test]
#[ignore = "requires prepared resources"]
fn components_match_damage_ignore_count_and_refuse_empty() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 3);
    sword.damage = 5;
    assert!(ingredient.test_in(&context, &sword));
    sword.damage = 0;
    assert!(!ingredient.test_in(&context, &sword));
    sword.damage = 5;
    sword.count = 0;
    assert!(!ingredient.test_in(&context, &sword));
    assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:air", 1)));
    assert!(!ingredient.is_simple());
}
#[test]
#[ignore = "requires prepared resources"]
fn components_strict_counts_max_damage_default_and_custom_name() {
    let items = resources();
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5},"strict":true}"#,
        &items,
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
    sword.damage = 5;
    // MAX_DAMAGE is a real prototype component, absent from the literal patch.
    assert!(
        items.component_defaults["minecraft:diamond_sword"]
            .contains_key(&"minecraft:max_damage".into())
    );
    assert!(!ingredient.test_in(&context, &sword));
    sword.data_mut().set_loot_component(
        "minecraft:custom_name",
        &Some(Tag::String("Named sword".into())),
    );
    assert!(!ingredient.test_in(&context, &sword));

    // A literal supported-only prototype isolates strict extra-component behavior.
    let mut supported = Items::empty();
    supported.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([
        ("minecraft:diamond_sword".into(), crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:damage":0,"minecraft:max_damage":1561,"minecraft:max_stack_size":1}).as_object().unwrap()).unwrap())
    ]));
    let complete = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5,"minecraft:max_damage":1561,"minecraft:max_stack_size":1},"strict":true}"#,
        &supported,
    );
    let mut plain = ItemStack::new("minecraft:diamond_sword", 1);
    plain.damage = 5;
    assert!(complete.test_in(&context, &plain));
    assert!(!complete.test_in(&context, &sword));
    let missing_max = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5,"minecraft:max_stack_size":1},"strict":true}"#,
        &supported,
    );
    assert!(
        !missing_max.test_in(&context, &plain),
        "the literal max_damage default must be named"
    );
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
