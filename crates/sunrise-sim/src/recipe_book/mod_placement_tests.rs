use super::*;
struct Registry;
impl DisplayResources for Registry {
    fn book(&self, _: &mod_api::RecipeBookTypeKey) -> bool {
        true
    }
    fn item(&self, id: &str) -> bool {
        matches!(id, "minecraft:stone" | "minecraft:dirt")
    }
    fn stack(&self, _: &mod_api::ItemStack) -> bool {
        false
    }
    fn tag(&self, _: &str) -> bool {
        false
    }
    fn component(&self, _: &str) -> bool {
        false
    }
    fn trim_pattern(&self, _: &str) -> bool {
        false
    }
    fn menu(&self, _: &str) -> bool {
        true
    }
    fn recipe_type(&self, _: &str) -> bool {
        true
    }
    fn recipe(&self, _: &str, _: &str) -> bool {
        true
    }
}
fn batch(info: Option<PlacementInfo>) -> Vec<RecipeBookCategory> {
    let mut wire = mod_api::wire::Writer::new();
    wire.put(&"demo:book".to_owned());
    vec![RecipeBookCategory {
        id: "demo:category".into(),
        recipe_type: "demo:cooking".into(),
        icon: "minecraft:stone".into(),
        secondary_icon: None,
        target_menu: "demo:pot".into(),
        book: mod_api::wire::Reader::decode(wire.bytes()).unwrap(),
        search_group: "demo:search".into(),
        displays: vec![RecipeBookDisplayEntry {
            recipe: "demo:soup".into(),
            displays: vec![],
            placement: info,
        }],
    }]
}
fn info() -> PlacementInfo {
    PlacementInfo {
        cells: vec![Some(mod_api::Ingredient::of(vec![
            "minecraft:stone".into(),
        ]))],
        shape: None,
    }
}
fn layout() -> crate::recipe_book::slot_layout::CheckedSlotLayout {
    crate::recipe_book::menu_layout::compile(
        mod_api::RecipeBookLayout::Furnace {
            input: 0,
            fuel: 1,
            result: 2,
        },
        crate::recipe_book::menu_layout::MenuLayoutMetadata {
            slots: 3,
            crafting: None,
        },
    )
    .unwrap()
}
#[test]
fn explicit_not_placeable_is_not_missing_authoritative_information() {
    let layout = layout();
    let missing = batch(None);
    let error = compile("demo", "demo:pot", &missing[0].displays[0], &layout).unwrap_err();
    assert!(
        matches!(error, DisplayLoadError::InvalidPlacement(reason) if reason.contains("mod demo") && reason.contains("PlacementInfo"))
    );
    let impossible = batch(Some(PlacementInfo::NOT_PLACEABLE));
    validate_batch(&impossible, &Registry).unwrap();
    assert!(
        compile("demo", "demo:pot", &impossible[0].displays[0], &layout)
            .unwrap()
            .is_none()
    );
    let available = batch(Some(info()));
    validate_batch(&available, &Registry).unwrap();
    assert!(
        compile("demo", "demo:pot", &available[0].displays[0], &layout)
            .unwrap()
            .is_some()
    );
}
#[test]
fn item_identity_holder_order_and_shape_are_checked_at_load() {
    let mut unknown = info();
    unknown.cells[0]
        .as_mut()
        .unwrap()
        .items
        .insert("demo:missing".into());
    assert!(validate_batch(&batch(Some(unknown)), &Registry).is_err());
    let mut order = info();
    order.cells[0].as_mut().unwrap().order.clear();
    assert!(validate_batch(&batch(Some(order)), &Registry).is_err());
    let mut shape = info();
    shape.shape = Some([2, 1]);
    assert!(validate_batch(&batch(Some(shape)), &Registry).is_err());
    let mut shape = info();
    shape.shape = Some([1, 1]);
    let values = batch(Some(shape));
    validate_batch(&values, &Registry).unwrap();
    assert!(compile("demo", "demo:pot", &values[0].displays[0], &layout()).is_err());
}
#[test]
fn in_process_validation_has_no_wire_depth_cap_while_network_sync_keeps_its_boundary() {
    let limit = usize::from(mod_api::wire::MAX_DEPTH);
    for (depth, wire_ok) in [(limit - 4, true), (limit - 3, false)] {
        let mut ingredient = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
        for _ in 0..depth {
            let mut parent = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
            parent.children = Some((Shape::Compound, vec![ingredient]));
            ingredient = parent;
        }
        let values = batch(Some(PlacementInfo {
            cells: vec![Some(ingredient)],
            shape: None,
        }));
        assert!(validate_batch(&values, &Registry).is_ok(), "depth {depth}");
        assert_eq!(
            crate::recipe_book::placement_sync::encode_catalogue(&values).is_ok(),
            wire_ok,
            "transport at depth {depth}"
        );
    }

    let mut tag = sunrise_core::nbt::Tag::Int(1);
    for _ in 0..513 {
        tag = sunrise_core::nbt::Tag::List(vec![tag]);
    }
    let mut deep_display = info();
    deep_display.cells[0].as_mut().unwrap().display =
        Some(Box::new([("data".into(), tag)].into_iter().collect()));
    let values = batch(Some(deep_display));
    assert!(validate_batch(&values, &Registry).is_ok());
    assert!(crate::recipe_book::placement_sync::encode_catalogue(&values).is_err());
}

#[test]
fn network_catalogue_projects_non_empty_custom_ingredients_and_composites_to_item_sets() {
    use crate::recipe_book::placement_sync::{decode_catalogue, encode_catalogue};
    let display_nbt: Box<sunrise_core::nbt::Compound> = Box::new(
        [("slot".into(), sunrise_core::nbt::Tag::Int(1))]
            .into_iter()
            .collect(),
    );
    let predicate = || {
        mod_api::__private::bind_ingredient_predicate(
            "demo",
            0,
            "demo:projected",
            mod_api::IngredientDefinition::new(vec![], |_, _| false),
        )
        .1
    };

    let expected_items = std::collections::HashSet::from([
        "minecraft:stone".to_owned(),
        "minecraft:dirt".to_owned(),
    ]);

    // Direct custom ingredient with partial holder order projects to a plain item set
    // covering all candidate items, while the server's original keeps its predicate.
    let mut direct = batch(Some(PlacementInfo {
        cells: vec![Some(mod_api::Ingredient {
            items: expected_items.clone(),
            order: vec!["minecraft:stone".into()],
            custom: Some(predicate()),
            children: None,
            rule: None,
            display: Some(display_nbt.clone()),
        })],
        shape: None,
    }));
    direct[0].displays[0]
        .displays
        .push(mod_api::RecipeBookDisplay::Shapeless {
            ingredients: vec![mod_api::RecipeSlotDisplay::Item("minecraft:stone".into())],
            result: mod_api::RecipeSlotDisplay::Empty,
            station: mod_api::RecipeSlotDisplay::Empty,
        });
    validate_batch(&direct, &Registry).unwrap();
    let received = decode_catalogue(&encode_catalogue(&direct).unwrap()).unwrap();
    validate_batch(&received, &Registry).unwrap();
    assert!(
        compile("demo", "demo:pot", &received[0].displays[0], &layout())
            .unwrap()
            .is_some()
    );
    let projected = received[0].displays[0].placement.as_ref().unwrap().cells[0]
        .as_ref()
        .unwrap();
    assert_eq!(projected.items, expected_items);
    assert_eq!(
        projected.order,
        vec!["minecraft:stone".to_owned(), "minecraft:dirt".to_owned()]
    );
    assert!(projected.custom.is_none());
    assert!(projected.children.is_none());
    assert!(projected.rule.is_none());
    assert_eq!(projected.display, Some(display_nbt.clone()));
    assert_eq!(
        received[0].displays[0].displays,
        direct[0].displays[0].displays
    );
    assert!(
        direct[0].displays[0].placement.as_ref().unwrap().cells[0]
            .as_ref()
            .unwrap()
            .custom
            .is_some()
    );

    // Composite ingredient with a custom descendant and empty parent order projects to a
    // plain item set carrying the composite's candidate items and full deterministic order.
    let mut child = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
    child.custom = Some(predicate());
    let composite = batch(Some(PlacementInfo {
        cells: vec![Some(mod_api::Ingredient {
            items: expected_items.clone(),
            order: Vec::new(),
            custom: None,
            children: Some((
                Shape::Compound,
                vec![
                    child,
                    mod_api::Ingredient::of(vec!["minecraft:dirt".into()]),
                ],
            )),
            rule: None,
            display: Some(display_nbt.clone()),
        })],
        shape: None,
    }));
    validate_batch(&composite, &Registry).unwrap();
    let received = decode_catalogue(&encode_catalogue(&composite).unwrap()).unwrap();
    validate_batch(&received, &Registry).unwrap();
    let projected = received[0].displays[0].placement.as_ref().unwrap().cells[0]
        .as_ref()
        .unwrap();
    assert_eq!(projected.items, expected_items);
    assert_eq!(
        projected.order,
        vec!["minecraft:dirt".to_owned(), "minecraft:stone".to_owned()]
    );
    assert!(projected.custom.is_none());
    assert!(projected.children.is_none());
    assert!(projected.rule.is_none());
    assert_eq!(projected.display, Some(display_nbt));
}

#[test]
fn network_catalogue_collapses_empty_candidate_ingredients_to_not_placeable() {
    use crate::recipe_book::placement_sync::{decode_catalogue, encode_catalogue};
    let mut declarations = batch(Some(PlacementInfo {
        cells: vec![
            Some(mod_api::Ingredient::of(vec!["minecraft:stone".into()])),
            Some(mod_api::Ingredient {
                items: Default::default(),
                order: Vec::new(),
                custom: Some(
                    mod_api::__private::bind_ingredient_predicate(
                        "demo",
                        0,
                        "demo:empty",
                        mod_api::IngredientDefinition::new(vec![], |_, _| false),
                    )
                    .1,
                ),
                children: None,
                rule: None,
                display: None,
            }),
        ],
        shape: Some([2, 1]),
    }));
    declarations[0].displays[0]
        .displays
        .push(mod_api::RecipeBookDisplay::Shapeless {
            ingredients: vec![mod_api::RecipeSlotDisplay::Item("minecraft:stone".into())],
            result: mod_api::RecipeSlotDisplay::Empty,
            station: mod_api::RecipeSlotDisplay::Empty,
        });
    validate_batch(&declarations, &Registry).unwrap();
    let received = decode_catalogue(&encode_catalogue(&declarations).unwrap()).unwrap();
    validate_batch(&received, &Registry).unwrap();
    assert_eq!(
        received[0].displays[0].placement,
        Some(PlacementInfo::NOT_PLACEABLE)
    );
    assert_eq!(
        received[0].displays[0].displays,
        declarations[0].displays[0].displays
    );
}

#[test]
fn non_simple_placement_accepts_empty_display_order_with_candidates() {
    let mut value = info();
    let ingredient = value.cells[0].as_mut().unwrap();
    ingredient.order.clear();
    ingredient.rule = Some(Box::new(
        mod_api::item::crafting::IngredientRule::Components {
            patch: [("minecraft:damage".into(), sunrise_core::nbt::Tag::Int(5))].into(),
            removed: Vec::new(),
            strict: false,
        },
    ));
    validate_batch(&batch(Some(value)), &Registry).unwrap();
}

#[test]
fn deep_native_declarations_are_stack_safe_across_index_compile_drop_and_send() {
    use crate::recipe_book::{
        ModBookIndex,
        placement_sync::{
            BorrowedCategory, BorrowedDisplayEntry, Refusal, decode_catalogue,
            encode_borrowed_catalogue, encode_catalogue,
        },
    };
    const DEPTH: usize = 16_000;
    let wrap = |mut leaf: mod_api::Ingredient| {
        for _ in 0..DEPTH {
            let mut parent = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
            parent.children = Some((Shape::Compound, vec![leaf]));
            leaf = parent;
        }
        batch(Some(PlacementInfo {
            cells: vec![Some(leaf)],
            shape: None,
        }))
    };

    // Valid deep declaration: validate, build index, compile placement, send refusal, drop.
    let valid = wrap(mod_api::Ingredient::of(vec!["minecraft:stone".into()]));
    let index = ModBookIndex::build(valid, &Registry).unwrap();
    let category = index.category("demo:category").unwrap();
    let root = category.displays[0].placement.as_ref().unwrap().cells[0]
        .as_ref()
        .unwrap();
    let ingredient = super::convert(root);
    let context = crate::test_resources::server_load()
        .loop_context(sunrise_world::native_load_context::LoopSide::Server, []);
    assert!(ingredient.test_in(
        &context,
        &crate::items::ItemStack::new("minecraft:stone", 1)
    ));
    assert!(!ingredient.test_in(&context, &crate::items::ItemStack::new("minecraft:dirt", 1)));
    let compiled = compile("demo", "demo:pot", &category.displays[0], &layout())
        .unwrap()
        .unwrap();
    let cloned_plan = compiled.clone();
    drop(cloned_plan);
    drop(compiled);
    let borrowed = vec![BorrowedCategory {
        category,
        displays: category
            .displays
            .iter()
            .map(|entry| BorrowedDisplayEntry {
                recipe: entry.recipe.as_str(),
                displays: entry.displays.clone(),
                placement: entry.placement.as_ref(),
            })
            .collect(),
    }];
    assert_eq!(encode_borrowed_catalogue(&borrowed), Err(Refusal::TooLarge));
    drop(borrowed);
    let cloned_category = category.clone();
    assert_eq!(encode_catalogue(&[cloned_category]), Err(Refusal::TooLarge));
    drop(index);

    // Invalid deep declaration: rejected during ModBookIndex::build and dropped without overflow.
    let invalid = wrap(mod_api::Ingredient::of(vec!["demo:missing".into()]));
    assert!(matches!(
        ModBookIndex::build(invalid, &Registry),
        Err(DisplayLoadError::InvalidPlacement(_))
    ));

    // Deep composite with a custom leaf projects flat on the send path and round-trips.
    let mut custom_leaf = mod_api::Ingredient::of(vec!["minecraft:stone".into()]);
    custom_leaf.custom = Some(
        mod_api::__private::bind_ingredient_predicate(
            "demo",
            0,
            "demo:deep",
            mod_api::IngredientDefinition::new(vec!["minecraft:stone".into()], |_, _| true),
        )
        .1,
    );
    let custom_index = ModBookIndex::build(wrap(custom_leaf), &Registry).unwrap();
    let custom_category = custom_index.category("demo:category").unwrap();
    let custom_borrowed = vec![BorrowedCategory {
        category: custom_category,
        displays: custom_category
            .displays
            .iter()
            .map(|entry| BorrowedDisplayEntry {
                recipe: entry.recipe.as_str(),
                displays: entry.displays.clone(),
                placement: entry.placement.as_ref(),
            })
            .collect(),
    }];
    let bytes = encode_borrowed_catalogue(&custom_borrowed).unwrap();
    drop(custom_borrowed);
    let decoded = decode_catalogue(&bytes).unwrap();
    let client_index = ModBookIndex::build(decoded, &Registry).unwrap();
    drop(client_index);
    drop(custom_index);
}
