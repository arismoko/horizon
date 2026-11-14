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
}

#[test]
fn custom_data_component_recipes_decode_and_match() {
    use crate::crafting::{Recipe, Recipes};
    const ID: &str = "minecraft:custom_data";
    let mut items = Items::empty();
    items.component_defaults = std::sync::Arc::new(std::collections::HashMap::from([(
        "minecraft:stick".into(),
        crate::json_nbt::json_to_compound(
            serde_json::json!({
                "minecraft:custom_data":{"id":"stick","unknown":{"values":[true,2],"text":"☃"}},
                "minecraft:max_stack_size":64
            })
            .as_object()
            .unwrap(),
        )
        .unwrap(),
    )]));
    std::sync::Arc::make_mut(&mut items.component_defaults).insert(
        "minecraft:blaze_rod".into(),
        [(
            ID.into(),
            Tag::String("{id:stick,unknown:{values:[true,2b],text:'☃'}}".into()),
        )]
        .into_iter()
        .collect(),
    );
    let literals = [
        (
            "compound",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"custom_data":{"unknown":{"text":"☃","values":[true,2]},"id":"stick"}}}"#,
        ),
        (
            "flattened",
            r#"{"type":"neoforge:components","items":["minecraft:stick","minecraft:blaze_rod"],"components":{"minecraft:custom_data":" {unknown:{text:'☃',values:[1b,2b]},id:'stick'} "}}"#,
        ),
        (
            "typed",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{bytes:[B;1,-1],ints:[I;2,3],longs:[L;4,5],mixed:[1b,2s,3,4L,1.25f,2.5d],zero:0.0f}"}}"#,
        ),
        (
            "unicode",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{'\\uD800':'\\N{HIGH SURROGATES D800}',name:'\\N{snowman}',cjk:'\\N{CJK UNIFIED IDEOGRAPHS 4E00}',numeric:0xffub,bool:bool(256),uuid:uuid('1-1-1-1-1')}"}}"#,
        ),
        (
            "empty_key",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":{"":1}}}"#,
        ),
        (
            "empty",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":{}}}"#,
        ),
        (
            "removed",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"!minecraft:custom_data":{}}}"#,
        ),
        (
            "strict",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":{"id":"stick","unknown":{"values":[true,2],"text":"☃"}}},"strict":true}"#,
        ),
        (
            "complete",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":{"id":"stick","unknown":{"values":[true,2],"text":"☃"}},"minecraft:max_stack_size":64},"strict":true}"#,
        ),
    ];
    let refused = [
        (
            "number",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":1}}"#,
        ),
        (
            "list",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":[]}}"#,
        ),
        (
            "snbt_list",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"[1,2]"}}"#,
        ),
        (
            "trailing",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{} trailing"}}"#,
        ),
        (
            "malformed",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{value:}"}}"#,
        ),
        (
            "overflow",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{value:128b}"}}"#,
        ),
        (
            "codepoint",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{value:'\\U00110000'}"}}"#,
        ),
        (
            "unicode_name",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{value:'\\N{LOW-LINE}'}"}}"#,
        ),
        (
            "empty_snbt_key",
            r#"{"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":"{'':1}"}}"#,
        ),
    ];
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let sources = literals
        .iter()
        .chain(&refused)
        .map(|(name, source)| {
            let ingredient: serde_json::Value = serde_json::from_str(source).unwrap();
            (
                (*name).to_owned(),
                serde_json::json!({
                    "type":"minecraft:crafting_shapeless",
                    "ingredients":[ingredient],
                    "result":{"id":"minecraft:diamond"}
                }),
            )
        })
        .collect();
    let recipes = Recipes::from_sources(
        root.path(),
        &items,
        &horizon_world::loot::PackOverlay::NONE,
        sources,
        &load,
    )
    .unwrap();
    let matches = |name: &str, stack: &ItemStack| {
        recipes.recipe_matches(&context, name, &[Some(stack.clone())], 1)
    };
    let plain = ItemStack::new("minecraft:stick", 1);
    for name in ["compound", "flattened", "complete"] {
        assert!(matches(name, &plain), "{name}");
    }
    assert!(!matches("strict", &plain));
    assert!(!matches("empty", &plain));
    assert!(!matches("removed", &plain));
    let blaze = ItemStack::new("minecraft:blaze_rod", 1);
    assert!(matches("flattened", &blaze));
    let mut strict = plain.clone();
    strict.data_mut().set_other(
        "!minecraft:max_stack_size",
        Some(Tag::Compound(Default::default())),
    );
    assert!(matches("strict", &strict));
    strict
        .data_mut()
        .set_other("minecraft:glider", Some(Tag::Int(7)));
    assert!(!matches("strict", &strict));
    let mut stack = plain.clone();
    stack.data_mut().set_loot_component(
        ID,
        &Some(Tag::compound([
            (
                "unknown",
                Tag::compound([
                    ("values", Tag::List(vec![Tag::Byte(1), Tag::Byte(2)])),
                    ("text", Tag::String("☃".into())),
                ]),
            ),
            ("id", Tag::String("stick".into())),
        ])),
    );
    assert!(matches("flattened", &stack));
    let Recipe::Shapeless { ingredients, .. } = recipes.recipe("flattened").unwrap() else {
        panic!("expected a loaded shapeless recipe");
    };
    assert_eq!(
        ingredients[0].shown.as_ref().unwrap()[0].custom_data(),
        stack.custom_data(),
    );
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for _ in 0..100 {
            assert!(ingredients[0].test_in(&context, std::hint::black_box(&stack)));
            assert!(ingredients[0].test_in(&context, std::hint::black_box(&plain)));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    let snbt = Tag::String("{id:stick,unknown:{values:[true,2b],text:'☃'}}".into());
    let patch = Tag::compound([(ID, snbt.clone())]);
    let mut loaded = plain.clone();
    (_, loaded.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(loaded.custom_data().is_some());
    assert!(matches("flattened", &loaded));
    let mut written = plain.clone();
    written.data_mut().set_other(ID, Some(snbt.clone()));
    assert!(written.custom_data().is_some());
    assert!(matches("flattened", &written));
    written.data_mut().set_loot_component(ID, &Some(snbt));
    assert!(matches("flattened", &written));
    let before = written.clone();
    written
        .data_mut()
        .set_other(ID, Some(Tag::String("{bad:}".into())));
    assert_eq!(written, before, "malformed replacement is atomic");
    written
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Int(5)));
    assert_eq!(written, before, "malformed loot replacement is atomic");
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        for candidate in [&loaded, &written, &blaze] {
            assert!(ingredients[0].test_in(&context, candidate));
        }
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    assert_eq!(allocations.bytes, 0);
    for payload in [
        Tag::compound([("id", Tag::String("stick".into()))]),
        Tag::compound([
            ("id", Tag::String("stick".into())),
            (
                "unknown",
                stack
                    .custom_data()
                    .unwrap()
                    .get(&"unknown".into())
                    .unwrap()
                    .clone(),
            ),
            ("extra", Tag::Byte(1)),
        ]),
        Tag::compound([
            ("id", Tag::String("minecraft:stick".into())),
            (
                "unknown",
                stack
                    .custom_data()
                    .unwrap()
                    .get(&"unknown".into())
                    .unwrap()
                    .clone(),
            ),
        ]),
        Tag::compound([
            ("id", Tag::String("stick".into())),
            (
                "unknown",
                Tag::compound([
                    ("values", Tag::List(vec![Tag::Byte(2), Tag::Byte(1)])),
                    ("text", Tag::String("☃".into())),
                ]),
            ),
        ]),
        Tag::compound([
            ("id", Tag::String("stick".into())),
            (
                "unknown",
                Tag::compound([
                    ("values", Tag::List(vec![Tag::Int(1), Tag::Int(2)])),
                    ("text", Tag::String("☃".into())),
                ]),
            ),
        ]),
    ] {
        stack.data_mut().set_loot_component(ID, &Some(payload));
        assert!(!matches("flattened", &stack));
    }
    stack
        .data_mut()
        .set_loot_component(ID, &Some(Tag::Compound(Default::default())));
    assert!(matches("empty", &stack));
    assert!(!matches("removed", &stack));
    stack.data_mut().set_loot_component(ID, &None);
    let patch = Tag::compound([("!minecraft:custom_data", Tag::Compound(Default::default()))]);
    (_, stack.extra) = crate::item_data::decode_components(Some(&patch));
    assert!(matches("removed", &stack));
    assert!(!matches("empty", &stack));
    assert!(!matches("compound", &stack));

    let typed = Tag::compound([
        ("bytes", Tag::ByteArray(vec![1, 255])),
        ("ints", Tag::IntArray(vec![2, 3])),
        ("longs", Tag::LongArray(vec![4, 5])),
        (
            "mixed",
            Tag::List(vec![
                Tag::Byte(1),
                Tag::Short(2),
                Tag::Int(3),
                Tag::Long(4),
                Tag::Float(1.25f32.to_bits()),
                Tag::Double(2.5f64.to_bits()),
            ]),
        ),
        ("zero", Tag::Float(0.0f32.to_bits())),
    ]);
    stack = plain.clone();
    stack
        .data_mut()
        .set_loot_component(ID, &Some(typed.clone()));
    assert!(matches("typed", &stack));
    let mut negative_zero = typed;
    *negative_zero.get_mut("zero").unwrap() = Tag::Float((-0.0f32).to_bits());
    stack
        .data_mut()
        .set_loot_component(ID, &Some(negative_zero));
    assert!(!matches("typed", &stack));
    stack
        .data_mut()
        .set_other(ID, Some(Tag::compound([("", Tag::Byte(1))])));
    assert!(matches("empty_key", &stack));
    let unicode = Tag::compound([
        (
            horizon_core::nbt::JavaString(vec![0xd800]),
            Tag::String(horizon_core::nbt::JavaString(vec![0xd800])),
        ),
        ("name".into(), Tag::String("☃".into())),
        ("cjk".into(), Tag::String("一".into())),
        ("numeric".into(), Tag::Byte(-1)),
        ("bool".into(), Tag::Byte(1)),
        ("uuid".into(), Tag::IntArray(vec![1, 65537, 65536, 1])),
    ]);
    stack.data_mut().set_other(ID, Some(unicode.clone()));
    assert!(matches("unicode", &stack));
    // Raw Java-string SNBT input also preserves lone UTF-16 units, not U+FFFD.
    let mut units: Vec<u16> = "{'".encode_utf16().collect();
    units.push(0xd800);
    units.extend(
        "':'\\uD800',name:'☃',cjk:'一',numeric:255ub,bool:true,uuid:uuid('1-1-1-1-1')}"
            .encode_utf16(),
    );
    stack
        .data_mut()
        .set_other(ID, Some(Tag::String(horizon_core::nbt::JavaString(units))));
    assert_eq!(stack.data().unwrap().other(ID), Some(&unicode));
    assert!(matches("unicode", &stack));
    let Recipe::Shapeless {
        ingredients: unicode_ingredients,
        ..
    } = recipes.recipe("unicode").unwrap()
    else {
        panic!("expected a loaded unicode recipe");
    };
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(unicode_ingredients[0].test_in(&context, &stack));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for (name, source) in refused {
        assert!(recipes.recipe(name).is_none(), "{name}");
        assert!(!matches(name, &plain), "{name}");
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        let rule = IngredientRule::Components {
            patch: crate::json_nbt::json_to_compound(value["components"].as_object().unwrap())
                .unwrap(),
            removed: Vec::new(),
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(
            error.contains("component minecraft:custom_data:"),
            "{error}"
        );
    }
    let mut empty_count = plain.clone();
    empty_count.count = 0;
    assert!(!matches("compound", &empty_count));
    assert!(!matches("compound", &ItemStack::new("minecraft:stone", 1)));

    // NbtOps compound payloads preserve UTF-16 and raw floating tags. Java record
    // equality canonicalizes NaNs, but neither coalesces tag types nor signed zero.
    let payload = Tag::compound([
        (
            horizon_core::nbt::JavaString(vec![0xd800]),
            Tag::String(horizon_core::nbt::JavaString(vec![0xdc00])),
        ),
        (
            "floats".into(),
            Tag::List(vec![
                Tag::Float(0x7fc0_0001),
                Tag::Double(0x7ff8_0000_0000_0001),
            ]),
        ),
    ]);
    let decoded =
        horizon_command::item::decoded_component_value(ID, &payload, &items.component_registry)
            .unwrap();
    assert_eq!(decoded, payload);
    let mut a = plain.clone();
    a.data_mut().set_loot_component(ID, &Some(decoded));
    let mut b = a.clone();
    let mut other = payload;
    *other.get_mut("floats").unwrap() = Tag::List(vec![
        Tag::Float(0xffc0_0002),
        Tag::Double(0xfff8_0000_0000_0002),
    ]);
    b.data_mut().set_loot_component(ID, &Some(other));
    let (_, allocations) = horizon_threads::allocation_probe::measure(|| {
        assert!(horizon_items::item_data::ingredient_components::component_equal(ID, &a, &b));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for (positive, negative) in [
        (Tag::Float(0), Tag::Float((-0.0f32).to_bits())),
        (Tag::Double(0), Tag::Double((-0.0f64).to_bits())),
    ] {
        a.data_mut()
            .set_other(ID, Some(Tag::compound([("zero", positive)])));
        b.data_mut()
            .set_other(ID, Some(Tag::compound([("zero", negative)])));
        assert!(!horizon_items::item_data::ingredient_components::component_equal(ID, &a, &b));
    }
}
