//! `/compute` (ComputeCommand): context number providers
//! (`context_int_provider`, `context_float_provider`) evaluated in a loot
//! context of the source, optionally with a block or a target entity.
use super::{Arg, Context, R, Target, err};
use serde_json::Value;
use std::borrow::Cow;
use sunrise_command::CommandError;
use sunrise_core::java_number::float_to_string;
use sunrise_game_data::text::Text;
use sunrise_level::providers::{ConditionContext, Evaluation};
use sunrise_sim::advancements::{self as adv, EntityView};

/// The loot context parameters of `LootContextSources`.
#[derive(Clone, Copy, Default)]
pub struct ComputeContext {
    /// `BLOCK_STATE` (`compute block <pos>`).
    pub block: Option<[i32; 3]>,
    /// `TARGET_ENTITY` (`compute entity <target>`).
    pub target: Option<Target>,
    /// `INTERACTING_ENTITY` (the player a composter's `addLayer` names).
    pub interacting: Option<Target>,
    /// A context with no `THIS_ENTITY` (`LootContextParamSets.BLOCK_INTERACT`)
    /// and this `ORIGIN`, instead of the command source's entity and position.
    pub without_this: Option<[f64; 3]>,
}

/// The entities and origin of a compute context's conditions: `this` and the
/// origin come from the command source unless the context has no `this`.
pub(in crate::game) fn context_entities<'a>(
    c: &ComputeContext,
    source_this: Option<&'a EntityView>,
    source_pos: [f64; 3],
    target: Option<&'a EntityView>,
    interacting: Option<&'a EntityView>,
) -> adv::Entities<'a> {
    adv::Entities {
        this: source_this.filter(|_| c.without_this.is_none()),
        origin: c.without_this.unwrap_or(source_pos),
        tool: None,
        event_block: None,
        target,
        interacting,
    }
}

/// An ArithmeticException's message.
pub(in crate::game) type Arith = String;

pub(in crate::game) struct ComputeCondition<'g, 'res> {
    pub(in crate::game) game: &'g mut crate::game::ServerGame<'res>,
    pub(in crate::game) context: &'g ComputeContext,
}

impl<'v> ConditionContext<'v> for ComputeCondition<'_, '_> {
    fn named(&self, id: &str) -> Option<Cow<'v, Value>> {
        self.game
            .provider_file("minecraft:predicate", id)
            .map(Cow::Owned)
    }

    fn kind<'n>(&self, condition: &'n Value) -> Option<&'n str> {
        condition
            .get("type")
            .or_else(|| condition.get("condition"))?
            .as_str()
    }

    fn chance<'n>(&self, condition: &'n Value) -> Option<&'n Value> {
        Some(&condition["chance"])
    }

    fn singleton_terms(&self) -> bool {
        false
    }

    fn leaf(&mut self, condition: &Value) -> bool {
        self.game.compute_leaf(condition, self.context)
    }
}

impl crate::game::ServerGame<'_> {
    /// A provider argument: its registry id (None when inline) and value.
    pub(in crate::game::server_side) fn provider_arg(
        &self,
        ctx: &Context,
        registry: &str,
    ) -> R<(Option<String>, Value)> {
        let raw = match ctx.get("provider") {
            Some(Arg::Resource(s) | Arg::Str(s)) => s.clone(),
            _ => return Err(err("command.failed", &[])),
        };
        let first = raw.trim_start().chars().next().unwrap_or(' ');
        if first == '{' || first == '[' || first == '-' || first.is_ascii_digit() || first == '"' {
            let (tag, _) = sunrise_core::nbt::parse_snbt(&raw)
                .map_err(|e| err("argument.nbt.invalid", &[&e]))?;
            return Ok((None, sunrise_game_data::scoreboard::tag_to_json(&tag)));
        }
        let id = if raw.contains(':') {
            raw
        } else {
            format!("minecraft:{raw}")
        };
        let value = self
            .provider_file(registry, &id)
            .ok_or_else(|| err("argument.resource_or_id.no_such_element", &[&id, &registry]))?;
        Ok((Some(id), value))
    }
    /// The compute context the command's source literal selects.
    pub(in crate::game::server_side) fn compute_context(
        &mut self,
        ctx: &Context,
    ) -> R<ComputeContext> {
        let mut c = ComputeContext::default();
        if ctx.literals.iter().any(|l| l == "block") && ctx.get("computePos").is_some() {
            c.block = Some(self.block_pos_arg(ctx, "computePos")?);
        }
        if ctx.get("computeTarget").is_some() {
            let t = self.entities(ctx, "computeTarget", false)?;
            if t.len() > 1 {
                return Err(err("argument.entity.toomany", &[]));
            }
            c.target = t.first().copied();
        }
        Ok(c)
    }
    /// `ComputeCommand.computeAsInt` / `computeAsFloat`.
    pub(in crate::game::server_side) fn cmd_compute(&mut self, ctx: &Context) -> R<i32> {
        let integer = ctx.literals.iter().any(|l| l == "integer");
        let registry = if integer {
            "minecraft:context_int_provider"
        } else {
            "minecraft:context_float_provider"
        };
        let (id, provider) = self.provider_arg(ctx, registry)?;
        let c = self.compute_context(ctx)?;
        let invalid = |value: String| -> CommandError {
            match &id {
                Some(id) => CommandError::new(Text::translate(
                    "command.compute.result.named.invalid",
                    &[Text::literal(id.clone()), Text::literal(value)],
                )),
                None => CommandError::new(Text::translate(
                    "command.compute.result.unnamed.invalid",
                    &[Text::literal(value)],
                )),
            }
        };
        let exact = |result: i32| match &id {
            Some(id) => Text::translate(
                "command.compute.result.named.exact",
                &[Text::literal(id.clone()), Text::literal(result.to_string())],
            ),
            None => Text::translate(
                "command.compute.result.unnamed.exact",
                &[Text::literal(result.to_string())],
            ),
        };
        if integer {
            let result = self.provider_int(&provider, &c).map_err(invalid)?;
            self.feedback(exact(result));
            return Ok(result);
        }
        let scale = match ctx.get("scale") {
            Some(Arg::Float(s)) => *s,
            _ => 1.0,
        };
        let original = self.provider_float(&provider, &c).map_err(invalid)?;
        if !original.is_finite() {
            return Err(invalid(float_to_string(original)));
        }
        // Mth.floor(original * scale)
        let result = (original * scale).floor() as i32;
        if result as f32 == original {
            self.feedback(exact(result));
        } else {
            let original_text = Text::literal(float_to_string(original));
            let message = match &id {
                Some(id) => Text::translate(
                    "command.compute.result.named.rounded",
                    &[
                        Text::literal(id.clone()),
                        original_text,
                        Text::literal(result.to_string()),
                    ],
                ),
                None => Text::translate(
                    "command.compute.result.unnamed.rounded",
                    &[original_text, Text::literal(result.to_string())],
                ),
            };
            self.feedback(message);
        }
        Ok(result)
    }

    // ---- evaluation ---------------------------------------------------------------

    pub fn provider_int(&mut self, v: &Value, c: &ComputeContext) -> Result<i32, Arith> {
        let providers = self.active_providers().clone();
        let mut random = self.common.world.random.clone();
        let result = Evaluation::new(&providers, &mut random).int(
            v,
            &mut ComputeCondition {
                game: self,
                context: c,
            },
        );
        self.common.world.random = random;
        result
    }

    pub fn provider_float(&mut self, v: &Value, c: &ComputeContext) -> Result<f32, Arith> {
        let providers = self.active_providers().clone();
        let mut random = self.common.world.random.clone();
        let result = Evaluation::new(&providers, &mut random).float(
            v,
            &mut ComputeCondition {
                game: self,
                context: c,
            },
        );
        self.common.world.random = random;
        result
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    /// A world with nothing in it: the entity conditions under test read none of it.
    pub(in crate::game) struct Void;
    impl adv::World for Void {
        fn is_loaded(&self, _: [i32; 3]) -> bool {
            false
        }
        fn dimension(&self) -> &str {
            "minecraft:overworld"
        }
        fn weather(&self) -> (bool, bool) {
            (false, false)
        }
        fn biome(&self, _: [i32; 3]) -> String {
            String::new()
        }
        fn structures(&self, _: [i32; 3]) -> Vec<String> {
            Vec::new()
        }
        fn block(&self, _: [i32; 3]) -> (String, BTreeMap<String, String>) {
            ("minecraft:air".into(), BTreeMap::new())
        }
        fn fluid(&self, _: [i32; 3]) -> Option<String> {
            None
        }
        fn can_see_sky(&self, _: [i32; 3]) -> bool {
            false
        }
        fn light(&self, _: [i32; 3]) -> i32 {
            0
        }
        fn smokey(&self, _: [i32; 3]) -> bool {
            false
        }
        fn in_tag(&self, _: &str, _: &str, _: &str) -> bool {
            false
        }
    }

    pub(in crate::game) fn view(kind: &str, position: [f64; 3]) -> EntityView {
        EntityView {
            kind: kind.into(),
            position,
            ..Default::default()
        }
    }

    pub(in crate::game) fn holds(cond: &Value, entities: &adv::Entities) -> bool {
        adv::conditions(cond, entities, &Void)
    }

    pub(in crate::game) fn of_type(who: &str, kind: &str) -> Value {
        json!({"condition": "minecraft:entity_properties", "entity": who,
            "predicate": {"minecraft:entity_type": kind}})
    }

    /// A command context's `this` and origin are the source's; `target_entity`
    /// and `interacting_entity` are the ones the context carries.
    #[test]
    fn a_compute_context_selects_the_entities_it_carries() {
        let (source, target, actor) = (
            view("minecraft:player", [0.0; 3]),
            view("minecraft:cow", [0.0; 3]),
            view("minecraft:zombie", [0.0; 3]),
        );
        let c = ComputeContext::default();
        let e = context_entities(
            &c,
            Some(&source),
            [1.0, 2.0, 3.0],
            Some(&target),
            Some(&actor),
        );
        assert_eq!(e.origin, [1.0, 2.0, 3.0]);
        assert!(holds(&of_type("this", "minecraft:player"), &e));
        assert!(holds(&of_type("target_entity", "minecraft:cow"), &e));
        assert!(!holds(&of_type("target_entity", "minecraft:zombie"), &e));
        assert!(holds(
            &of_type("interacting_entity", "minecraft:zombie"),
            &e
        ));
        let bare = context_entities(&c, Some(&source), [0.0; 3], None, None);
        assert!(!holds(&of_type("target_entity", "minecraft:cow"), &bare));
        assert!(!holds(
            &of_type("interacting_entity", "minecraft:zombie"),
            &bare
        ));
    }

    /// `BLOCK_INTERACT` (the composter) has no `THIS_ENTITY` and measures from
    /// the block's centre, whoever ran the command or whatever the source is.
    #[test]
    fn a_context_without_this_measures_from_its_own_origin() {
        let source = view("minecraft:player", [0.5, 64.0, 0.5]);
        let actor = view("minecraft:player", [10.5, 64.0, 0.5]);
        let c = ComputeContext {
            without_this: Some([10.5, 64.0, 0.5]),
            ..ComputeContext::default()
        };
        let e = context_entities(&c, Some(&source), source.position, None, Some(&actor));
        assert!(!holds(&of_type("this", "minecraft:player"), &e));
        assert!(holds(
            &of_type("interacting_entity", "minecraft:player"),
            &e
        ));
        // The interacting player stands on the origin, ten blocks from the source.
        let near = json!({"condition": "minecraft:entity_properties", "entity": "interacting_entity",
            "predicate": {"minecraft:distance": {"absolute": {"max": 1.0}}}});
        assert!(holds(&near, &e));
        let from_source = context_entities(
            &ComputeContext::default(),
            Some(&source),
            source.position,
            None,
            Some(&actor),
        );
        assert!(!holds(&near, &from_source));
    }
}
