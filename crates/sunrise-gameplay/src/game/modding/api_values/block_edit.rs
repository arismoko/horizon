#[cfg(test)]
mod tests {
    use mod_api::{BlockEditError, StateConstraint, StatePredicate};
    use std::{
        borrow::Cow,
        collections::{HashMap, HashSet},
    };
    use sunrise_gameplay_common::game::api_values::{
        TryIntoApi,
        block_edit::{BlockStateId, CompiledBlockFilter, compile_filter, prepare},
    };
    use sunrise_world::registry::BlockRegistry;

    fn registry() -> BlockRegistry {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"false","right":"false"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"true","right":"true"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        sunrise_gameplay_common::game::api_values::resolve_block_definitions(&registry).unwrap();
        registry
    }

    #[test]
    fn typed_bulk_sparse_predicates_use_existing_matcher_and_unknown_tag_properties_never_match() {
        let registry = registry();
        let state = BlockStateId(2).try_into_api().unwrap();
        let exact =
            StatePredicate::block(state.block(), &[StateConstraint::new("left", "true")]).unwrap();
        let filter = compile_filter(&exact).unwrap();
        let tags = HashMap::from([(
            "stairs".to_owned(),
            HashSet::from(["minecraft:stairs".to_owned()]),
        )]);
        let tags = [&tags, &tags];
        assert!(filter.matches(&registry, tags, BlockStateId(2)));
        assert!(!filter.matches(&registry, tags, BlockStateId(1)));
        let wildcard = StatePredicate::block(state.block(), &[]).unwrap();
        let wildcard = compile_filter(&wildcard).unwrap();
        assert!(wildcard.matches(&registry, tags, BlockStateId(1)));
        assert!(wildcard.matches(&registry, tags, BlockStateId(2)));

        // Use resolved tag names and local membership, without interning test keys process-wide.
        let tag_matches = |id: &'static str, constraints: &[StateConstraint], state| {
            CompiledBlockFilter {
                id: Cow::Borrowed(id),
                tag: true,
                constraints,
            }
            .matches(&registry, tags, BlockStateId(state))
        };
        assert!(tag_matches("minecraft:stairs", &[], 2));
        assert!(!tag_matches("minecraft:stairs", &[], 0));
        assert!(!tag_matches(
            "minecraft:stairs",
            &[StateConstraint::new("unknown", "anything")],
            2,
        ));
        assert!(!tag_matches(
            "minecraft:stairs",
            &[StateConstraint::new("left", "invalid")],
            2,
        ));
        assert!(!tag_matches("test:empty", &[], 2));
    }

    #[test]
    fn typed_bulk_preparation_rejects_late_bad_values_before_any_write() {
        let _registry = registry();
        let state = BlockStateId(2).try_into_api().unwrap();
        let good = StatePredicate::block(state.block(), &[]).unwrap();
        let mut edit = mod_api::BlockEdit {
            palette: vec![state],
            filters: vec![good],
            runs: vec![],
        };
        let prepared = prepare(&edit).unwrap();
        assert_eq!(prepared.palette, [2]);
        assert_eq!(prepared.filters.len(), 1);
        edit.palette
            .push(mod_api::__private::block_state_from_ordinal(
                state.block(),
                u16::MAX,
            ));
        assert!(
            matches!(prepare(&edit), Err(BlockEditError::InvalidState(_))),
            "late bad palette state must refuse the whole prepared edit"
        );
    }
}
