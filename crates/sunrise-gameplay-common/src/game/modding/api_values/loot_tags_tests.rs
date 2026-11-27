struct TestTags;
impl engine::LootTags for TestTags {
    fn members(&self, _: &str, _: &str) -> Option<Vec<String>> {
        None
    }
    fn item_tag_names(&self) -> Option<&engine::ItemTagNames> {
        None
    }
}
struct TagReader;
#[mod_api::__subscriptions]
impl Hooks for TagReader {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_loot_function_type(
            &mod_api::Identifier::parse("tag_reader:count").unwrap(),
            TagReader,
        );
        r.register_loot_condition_type(
            &mod_api::Identifier::parse("tag_reader:member").unwrap(),
            TagReader,
        );
    }
}
struct TagReaderRule {
    tag: String,
}
impl mod_api::ModLootFunctionCodec for TagReader {
    fn decode(
        &self,
        data: &serde_json::Value,
    ) -> Result<std::sync::Arc<dyn ModLootFunction>, String> {
        Ok(std::sync::Arc::new(TagReaderRule {
            tag: data["tag"]
                .as_str()
                .unwrap_or("test:ingredients")
                .to_owned(),
        }))
    }
}
impl ModLootFunction for TagReaderRule {
    fn apply(
        &self,
        stack: &mut mod_api::LootStackEditor<'_>,
        context: &mut dyn LootTypeContext,
    ) -> Result<(), String> {
        assert!(context.params().tool().is_none());
        let tags = context.tags();
        let tag = tags
            .key(&mod_api::Identifier::parse(&self.tag).unwrap())
            .unwrap()
            .typed();
        let count = match tags.items(tag) {
            mod_api::TagMembers::Members(members) if members.is_empty() => 1,
            mod_api::TagMembers::Members(members) => {
                assert_eq!(
                    members
                        .iter()
                        .map(|key| mod_api::__private::item_key_id(*key).unwrap())
                        .collect::<Vec<_>>(),
                    ["minecraft:potion", "minecraft:stick"]
                );
                2
            }
            mod_api::TagMembers::Missing => 0,
            mod_api::TagMembers::Unavailable => panic!("loot has a loaded item table"),
            other => panic!("unexpected tag lookup result: {other:?}"),
        };
        stack.set_count(count);
        Ok(())
    }
}
impl mod_api::ModLootConditionCodec for TagReader {
    fn decode(
        &self,
        _: &serde_json::Value,
    ) -> Result<std::sync::Arc<dyn mod_api::ModLootCondition>, String> {
        Ok(std::sync::Arc::new(TagReader))
    }
}
impl mod_api::ModLootCondition for TagReader {
    fn test(&self, context: &mut dyn LootTypeContext) -> Result<bool, String> {
        let tags = context.tags();
        let tag = tags
            .key(&mod_api::Identifier::parse("test:ingredients").unwrap())
            .unwrap()
            .typed();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let holds = tags.is_in_tag(stick, tag).unwrap();
        let missing = tags
            .key(&mod_api::Identifier::parse("test:missing").unwrap())
            .unwrap()
            .typed();
        assert_eq!(tags.is_in_tag(stick, missing), Some(false));
        assert_eq!(tags.items(missing), mod_api::TagMembers::Missing);
        Ok(holds)
    }
}
#[test]
#[ignore = "daily"]
#[cfg(any())]
fn loot_types_read_current_membership_and_order_after_tag_reload() {
    use crate::game::{loot_tags::LootTags, native_entity_refs::tests::run_fresh_cases};
    use mod_api::{Identifier, Tags};
    use std::collections::HashMap;
    use sunrise_items::items::{Items, TagRegistry};

    if run_fresh_cases(
        "game::api_values::loot_values::tests::loot_types_read_current_membership_and_order_after_tag_reload",
        "SUNRISE_LOOT_TAG_LIST_FIXTURE",
        &["reload"],
    ) {
        return;
    }
    super::super::prepare_stack_boundary_tests();
    let mut items = Items::for_tests(HashMap::new());
    with_mods(
        vec![ModEntry::new("tag_reader", Box::new(TagReader))].into(),
        || {
            super::super::prepare_stack_boundary_tests();
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            super::super::resolve_stack_boundary_test_items();
            let name = Identifier::parse("test:ingredients").unwrap();
            let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
            for (members, expected_count, expected_holds, item_tags, tag_names) in [
                (
                    Some(vec![
                        "minecraft:potion".to_owned(),
                        "minecraft:stick".to_owned(),
                    ]),
                    2,
                    true,
                    vec![name.clone()],
                    vec![name.clone()],
                ),
                (Some(Vec::new()), 1, false, vec![], vec![name]),
                (None, 0, false, vec![], vec![]),
            ] {
                items.set_tags(
                    TagRegistry::Item,
                    members
                        .into_iter()
                        .map(|members| ("test:ingredients".into(), members))
                        .collect(),
                );
                let tags = LootTags(&items);
                assert_eq!(tags.item_tags(stick), Some(item_tags));
                assert_eq!(tags.item_tag_names(), Some(tag_names));

                let mut value = stack();
                let mut holds = false;
                apply_type(
                    "tag_reader:count",
                    &serde_json::json!({}),
                    &context(),
                    &mut value,
                    &items,
                );
                test_type(
                    "tag_reader:member",
                    &serde_json::json!({}),
                    &context(),
                    &mut holds,
                    &items,
                );
                assert_eq!(value.count, expected_count);
                assert_eq!(holds, expected_holds);
            }
        },
    );
}

#[test]
fn registered_loot_function_reads_only_tags_bound_by_the_pack_loader() {
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::loot::{LootData, PackOverlay};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let cases = crate::game::native_tags::PACK_BINDING_CASES;
    let mut files = Vec::new();
    for (name, bytes, _) in cases {
        files.push((format!("tags/binding:item/{name}"), vec![bytes.to_vec()]));
        let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}],
            "modifier":{"type":"tag_reader:count","tag":format!("binding:{name}")}});
        files.push((
            format!("loot_table/binding:{name}"),
            vec![serde_json::to_vec(&table).unwrap()],
        ));
    }
    with_mods(
        vec![ModEntry::new("tag_reader", Box::new(TagReader))].into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let loot = LootData::load_with_types(
                root.path(),
                &PackOverlay::new(&files),
                &defaults,
                crate::game::loot_types::capture(),
            )
            .unwrap();
            for (name, _, is_bound) in cases {
                let drops = loot.roll(
                    &format!("binding:{name}"),
                    &mut LegacyRandom::new(0),
                    0.0,
                    None,
                    None,
                );
                assert_eq!(
                    drops.iter().map(|stack| stack.count).sum::<i32>(),
                    i32::from(is_bound),
                    "{name}"
                );
            }
        },
    );
}

/// A loot function reading damage type tags through its context, recording what it read:
/// a pack type in and out of a pack tag, an absent tag and an unknown type.
#[cfg(any())]
struct DamageTagReader(std::sync::Arc<std::sync::Mutex<Vec<[Option<bool>; 4]>>>);
#[cfg(any())]
#[mod_api::__subscriptions]
impl Hooks for DamageTagReader {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_loot_function_type(
            &mod_api::Identifier::parse("damage_tag_reader:probe").unwrap(),
            DamageTagReader(self.0.clone()),
        );
    }
}

#[cfg(any())]
impl mod_api::ModLootFunctionCodec for DamageTagReader {
    fn decode(&self, _: &serde_json::Value) -> Result<std::sync::Arc<dyn ModLootFunction>, String> {
        Ok(std::sync::Arc::new(DamageTagReader(self.0.clone())))
    }
}
#[cfg(any())]
impl ModLootFunction for DamageTagReader {
    fn apply(
        &self,
        _: &mut mod_api::LootStackEditor<'_>,
        context: &mut dyn LootTypeContext,
    ) -> Result<(), String> {
        let tags = context.tags();
        let key = |id| {
            tags.key(&mod_api::Identifier::parse(id).unwrap())
                .unwrap()
                .typed()
        };
        let (heat, absent) = (key("dmg:heat"), key("dmg:absent"));
        self.0.lock().unwrap().push([
            tags.damage_type_in_tag("dmg:hot", heat),
            tags.damage_type_in_tag("dmg:cold", heat),
            tags.damage_type_in_tag("dmg:hot", absent),
            tags.damage_type_in_tag("dmg:unknown", heat),
        ]);
        Ok(())
    }
}

#[test]
#[ignore = "daily"]
#[cfg(any())]
fn a_loot_function_reads_the_bound_damage_type_tags() {
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::loot::{LootData, PackOverlay};
    if run_fresh_cases(
        "game::api_values::loot_values::tests::a_loot_function_reads_the_bound_damage_type_tags",
        "SUNRISE_TEST_LOOT_DAMAGE_TYPE_TAGS",
        &["loot"],
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let damage_type = |message: &str| {
        serde_json::json!({"message_id": message, "scaling": "never", "exhaustion": 0.1})
            .to_string()
            .into_bytes()
    };
    let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}],
        "modifier":{"type":"damage_tag_reader:probe"}});
    let files = vec![
        ("damage_type/dmg:hot".to_owned(), vec![damage_type("hot")]),
        ("damage_type/dmg:cold".to_owned(), vec![damage_type("cold")]),
        (
            "tags/dmg:damage_type/heat".to_owned(),
            vec![br#"{"values":["dmg:hot"]}"#.to_vec()],
        ),
        (
            "loot_table/dmg:probe".to_owned(),
            vec![serde_json::to_vec(&table).unwrap()],
        ),
    ];
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "damage_tag_reader",
            Box::new(DamageTagReader(seen.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let loot = LootData::load_with_types(
                root.path(),
                &PackOverlay::new(&files),
                &defaults,
                crate::game::loot_types::capture(),
            )
            .unwrap();
            let drops = loot.roll("dmg:probe", &mut LegacyRandom::new(0), 0.0, None, None);
            assert_eq!(drops.len(), 1, "the function ran and kept the stick");
        },
    );
    assert_eq!(
        *seen.lock().unwrap(),
        [[Some(true), Some(false), Some(false), None]]
    );
}

/// A loot function listing item tags through its context: the item tags an item is in and
/// every bound item tag's name, as the loot evaluator's own tag source binds them.
#[cfg(any())]
struct ItemTagLister(std::sync::Arc<std::sync::Mutex<Vec<ItemTagLists>>>);
#[cfg(any())]
type ItemTagLists = (
    Option<Vec<mod_api::Identifier>>,
    Option<Vec<mod_api::Identifier>>,
);
#[cfg(any())]
#[mod_api::__subscriptions]
impl Hooks for ItemTagLister {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        r.register_loot_function_type(
            &mod_api::Identifier::parse("item_tag_lister:probe").unwrap(),
            ItemTagLister(self.0.clone()),
        );
    }
}

#[cfg(any())]
impl mod_api::ModLootFunctionCodec for ItemTagLister {
    fn decode(&self, _: &serde_json::Value) -> Result<std::sync::Arc<dyn ModLootFunction>, String> {
        Ok(std::sync::Arc::new(ItemTagLister(self.0.clone())))
    }
}
#[cfg(any())]
impl ModLootFunction for ItemTagLister {
    fn apply(
        &self,
        _: &mut mod_api::LootStackEditor<'_>,
        context: &mut dyn LootTypeContext,
    ) -> Result<(), String> {
        let tags = context.tags();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let sorted = |list: Option<Vec<mod_api::Identifier>>| {
            list.map(|mut list| {
                list.sort_by_key(ToString::to_string);
                list
            })
        };
        self.0
            .lock()
            .unwrap()
            .push((sorted(tags.item_tags(stick)), sorted(tags.item_tag_names())));
        Ok(())
    }
}

/// `item_tags` and `item_tag_names` inside a loot callback read the `LootData` tags the pack
/// loader bound: the stick's tag and the empty one, and no tag whose file name is not an
/// identifier, which the loader skips as a pack never lists it.
#[test]
#[ignore = "daily"]
#[cfg(any())]
fn a_loot_function_lists_the_loot_datas_item_tags() {
    use crate::game::native_entity_refs::tests::run_fresh_cases;
    use sunrise_core::random::LegacyRandom;
    use sunrise_world::loot::{LootData, PackOverlay};
    if run_fresh_cases(
        "game::api_values::loot_values::tests::a_loot_function_lists_the_loot_datas_item_tags",
        "SUNRISE_TEST_LOOT_ITEM_TAG_LISTS",
        &["loot"],
    ) {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("items.json"), r#"{"minecraft:stick":{}}"#).unwrap();
    let defaults = sunrise_world::item_defaults::read(root.path()).unwrap();
    let table = serde_json::json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:stick"}]}],
        "modifier":{"type":"item_tag_lister:probe"}});
    let files = vec![
        (
            "tags/lists:item/sticks".to_owned(),
            vec![br#"{"values":["minecraft:stick"]}"#.to_vec()],
        ),
        (
            "tags/lists:item/empty".to_owned(),
            vec![br#"{"values":[]}"#.to_vec()],
        ),
        (
            "tags/lists:item/Bad".to_owned(),
            vec![br#"{"values":["minecraft:stick"]}"#.to_vec()],
        ),
        (
            "loot_table/lists:probe".to_owned(),
            vec![serde_json::to_vec(&table).unwrap()],
        ),
    ];
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    with_mods(
        vec![ModEntry::new(
            "item_tag_lister",
            Box::new(ItemTagLister(seen.clone())),
        )]
        .into(),
        || {
            mod_platform::__private::part_tests::install_parts_fixture(mod_api::__private::mods())
                .unwrap();
            mod_api::__private::resolve_items(["minecraft:stick"]).unwrap();
            let loot = LootData::load_with_types(
                root.path(),
                &PackOverlay::new(&files),
                &defaults,
                crate::game::loot_types::capture(),
            )
            .unwrap();
            let drops = loot.roll("lists:probe", &mut LegacyRandom::new(0), 0.0, None, None);
            assert_eq!(drops.len(), 1, "the function ran and kept the stick");
        },
    );
    let id = |text: &str| mod_api::Identifier::parse(text).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [(
            Some(vec![id("lists:sticks")]),
            Some(vec![id("lists:empty"), id("lists:sticks")])
        )]
    );
}
