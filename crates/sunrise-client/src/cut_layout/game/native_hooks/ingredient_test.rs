#![cfg(test)]

#[cfg(test)]
mod enumeration_tests {
    use sunrise_gameplay_common::game::tables::GameItems;
    #[test]
    #[ignore = "daily"]
    fn ingredient_keys_are_exact_for_tags_shapes_and_refuse_component_predicates() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let host: &dyn mod_api::Host = game;
                // The host JSON convenience helper accepts only flat inputs. Exercise
                // shapes through the shared parser and the real API conversion boundary.
                let parse = |json| {
                    use sunrise_gameplay_common::game::api_values::TryIntoApi;
                    sunrise_sim::crafting::Ingredient::parse(
                        &serde_json::from_str(json).unwrap(),
                        game.items(),
                    )
                    .unwrap()
                    .try_into_api()
                    .unwrap()
                };
                let assert_exact = |ingredient: &mod_api::Ingredient| {
                    let keys = ingredient.item_keys(host).unwrap();
                    for name in &game.items().registry_order {
                        let key = mod_api::__private::item_key(name).unwrap();
                        assert_eq!(
                            keys.contains(&key),
                            ingredient.test(host, &mod_api::ItemStack::of(key)),
                            "{name}"
                        );
                    }
                    keys
                };
                let plain = parse(r#""minecraft:stick""#);
                assert_eq!(
                    assert_exact(&plain).as_ref(),
                    &[mod_api::__private::item_key("minecraft:stick").unwrap()]
                );
                let planks = parse(r##""#minecraft:planks""##);
                let keys = assert_exact(&planks);
                assert!(
                    keys.contains(&mod_api::__private::item_key("minecraft:oak_planks").unwrap())
                );
                assert!(
                    keys.contains(&mod_api::__private::item_key("minecraft:birch_planks").unwrap())
                );
                assert!(!keys.contains(&mod_api::__private::item_key("minecraft:stick").unwrap()));

                // Parent display lists need not be trusted: use the actual engine shape rule.
                let mut difference = mod_api::Ingredient::of(Vec::new());
                difference.children = Some((
                    mod_api::item::crafting::Shape::Difference,
                    vec![planks.clone(), parse(r#""minecraft:birch_planks""#)],
                ));
                let keys = assert_exact(&difference);
                assert!(
                    keys.contains(&mod_api::__private::item_key("minecraft:oak_planks").unwrap())
                );
                assert!(
                    !keys
                        .contains(&mod_api::__private::item_key("minecraft:birch_planks").unwrap())
                );

                let mut universal = mod_api::Ingredient::of(Vec::new());
                universal.children =
                    Some((mod_api::item::crafting::Shape::Intersection, Vec::new()));
                assert_eq!(
                    assert_exact(&universal).len(),
                    game.items().registry_order.len()
                );

                let component = parse(
                    r#"{"type":"neoforge:components","items":"minecraft:diamond_sword","components":{"minecraft:damage":5}}"#,
                );
                assert_eq!(
                    component.item_keys(host),
                    Err(mod_api::IngredientKeysError::NotEnumerable)
                );
                let mut mixed = mod_api::Ingredient::of(Vec::new());
                mixed.children = Some((
                    mod_api::item::crafting::Shape::Compound,
                    vec![plain, component],
                ));
                assert_eq!(
                    mixed.item_keys(host),
                    Err(mod_api::IngredientKeysError::NotEnumerable)
                );
                assert_eq!(
                    mod_api::Ingredient::of(vec!["test:absent".into()]).item_keys(host),
                    Err(mod_api::IngredientKeysError::UnknownItem)
                );
            });
        });
    }
}
