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
fn components_dyed_color_decodes_rgb_forms_and_matches_borrowed_values() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:leather_helmet".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({"minecraft:dyed_color":[1,0,0]})
                .as_object()
                .unwrap(),
        )
        .unwrap(),
    )]));
    let integer = parse(
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":-65536}}"#,
        &items,
    );
    let vector = parse(
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1.0,0.0,0.0]}}"#,
        &items,
    );
    assert_eq!(
        vector.shown.as_ref().unwrap()[0].data().unwrap().dyed_color,
        Some(-65536)
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let default = ItemStack::new("minecraft:leather_helmet", 1);
    let mut typed = default.clone();
    typed.data_mut().dyed_color = Some(-65536);
    let mut raw = default.clone();
    raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(0)])),
    );
    for ingredient in [&integer, &vector] {
        assert!(ingredient.test_in(&context, &default));
        assert!(ingredient.test_in(&context, &typed));
        assert!(ingredient.test_in(&context, &raw));
    }
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            for ingredient in [&integer, &vector] {
                for stack in [&default, &typed, &raw] {
                    assert!(std::hint::black_box(ingredient.test_in(&context, stack)));
                }
            }
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    // RGB equality retains the integer's alpha bits; rendering opacity is unrelated.
    typed.data_mut().dyed_color = Some(16711680);
    assert!(!integer.test_in(&context, &typed));
    typed.data_mut().dyed_color = Some(-16776961);
    assert!(!vector.test_in(&context, &typed));
    raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::String("not a color".into())),
    );
    assert!(!integer.test_in(&context, &raw));

    // All boxed number widths and NbtOps collection forms reach the same codec.
    for (value, expected) in [
        (Tag::Byte(-1), -1),
        (Tag::Short(-32768), -32768),
        (Tag::Int(305419896), 305419896),
        (Tag::Long(4294901760), -65536),
        (Tag::Float((-65536.5f32).to_bits()), -65536),
        (Tag::Double((-65536.5f64).to_bits()), -65536),
        (Tag::ByteArray(vec![1, 0, 0]), -65536),
        (Tag::IntArray(vec![1, 0, 0]), -65536),
        (Tag::LongArray(vec![1, 0, 0]), -65536),
        (
            Tag::List(vec![
                Tag::Double(0.5f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
                Tag::Double(0.0f64.to_bits()),
            ]),
            -8454144,
        ),
        // Channels have no 0..1 restriction, and floor before byte masking.
        (Tag::ByteArray(vec![255, 2, 0]), 0xff01_fe00u32 as i32),
        (
            Tag::List(vec![
                Tag::Float(f32::NAN.to_bits()),
                Tag::Float(f32::INFINITY.to_bits()),
                Tag::Float(f32::NEG_INFINITY.to_bits()),
            ]),
            0xff00_ff00u32 as i32,
        ),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:dyed_color",
                &value,
                &items.component_registry,
            )
            .unwrap(),
            Tag::Int(expected)
        );
        let patch = Tag::compound([("minecraft:dyed_color", value)]);
        let (_, data) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(data.unwrap().dyed_color, Some(expected));
    }

    for source in [
        r##"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":"#ff0000"}}"##,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,0]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,0,0,1]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":[1,"red",0]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:leather_helmet","components":{"minecraft:dyed_color":{"rgb":16711680,"show_in_tooltip":true}}}"#,
    ] {
        let json: serde_json::Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none());
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:dyed_color"), "{error}");
    }
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:leather_helmet","minecraft:stick"],"components":{"!minecraft:dyed_color":{}}}"#,
        &items,
    );
    assert!(!absence.test_in(&context, &default));
    let mut populated_typed = default.clone();
    populated_typed.data_mut().dyed_color = Some(-65536);
    assert!(!absence.test_in(&context, &populated_typed));
    let mut populated_raw = default.clone();
    populated_raw.data_mut().set_other(
        "minecraft:dyed_color",
        Some(Tag::List(vec![Tag::Int(1), Tag::Int(0), Tag::Int(0)])),
    );
    assert!(!absence.test_in(&context, &populated_raw));
    let mut removed = default.clone();
    removed.data_mut().set_loot_component(
        "!minecraft:dyed_color",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!integer.test_in(&context, &removed));
    assert!(!vector.test_in(&context, &removed));
    assert!(absence.test_in(&context, &ItemStack::new("minecraft:stick", 1)));
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
fn components_map_id_decodes_signed_numbers_and_matches_live_values() {
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7.75}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":4294967289}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::validate_recipe(&json, &items, "fixture:map_id"));
        let ingredient = Ingredient::parse(&json, &items).unwrap();
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0].data().unwrap().map_id,
            Some(-7),
        );
        for value in [
            Tag::Byte(-7),
            Tag::Short(-7),
            Tag::Int(-7),
            Tag::Long(4_294_967_289),
            Tag::Float((-7.75_f32).to_bits()),
            Tag::Double((-7.75_f64).to_bits()),
        ] {
            assert_eq!(
                horizon_command::item::decoded_component_value(
                    "minecraft:map_id",
                    &value,
                    &items.component_registry,
                )
                .unwrap(),
                Tag::Int(-7),
            );
            let mut stack = ItemStack::new("minecraft:filled_map", 1);
            stack
                .data_mut()
                .set_other("minecraft:map_id", Some(value.clone()));
            assert!(ingredient.test_in(&context, &stack));
            let patch = Tag::compound([("minecraft:map_id", value)]);
            (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
            assert_eq!(stack.data().unwrap().map_id, Some(-7));
            assert!(ingredient.test_in(&context, &stack));
        }
        let mut different = ItemStack::new("minecraft:filled_map", 1);
        different.data_mut().map_id = Some(-6);
        assert!(!ingredient.test_in(&context, &different));
        assert!(!ingredient.test_in(&context, &ItemStack::new("minecraft:filled_map", 1)));
    }

    let bad: Value = serde_json::from_str(
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":"-7"}}"#,
    )
    .unwrap();
    let rule = IngredientRule::Components {
        patch: crate::json_nbt::json_to_compound(bad["components"].as_object().unwrap()).unwrap(),
        removed: Vec::new(),
        strict: false,
    };
    assert_eq!(
        components::validate(&rule, &items).unwrap_err(),
        "component minecraft:map_id: Not a number",
    );
    assert!(Ingredient::parse(&bad, &items).is_none());
    let bad_recipe = serde_json::json!({
        "type": "minecraft:crafting_shapeless",
        "ingredients": [bad],
        "result": {"id": "minecraft:map"},
    });
    assert!(!Ingredient::validate_recipe(
        &bad_recipe,
        &items,
        "fixture:bad_map_id"
    ));

    let mut defaults = Items::empty();
    defaults.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:filled_map".into(),
        [(
            "minecraft:map_id".into(),
            Tag::Double((-7.75_f64).to_bits()),
        )]
        .into_iter()
        .collect(),
    )]));
    let ingredient = parse(
        r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"minecraft:map_id":-7}}"#,
        &defaults,
    );
    let mut inherited = ItemStack::new("minecraft:filled_map", 1);
    assert!(ingredient.test_in(&context, &inherited));
    inherited.data_mut().map_id = Some(-6);
    assert!(!ingredient.test_in(&context, &inherited));

    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:filled_map","components":{"!minecraft:map_id":{}}}"#;
    let absence = parse(absence_source, &defaults);
    let mut removed = ItemStack::new("minecraft:filled_map", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_other(
        "!minecraft:map_id",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!ingredient.test_in(&context, &removed));
    let no_default_absence = parse(absence_source, &items);
    let mut bare = ItemStack::new("minecraft:filled_map", 1);
    assert!(no_default_absence.test_in(&context, &bare));
    bare.data_mut().map_id = Some(-7);
    assert!(!no_default_absence.test_in(&context, &bare));
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
fn ominous_bottle_amplifier_codec_loads_matches_and_rejects_out_of_range() {
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:ominous_bottle".into(),
        [("minecraft:ominous_bottle_amplifier".into(), Tag::Byte(0))]
            .into_iter()
            .collect(),
    )]));
    let source = r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":3.9}}"#;
    let ingredient = parse(source, &items);
    assert_eq!(
        ingredient.shown.as_ref().unwrap()[0]
            .data()
            .unwrap()
            .ominous_amplifier,
        Some(3)
    );
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipes = crate::crafting::Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        vec![(
            "fixture:ominous_amplifier".into(),
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
    let mut bottle = ItemStack::new("minecraft:ominous_bottle", 1);
    bottle.data_mut().ominous_amplifier = Some(3);
    assert!(ingredient.test_in(&context, &bottle));
    assert_eq!(
        recipes
            .craft(&context, &[Some(bottle.clone())], 1)
            .unwrap()
            .item
            .as_ref(),
        "minecraft:diamond"
    );
    bottle.data_mut().ominous_amplifier = Some(4);
    assert!(!ingredient.test_in(&context, &bottle));
    assert!(
        recipes
            .craft(&context, &[Some(bottle.clone())], 1)
            .is_none()
    );

    // Codec.INT uses Number.intValue, including long truncation and fractional floats.
    for value in [
        Tag::Byte(3),
        Tag::Short(3),
        Tag::Int(3),
        Tag::Long(4_294_967_299),
        Tag::Float(3.9f32.to_bits()),
        Tag::Double(3.9f64.to_bits()),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(
                "minecraft:ominous_bottle_amplifier",
                &value,
                &items.component_registry,
            )
            .unwrap(),
            Tag::Int(3)
        );
        bottle
            .data_mut()
            .set_other("minecraft:ominous_bottle_amplifier", Some(value.clone()));
        let (matches, allocations) =
            horizon_threads::allocation_probe::measure(|| ingredient.test_in(&context, &bottle));
        assert!(matches);
        assert_eq!(allocations.calls, 0, "{allocations:?}");
        let patch = Tag::compound([("minecraft:ominous_bottle_amplifier", value)]);
        (_, bottle.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(bottle.data().unwrap().ominous_amplifier, Some(3));
        assert!(ingredient.test_in(&context, &bottle));
    }
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(std::hint::black_box(ingredient.test_in(&context, &bottle)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");

    let zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":0}}"#,
        &items,
    );
    let four = parse(
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4}}"#,
        &items,
    );
    let plain = ItemStack::new("minecraft:ominous_bottle", 1);
    let (matches, allocations) =
        horizon_threads::allocation_probe::measure(|| zero.test_in(&context, &plain));
    assert!(matches);
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    bottle.data_mut().ominous_amplifier = Some(4);
    assert!(four.test_in(&context, &bottle));
    let mut live_four = ItemStack::new("minecraft:ominous_bottle", 1);
    live_four.data_mut().ominous_amplifier = Some(4);
    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4.0}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":4.5}}"#,
    ] {
        let boundary = parse(source, &items);
        assert_eq!(
            boundary.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .ominous_amplifier,
            Some(4)
        );
        assert!(boundary.test_in(&context, &live_four));
    }
    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"!minecraft:ominous_bottle_amplifier":{}}}"#;
    let absence = parse(absence_source, &items);
    let mut removed = ItemStack::new("minecraft:ominous_bottle", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_other(
        "!minecraft:ominous_bottle_amplifier",
        Some(Tag::compound([] as [(&str, Tag); 0])),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!zero.test_in(&context, &removed));
    assert!(!ingredient.test_in(&context, &removed));
    assert!(!absence.test_in(&context, &live_four));
    let no_defaults = Items::empty();
    assert!(
        parse(absence_source, &no_defaults)
            .test_in(&context, &ItemStack::new("minecraft:ominous_bottle", 1),)
    );
    for value in [Tag::Int(-1), Tag::Int(5), Tag::String("3".into())] {
        bottle
            .data_mut()
            .set_other("minecraft:ominous_bottle_amplifier", Some(value.clone()));
        assert!(!zero.test_in(&context, &bottle));
        assert!(!four.test_in(&context, &bottle));
        let patch = Tag::compound([("minecraft:ominous_bottle_amplifier", value)]);
        (_, bottle.extra) = crate::item_data::decode_components(Some(&patch));
        assert_eq!(bottle.data().unwrap().ominous_amplifier, None);
        assert!(!zero.test_in(&context, &bottle));
        assert!(!four.test_in(&context, &bottle));
    }

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":-1}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":5}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":"3"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:ominous_bottle","components":{"minecraft:ominous_bottle_amplifier":1e100}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:ominous_bottle_amplifier:"),
            "{error}"
        );
        assert!(Ingredient::parse(&json, &items).is_none());
        let refused = crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:invalid_amplifier".into(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless",
                    "ingredients":[json],
                    "result":{"id":"minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap();
        assert!(refused.crafting.is_empty());
        assert!(refused.by_type.values().all(|recipes| recipes.is_empty()));
    }
}

#[test]
fn potion_duration_scale_decodes_and_matches_float_values() {
    const ID: &str = "minecraft:potion_duration_scale";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:potion".into(),
        horizon_core::nbt::Compound::from_iter([(ID.into(), Tag::Double(1.5f64.to_bits()))]),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let positive = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":1.5}}"#,
        &items,
    );
    let mut stack = ItemStack::new("minecraft:potion", 1);
    assert!(positive.test_in(&context, &stack), "borrowed item default");
    for value in [
        Tag::Float(1.5f32.to_bits()),
        Tag::Double(1.50000000001f64.to_bits()),
    ] {
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(positive.test_in(&context, &stack), "floatValue rounding");
    }
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float(1.25f32.to_bits())));
    assert!(!positive.test_in(&context, &stack));

    let one = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":1}}"#,
        &items,
    );
    for value in [
        Tag::Byte(1),
        Tag::Short(1),
        Tag::Int(1),
        Tag::Long(1),
        Tag::Float(1.0f32.to_bits()),
        Tag::Double(1.0f64.to_bits()),
    ] {
        assert_eq!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .unwrap(),
            Tag::Float(0x3f80_0000),
        );
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(one.test_in(&context, &stack));
    }
    let large = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":4294967296}}"#,
        &items,
    );
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Long(4_294_967_296)));
    assert!(
        large.test_in(&context, &stack),
        "boxed Long does not truncate"
    );

    let zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":0}}"#,
        &items,
    );
    // JsonOps converts the integral decimal -0.0 to a zero byte before decoding.
    let json_zero = parse(
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":-0.0}}"#,
        &items,
    );
    for value in [Tag::Byte(0), Tag::Double(f64::MIN_POSITIVE.to_bits())] {
        assert_eq!(
            horizon_command::item::decoded_component_value(ID, &value, &items.component_registry)
                .unwrap(),
            Tag::Float(0),
        );
        stack.data_mut().set_loot_component(ID, &Some(value));
        assert!(zero.test_in(&context, &stack));
        assert!(json_zero.test_in(&context, &stack));
    }
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float((-0.0f32).to_bits())));
    assert!(
        !zero.test_in(&context, &stack),
        "Float.equals distinguishes zero signs"
    );

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":-1}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":"fast"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"minecraft:potion_duration_scale":{}}}"#,
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
        assert!(error.contains(ID), "{error}");
    }
    for value in [
        Tag::Float((-0.0f32).to_bits()),
        Tag::Double((-0.0f64).to_bits()),
        Tag::Double((-f64::MIN_POSITIVE).to_bits()),
        Tag::Float(f32::NAN.to_bits()),
        Tag::Float(0x7f80_0001),
        Tag::Double(f64::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Double(f64::MAX.to_bits()),
    ] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from_iter([(ID.into(), value)]),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
    }
    assert_eq!(
        horizon_command::item::decoded_component_value(
            ID,
            &Tag::Float(f32::MAX.to_bits()),
            &items.component_registry,
        )
        .unwrap(),
        Tag::Float(0x7f7f_ffff),
    );
    let absence_source = r#"{"type":"neoforge:components","items":"minecraft:potion","components":{"!minecraft:potion_duration_scale":{}}}"#;
    let absence = parse(absence_source, &items);
    let mut removed = ItemStack::new("minecraft:potion", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:potion_duration_scale",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!positive.test_in(&context, &removed));
    let mut populated = ItemStack::new("minecraft:potion", 1);
    populated
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Float(1.5_f32.to_bits())));
    assert!(!absence.test_in(&context, &populated));
    assert!(
        parse(absence_source, &Items::empty())
            .test_in(&context, &ItemStack::new("minecraft:potion", 1),)
    );
}

#[test]
fn minimum_attack_charge_recipe_decodes_and_matches_float_values() {
    const ID: &str = "minecraft:minimum_attack_charge";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:diamond_sword".into(),
        horizon_core::nbt::Compound::from([(ID.into(), Tag::Double(0.35_f64.to_bits()))]),
    )]));
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let recipe = |ingredient: &Value| {
        crate::crafting::Recipes::from_sources(
            root.path(),
            &items,
            &horizon_world::loot::PackOverlay::NONE,
            vec![(
                "fixture:minimum_attack_charge".into(),
                serde_json::json!({
                    "type": "minecraft:crafting_shapeless",
                    "ingredients": [ingredient],
                    "result": {"id": "minecraft:diamond"}
                }),
            )],
            &load,
        )
        .unwrap()
    };
    let stack = |value: Tag| {
        let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
        sword.data_mut().set_loot_component(ID, &Some(value));
        sword
    };
    for (source, value, actual) in [
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0}}"#,
            0.0_f32,
            Tag::Short(0),
        ),
        // JsonOps-to-NBT numeric conversion canonicalizes this exact integer zero.
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":-0.0}}"#,
            0.0,
            Tag::Byte(0),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1}}"#,
            1.0,
            Tag::Long(1),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0.35}}"#,
            0.35,
            Tag::Float(0.35_f32.to_bits()),
        ),
        (
            r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1.00000001}}"#,
            1.0,
            Tag::Int(1),
        ),
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert_eq!(recipe(&json).crafting.len(), 1, "{source}");
        let ingredient = Ingredient::parse(&json, &items).unwrap();
        assert_eq!(
            ingredient.shown.as_ref().unwrap()[0]
                .data()
                .unwrap()
                .other(ID),
            Some(&Tag::Float(value.to_bits())),
            "the published patch must contain the decoded float"
        );
        assert!(ingredient.test_in(&context, &stack(actual)), "{source}");
        assert!(
            !ingredient.test_in(&context, &stack(Tag::Double(0.75_f64.to_bits()))),
            "{source}"
        );
        assert_eq!(
            ingredient.test_in(&context, &ItemStack::new("minecraft:diamond_sword", 1)),
            value == 0.35,
            "the borrowed double-valued item default has the same float value"
        );
        for actual in [
            Tag::Byte(value as i8),
            Tag::Short(value as i16),
            Tag::Int(value as i32),
            Tag::Long(value as i64),
            Tag::Float(value.to_bits()),
            Tag::Double(f64::from(value).to_bits()),
        ] {
            if value == 0.0 || value == 1.0 {
                assert!(ingredient.test_in(&context, &stack(actual)));
            }
        }
    }
    let interior = parse(
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":0.35}}"#,
        &items,
    );
    assert!(interior.test_in(&context, &stack(Tag::Double(0.350000001_f64.to_bits()))));
    assert!(!interior.test_in(&context, &stack(Tag::Double(0.3500001_f64.to_bits()))));
    let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
    let patch = Tag::compound([(
        "!minecraft:minimum_attack_charge",
        Tag::Compound(Default::default()),
    )]);
    (_, removed.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(!interior.test_in(&context, &removed));

    for source in [
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":-0.01}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1.01}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":"0.5"}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":{}}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":[]}}"#,
        r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:minimum_attack_charge":1e300}}"#,
    ] {
        let json: Value = serde_json::from_str(source).unwrap();
        assert!(Ingredient::parse(&json, &items).is_none(), "{source}");
        assert!(recipe(&json).crafting.is_empty(), "{source}");
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(json["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
    }
    // JSON cannot spell NaN or infinity. The same recipe validator must refuse
    // these numeric tags, including negative values that narrow to negative zero.
    for value in [
        Tag::Float(f32::NAN.to_bits()),
        Tag::Double(f64::NAN.to_bits()),
        Tag::Float(f32::INFINITY.to_bits()),
        Tag::Float(f32::NEG_INFINITY.to_bits()),
        Tag::Float((-0.0_f32).to_bits()),
        Tag::Double((-0.0_f64).to_bits()),
        Tag::Double((-1e-300_f64).to_bits()),
    ] {
        let rule = IngredientRule::Components {
            patch: horizon_core::nbt::Compound::from([(ID.into(), value)]),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains(ID), "{error}");
        assert!(error.contains("within range [0.0;1.0]"), "{error}");
    }
    let absence = parse(
        r#"{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:iron_sword"],"components":{"!minecraft:minimum_attack_charge":{}}}"#,
        &items,
    );
    let positive = parse(
        r#"{"type":"neoforge:components","items":["minecraft:diamond_sword","minecraft:iron_sword"],"components":{"minecraft:minimum_attack_charge":0.35}}"#,
        &items,
    );
    let mut removed = ItemStack::new("minecraft:diamond_sword", 1);
    assert!(!absence.test_in(&context, &removed));
    removed.data_mut().set_loot_component(
        "!minecraft:minimum_attack_charge",
        &Some(Tag::Compound(Default::default())),
    );
    assert!(absence.test_in(&context, &removed));
    assert!(!positive.test_in(&context, &removed));
    assert!(!absence.test_in(&context, &stack(Tag::Float(0.35_f32.to_bits()))));
    let no_default = ItemStack::new("minecraft:iron_sword", 1);
    assert!(absence.test_in(&context, &no_default));
    assert!(!positive.test_in(&context, &no_default));
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
