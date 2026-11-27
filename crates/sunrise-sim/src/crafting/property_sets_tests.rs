use super::*;
use std::sync::Arc;
use sunrise_protocol::{
    buf::{Codec, Reader},
    packets::recipes::UpdateRecipes,
    registry::Ids,
};
use sunrise_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs};

struct Sets;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Sets {
    fn recipe_property_sets(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::RecipePropertySetsEvent<'_>,
    ) {
        for (key, items) in [
            (
                "pot:input",
                vec!["minecraft:emerald", "minecraft:air", "missing:item"],
            ),
            ("foreign:input", vec!["minecraft:diamond"]),
            ("minecraft:custom_input", vec!["minecraft:emerald"]),
            ("BAD:input", vec!["minecraft:diamond"]),
            ("minecraft:furnace_input", vec!["minecraft:diamond"]),
        ] {
            event.register(mod_api::RecipePropertyAddition::new(
                key,
                items.into_iter().map(str::to_owned).collect(),
            ));
        }
    }
}
struct Extend;
#[mod_api::__subscriptions]
impl mod_api::Hooks for Extend {
    fn recipe_property_sets(
        &self,
        _: &dyn mod_api::DataLoadContext,
        event: &mut mod_api::RecipePropertySetsEvent<'_>,
    ) {
        event.register(mod_api::RecipePropertyAddition::new(
            "pot:input",
            vec!["minecraft:diamond".into(), "minecraft:emerald".into()],
        ));
        event.register(mod_api::RecipePropertyAddition::new(
            "pot:foreign_new",
            vec!["minecraft:diamond".into()],
        ));
        event.register(mod_api::RecipePropertyAddition::new(
            "unknown_property_test",
            vec!["minecraft:diamond".into()],
        ));
    }
}

#[test]
fn valid_namespaced_custom_sets_survive_dispatch_and_sync_while_invalid_keys_do_not() {
    let context = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(
            Default::default(),
            "nonexistent-property-test-settings".into(),
        )),
        ["pot", "extension"],
    );
    mod_platform::testing::with_mods(
        vec![
            mod_api::ModEntry::new("pot", Box::new(Sets)),
            mod_api::ModEntry::new("extension", Box::new(Extend)),
        ]
        .into(),
        || {
            crate::test_log::install();
            crate::test_log::clear();
            let resolved = ResolvedSources::resolve(vec![], &Items::empty(), &context).unwrap();
            assert_eq!(resolved.property_additions.0["pot:input"].first_mod, "pot");
            assert!(crate::test_log::records().iter().any(|(level, message)| {
                *level == log::Level::Warn
                    && message.contains("extension: unknown set unknown_property_test")
            }));
            assert!(resolved.property_additions.get("custom_input").is_none());
            assert!(resolved.property_additions.get("BAD:input").is_none());
            assert!(
                !resolved
                    .property_additions
                    .adds("pot:input", "minecraft:air")
            );
            let mut sync = crate::lan::data::SyncData::empty();
            sync.builtin.registries.insert(
                "minecraft:item".into(),
                Ids::from_names(vec![
                    "minecraft:air".into(),
                    "minecraft:emerald".into(),
                    "minecraft:diamond".into(),
                ]),
            );
            let (bytes, _) = crate::lan::recipes::update_recipes(&sync, &resolved);
            let packet = UpdateRecipes::read(&mut Reader::new(&bytes)).unwrap();
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "pot:input")
                    .unwrap()
                    .1,
                vec![1, 2]
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "foreign:input")
                    .unwrap()
                    .1,
                vec![2]
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "pot:foreign_new")
                    .unwrap()
                    .1,
                vec![2]
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "minecraft:custom_input")
                    .unwrap()
                    .1,
                vec![1]
            );
            assert_eq!(
                packet
                    .properties
                    .iter()
                    .find(|(key, _)| key == "minecraft:furnace_input")
                    .unwrap()
                    .1,
                vec![2]
            );
            assert_eq!(packet.properties.len(), 13);
        },
    );
}
