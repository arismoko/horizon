//! Borrowed loot views and editors at the guarded game boundary. No inventory count rules apply.
use mod_api::__private::LootStackAccess;
use sunrise_world::loot as engine;

/// Adapt the evaluator's borrowed rows by value, not a loan of a temporary context.
pub fn loot_params(facts: engine::LootContext<'_>) -> mod_api::LootParamsView<'_> {
    use mod_api::__private::{
        LootDamageAccess, LootEntityAccess, LootLevelAccess, LootParamsParts,
    };
    mod_api::LootParamsView::new(
        LootParamsParts {
            queried_table_id: facts.queried_table_id,
            queried_stack: facts.queried_stack.map(|stack| {
                (
                    stack as &dyn LootStackAccess,
                    queried_inventory_view
                        as fn(&dyn LootStackAccess) -> Option<mod_api::ItemStackView<'_>>,
                )
            }),
            entity: facts.entity.map(|entity| entity as &dyn LootEntityAccess),
            last_damage_player: facts.player.map(|entity| entity as &dyn LootEntityAccess),
            interacting_entity: facts
                .interacting_entity
                .map(|entity| entity as &dyn LootEntityAccess),
            damage: facts.damage.map(|damage| damage as &dyn LootDamageAccess),
            tool: facts.tool,
            block: facts.block,
            biome: facts.biome,
            origin: facts.place.and_then(|place| place.origin),
            block_entity: facts.place.and_then(|place| place.block_entity.as_ref()),
            level: facts
                .place
                .and_then(|place| place.level.as_ref())
                .map(|level| level as &dyn LootLevelAccess),
            luck: facts.luck,
        },
        sunrise_sim::api_text::captured_text,
    )
}

fn encode_loot_name(
    text: &mod_api::text::Text,
) -> Result<serde_json::Value, mod_api::TextFormError> {
    sunrise_sim::api_text::to_serialized_engine(text).map(|text| text.component())
}

/// Borrow one raw loot candidate without converting it to an owned API stack.
pub fn loot_stack_view(stack: &dyn engine::LootStackAccess) -> mod_api::LootStackView<'_> {
    mod_api::LootStackView::new(stack, sunrise_sim::api_text::captured_text)
}

/// Borrow the evaluator's live candidate for one function invocation.
pub fn loot_stack_editor(
    stack: &mut dyn engine::LootStackEditAccess,
) -> mod_api::LootStackEditor<'_> {
    mod_api::LootStackEditor::new(
        stack,
        sunrise_sim::api_text::captured_text,
        encode_loot_name,
    )
}

/// Borrow the evaluator's live generated loot list for one global modifier invocation.
pub fn loot_list(stacks: &mut dyn engine::LootListAccess) -> mod_api::LootList<'_> {
    mod_api::LootList::new(
        stacks,
        sunrise_sim::api_text::captured_text,
        encode_loot_name,
    )
}

/// The damage component the ordered patch leaves on a stack of base damage `base`, read as
/// `LootCandidate::write_component` writes it. Merely reading these scalars must not
/// materialize the rest of the patch.
fn queried_damage(
    base: u32,
    components: &[(String, Option<sunrise_core::nbt::Tag>)],
) -> Option<u32> {
    let mut damage = base;
    let mut explicit = false;
    for (id, value) in components {
        match (id.as_str(), value) {
            ("minecraft:damage", None) | ("!minecraft:damage", Some(_)) => {
                damage = 0;
                explicit = false;
            }
            ("minecraft:damage", Some(tag)) => {
                damage = sunrise_world::datafix::dynamic::as_int(Some(tag), 0).max(0) as u32;
                explicit = true;
            }
            _ => {}
        }
    }
    (damage > 0 || explicit).then_some(damage)
}

fn queried_inventory_view(stack: &dyn LootStackAccess) -> Option<mod_api::ItemStackView<'_>> {
    let item = mod_api::__private::item_key(stack.item_id())?;
    let count = std::num::NonZeroU32::new(u32::try_from(stack.count()).ok()?)?;
    let damage = queried_damage(u32::try_from(stack.damage()).ok()?, stack.components());
    Some(mod_api::ItemStackView::queried_loot(
        (item, count, damage),
        stack,
        project_queried_stack,
    ))
}

fn project_queried_stack(
    stack: &dyn LootStackAccess,
    metadata: super::StackReadMetadata,
) -> Option<mod_api::ItemStack> {
    // This is called only by ItemStackView::project, when the mod requests an owned value.
    let materialized = sunrise_items::items::ItemStack::from_loot(&engine::LootStack {
        item: stack.item_id().to_owned(),
        count: stack.count(),
        damage: stack.damage(),
        enchantments: stack.enchantments().to_vec(),
        potion: stack.potion().map(str::to_owned),
        map: stack.map(),
        exploration: stack.exploration().map(|map| {
            Box::new(engine::ExplorationMap {
                destination: map.destination().to_owned(),
                decoration: map.decoration().to_owned(),
                zoom: map.zoom(),
                search_radius: map.search_radius(),
                skip_existing_chunks: map.skip_existing_chunks(),
            })
        }),
        ominous_amplifier: stack.ominous_amplifier(),
        stew_effects: stack.stew_effects().to_vec(),
        names: stack.names().to_vec(),
        components: stack.components().to_vec(),
    });
    super::project_stack(&materialized, metadata).ok()
}

pub fn dispatch_loot_ability(
    tool: &dyn engine::LootAbilityTool,
    ability: &str,
    tags: &dyn engine::LootTags,
    holds: &mut bool,
) {
    if tool.is_empty()
        || !mod_api::__private::item_listens(mod_api::HookId::CanPerformAction, tool.item_id())
    {
        return;
    }
    let tags = crate::game::loot_tags::LootTags(tags);
    let context = mod_api::ItemAbilityContext::new(&tags);
    if let Some(loot_stack) = tool.as_loot_stack() {
        let Some(stack) = queried_inventory_view(loot_stack) else {
            return;
        };
        mod_api::dispatch_can_perform_action(
            &context,
            mod_api::__private::CanPerformActionCall {
                stack,
                ability,
                can_perform: holds,
            },
        );
        return;
    }
    let Some((count, damage)) = tool.resident_scalars() else {
        return;
    };
    let Some(key) = mod_api::__private::item_key(tool.item_id()) else {
        return;
    };
    let project = |metadata| super::projected_stack(metadata, tool.encode_components()).ok();
    mod_api::dispatch_can_perform_action(
        &context,
        mod_api::__private::CanPerformActionCall {
            stack: mod_api::ItemStackView::resident((key, count, damage), &project),
            ability,
            can_perform: holds,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The borrowed scalar follows the same ordered writes and removals as materializing the
    /// stack: a write over zero base damage, a removal, and a later write.
    #[test]
    fn borrowed_damage_matches_materialization_of_ordered_writes() {
        use sunrise_core::nbt::Tag;
        let removal = || Tag::Compound(Default::default());
        let write = |value| ("minecraft:damage".to_owned(), Some(Tag::Int(value)));
        let cases: [(u32, Vec<(String, Option<Tag>)>); 5] = [
            (0, vec![write(19)]),
            (7, vec![write(19)]),
            (7, vec![write(19), ("minecraft:damage".into(), None)]),
            (
                7,
                vec![("!minecraft:damage".into(), Some(removal())), write(3)],
            ),
            (0, vec![write(0)]),
        ];
        for (base, components) in cases {
            let borrowed = queried_damage(base, &components);
            let stack = sunrise_items::items::ItemStack::from_loot(&engine::LootStack {
                item: "minecraft:stick".into(),
                count: 1,
                damage: base as i32,
                enchantments: Vec::new(),
                potion: None,
                map: false,
                exploration: None,
                ominous_amplifier: None,
                stew_effects: Vec::new(),
                names: Vec::new(),
                components: components.clone(),
            });
            assert_eq!(
                borrowed.unwrap_or(0),
                stack.damage,
                "base {base}, patch {components:?}"
            );
        }
        assert_eq!(queried_damage(0, &[write(19)]), Some(19));
        assert_eq!(queried_damage(0, &[write(0)]), Some(0));
        assert_eq!(
            queried_damage(0, &[write(19), ("minecraft:damage".into(), None)]),
            None
        );
    }
    include!("loot_tags_tests.rs");
    use mod_api::{
        Hooks, LootStackEditError, LootStackEditor, LootTypeContext, ModEntry,
        ModLootConditionCodec, ModLootFunction, ModLootFunctionCodec,
    };
    fn apply_type(
        kind: &str,
        data: &serde_json::Value,
        ctx: &engine::LootContext<'_>,
        stack: &mut engine::LootStack,
        tags: &dyn engine::LootTags,
    ) {
        let types = crate::game::loot_types::capture();
        if !types.contains(kind, true) {
            return;
        }
        let mut entry = data.clone();
        if let Some(object) = entry.as_object_mut() {
            object.insert("type".into(), serde_json::Value::String(kind.into()));
        }
        let modifiers = std::sync::Arc::new(std::collections::HashMap::from([(
            "test:entry".into(),
            entry,
        )]));
        let bindings = types
            .bind_generation(
                std::sync::Arc::default(),
                std::sync::Arc::default(),
                std::sync::Arc::clone(&modifiers),
                Vec::new(),
                Default::default(),
            )
            .unwrap();
        let mut random = sunrise_core::random::LegacyRandom::new(0);
        bindings
            .function(None, kind, &modifiers["test:entry"])
            .unwrap()
            .apply(ctx, &mut random, tags, stack);
    }
    fn test_type(
        kind: &str,
        data: &serde_json::Value,
        ctx: &engine::LootContext<'_>,
        holds: &mut bool,
        tags: &dyn engine::LootTags,
    ) {
        let types = crate::game::loot_types::capture();
        if !types.contains(kind, false) {
            return;
        }
        let mut entry = data.clone();
        if let Some(object) = entry.as_object_mut() {
            object.insert("type".into(), serde_json::Value::String(kind.into()));
        }
        let predicates = std::sync::Arc::new(std::collections::HashMap::from([(
            "test:entry".into(),
            entry,
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
        let mut random = sunrise_core::random::LegacyRandom::new(0);
        *holds = bindings
            .condition(None, kind, &predicates["test:entry"])
            .unwrap()
            .test(ctx, &mut random, tags);
    }
    use mod_platform::testing::with_mods;
    use sunrise_core::nbt::Tag;
    fn stack() -> engine::LootStack {
        engine::LootStack {
            item: "minecraft:stick".into(),
            count: -7,
            damage: -3,
            enchantments: vec![],
            potion: None,
            map: true,
            exploration: None,
            ominous_amplifier: Some(2),
            stew_effects: vec![],
            names: vec![],
            components: vec![
                ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
                ("test:byte".into(), Some(Tag::Byte(7))),
            ],
        }
    }
    fn context() -> engine::LootContext<'static> {
        engine::LootContext {
            luck: 1.5,
            ..Default::default()
        }
    }

    struct Facts;
    #[mod_api::__subscriptions]
    impl Hooks for Facts {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_loot_condition_type(
                &mod_api::Identifier::parse("facts:read").unwrap(),
                Facts,
            );
        }
    }
    impl ModLootConditionCodec for Facts {
        fn decode(
            &self,
            _: &serde_json::Value,
        ) -> Result<std::sync::Arc<dyn mod_api::ModLootCondition>, String> {
            Ok(std::sync::Arc::new(Facts))
        }
    }
    impl mod_api::ModLootCondition for Facts {
        fn test(&self, context: &mut dyn LootTypeContext) -> Result<bool, String> {
            let params = context.params();
            let victim = params.entity().unwrap();
            assert_eq!(victim.vehicle().unwrap().kind(), "minecraft:boat");
            assert_eq!(victim.sheep(), Some(("red", true)));
            assert!(victim.cube_size().is_none());
            let candidate = victim.equipment("mainhand").unwrap();
            assert_eq!(candidate.item_id(), "minecraft:stick");
            assert_eq!(
                (candidate.count(), candidate.damage(), candidate.map()),
                (-7, -3, true)
            );
            let mut edits = candidate.components();
            assert_eq!(
                edits.next(),
                Some(("test:array", Some(&Tag::ByteArray(vec![7]))))
            );
            assert_eq!(edits.next(), Some(("test:byte", None)));
            assert!(victim.equipment("offhand").is_none());
            let damage = params.damage().unwrap();
            assert_eq!(damage.source().unwrap().kind(), "minecraft:skeleton");
            assert_eq!(damage.direct().unwrap().kind(), "minecraft:arrow");
            assert_eq!(params.origin(), Some([1.0, 2.0, 3.0]));
            let level = params.level().unwrap();
            assert_eq!(
                (level.dimension(), level.game_time(), level.day_time()),
                ("test:level", 17, -3)
            );
            assert!(level.raining() && !level.thundering());
            assert!(params.block_entity().is_none());
            assert_eq!(
                params.interacting_entity().map(|entity| entity.kind()),
                Some("minecraft:player")
            );
            assert!(params.tool().is_none());
            assert!(params.block().is_none());
            Ok(true)
        }
    }

    #[test]
    fn loot_params_lend_only_supplied_facts_without_drawing() {
        use sunrise_core::random::{LegacyRandom, RandomSource};
        let mut victim = engine::LootEntity::new("minecraft:sheep");
        victim.sheep = Some(("red".into(), true));
        victim.vehicle = Some(Box::new(engine::LootEntity::new("minecraft:boat")));
        let mut equipment = stack();
        equipment.components[1].1 = None;
        victim.equipment.push(("mainhand".into(), equipment));
        let actor = engine::LootEntity::new("minecraft:player");
        let damage = engine::LootDamage {
            kind: "minecraft:arrow".into(),
            source: Some(engine::LootEntity::new("minecraft:skeleton")),
            direct: Some(engine::LootEntity::new("minecraft:arrow")),
            is_direct: false,
        };
        let place = engine::LootPlace {
            origin: Some([1.0, 2.0, 3.0]),
            level: Some(engine::LootLevel {
                dimension: "test:level".into(),
                game_time: 17,
                day_time: -3,
                clocks: Default::default(),
                raining: true,
                thundering: false,
            }),
            ..Default::default()
        };
        engine::with_native_loot_place(place, || {
            let place = engine::current_native_loot_place().unwrap();
            let facts = engine::LootContext {
                entity: Some(&victim),
                interacting_entity: Some(&actor),
                damage: Some(&damage),
                place: Some(&place),
                ..Default::default()
            };
            let mut random = LegacyRandom::new(71);
            with_mods(vec![ModEntry::new("facts", Box::new(Facts))].into(), || {
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                let mut holds = false;
                test_type(
                    "facts:read",
                    &serde_json::json!({}),
                    &facts,
                    &mut holds,
                    &TestTags,
                );
                assert!(holds);
            });
            assert_eq!(random.next_long(), LegacyRandom::new(71).next_long());
        });
    }

    struct Edit;
    #[mod_api::__subscriptions]
    impl Hooks for Edit {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_loot_function_type(&mod_api::Identifier::parse("edit:edit").unwrap(), Edit);
        }
    }
    impl ModLootFunctionCodec for Edit {
        fn decode(
            &self,
            _: &serde_json::Value,
        ) -> Result<std::sync::Arc<dyn ModLootFunction>, String> {
            Ok(std::sync::Arc::new(Edit))
        }
    }
    impl ModLootFunction for Edit {
        fn apply(
            &self,
            stack: &mut LootStackEditor<'_>,
            context: &mut dyn LootTypeContext,
        ) -> Result<(), String> {
            assert_eq!(context.params().luck(), 1.5);
            {
                let view = stack.view();
                assert_eq!(view.count(), -7);
                let mut components = view.components();
                assert_eq!(
                    components.next(),
                    Some(("test:array", Some(&Tag::ByteArray(vec![7]))))
                );
                assert_eq!(components.next(), Some(("test:byte", Some(&Tag::Byte(7)))));
                assert_eq!(components.next(), None);
            }
            stack.set_count(-12);
            stack.remove_component("test:array")?;
            stack.push_component("test:array", Tag::List(vec![Tag::Int(7)]))?;
            stack.push_component("test:array", Tag::ByteArray(vec![8]))?;
            Ok(())
        }
    }
    #[test]
    fn loot_type_preserves_signed_candidates_and_ordered_nbt() {
        super::super::prepare_stack_boundary_tests();
        let mut output = stack();
        with_mods(vec![ModEntry::new("edit", Box::new(Edit))].into(), || {
            super::super::prepare_stack_boundary_tests();
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            super::super::resolve_stack_boundary_test_items();
            apply_type(
                "edit:edit",
                &serde_json::json!({}),
                &context(),
                &mut output,
                &TestTags,
            );
        });
        assert_eq!(output.count, -12);
        assert_eq!(output.damage, -3);
        assert!(output.map);
        assert_eq!(
            output.components,
            vec![
                ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
                ("test:byte".into(), Some(Tag::Byte(7))),
                ("test:array".into(), None),
                ("test:array".into(), Some(Tag::List(vec![Tag::Int(7)]))),
                ("test:array".into(), Some(Tag::ByteArray(vec![8]))),
            ]
        );
    }
    #[test]
    fn loot_no_subscriber_does_not_convert() {
        super::super::prepare_stack_boundary_tests();
        with_mods(Vec::new().into(), || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            let mut output = stack();
            let before = output.clone();
            let mut holds = false;
            for _ in 0..1000 {
                apply_type(
                    "edit:edit",
                    &serde_json::json!({}),
                    &context(),
                    &mut output,
                    &TestTags,
                );
                test_type(
                    "edit:edit",
                    &serde_json::json!({}),
                    &context(),
                    &mut holds,
                    &TestTags,
                );
            }
            assert_eq!(output, before);
            assert!(!holds);
        });
    }
    #[test]
    fn loot_patch_depth_matches_the_vanilla_nbt_limit() {
        super::super::prepare_stack_boundary_tests();
        let mut target = stack();
        let mut value = loot_stack_view(&target).to_owned().unwrap();
        let mut tag = Tag::Byte(7);
        for _ in 0..512 {
            tag = Tag::List(vec![tag]);
        }
        value.components = vec![("test:deep".into(), Some(tag))];
        assert!(
            loot_stack_editor(&mut target)
                .replace(value.clone())
                .is_ok()
        );
        let tag = value.components[0].1.take().unwrap();
        value.components[0].1 = Some(Tag::List(vec![tag]));
        assert_eq!(
            loot_stack_editor(&mut target).replace(value),
            Err(LootStackEditError::InvalidPatch)
        );
    }

    #[test]
    fn loot_writeback_keeps_repeated_patch_order() {
        super::super::prepare_stack_boundary_tests();
        let mut value =
            mod_api::LootStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), -3);
        value.components = vec![
            ("test:value".into(), Some(Tag::Byte(7))),
            ("test:value".into(), Some(Tag::ByteArray(vec![8]))),
        ];
        let mut list = engine::LootStacks::default();
        loot_list(&mut list).push(value).unwrap();
        assert_eq!(list.0[0].count, -3);
        assert_eq!(
            list.0[0].components,
            vec![
                ("test:value".into(), Some(Tag::Byte(7))),
                ("test:value".into(), Some(Tag::ByteArray(vec![8]))),
            ]
        );
    }
    #[test]
    fn loot_writeback_keeps_component_removals() {
        super::super::prepare_stack_boundary_tests();
        let mut value =
            mod_api::LootStack::new(mod_api::__private::item_key("minecraft:stick").unwrap(), 1);
        value.components = vec![("test:removed".into(), None)];
        let mut list = engine::LootStacks::default();
        loot_list(&mut list).push(value).unwrap();
        assert_eq!(list.0[0].components, vec![("test:removed".into(), None)]);
    }

    struct ScopedInvalid(std::sync::Arc<std::sync::atomic::AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for ScopedInvalid {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            r.register_loot_function_type(
                &mod_api::Identifier::parse("invalid:invalid").unwrap(),
                ScopedInvalid(self.0.clone()),
            );
        }
    }
    impl ModLootFunctionCodec for ScopedInvalid {
        fn decode(
            &self,
            _: &serde_json::Value,
        ) -> Result<std::sync::Arc<dyn ModLootFunction>, String> {
            Ok(std::sync::Arc::new(ScopedInvalid(self.0.clone())))
        }
    }
    impl ModLootFunction for ScopedInvalid {
        fn apply(
            &self,
            stack: &mut LootStackEditor<'_>,
            _: &mut dyn LootTypeContext,
        ) -> Result<(), String> {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
            stack.set_count(-19);
            stack.remove_component("test:array")?;
            stack.push_component("test:array", Tag::ByteArray(vec![9]))?;
            let mut text = mod_api::text::Text::literal("end");
            for _ in 0..513 {
                let mut parent = mod_api::text::Text::literal("");
                parent.children.push(text);
                text = parent;
            }
            assert!(matches!(
                stack.push_name("minecraft:custom_name", &text),
                Err(LootStackEditError::InvalidName(_))
            ));
            let mut deep = Tag::Byte(7);
            for _ in 0..513 {
                deep = Tag::List(vec![deep]);
            }
            assert_eq!(
                stack.push_component("test:deep", deep),
                Err(LootStackEditError::InvalidPatch)
            );
            Err("refused invalid write".into())
        }
    }
    #[test]
    fn loot_scoped_editor_keeps_prior_edits_and_refuses_invalid_writes() {
        super::super::prepare_stack_boundary_tests();
        let mut output = stack();
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        with_mods(
            vec![ModEntry::new(
                "invalid",
                Box::new(ScopedInvalid(called.clone())),
            )]
            .into(),
            || {
                super::super::prepare_stack_boundary_tests();
                mod_platform::__private::part_tests::install_parts_fixture(
                    mod_api::__private::mods(),
                )
                .unwrap();
                super::super::resolve_stack_boundary_test_items();
                apply_type(
                    "invalid:invalid",
                    &serde_json::json!({}),
                    &context(),
                    &mut output,
                    &TestTags,
                );
            },
        );
        assert!(called.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(output.count, -19);
        assert!(output.names.is_empty());
        assert_eq!(
            output.components,
            vec![
                ("test:array".into(), Some(Tag::ByteArray(vec![7]))),
                ("test:byte".into(), Some(Tag::Byte(7))),
                ("test:array".into(), None),
                ("test:array".into(), Some(Tag::ByteArray(vec![9]))),
            ]
        );
    }
}

#[cfg(test)]
#[path = "loot_query_tests.rs"]
mod query_tests;
