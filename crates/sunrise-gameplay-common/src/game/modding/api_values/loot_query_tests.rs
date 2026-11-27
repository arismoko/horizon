use super::*;
use mod_api::{Hooks, ItemPart, Registrar, RegistrationContext};
struct Reader;
#[mod_api::__subscriptions]
impl Hooks for Reader {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        r.register_loot_condition_type(
            &mod_api::Identifier::parse("reader:query").unwrap(),
            Reader,
        );
    }
}
impl mod_api::ModLootConditionCodec for Reader {
    fn decode(
        &self,
        _: &serde_json::Value,
    ) -> Result<std::sync::Arc<dyn mod_api::ModLootCondition>, String> {
        Ok(std::sync::Arc::new(Reader))
    }
}
impl mod_api::ModLootCondition for Reader {
    fn test(&self, context: &mut dyn mod_api::LootTypeContext) -> Result<bool, String> {
        let params = context.params();
        assert_eq!(params.queried_table_id(), "minecraft:blocks/short_grass");
        let stack = params.queried_stack().unwrap();
        assert_eq!(
            mod_api::__private::item_key_id(stack.item()).unwrap(),
            "minecraft:stick"
        );
        assert_eq!(stack.count(), 3);
        assert_eq!(stack.damage(), 2);
        let retained = stack.project().unwrap();
        assert_eq!((retained.count(), retained.damage()), (3, 2));
        Ok(true)
    }
}
#[test]
fn queried_parameters_reach_native_mod_condition() {
    crate::game::api_values::prepare_stack_boundary_tests();
    let stack = engine::LootStack {
        item: "minecraft:stick".into(),
        count: 3,
        damage: 2,
        enchantments: vec![],
        potion: None,
        map: false,
        exploration: None,
        ominous_amplifier: None,
        stew_effects: vec![],
        names: vec![],
        components: vec![],
    };
    let ctx = engine::LootContext {
        queried_table_id: Some("minecraft:blocks/short_grass"),
        queried_stack: Some(&stack),
        ..Default::default()
    };
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("reader", Box::new(Reader))].into(),
        || {
            crate::game::api_values::prepare_stack_boundary_tests();
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            crate::game::api_values::resolve_stack_boundary_test_items();
            let mut random = sunrise_core::random::LegacyRandom::new(0);
            let types = crate::game::loot_types::capture();
            let predicates = std::sync::Arc::new(std::collections::HashMap::from([(
                "reader:pred".into(),
                serde_json::json!({"type": "reader:query"}),
            )]));
            let bindings = types
                .bind_generation(
                    std::sync::Arc::default(),
                    std::sync::Arc::clone(&predicates),
                    std::sync::Arc::default(),
                    Vec::new(),
                    Default::default(),
                )
                .unwrap();
            let holds = bindings
                .condition(None, "reader:query", &predicates["reader:pred"])
                .unwrap()
                .test(&ctx, &mut random, &Tags);
            assert!(holds);
        },
    );
}
#[derive(Clone)]
struct Ability {
    answer: bool,
    expected_item: &'static str,
    expected_initial: bool,
}
#[mod_api::sunrise_part]
impl ItemPart for Ability {
    fn can_perform_action(
        &self,
        context: &mod_api::ItemAbilityContext<'_>,
        event: &mut mod_api::CanPerformActionEvent<'_>,
    ) {
        assert_eq!(
            mod_api::__private::item_key_id(event.stack.item()),
            Some(self.expected_item)
        );
        let tag = context
            .tags()
            .key(&mod_api::Identifier::parse("test:ability_tools").unwrap())
            .expect("tag key resolves");
        assert_eq!(
            context.tags().is_in_tag(event.stack.item(), tag.typed()),
            Some(true)
        );
        if event.ability == "shears_dig" {
            assert_eq!(event.can_perform, self.expected_initial);
            event.can_perform = self.answer;
        }
    }
}
struct Abilities;
#[mod_api::__subscriptions]
impl Hooks for Abilities {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let grant = r.item_key("minecraft:stick");
        let deny = r.item_key("minecraft:shears");
        r.item_part(
            grant,
            Ability {
                answer: true,
                expected_item: "minecraft:stick",
                expected_initial: false,
            },
        );
        r.item_part(
            deny,
            Ability {
                answer: false,
                expected_item: "minecraft:shears",
                expected_initial: true,
            },
        );
    }
}
#[test]
#[ignore = "daily"]
fn queried_ability_parts_grant_and_deny() {
    crate::game::in_own_process(
        "game::api_values::loot_values::query_tests::queried_ability_parts_grant_and_deny",
        queried_ability_parts_grant_and_deny_isolated,
    );
}
fn queried_ability_parts_grant_and_deny_isolated() {
    crate::game::api_values::prepare_stack_boundary_tests();
    assert!(
        sunrise_world::native_load_hooks::install(
            sunrise_world::native_load_hooks::NativeLoadHooks {
                item_ability_default: sunrise_items::item_abilities::vanilla_can_perform,
                loot_ability: dispatch_loot_ability,
                loot_table_load: |_, _, _| {},
                modify_default_components: |_, _, _| {},
            }
        )
        .is_ok()
    );
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("abilities", Box::new(Abilities))].into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_api::__private::mark_game_thread();
        mod_api::__private::resolve_items([
            "minecraft:stick",
            "minecraft:shears",
            "minecraft:stone_pickaxe",
        ])
        .unwrap();
        for (item, initial, expected) in [
            ("minecraft:stick", false, true),
            ("minecraft:shears", true, false),
            ("minecraft:stone_pickaxe", false, false),
        ] {
            let tool = engine::LootStack {
                item: item.into(),
                count: 1,
                damage: 0,
                enchantments: vec![],
                potion: None,
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: vec![],
                names: vec![],
                components: vec![],
            };
            let items = sunrise_items::items::Items::for_tests(Default::default());
            let mut holds = items.vanilla_can_perform(item, "shears_dig");
            assert_eq!(holds, initial);
            dispatch_loot_ability(&tool, "shears_dig", &Tags, &mut holds);
            assert_eq!(holds, expected);

            // Resident `sunrise_items::ItemStack` tools also flow through lazily without eager conversion.
            let resident = sunrise_items::items::ItemStack::new(item, 1);
            let mut resident_holds = items.vanilla_can_perform(item, "shears_dig");
            dispatch_loot_ability(&resident, "shears_dig", &Tags, &mut resident_holds);
            assert_eq!(resident_holds, expected);

            let query = engine::query::LootQuery::default();
            query.set_id("minecraft:blocks/short_grass");
            for ability in ["shears_dig", "unknown_ability", "minecraft:shears_dig"] {
                let condition = serde_json::json!({"type":"neoforge:can_item_perform_ability","ability":ability});
                let answer = engine::query::condition(
                    &condition,
                    &query,
                    Some(&tool as &dyn engine::LootAbilityTool),
                    &Tags,
                    |item, ability| items.vanilla_can_perform(item, ability),
                );
                assert_eq!(answer, Some(ability == "shears_dig" && expected));
                assert_eq!(
                    engine::query::condition(&condition, &query, None, &Tags, |item, ability| {
                        items.vanilla_can_perform(item, ability)
                    },),
                    Some(false)
                );
            }
            // Empty or air tools and unrelated conditions never dispatch.
            let empty_tool = engine::LootStack {
                count: 0,
                ..tool.clone()
            };
            let air_tool = engine::LootStack {
                item: "minecraft:air".into(),
                ..tool.clone()
            };
            let ability_condition = serde_json::json!({"type":"neoforge:can_item_perform_ability","ability":"shears_dig"});
            for skipped in [&empty_tool, &air_tool] {
                assert_eq!(
                    engine::query::condition(
                        &ability_condition,
                        &query,
                        Some(skipped as &dyn engine::LootAbilityTool),
                        &Tags,
                        |item, ability| items.vanilla_can_perform(item, ability),
                    ),
                    Some(false)
                );
            }
            let unrelated = serde_json::json!({"type":"minecraft:random_chance","chance":0.5});
            assert_eq!(
                engine::query::condition(
                    &unrelated,
                    &query,
                    Some(&tool as &dyn engine::LootAbilityTool),
                    &Tags,
                    |item, ability| items.vanilla_can_perform(item, ability),
                ),
                None
            );
        }
    });
}

struct Tags;
impl engine::LootTags for Tags {
    fn members(&self, registry: &str, tag: &str) -> Option<Vec<String>> {
        (registry == "item" && tag == "test:ability_tools")
            .then(|| vec!["minecraft:stick".into(), "minecraft:shears".into()])
    }
    fn item_tag_names(&self) -> Option<&engine::ItemTagNames> {
        None
    }
}
