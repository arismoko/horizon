//! Direct, owner-scoped adapters for the two loot map-codec registries.
use super::api_values::loot_values::{loot_params, loot_stack_editor};
#[cfg(test)]
#[path = "loot_types/tests.rs"]
mod tests;
use mod_api::{ModLootCondition, ModLootConditionCodec, ModLootFunction, ModLootFunctionCodec};
use std::{collections::HashMap, sync::Arc};
use sunrise_world::loot::{
    self,
    types::{
        BoundCondition, BoundFunction, DecodedCondition, DecodedFunction, Registry, RollRandom,
        Types,
    },
};

pub fn capture() -> Types {
    let (functions, conditions) = mod_api::__private::loot_type_declarations();
    Types::new(Arc::new(Codecs {
        functions: functions
            .into_iter()
            .filter(|(id, _)| !sunrise_world::loot::types::is_builtin(id, true))
            .collect(),
        conditions: conditions
            .into_iter()
            .filter(|(id, _)| !sunrise_world::loot::types::is_builtin(id, false))
            .collect(),
    }))
}
struct Codecs {
    functions: HashMap<String, Arc<dyn ModLootFunctionCodec>>,
    conditions: HashMap<String, Arc<dyn ModLootConditionCodec>>,
}
impl Registry for Codecs {
    fn contains(&self, kind: &str, function: bool) -> bool {
        if function {
            self.functions.contains_key(kind)
        } else {
            self.conditions.contains_key(kind)
        }
    }
    fn conditional(&self, kind: &str) -> bool {
        self.functions
            .get(kind)
            .is_none_or(|codec| codec.conditional())
    }
    fn decode_function(
        &self,
        kind: &str,
        data: &serde_json::Value,
    ) -> Result<DecodedFunction, String> {
        let codec = self
            .functions
            .get(kind)
            .ok_or_else(|| format!("unknown loot function type {kind}"))?;
        let rule = codec.decode(data)?;
        let referenced_params = rule
            .referenced_context_params()
            .iter()
            .map(ToString::to_string)
            .collect();
        Ok(DecodedFunction {
            rule: Arc::new(BoundFunctionAdapter(rule)),
            referenced_params,
        })
    }
    fn decode_condition(
        &self,
        kind: &str,
        data: &serde_json::Value,
    ) -> Result<DecodedCondition, String> {
        let codec = self
            .conditions
            .get(kind)
            .ok_or_else(|| format!("unknown loot condition type {kind}"))?;
        let rule = codec.decode(data)?;
        let referenced_params = rule
            .referenced_context_params()
            .iter()
            .map(ToString::to_string)
            .collect();
        Ok(DecodedCondition {
            rule: Arc::new(BoundConditionAdapter(rule)),
            referenced_params,
        })
    }
}

struct BoundFunctionAdapter(Arc<dyn ModLootFunction>);
impl BoundFunction for BoundFunctionAdapter {
    fn apply(
        &self,
        ctx: &loot::LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn loot::LootTags,
        stack: &mut dyn loot::LootStackEditAccess,
    ) -> Result<(), String> {
        let mut editor = loot_stack_editor(stack);
        let mut context = Context {
            facts: ctx,
            random,
            tags: crate::game::loot_tags::LootTags(tags),
        };
        self.0.apply(&mut editor, &mut context)
    }
}

struct BoundConditionAdapter(Arc<dyn ModLootCondition>);
impl BoundCondition for BoundConditionAdapter {
    fn test(
        &self,
        ctx: &loot::LootContext<'_>,
        random: &mut dyn RollRandom,
        tags: &dyn loot::LootTags,
    ) -> Result<bool, String> {
        let mut context = Context {
            facts: ctx,
            random,
            tags: crate::game::loot_tags::LootTags(tags),
        };
        self.0.test(&mut context)
    }
}

struct Context<'a> {
    facts: &'a loot::LootContext<'a>,
    random: &'a mut dyn RollRandom,
    tags: crate::game::loot_tags::LootTags<'a>,
}
impl mod_api::LootTypeContext for Context<'_> {
    fn params(&self) -> mod_api::LootParamsView<'_> {
        loot_params(*self.facts)
    }
    fn tags(&self) -> &dyn mod_api::Tags {
        &self.tags
    }
    fn random(&mut self) -> &mut dyn mod_api::RandomSource {
        self
    }
}
impl mod_api::RandomSource for Context<'_> {
    fn next_int(&mut self) -> i32 {
        self.random.next_int()
    }
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.random.next_int_bound(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.random.next_long()
    }
    fn next_float(&mut self) -> f32 {
        self.random.next_float()
    }
    fn next_double(&mut self) -> f64 {
        self.random.next_double()
    }
    fn next_boolean(&mut self) -> bool {
        self.random.next_boolean()
    }
    fn next_gaussian(&mut self) -> f64 {
        self.random.next_gaussian()
    }
}
