use super::*;

/// The block states mods add to vanilla POI types, once the block registry is resolved.
pub fn native_extend_poi_types(
    context: &mod_api::RegistrationContext,
    extensions: &mut mod_api::PoiTypeExtensionRegistry<'_>,
) {
    crate::game::native_hooks::fire::fire_extend_poi_types(context, |listen| {
        extensions.register_listener(listen)
    });
}

#[cfg(test)]
mod tests {
    use mod_api::{
        ExtendPoiTypesEvent, Hooks, Identifier, ModEntry, RegistrationContext, StateConstraint,
        StatePredicate,
    };
    use mod_platform::testing::with_mods;
    use sunrise_world::{chunk::BlockState, poi::PoiExtension, registry::BlockRegistry};

    struct TestMod {
        fails: bool,
    }
    #[mod_api::__subscriptions]
    impl Hooks for TestMod {
        fn extend_poi_types(&self, _: &RegistrationContext, event: &mut ExtendPoiTypesEvent<'_>) {
            let key = |id: &str| mod_api::__private::block_key(id).unwrap();
            if self.fails {
                let states = StatePredicate::block(key("minecraft:stone"), &[]).unwrap();
                event
                    .extensions
                    .extend(Identifier::parse("minecraft:home").unwrap(), states);
                panic!("extension failed after adding a valid predicate");
            }
            let additions = [
                (
                    "minecraft:home",
                    StatePredicate::block(key("minecraft:hay_block"), &[]).unwrap(),
                ),
                (
                    "minecraft:home",
                    StatePredicate::block(key("minecraft:bell"), &[]).unwrap(),
                ),
                (
                    "minecraft:meeting",
                    StatePredicate::block(
                        key("minecraft:red_bed"),
                        &[StateConstraint::new("part", "foot")],
                    )
                    .unwrap(),
                ),
                (
                    "minecraft:nowhere",
                    StatePredicate::block(key("minecraft:stone"), &[]).unwrap(),
                ),
                (
                    "minecraft:armorer",
                    // POI registration refuses the tag selector itself, without looking up a name.
                    StatePredicate::tag(
                        mod_api::__private::parts_installation::invalid_tag_key(),
                        &[],
                    )
                    .unwrap(),
                ),
            ];
            for (poi, states) in additions {
                event
                    .extensions
                    .extend(Identifier::parse(poi).unwrap(), states);
            }
        }
    }

    fn state(name: &str, properties: &[(&str, &str)]) -> BlockState {
        BlockState {
            name: name.into(),
            properties: properties
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn only_completed_poi_extensions_expand_into_their_full_states() {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stone":1,"minecraft:hay_block":3,"minecraft:bell":5,"minecraft:red_bed":6},"states":[
            ["a",true,false,"","minecraft:air"],
            ["s",false,false,"","minecraft:stone"],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"x"}}],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"y"}}],
            ["h",false,false,"",{"id":"minecraft:hay_block","properties":{"axis":"z"}}],
            ["b",false,false,"",{"id":"minecraft:bell","properties":{"attachment":"floor"}}],
            ["r",false,false,"",{"id":"minecraft:red_bed","properties":{"part":"head"}}],
            ["r",false,false,"",{"id":"minecraft:red_bed","properties":{"part":"foot"}}]
        ]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        crate::game::api_values::resolve_block_definitions(&registry).unwrap();
        with_mods(
            vec![
                ModEntry::new("failed_poi", Box::new(TestMod { fails: true })),
                ModEntry::new("poi_test", Box::new(TestMod { fails: false })),
            ]
            .into(),
            || {
                let (additions, refusals) = crate::game::mods::collect_poi_extensions(&registry);
                let addition = |name: &str, properties: &[(&str, &str)], poi: &str| PoiExtension {
                    state: state(name, properties),
                    poi: poi.into(),
                };
                assert_eq!(
                    additions,
                    [
                        addition("minecraft:hay_block", &[("axis", "x")], "minecraft:home"),
                        addition("minecraft:hay_block", &[("axis", "y")], "minecraft:home"),
                        addition("minecraft:hay_block", &[("axis", "z")], "minecraft:home"),
                        addition(
                            "minecraft:bell",
                            &[("attachment", "floor")],
                            "minecraft:home"
                        ),
                        addition(
                            "minecraft:red_bed",
                            &[("part", "foot")],
                            "minecraft:meeting"
                        ),
                        addition("minecraft:stone", &[], "minecraft:nowhere"),
                    ]
                );
                assert_eq!(
                    mod_api::__private::take_hook_panics(),
                    [mod_api::__private::HookPanicReport {
                        mod_id: "failed_poi",
                        hook: "extend_poi_types",
                    }]
                );
                // The tag predicate is the one refusal the engine makes before the table does.
                assert_eq!(refusals.len(), 1, "{refusals:?}");
                assert!(
                    refusals[0].contains("minecraft:armorer") && refusals[0].contains("tag"),
                    "{}",
                    refusals[0]
                );
            },
        );
    }
}
