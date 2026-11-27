#![cfg(test)]
#[cfg(test)]
pub(crate) mod equippable_slot_tests {
    use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1;
    use sunrise_items::items::ItemStack;

    /// `Equippable.slot`: armour goes where it is worn, a plain item goes nowhere.
    #[test]
    #[ignore = "daily"]
    fn the_equippable_slot_follows_the_equippable_component() {
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            let slot = |item: &str| game.item_equippable_slot(&ItemStack::new(item, 1));
            assert_eq!(slot("minecraft:iron_helmet").as_deref(), Some("head"));
            assert_eq!(slot("minecraft:iron_boots").as_deref(), Some("feet"));
            assert_eq!(slot("minecraft:dirt"), None);
        });
    }
}

#[cfg(test)]
pub(crate) mod enchantment_conflict_tests {
    use mod_api::{Identifier, ItemResource, LevelRef, WorldEpoch};
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::native_components::ItemStack;
    use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1;
    use sunrise_gameplay_common::game::tables::GameItems;
    use {
        sunrise_gameplay_common::game::api_values::stack_value,
        sunrise_gameplay_common::game::api_values::test_snapshot,
    };

    #[test]
    #[ignore = "daily"]
    fn enchanting_a_resource_uses_the_active_levels_random_and_preserves_the_input() {
        with_game(|game| {
            let level = game.level("minecraft:overworld").unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let stack = ItemStack::new("minecraft:book", 1);
            let resource = ItemResource::of(&test_snapshot(&stack));
            let pool = Identifier::parse("minecraft:in_enchanting_table").unwrap();
            let missing = Identifier::parse("fixture:missing").unwrap();
            let mut expected_random = game.world.random.clone();

            assert!(
                resource
                    .enchant_randomly(game, stale, 30, &pool)
                    .unwrap()
                    .is_none()
            );
            assert!(
                resource
                    .enchant_randomly(game, level, 30, &missing)
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                game.world.random.clone().next_long(),
                expected_random.clone().next_long()
            );

            // Vanilla still rolls enchantments and advances the RNG at zero power.
            let power = 0;
            let definitions = &game.stations().enchantments;
            let expected = definitions.select(
                game.items(),
                &mut expected_random,
                &stack,
                power,
                definitions.tag("minecraft:in_enchanting_table"),
            );
            assert!(!expected.is_empty());
            let result = resource
                .enchant_randomly(game, level, power, &pool)
                .unwrap()
                .unwrap();
            let result = stack_value(result.as_stack()).unwrap();
            assert_eq!(&*result.item, "minecraft:enchanted_book");
            assert_eq!(result.stored_enchantments().len(), expected.len());
            for (id, level) in expected {
                assert!(result.stored_enchantments().contains(&(id, level)));
            }
            assert!(
                stack_value(resource.as_stack())
                    .unwrap()
                    .all_enchantments()
                    .is_empty()
            );
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
        });
    }

    /// `EnchantmentHelper.modifyDurabilityToRepairFromXp` through `ItemResource`: vanilla
    /// Mending's `repair_with_xp` multiplies the points by 2, a stack without the effect keeps
    /// them, and an unconditioned effect draws nothing from the level's random.
    #[test]
    #[ignore = "daily"]
    fn durability_to_repair_from_xp_runs_the_stacks_repair_with_xp_effects() {
        with_game(|game| {
            let mut mending = ItemStack::new("minecraft:diamond_pickaxe", 1);
            mending.enchant("minecraft:mending", 1);
            let mending = ItemResource::of(&test_snapshot(&mending));
            let mut unbreaking = ItemStack::new("minecraft:diamond_pickaxe", 1);
            unbreaking.enchant("minecraft:unbreaking", 3);
            let unbreaking = ItemResource::of(&test_snapshot(&unbreaking));
            let expected_random = game.world.random.clone();

            assert_eq!(mending.durability_to_repair_from_xp(game, 7), 14);
            assert_eq!(mending.durability_to_repair_from_xp(game, 0), 0);
            // Vanilla starts from the points and lets the effects change them: none, none changed.
            assert_eq!(unbreaking.durability_to_repair_from_xp(game, 7), 7);
            assert_eq!(
                game.world.random.clone().next_long(),
                expected_random.clone().next_long()
            );
        });
    }

    /// `Enchantment.areCompatible`: the same enchantment twice and the exclusive sets conflict,
    /// others and unknown ids do not.
    #[test]
    #[ignore = "daily"]
    fn enchantments_compatible_follows_the_exclusive_sets() {
        use sunrise_gameplay_common::game::tables::WorldTablesExt as _;
        sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
            assert!(!game.enchantments_compatible("minecraft:sharpness", "minecraft:smite"));
            assert!(!game.enchantments_compatible("minecraft:sharpness", "minecraft:sharpness"));
            assert!(game.enchantments_compatible("minecraft:sharpness", "minecraft:unbreaking"));
            assert!(game.enchantments_compatible("minecraft:sharpness", "mod:unknown"));
        });
    }
}

#[cfg(test)]
pub(crate) mod item_resource_property_tests {
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1;
    #[cfg(feature = "client")]
    use sunrise_gameplay_common::game::tables::GameItems;
    pub(crate) fn engine(
        resource: &mod_api::ItemResource,
        count: u32,
    ) -> sunrise_items::items::ItemStack {
        sunrise_gameplay_common::game::api_values::stack_value(&resource.to_stack(count).unwrap())
            .unwrap()
    }
    #[allow(unused_imports)]
    use serde_json::Value;
    pub(crate) mod support {
        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixture_support/item_resource_properties.rs"
        ));
    }
    /// All seven reads and their game consumers, with independently recorded vanilla outcomes.
    #[test]
    #[ignore = "daily"]
    fn effective_resource_properties_match_vanilla() {
        run_effective_resource_properties_match_vanilla();
    }

    #[allow(unused_imports)]
pub(crate) use crate::game::server_side::native_components_server::item_resource_property_tests::run_effective_resource_properties_match_vanilla;
    /// Changed defaults and a patched mod consumable exercise the real game consumers.
    #[cfg(feature = "client")]
    #[test]
    #[ignore = "daily"]
    fn item_resource_changed_defaults_and_consumption() {
        run_item_resource_changed_defaults_and_consumption();
    }

    #[cfg(feature = "client")]
    pub(crate) fn run_item_resource_changed_defaults_and_consumption() {
        use mod_api::{ComponentId, ItemResource, Tag};
        use serde_json::json;
        use std::{fs, path::Path};
        use sunrise_items::items::ItemStack;
        fn link_fixture_source(source: &Path, target: &Path) {
            #[cfg(unix)]
            std::os::unix::fs::symlink(source, target).unwrap();
            #[cfg(not(unix))]
            {
                // Windows file hard links need no symlink privilege. Copy across volumes.
                if source.is_dir() {
                    fs::create_dir(target).unwrap();
                    for entry in fs::read_dir(source).unwrap() {
                        let entry = entry.unwrap();
                        link_fixture_source(&entry.path(), &target.join(entry.file_name()));
                    }
                } else if fs::hard_link(source, target).is_err() {
                    fs::copy(source, target).unwrap();
                }
            }
        }
        let resources = std::path::PathBuf::from(std::env::var("SUNRISE_TEST_RESOURCES").unwrap());
        // A private default report goes through the owning resource load and all its metadata
        // readers, exactly as a registration-time change does. Shared resources stay read-only.
        let root =
            std::env::temp_dir().join(format!("sunrise-property-defaults-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        for entry in fs::read_dir(&resources).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() != "items.json" && entry.file_name() != "command-registries.json" {
                link_fixture_source(&entry.path(), &root.join(entry.file_name()));
            }
        }
        let mut defaults: Value =
            serde_json::from_slice(&fs::read(resources.join("items.json")).unwrap()).unwrap();
        defaults["minecraft:iron_sword"]["minecraft:damage"] = json!(75);
        defaults["minecraft:iron_sword"]["minecraft:enchantment_glint_override"] = json!(false);
        defaults["minecraft:bundle"]["minecraft:bundle_contents"] =
            json!([{"id": "minecraft:stone", "count": 32}]);
        defaults["sunrise:consumable_fixture"] = defaults["minecraft:stick"].clone();
        defaults["minecraft:stick"]["minecraft:bundle_contents"] =
            json!([{"id": "minecraft:stone", "count": 32}]);
        defaults["minecraft:red_bundle"]
            .as_object_mut()
            .unwrap()
            .remove("minecraft:bundle_contents");
        // A mod item belongs to the registry as well as the component report.
        let mut registries: Value =
            serde_json::from_slice(&fs::read(resources.join("command-registries.json")).unwrap())
                .unwrap();
        registries["items"]
            .as_array_mut()
            .unwrap()
            .push(json!("sunrise:consumable_fixture"));
        fs::write(
            root.join("command-registries.json"),
            serde_json::to_vec(&registries).unwrap(),
        )
        .unwrap();
        defaults["minecraft:wooden_sword"]["minecraft:repairable"] = json!({"items":"#planks"});
        fs::write(
            root.join("items.json"),
            serde_json::to_vec(&defaults).unwrap(),
        )
        .unwrap();
        sunrise_gameplay::game::native_entity_refs::tests::with_game_at(root.clone(), |game| {
            let save_root = game.test_save_root();
            game.test_acquire_session_lock(&save_root);
            let wood = ItemResource::of(&sunrise_gameplay_common::game::api_values::test_snapshot(
                &ItemStack::new("minecraft:wooden_sword", 1),
            ));
            let plank =
                ItemResource::of(&sunrise_gameplay_common::game::api_values::test_snapshot(
                    &ItemStack::new("minecraft:oak_planks", 1),
                ));
            assert!(wood.repairable(game, &plank));
            assert!(sunrise_sim::stations::is_valid_repair_item(
                &game.items().enchantments,
                game.items(),
                &engine(&wood, 1),
                &engine(&plank, 1)
            ));
            assert!(!sunrise_items::enchant::Enchantments::item_in(
                game.items(),
                &["#planks".into()],
                "minecraft:iron_ingot"
            ));
            let stack = ItemStack::new("minecraft:iron_sword", 1);
            let resource = ItemResource::of(
                &sunrise_gameplay_common::game::api_values::test_snapshot(&stack),
            );
            let bar = resource.bar(game).unwrap();
            // The independent oracle records the equivalent effective damage as a stack patch.
            let expected: Value = include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/item_resource_properties_expected.jsonl"
            ))
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|row| row["name"] == "prototype-damage-equivalent")
            .unwrap();
            assert_eq!(
                serde_json::json!({"width":bar.width,"color":bar.color}),
                expected["bar"]
            );
            let info = game.items().get(&stack.item);
            assert_eq!(stack.effective_damage(info), 75);
            assert_eq!(
                sunrise_client::hud_overlays::damage_bar(
                    stack.effective_damage(info),
                    stack.max_damage(info)
                ),
                Some((bar.width, bar.color))
            );
            let damage = ComponentId::new("minecraft:damage").unwrap();
            let zero = resource.with_component(game, &damage, Tag::Int(0)).unwrap();
            assert_eq!(zero.component(&damage), Some(Tag::Int(0)));
            assert_eq!(engine(&zero, 1).effective_damage(info), 0);
            assert!(zero.bar(game).is_none());
            let removed = resource.without_component(game, &damage);
            assert_eq!(engine(&removed, 1).effective_damage(info), 0);
            assert!(removed.bar(game).is_none());

            let bundle_stack = ItemStack::new("minecraft:bundle", 1);
            let stone = ItemStack::new("minecraft:stone", 1);
            let bundle = ItemResource::of(&test_snapshot(&bundle_stack));
            // Vanilla BundleItem: half a stack weighs 1/2, showing 1 + floor(12 / 2)
            // pixels in the not-full colour. Bar, capacity, tooltip and nested weight
            // all read the same effective component.
            let bar = bundle.bar(game).unwrap();
            assert_eq!((bar.width, bar.color), (7, 0x7087FF));
            let items = game.items();
            assert_eq!(
                sunrise_client::bundle::Contents::of(items, &bundle_stack)
                    .unwrap()
                    .items(),
                &[ItemStack::new("minecraft:stone", 32)]
            );
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(items, &bundle_stack, &stone),
                Some(32)
            );
            let layout =
                sunrise_client::bundle::tooltip_layout(items, &bundle_stack, 0.0, 0.0, 96.0, 2)
                    .unwrap();
            assert_eq!(layout.slots.len(), 1);
            assert_eq!(layout.slots[0].1, ItemStack::new("minecraft:stone", 32));
            assert_eq!(layout.fill, 47.0);
            assert_eq!(layout.label, None);

            // An explicitly empty or removed component suppresses the prototype's
            // contents rather than inheriting them.
            let contents = ComponentId::new("minecraft:bundle_contents").unwrap();
            let empty = bundle
                .with_component(game, &contents, Tag::List(Vec::new()))
                .unwrap();
            let empty_stack = engine(&empty, 1);
            assert!(empty.bar(game).is_none());
            assert!(
                sunrise_client::bundle::Contents::of(items, &empty_stack)
                    .unwrap()
                    .items()
                    .is_empty()
            );
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(items, &empty_stack, &stone),
                Some(64)
            );
            assert_eq!(
                sunrise_client::bundle::tooltip_layout(items, &empty_stack, 0.0, 0.0, 96.0, 2)
                    .unwrap()
                    .label,
                Some("item.minecraft.bundle.empty")
            );

            // Emptying a bundle with nonempty defaults preserves explicit emptiness.
            let mut emptied = bundle_stack.clone();
            let mut mutable = sunrise_client::bundle::Contents::of(items, &emptied).unwrap();
            assert_eq!(
                mutable.remove_one(items),
                Some(ItemStack::new("minecraft:stone", 32))
            );
            mutable.store(items, &mut emptied);
            assert!(
                sunrise_client::bundle::Contents::of(items, &emptied)
                    .unwrap()
                    .items()
                    .is_empty()
            );
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(items, &emptied, &stone),
                Some(64)
            );
            assert_eq!(
                sunrise_client::bundle::tooltip_layout(items, &emptied, 0.0, 0.0, 96.0, 2)
                    .unwrap()
                    .label,
                Some("item.minecraft.bundle.empty")
            );
            assert!(
                ItemResource::of(&test_snapshot(&emptied))
                    .bar(game)
                    .is_none()
            );

            let removed_bundle = bundle.without_component(game, &contents);
            let removed_bundle_stack = engine(&removed_bundle, 1);

            // An authoritative inventory update carries the patch, not the effective
            // prototype value. Check all three states through the real slot codec.
            use sunrise_protocol::{
                buf::{Codec, Reader},
                item::{Component, component::BUNDLE_CONTENTS},
                packets::play::ContainerSetSlot,
                registry::{Builtin, Dynamic},
            };
            let builtin = Builtin::load(&Builtin::find(&resources).unwrap()).unwrap();
            let synced = Dynamic::default();
            for (stack, expected) in [
                (&bundle_stack, Some(32)),
                (&emptied, Some(0)),
                (&removed_bundle_stack, None),
            ] {
                let packet = ContainerSetSlot {
                    container: 0,
                    state: 1,
                    slot: 36,
                    item: sunrise_client::lan::guest::stack_to_network(
                        |name| builtin.get("minecraft:item").unwrap().id(name).unwrap(),
                        |registry, name| builtin.get(registry)?.id(name),
                        &Some(stack.clone()),
                    ),
                };
                let mut writer = sunrise_core::vanilla_bytes::Writer::new();
                ContainerSetSlot::write(&packet, &mut writer);
                let received = ContainerSetSlot::read(&mut Reader::new(&writer.data))
                    .unwrap()
                    .item
                    .unwrap();
                match expected {
                    Some(0) => assert!(matches!(
                        received.component(BUNDLE_CONTENTS),
                        Some(Component::Stacks(contents)) if contents.is_empty()
                    )),
                    _ => assert!(received.component(BUNDLE_CONTENTS).is_none()),
                }
                assert_eq!(
                    received.removed.contains(&BUNDLE_CONTENTS),
                    expected.is_none()
                );
                let adopted = sunrise_client::item_sync::from_network(&received, &builtin, &synced);
                let contents = sunrise_client::bundle::contents_of(items, &adopted);
                assert_eq!(
                    contents.map(|contents| contents.iter().map(|stack| stack.count).sum::<u32>()),
                    expected,
                );
                assert_eq!(
                    sunrise_items::properties::stack_bar(items, &adopted, items.get(&adopted.item))
                        .map(|bar| bar.width),
                    (expected == Some(32)).then_some(7),
                );
            }

            assert!(removed_bundle.bar(game).is_none());
            assert!(sunrise_client::bundle::Contents::of(items, &removed_bundle_stack).is_none());
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(
                    items,
                    &removed_bundle_stack,
                    &stone
                ),
                None
            );
            assert!(
                sunrise_client::bundle::tooltip_layout(
                    items,
                    &removed_bundle_stack,
                    0.0,
                    0.0,
                    96.0,
                    2
                )
                .is_none()
            );

            // Nested prototype-backed bundle: 1/2 contents + 1/16 overhead = 9/16,
            // leaving 28/64 room and drawing 1 + floor(9 * 12 / 16) = 7 pixels.
            let mut outer = ItemStack::new("minecraft:blue_bundle", 1);
            outer.data_mut().bundle_contents = vec![bundle_stack];
            let outer_bar = ItemResource::of(&test_snapshot(&outer)).bar(game).unwrap();
            assert_eq!((outer_bar.width, outer_bar.color), (7, 0x7087FF));
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(items, &outer, &stone),
                Some(28)
            );
            assert_eq!(
                sunrise_client::bundle::tooltip_layout(items, &outer, 0.0, 0.0, 96.0, 2)
                    .unwrap()
                    .fill,
                52.0
            );
            // An outer bundle holding the emptied inner bundle counts only its 1/16 overhead.
            let mut outer_empty_inner = ItemStack::new("minecraft:blue_bundle", 1);
            outer_empty_inner.data_mut().bundle_contents = vec![emptied];
            let inner_empty_bar = ItemResource::of(&test_snapshot(&outer_empty_inner))
                .bar(game)
                .unwrap();
            assert_eq!(
                (inner_empty_bar.width, inner_empty_bar.color),
                (1, 0x7087FF)
            );
            assert_eq!(
                sunrise_client::bundle::Contents::room_in_stack(items, &outer_empty_inner, &stone),
                Some(60)
            );

            // Vanilla ItemStack.isBarVisible/getBarWidth/getBarColor dispatch to the
            // registered item behavior, independently of mutable prototype defaults.
            let stick_stack = ItemStack::new("minecraft:stick", 1);
            let stick = ItemResource::of(&test_snapshot(&stick_stack));
            assert!(stick.bar(game).is_none());
            assert!(
                sunrise_items::properties::stack_bar(
                    items,
                    &stick_stack,
                    items.get(&stick_stack.item)
                )
                .is_none()
            );

            let red_bundle =
                ItemResource::of(&test_snapshot(&ItemStack::new("minecraft:red_bundle", 1)));
            assert!(red_bundle.bar(game).is_none());
            let patched_red = red_bundle
                .with_component(
                    game,
                    &contents,
                    Tag::List(vec![Tag::compound([
                        ("id", Tag::String("minecraft:stone".into())),
                        ("count", Tag::Int(32)),
                    ])]),
                )
                .unwrap();
            let red_bar = patched_red.bar(game).unwrap();
            assert_eq!((red_bar.width, red_bar.color), (7, 0x7087FF));

            let enchanted = resource
                .with_component(
                    game,
                    &ComponentId::new("minecraft:enchantments").unwrap(),
                    Tag::compound([("minecraft:sharpness", Tag::Int(1))]),
                )
                .unwrap();
            let glint = game
                .items()
                .enchantments
                .props(&stack.item)
                .and_then(|p| p.glint);
            assert_eq!(glint, Some(false));
            assert!(!enchanted.foil(game));
            assert!(!engine(&enchanted, 1).has_foil(glint));
            let inherited = enchanted.without_component(
                game,
                &ComponentId::new("minecraft:enchantment_glint_override").unwrap(),
            );
            assert!(inherited.foil(game));
            assert!(engine(&inherited, 1).has_foil(glint));

            let base = ItemResource::of(&sunrise_gameplay_common::game::api_values::test_snapshot(
                &ItemStack::new("sunrise:consumable_fixture", 2),
            ));
            let consumable = ComponentId::new("minecraft:consumable").unwrap();
            let patched = base
                .with_component(
                    game,
                    &consumable,
                    Tag::compound([
                        ("consume_seconds", Tag::Float(0.1_f32.to_bits())),
                        ("has_consume_particles", Tag::Byte(0)),
                    ]),
                )
                .unwrap();
            let stack = engine(&patched, 2);
            let info = game.items().get(&stack.item).unwrap().clone();
            assert_eq!(game.use_duration(&stack), 2);
            assert!(!game.mod_use(&stack));
            assert!(
                engine(&patched.without_component(game, &consumable), 1)
                    .consumable(Some(&info))
                    .is_none()
            );
            game.creative = false;
            game.server_set_acting_use(true);
            game.common.inventory[game.common.selected] = Some(stack);
            assert!(game.server_start_eating(&info));
            assert!(game.using.is_some());
            game.tick_using_server();
            assert_eq!(game.held().unwrap().count, 2);
            game.tick_using_server();
            assert!(game.using.is_none());
            assert_eq!(
                game.held().unwrap().count,
                1,
                "completion must consume the patched mod item"
            );
            game.test_clear_session_lock();
        });
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
pub(crate) mod hot_key_tests {
    use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1;
    #[test]
    #[ignore = "daily"]
    fn resolved_keys_select_fuel_and_compost_metadata() {
        mod_platform::testing::with_mods(Vec::new().into(), || {
            sunrise_gameplay::game::native_entity_refs::tests::with_game(|game| {
                let key = |name| mod_api::__private::item_key(name).unwrap();
                assert_eq!(game.item_burn_time(key("minecraft:coal")), Some(1600));
                assert_eq!(game.item_burn_time(key("minecraft:stone")), None);
                assert_eq!(game.item_compost_chance(key("minecraft:wheat")), Some(0.65));
                assert_eq!(game.item_compost_chance(key("minecraft:stone")), None);
            });
        });
    }
}

#[cfg(test)]
pub(crate) mod combat_query_tests {
    use sunrise_core::geometry::{Aabb, Vec3};
    use sunrise_core::random::RandomSource;
    use sunrise_gameplay::game::native_entity_refs::tests::with_game;
    use sunrise_gameplay_common::game::api_values::test_snapshot;
    use sunrise_sim::motion::PlayerMotion;
    use {
        crate::game::native_components::*,
        sunrise_gameplay_common::game::native_components::{ItemStack, stack_attack_bonus_preview},
    };

    fn actor(game: &mut sunrise_gameplay::game::ServerGame<'_>) -> mod_api::LivingRef {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        sunrise_gameplay::game::native_entity_refs::tests::bind_local_player(game, 20, 123);
        mod_api::LivingRef::new(game.entity(20).unwrap())
    }
    fn resource(stack: &ItemStack) -> mod_api::ItemResource {
        mod_api::ItemResource::of(&test_snapshot(stack))
    }
    fn patch(stack: &mut ItemStack, id: &str, value: serde_json::Value) {
        stack
            .data_mut()
            .set_other(id, Some(sunrise_game_data::scoreboard::json_to_tag(&value)));
    }

    #[test]
    fn stack_combat_random_preview_repeats_without_consuming_attack_rng() {
        use sunrise_core::random::LegacyRandom;
        use sunrise_items::enchant::{EffectContext, Enchantments};
        let definition = serde_json::json!({
            "max_level": 1,
            "effects": {"minecraft:damage": [{
                "requirements": {"condition": "minecraft:random_chance", "chance": 0.5},
                "effect": {"type": "minecraft:add", "value": 5.0}
            }]}
        });
        let enchantments = Enchantments::default()
            .with_entries([("test:random_damage", &definition)])
            .unwrap()
            .unwrap();
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.enchant("test:random_damage", 1);
        for (seed, expected) in [(0, 0.0), (4096, 5.0)] {
            let mut live = LegacyRandom::new(seed);
            let ctx = EffectContext::default();
            for _ in 0..3 {
                assert_eq!(
                    stack_attack_bonus_preview(&enchantments, &stack, &ctx, &live, 7.0),
                    expected
                );
            }
            // The actual attack still sees the first random draw after all previews.
            assert_eq!(
                crate::game::combat_common::stack_enchanted_damage(
                    &enchantments,
                    &stack,
                    &ctx,
                    &mut live,
                    7.0
                ) - 7.0,
                expected
            );
        }
    }

    #[test]
    #[ignore = "daily"]
    fn stack_combat_queries_match_attacks_and_preserve_equipment_and_rng() {
        with_game(|game| {
            let attacker = actor(game);
            let target = attacker.entity();
            let sword = ItemStack::new("minecraft:diamond_sword", 1);
            game.common.inventory[game.common.selected] = Some(sword.clone());
            let plain = resource(&sword);
            assert_eq!(
                plain.attack_damage_bonus(game, attacker, target, 7.0),
                Some(0.0)
            );
            assert_eq!(
                plain
                    .damage_source(game, attacker)
                    .unwrap()
                    .damage_type
                    .to_string(),
                "minecraft:player_attack"
            );
            let range = plain.entity_attack_range(game, attacker).unwrap();
            assert_eq!((range.min, range.max, range.hitbox_margin), (0.0, 3.0, 0.0));
            let mut weapon = sword.clone();
            weapon.enchant("minecraft:sharpness", 3);
            let enchanted = resource(&weapon);
            let mut random = game.world.random.clone();
            assert_eq!(
                enchanted.attack_damage_bonus(game, attacker, target, 7.0),
                Some(2.0)
            );
            assert_eq!(game.world.random.next_float(), random.next_float());
            assert_eq!(
                game.held().unwrap().enchantments().len(),
                0,
                "query did not equip the weapon"
            );
            game.common.inventory[game.common.selected] = Some(weapon.clone());
            assert_eq!(
                enchanted
                    .attack_damage_bonus(game, attacker, target, 7.0)
                    .unwrap(),
                game.enchanted_damage("minecraft:player", 7.0, Some(target.id())) - 7.0
            );
            patch(
                &mut weapon,
                "!minecraft:enchantments",
                serde_json::json!({}),
            );
            // Explicit removal is tested on a stack without a simultaneous positive override.
            weapon.data_mut().enchantments.clear();
            assert_eq!(
                resource(&weapon).attack_damage_bonus(game, attacker, target, 7.0),
                Some(0.0)
            );

            let mut spear = ItemStack::new("minecraft:wooden_spear", 1);
            patch(
                &mut spear,
                "minecraft:attack_range",
                serde_json::json!({"min_reach": 2.0, "max_reach": 4.5, "hitbox_margin": 0.125}),
            );
            let range = resource(&spear)
                .entity_attack_range(game, attacker)
                .unwrap();
            assert_eq!(
                (range.min, range.max, range.hitbox_margin),
                (2.0, 4.5, 0.125)
            );
            game.common.inventory[game.common.selected] = Some(spear.clone());
            let eye = game.eye_position().unwrap();
            for distance in [1.875, 4.625, 7.625, 7.626] {
                let box_ = Aabb::new(
                    Vec3::new(eye[0] - 0.3, eye[1] - 1.0, eye[2] + distance),
                    Vec3::new(eye[0] + 0.3, eye[1] + 1.0, eye[2] + distance + 0.6),
                );
                assert_eq!(
                    game.within_attack_range(&box_),
                    range.contains(distance, 3.0)
                );
            }
            spear.data_mut().set_other("minecraft:attack_range", None);
            patch(&mut spear, "!minecraft:attack_range", serde_json::json!({}));
            assert_eq!(
                resource(&spear)
                    .entity_attack_range(game, attacker)
                    .unwrap(),
                plain.entity_attack_range(game, attacker).unwrap()
            );

            let mut mace = ItemStack::new("minecraft:mace", 1);
            mace.enchant("minecraft:density", 2);
            game.player.as_mut().unwrap().fall_distance = 4.0;
            let preview = resource(&mace);
            assert_eq!(
                preview
                    .damage_source(game, attacker)
                    .unwrap()
                    .damage_type
                    .to_string(),
                "minecraft:mace_smash"
            );
            assert_eq!(
                preview.attack_damage_bonus(game, attacker, target, 6.0),
                Some(18.0)
            );
            game.common.inventory[game.common.selected] = Some(mace.clone());
            assert_eq!(game.smash_damage_bonus("minecraft:player", None), 18.0);
            patch(
                &mut mace,
                "minecraft:damage_type",
                serde_json::json!("minecraft:magic"),
            );
            let overridden = resource(&mace).damage_source(game, attacker).unwrap();
            assert_eq!(overridden.damage_type.to_string(), "minecraft:magic");
            assert_eq!(overridden.direct, Some(target));
            assert_eq!(overridden.causing, Some(target));
            game.common.inventory[game.common.selected] = Some(mace);
            assert_eq!(
                sunrise_gameplay_common::game::creatures::source_damage_type(
                    game.attack_source([0.0; 3])
                ),
                "minecraft:magic"
            );

            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(sunrise_level::chunk::Chunk::empty(0, 0)),
            );
            game.mobs
                .push(sunrise_sim::mobs::Mob::new(21, kind, Vec3::ZERO, 0.0));
            let zombie = mod_api::LivingRef::new(game.entity(21).unwrap());
            assert_eq!(
                plain
                    .damage_source(game, zombie)
                    .unwrap()
                    .damage_type
                    .to_string(),
                "minecraft:mob_attack"
            );
            assert_eq!(plain.entity_attack_range(game, zombie).unwrap().max, 3.0);
            let mob_spear = resource(&ItemStack::new("minecraft:wooden_spear", 1))
                .entity_attack_range(game, zombie)
                .unwrap();
            assert_eq!(
                (mob_spear.min, mob_spear.max, mob_spear.hitbox_margin),
                (1.0, 2.25, 0.125)
            );
            assert_eq!(
                enchanted.attack_damage_bonus(game, zombie, target, 7.0),
                Some(2.0)
            );
            let stale =
                mod_api::LivingRef::new(mod_api::EntityRef::new(mod_api::WorldEpoch::new(999), 20));
            assert!(plain.entity_attack_range(game, stale).is_none());
            assert!(plain.damage_source(game, stale).is_none());
            assert!(
                plain
                    .attack_damage_bonus(game, stale, target, 1.0)
                    .is_none()
            );
            assert!(
                plain
                    .attack_damage_bonus(game, attacker, stale.entity(), 1.0)
                    .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn stack_combat_immunity_honors_default_override_tag_and_removal() {
        with_game(|game| {
            let attacker = actor(game);
            let mut stack = ItemStack::new("minecraft:netherite_chestplate", 1);
            let mut source = resource(&stack).damage_source(game, attacker).unwrap();
            source.damage_type = mod_api::Identifier::parse("minecraft:lava").unwrap();
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(false));
            assert_eq!(
                game.stack_can_be_hurt_by(&stack, "minecraft:lava"),
                Some(false)
            );
            source.damage_type = mod_api::Identifier::parse("minecraft:cactus").unwrap();
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(true));
            patch(
                &mut stack,
                "minecraft:damage_resistant",
                serde_json::json!({"types": ["minecraft:cactus"]}),
            );
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(false));
            assert_eq!(
                game.stack_can_be_hurt_by(&stack, "minecraft:cactus"),
                Some(false)
            );
            source.damage_type = mod_api::Identifier::parse("minecraft:lava").unwrap();
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(true));
            patch(
                &mut stack,
                "minecraft:damage_resistant",
                serde_json::json!({"types": "#minecraft:is_fire"}),
            );
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(false));
            stack
                .data_mut()
                .set_other("minecraft:damage_resistant", None);
            patch(
                &mut stack,
                "!minecraft:damage_resistant",
                serde_json::json!({}),
            );
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), Some(true));
            source.damage_type = mod_api::Identifier::parse("test:unknown").unwrap();
            assert_eq!(resource(&stack).can_be_hurt_by(game, &source), None);
        });
    }
    #[test]
    #[ignore = "daily"]
    fn stack_combat_prototype_overrides_and_removals_match_attacks() {
        with_game(|game| {
            let attacker = actor(game);
            let target = attacker.entity();
            let prototypes = std::sync::Arc::make_mut(
                &mut std::sync::Arc::make_mut(&mut game.pack.item_models).prototypes,
            );
            let prototype = prototypes.get_mut("minecraft:diamond_sword").unwrap();
            prototype.insert(
                "minecraft:enchantments".into(),
                serde_json::json!({"minecraft:sharpness": 3}),
            );
            prototype.insert(
                "minecraft:damage_type".into(),
                serde_json::json!("minecraft:magic"),
            );
            let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
            let defaults = resource(&sword);
            assert_eq!(
                defaults.attack_damage_bonus(game, attacker, target, 7.0),
                Some(2.0)
            );
            assert_eq!(
                defaults
                    .damage_source(game, attacker)
                    .unwrap()
                    .damage_type
                    .to_string(),
                "minecraft:magic"
            );
            game.common.inventory[game.common.selected] = Some(sword.clone());
            assert_eq!(
                game.enchanted_damage("minecraft:player", 7.0, Some(target.id())),
                9.0
            );
            assert_eq!(
                sunrise_gameplay_common::game::creatures::source_damage_type(
                    game.attack_source([0.0; 3])
                ),
                "minecraft:magic"
            );
            patch(&mut sword, "!minecraft:enchantments", serde_json::json!({}));
            patch(&mut sword, "!minecraft:damage_type", serde_json::json!({}));
            let removed = resource(&sword);
            assert_eq!(
                removed.attack_damage_bonus(game, attacker, target, 7.0),
                Some(0.0)
            );
            assert_eq!(
                removed
                    .damage_source(game, attacker)
                    .unwrap()
                    .damage_type
                    .to_string(),
                "minecraft:player_attack"
            );
            game.common.inventory[game.common.selected] = Some(sword);
            assert_eq!(
                game.enchanted_damage("minecraft:player", 7.0, Some(target.id())),
                7.0
            );
            assert_eq!(
                sunrise_gameplay_common::game::creatures::source_damage_type(
                    game.attack_source([0.0; 3])
                ),
                "minecraft:player_attack"
            );
        });
    }
}

use sunrise_gameplay_common::game::native_components::NativeComponentsGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
