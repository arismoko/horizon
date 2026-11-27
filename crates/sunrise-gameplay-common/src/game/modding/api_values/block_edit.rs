//! Cold, all-or-nothing preparation of typed bulk edit values.
pub use super::{TryIntoApi, TryIntoEngine, block_metadata_table};
use mod_api::{BlockEditError, BlockMetadata, BlockStateError, StateConstraint, StatePredicate};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};
pub use sunrise_world::registry::{BlockRegistry, BlockStateId};

pub struct PreparedBlockEdit<'a> {
    pub palette: Vec<u32>,
    pub filters: Vec<CompiledBlockFilter<'a>>,
}

/// Resolve every palette state and compile every filter before a world mutation is possible.
pub fn prepare(edit: &mod_api::BlockEdit) -> Result<PreparedBlockEdit<'_>, BlockEditError> {
    let palette = edit
        .palette
        .iter()
        .map(|state| {
            let state: BlockStateId = state.try_into_engine(&())?;
            Ok(state.0)
        })
        .collect::<Result<Vec<_>, BlockEditError>>()?;
    let filters = edit
        .filters
        .iter()
        .map(compile_filter)
        .collect::<Result<Vec<_>, BlockEditError>>()?;
    Ok(PreparedBlockEdit { palette, filters })
}

/// A resolved name and the predicate's original, already validated constraints.
pub struct CompiledBlockFilter<'a> {
    pub id: Cow<'static, str>,
    pub tag: bool,
    pub constraints: &'a [StateConstraint],
}

impl CompiledBlockFilter<'_> {
    pub fn matches(
        &self,
        registry: &BlockRegistry,
        tags: [&HashMap<String, HashSet<String>>; 2],
        state: BlockStateId,
    ) -> bool {
        block_predicate_matches_state(
            registry,
            tags,
            &self.id,
            self.tag,
            self.constraints
                .iter()
                .map(|constraint| (constraint.name.as_str(), constraint.value.as_str())),
            state,
        )
    }
}

/// Match a state against a block name or tag and its required properties.
pub fn block_predicate_matches_state<'p>(
    registry: &BlockRegistry,
    tags: [&HashMap<String, HashSet<String>>; 2],
    id: &str,
    tag: bool,
    mut properties: impl Iterator<Item = (&'p str, &'p str)>,
    state: BlockStateId,
) -> bool {
    let Some(s) = registry.state(state) else {
        return false;
    };
    let name_ok = if tag {
        let path = id.trim_start_matches("minecraft:");
        tags[0]
            .get(path)
            .or_else(|| tags[1].get(id))
            .is_some_and(|set| {
                set.contains(&s.name) || set.contains(s.name.trim_start_matches("minecraft:"))
            })
    } else {
        s.name == id
    };
    name_ok && properties.all(|(k, v)| s.properties.get(k).map(String::as_str) == Some(v))
}

pub fn compile_filter(
    predicate: &StatePredicate,
) -> Result<CompiledBlockFilter<'_>, BlockEditError> {
    let (id, tag) = if let Some(key) = predicate.block_key() {
        let name = block_metadata_table()?
            .name(key)
            .map_err(|error| match error {
                BlockStateError::Unavailable => BlockEditError::Unavailable,
                _ => BlockEditError::InvalidPredicate,
            })?;
        (Cow::Borrowed(name), false)
    } else {
        // Only the tag's name is retained here; membership is read when applying the edit.
        let name = predicate
            .tag_key()
            .and_then(|key| mod_api::__private::with_tag_name(key, |name| name.map(str::to_owned)))
            .ok_or(BlockEditError::InvalidPredicate)?;
        (Cow::Owned(name), true)
    };
    Ok(CompiledBlockFilter {
        id,
        tag,
        constraints: predicate.constraints(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::TryIntoApi;

    fn registry() -> BlockRegistry {
        mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
        let facts = r#"{"defaults":{"minecraft:air":0,"minecraft:stairs":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"false","right":"false"}}],["s",false,false,"",{"id":"minecraft:stairs","properties":{"left":"true","right":"true"}}]]}"#;
        let registry = BlockRegistry::from_facts_json(facts.as_bytes()).unwrap();
        super::super::resolve_block_definitions(&registry).unwrap();
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
