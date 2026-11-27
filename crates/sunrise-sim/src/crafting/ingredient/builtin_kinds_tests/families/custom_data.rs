use super::*;
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
        &sunrise_world::loot::PackOverlay::NONE,
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
    let (_, allocations) = sunrise_threads::allocation_probe::measure(|| {
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
    let (_, allocations) = sunrise_threads::allocation_probe::measure(|| {
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
    // A loot removal drops the stack's own value and leaves no `!` marker, so the
    // stick's prototype custom data shows through; only `!id` patches record a removal.
    stack.data_mut().set_loot_component(ID, &None);
    assert!(!matches("removed", &stack));
    assert!(!matches("empty", &stack));
    assert!(matches("compound", &stack));
    let absent = parse(
        r#"{"type":"neoforge:components","items":"minecraft:stone","components":{"!minecraft:custom_data":{}}}"#,
        &items,
    );
    assert!(absent.test_in(&context, &ItemStack::new("minecraft:stone", 1)));
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
            sunrise_core::nbt::JavaString(vec![0xd800]),
            Tag::String(sunrise_core::nbt::JavaString(vec![0xd800])),
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
        .set_other(ID, Some(Tag::String(sunrise_core::nbt::JavaString(units))));
    assert_eq!(stack.data().unwrap().other(ID), Some(&unicode));
    assert!(matches("unicode", &stack));
    let Recipe::Shapeless {
        ingredients: unicode_ingredients,
        ..
    } = recipes.recipe("unicode").unwrap()
    else {
        panic!("expected a loaded unicode recipe");
    };
    let (_, allocations) = sunrise_threads::allocation_probe::measure(|| {
        assert!(unicode_ingredients[0].test_in(&context, &stack));
    });
    assert_eq!(allocations.calls, 0, "{allocations:?}");
    for (_, source) in refused {
        let value: serde_json::Value = serde_json::from_str(source).unwrap();
        super::decoder_contracts::assert_malformed_decoder_refuses_load(
            &items,
            value["components"].clone(),
        );
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
            sunrise_core::nbt::JavaString(vec![0xd800]),
            Tag::String(sunrise_core::nbt::JavaString(vec![0xdc00])),
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
        sunrise_command::item::decoded_component_value(ID, &payload, &items.component_registry)
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
    let (_, allocations) = sunrise_threads::allocation_probe::measure(|| {
        assert!(sunrise_items::item_data::ingredient_components::component_equal(ID, &a, &b));
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
        assert!(!sunrise_items::item_data::ingredient_components::component_equal(ID, &a, &b));
    }
}

#[test]
fn custom_data_refusals_name_the_component_and_refuse_the_whole_load() {
    use crate::crafting::Recipes;
    let items = Items::empty();
    let load = load();
    let context = load.loop_context(LoopSide::Server, []);
    let root = tempfile::tempdir().unwrap();
    let valid = serde_json::json!({"type":"neoforge:components","items":"minecraft:stick","components":{"minecraft:custom_data":{"answer":42}}});
    let predicate = Ingredient::parse(&valid, &items).unwrap();
    for payload in [
        Tag::End,
        Tag::Byte(0),
        Tag::List(vec![]),
        Tag::String(sunrise_core::nbt::JavaString(vec![0xd800])),
    ] {
        let rule = IngredientRule::Components {
            patch: [("minecraft:custom_data".into(), payload.clone())]
                .into_iter()
                .collect(),
            removed: vec![],
            strict: false,
        };
        let error = components::validate(&rule, &items).unwrap_err();
        assert!(error.contains("minecraft:custom_data"), "{error}");
        let patch = Tag::compound([("minecraft:custom_data", payload)]);
        let (damage, extra) = crate::item_data::decode_components(Some(&patch));
        let candidate = ItemStack {
            item: "minecraft:stick".into(),
            count: 1,
            damage,
            extra,
        };
        assert!(!predicate.test_in(&context, &candidate));
    }
    for (component, payload) in [
        ("minecraft:custom_data", serde_json::Value::Null),
        ("minecraft:custom_data", serde_json::json!([])),
        ("fixture:unknown", serde_json::json!({})),
    ] {
        let malformed = serde_json::json!({"type":"neoforge:components","items":"minecraft:stick","components":{component:payload}});
        let error = components::validate_recipe_components(&malformed, &items).unwrap_err();
        assert!(error.contains(component), "{error}");
        let recipe = |ingredient| serde_json::json!({"type":"minecraft:crafting_shapeless","ingredients":[ingredient],"result":{"id":"minecraft:diamond"}});
        // A malformed file of a supported type refuses the whole candidate load, naming the file.
        let error = Recipes::from_sources(
            root.path(),
            &items,
            &sunrise_world::loot::PackOverlay::NONE,
            vec![
                ("valid".into(), recipe(valid.clone())),
                ("bad".into(), recipe(malformed)),
            ],
            &load,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("bad"), "{error}");
        let recipes = Recipes::from_sources(
            root.path(),
            &items,
            &sunrise_world::loot::PackOverlay::NONE,
            vec![("valid".into(), recipe(valid.clone()))],
            &load,
        )
        .unwrap();
        assert!(recipes.recipe("valid").is_some());
    }
}

#[test]
fn optional_component_fields_treat_json_null_as_absent() {
    let patch = serde_json::json!({"minecraft:use_effects":{"can_sprint":null,"interact_vibrations":null,"speed_multiplier":null}});
    assert_eq!(
        components::json_patch(patch.as_object().unwrap()).unwrap(),
        [(
            "minecraft:use_effects".into(),
            Tag::compound([] as [(&str, Tag); 0])
        )]
        .into_iter()
        .collect()
    );
}
