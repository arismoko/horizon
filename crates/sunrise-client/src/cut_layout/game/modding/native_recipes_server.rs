#![cfg(test)]

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::{ItemStack, RecipeInput, RecipeLookupError};
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;

    fn stack(item: &str) -> Option<ItemStack> {
        let name = if item.contains(':') {
            item.to_string()
        } else {
            format!("minecraft:{item}")
        };
        Some(ItemStack::new(mod_api::__private::item_key(&name).unwrap(), 1).unwrap())
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_matches_the_grid_and_returns_what_each_slot_keeps() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let kind = mod_api::Identifier::parse("minecraft:crafting").unwrap();
            let lookup = |width, height, slots: &[Option<mod_api::ItemStack>]| {
                level.recipe_for(
                    game,
                    &kind,
                    &RecipeInput::Crafting {
                        width,
                        height,
                        slots: slots.to_vec(),
                    },
                )
            };
            let cake = [
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("milk_bucket"),
                stack("sugar"),
                stack("egg"),
                stack("sugar"),
                stack("wheat"),
                stack("wheat"),
                stack("wheat"),
            ];
            let made = lookup(3, 3, &cake).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:cake").unwrap(), 1)
            );
            let bucket = stack("bucket");
            let mut kept = vec![bucket.clone(), bucket.clone(), bucket];
            kept.extend([None, None, None, None, None, None]);
            assert_eq!(made.remainders, kept);

            let planks = [stack("oak_planks"), stack("oak_planks")];
            let made = lookup(1, 2, &planks).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, [None, None]);
            let mut corner = vec![None; 9];
            corner[5] = stack("birch_planks");
            corner[8] = stack("birch_planks");
            let made = lookup(3, 3, &corner).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (mod_api::__private::item_key("minecraft:stick").unwrap(), 4)
            );
            assert_eq!(made.remainders, vec![None; 9]);

            let rocket = [stack("paper"), stack("gunpowder")];
            let made = lookup(2, 1, &rocket).unwrap().unwrap();
            assert_eq!(
                (made.output.item(), made.output.count()),
                (
                    mod_api::__private::item_key("minecraft:firework_rocket").unwrap(),
                    3
                )
            );

            assert!(lookup(1, 1, &[stack("dirt")]).unwrap().is_none());
            assert!(lookup(1, 1, &[None]).unwrap().is_none());
            assert!(lookup(4, 1, &[None, None, None, None]).unwrap().is_none());
            assert!(lookup(0, 0, &[]).unwrap().is_none());
            assert!(lookup(1, 2, &planks[..1]) == Err(RecipeLookupError::InvalidInput));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_dispatches_cooking_and_stonecutting_and_pack_recipes() {
        struct PackSmelting;
        #[mod_api::__subscriptions]
        impl mod_api::Hooks for PackSmelting {
            fn data_pack_files(
                &self,
                _: &mut dyn mod_api::ServerHost,
                event: &mut mod_api::DataPackFilesEvent<'_>,
            ) {
                event.files.push((
                    "recipe/test:pack_smelting".into(),
                    br#"{"type":"minecraft:smelting","ingredient":"minecraft:dirt","result":{"id":"minecraft:diamond"},"experience":0.25,"cookingtime":42}"#
                        .to_vec(),
                ));
            }
        }
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("test_pack", Box::new(PackSmelting))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = |name| mod_api::Identifier::parse(name).unwrap();
                let raw = mod_api::ItemStack::new(
                    mod_api::__private::item_key("minecraft:raw_iron").unwrap(),
                    65,
                )
                .unwrap();
                for (name, ticks) in [("minecraft:smelting", 200), ("minecraft:blasting", 200)] {
                    let made = level
                        .recipe_for(game, &kind(name), &RecipeInput::Single(raw.clone()))
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        made.output.item(),
                        mod_api::__private::item_key("minecraft:iron_ingot").unwrap()
                    );
                    assert_eq!(made.output.count(), 1);
                    assert_eq!(made.processing_time, ticks);
                    assert_eq!(made.experience, 0.7);
                }
                for (name, ticks) in [
                    ("minecraft:smoking", 200),
                    ("minecraft:campfire_cooking", 600),
                ] {
                    let made = level
                        .recipe_for(
                            game,
                            &kind(name),
                            &RecipeInput::Single(stack("beef").unwrap()),
                        )
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        made.output.item(),
                        mod_api::__private::item_key("minecraft:cooked_beef").unwrap()
                    );
                    assert_eq!(
                        (made.output.count(), made.processing_time, made.experience),
                        (1, ticks, 0.35)
                    );
                }
                let cut = level
                    .recipe_for(
                        game,
                        &kind("minecraft:stonecutting"),
                        &RecipeInput::Single(stack("mossy_stone_bricks").unwrap()),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    cut.output.item(),
                    mod_api::__private::item_key("minecraft:mossy_stone_brick_slab").unwrap()
                );
                assert_eq!(cut.output.count(), 2);
                game.reload_datapacks(false, &[]).unwrap();
                let made = level
                    .recipe_for(
                        game,
                        &kind("minecraft:smelting"),
                        &RecipeInput::Single(stack("dirt").unwrap()),
                    )
                    .unwrap()
                    .unwrap();
                assert_eq!(made.recipe, kind("test:pack_smelting"));
                assert_eq!(
                    made.output.item(),
                    mod_api::__private::item_key("minecraft:diamond").unwrap()
                );
                assert_eq!((made.experience, made.processing_time), (0.25, 42));
                assert_eq!(
                    level.recipe_for(
                        game,
                        &kind("minecraft:blasting"),
                        &RecipeInput::Single(stack("dirt").unwrap())
                    ),
                    Ok(None)
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_preserves_smithing_order_and_damage() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = |name| mod_api::Identifier::parse(name).unwrap();
                let mut sword = stack("diamond_sword").unwrap();
                sword.set_damage(17);
                let input = RecipeInput::Smithing {
                    template: stack("netherite_upgrade_smithing_template"),
                    base: Some(sword),
                    addition: stack("netherite_ingot"),
                };
                let made = level
                    .recipe_for(game, &kind("minecraft:smithing"), &input)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    made.output.item(),
                    mod_api::__private::item_key("minecraft:netherite_sword").unwrap()
                );
                assert_eq!(made.output.damage(), 17);
                let wrong_order = RecipeInput::Smithing {
                    template: stack("netherite_ingot"),
                    base: stack("diamond_sword"),
                    addition: stack("netherite_upgrade_smithing_template"),
                };
                assert_eq!(
                    level.recipe_for(game, &kind("minecraft:smithing"), &wrong_order),
                    Ok(None)
                );
                assert_eq!(
                    level.recipe_for(game, &kind("minecraft:smelting"), &input),
                    Err(RecipeLookupError::InvalidInput)
                );
                assert_eq!(
                    level.recipe_for(
                        game,
                        &kind("minecraft:smithing"),
                        &RecipeInput::Smithing {
                            template: None,
                            base: None,
                            addition: None
                        }
                    ),
                    Ok(None)
                );
            })
        });
    }

    #[test]
    #[ignore = "daily"]
    fn recipe_for_refuses_mod_owned_types_instead_of_empty_matches() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            with_game(|game| {
                let level = game.level(game.dim.id()).unwrap();
                let kind = mod_api::Identifier::parse("test:crusher").unwrap();
                assert_eq!(
                    level.recipe_for(game, &kind, &RecipeInput::Single(stack("dirt").unwrap())),
                    Err(RecipeLookupError::NotMatchable)
                );
            })
        });
    }

    #[test]
    #[ignore = "a benchmark: run it in release with SUNRISE_TEST_RESOURCES"]
    fn craft_cost() {
        with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let mut cake = vec![stack("milk_bucket"); 3];
            cake.extend([stack("sugar"), stack("egg"), stack("sugar")]);
            cake.extend(vec![stack("wheat"); 3]);
            let grids: [(&str, u32, u32, Vec<Option<ItemStack>>); 4] = [
                ("cake (3x3)", 3, 3, cake),
                ("sticks (1x2)", 1, 2, vec![stack("oak_planks"); 2]),
                (
                    "firework rocket (2x1, special)",
                    2,
                    1,
                    vec![stack("paper"), stack("gunpowder")],
                ),
                ("dirt (1x1, no match)", 1, 1, vec![stack("dirt")]),
            ];
            println!("{} crafting recipes", game.recipes().crafting.len());
            const CALLS: u32 = 10_000;
            let kind = mod_api::Identifier::parse("minecraft:crafting").unwrap();
            for (name, width, height, grid) in grids {
                let input = RecipeInput::Crafting {
                    width,
                    height,
                    slots: grid,
                };
                let start = std::time::Instant::now();
                for _ in 0..CALLS {
                    let _ = std::hint::black_box(level.recipe_for(game, &kind, &input));
                }
                println!(
                    "{name}: {:.1} µs a call",
                    start.elapsed().as_secs_f64() * 1e6 / f64::from(CALLS)
                );
            }
        });
    }
}
