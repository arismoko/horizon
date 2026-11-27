//! Native worker adapters for API-owned rule test (`BuiltInRegistries.RULE_TEST`)
//! and position rule test (`BuiltInRegistries.POS_RULE_TEST`) types.
//!
//! Block, tag, fluid and property tables are captured once when a processor list
//! or feature decodes, so per-block evaluation does no registry lookup or
//! allocation. Panics inside mod decoders or evaluators are caught and returned
//! as errors naming the mod and type; nothing is substituted.
use super::native_worldgen::WorkerResources;
use mod_api::{
    BlockKey, BlockMetadata, BlockState, PosRuleTest, PosRuleTestContext, PosRuleTestDecodeContext,
    PosRuleTestRef, PosRuleTestType, RandomSource, RuleTest, RuleTestContext,
    RuleTestDecodeContext, RuleTestRef, RuleTestType, StructureError,
};
use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use sunrise_core::nbt::Tag;
use sunrise_world::block_properties::BoundBlockProperty;
use sunrise_world::fluid::{FluidKind, FluidStateTable};
use sunrise_world::registry::BlockStateId;
use sunrise_world::worldgen::mod_rule_tests::{
    ModPosRuleTest, ModPosRuleTestContext, ModPosRuleTestType, ModPosRuleTestTypes, ModRuleTest,
    ModRuleTestContext, ModRuleTestType, ModRuleTestTypes, PosRuleTestDecodeScope,
    RuleTestDecodeScope,
};
use sunrise_world::worldgen::rule_test::RuleTest as EngineRuleTest;
use sunrise_world::worldgen::structure::processor::PosRule as EnginePosRule;

fn panic_detail(payload: Box<dyn Any + Send>) -> Option<String> {
    if let Some(s) = payload.downcast_ref::<&str>() {
        Some((*s).to_owned())
    } else {
        payload.downcast::<String>().ok().map(|s| *s)
    }
}

fn format_panic(
    kind: &str,
    mod_id: &str,
    type_id: &str,
    phase: &str,
    payload: Box<dyn Any + Send>,
) -> String {
    match panic_detail(payload) {
        Some(detail) if !detail.is_empty() => {
            format!("mod {mod_id}: {kind} {type_id} panicked during {phase}: {detail}")
        }
        _ => format!("mod {mod_id}: {kind} {type_id} panicked during {phase}"),
    }
}

fn config_to_tag(config: &serde_json::Value) -> Result<Tag, String> {
    match sunrise_core::json_nbt::json_to_tag(config) {
        Ok(tag) => Ok(tag),
        Err(err) => {
            if let Some(map) = config.as_object() {
                for (key, value) in map {
                    if let Err(field_err) = sunrise_core::json_nbt::json_to_tag(value) {
                        return Err(format!("field {key}: {field_err}"));
                    }
                }
            }
            Err(err.to_string())
        }
    }
}

fn engine_rule_test(handle: &RuleTestRef) -> Option<Arc<EngineRuleTest>> {
    mod_api::__private::rule_test_payload(handle)
        .clone()
        .downcast::<EngineRuleTest>()
        .ok()
}

fn engine_pos_rule_test(handle: &PosRuleTestRef) -> Option<Arc<EnginePosRule>> {
    mod_api::__private::pos_rule_test_payload(handle)
        .clone()
        .downcast::<EnginePosRule>()
        .ok()
}

struct RuleDecoding<'a> {
    scope: &'a RuleTestDecodeScope<'a>,
    worker: &'a WorkerResources,
}
impl RuleTestDecodeContext for RuleDecoding<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.scope.bindings().registry(), block, properties)
    }
    fn block_states(&self, block: &str) -> Result<Vec<BlockState>, StructureError> {
        let id = sunrise_world::worldgen::tags::identifier(block).map_err(|e| {
            StructureError::new(std::io::Error::other(format!(
                "invalid block id {block}: {e}"
            )))
        })?;
        let states = self.scope.bindings().block_states(&id).ok_or_else(|| {
            StructureError::new(std::io::Error::other(format!("unknown block {id}")))
        })?;
        states
            .iter()
            .map(|&s| {
                self.worker.index.to_api(s).map_err(|e| {
                    StructureError::new(std::io::Error::other(format!("block {id}: {e:?}")))
                })
            })
            .collect()
    }
    fn decode_block_state(&self, config: &Tag) -> Result<BlockState, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let state = self
            .scope
            .bindings()
            .decode_block_state(&value.to_string())
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        self.worker.index.to_api(state).map_err(|e| {
            StructureError::new(std::io::Error::other(format!("invalid block state: {e:?}")))
        })
    }
    fn block_tag(&self, tag: &str) -> Result<Vec<BlockState>, StructureError> {
        let raw = tag.strip_prefix('#').unwrap_or(tag);
        let id = sunrise_world::worldgen::tags::identifier(raw).map_err(|e| {
            StructureError::new(std::io::Error::other(format!(
                "invalid block tag {tag}: {e}"
            )))
        })?;
        let states = self.scope.bindings().tag_states(&id).ok_or_else(|| {
            StructureError::new(std::io::Error::other(format!("unknown block tag {id}")))
        })?;
        states
            .iter()
            .map(|&s| {
                self.worker.index.to_api(s).map_err(|e| {
                    StructureError::new(std::io::Error::other(format!("block tag {id}: {e:?}")))
                })
            })
            .collect()
    }
    fn rule_test(&self, config: &Tag) -> Result<RuleTestRef, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let rule = self
            .scope
            .decode_rule_test(&value.to_string())
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let mod_rule = rule.as_mod().and_then(|inner| {
            let inner: &dyn Any = &**inner;
            inner.downcast_ref::<ModRule>().map(|r| r.inner.clone())
        });
        Ok(mod_api::__private::rule_test_ref(Arc::new(rule), mod_rule))
    }
}

struct RuleTypeAdapter {
    mod_id: &'static str,
    type_id: String,
    inner: Arc<dyn RuleTestType>,
}
impl ModRuleTestType for RuleTypeAdapter {
    fn decode(
        &self,
        config: &serde_json::Value,
        scope: &RuleTestDecodeScope<'_>,
    ) -> Result<Arc<dyn ModRuleTest>, String> {
        let worker =
            WorkerResources::get_arc(scope.bindings().registry()).map_err(|e| e.to_string())?;
        let tag = config_to_tag(config)?;
        let context = RuleDecoding {
            scope,
            worker: &worker,
        };
        let decoded = catch_unwind(AssertUnwindSafe(|| self.inner.decode(&context, &tag)))
            .map_err(|payload| {
                format_panic("rule test", self.mod_id, &self.type_id, "decode", payload)
            })?
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(ModRule {
            mod_id: self.mod_id,
            type_id: self.type_id.clone(),
            inner: decoded,
            worker,
            air: scope.bindings().air_states(),
            fluids: scope.bindings().fluid_table().clone(),
            can_occlude: scope.bindings().can_occlude().cloned(),
        }))
    }
}

struct ModRule {
    mod_id: &'static str,
    type_id: String,
    inner: Arc<dyn RuleTest>,
    worker: Arc<WorkerResources>,
    air: [BlockStateId; 3],
    fluids: Arc<FluidStateTable>,
    can_occlude: Option<BoundBlockProperty>,
}
impl fmt::Debug for ModRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModRule")
            .field("mod_id", &self.mod_id)
            .field("type_id", &self.type_id)
            .field("inner", &self.inner)
            .finish()
    }
}

struct RuleView<'a> {
    rule: &'a ModRule,
    inner: &'a dyn ModRuleTestContext,
    nested_error: RefCell<Option<String>>,
}
impl RuleTestContext for RuleView<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.rule.worker.metadata
    }
    fn is_air(&self, state: BlockState) -> bool {
        self.rule
            .worker
            .index
            .to_engine(state)
            .is_ok_and(|id| self.rule.air.contains(&id))
    }
    fn is_liquid(&self, state: BlockState) -> bool {
        self.rule
            .worker
            .index
            .to_engine(state)
            .ok()
            .and_then(|id| self.rule.fluids.get(id))
            .is_some_and(|fluid| fluid.kind() != FluidKind::Empty)
    }
    fn can_occlude(&self, state: BlockState) -> Option<bool> {
        let prop = self.rule.can_occlude.as_ref()?;
        let id = self.rule.worker.index.to_engine(state).ok()?;
        Some(prop.contains(id))
    }
    fn test_rule(
        &self,
        rule: &RuleTestRef,
        state: BlockState,
        pos: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, StructureError> {
        let engine_rule = engine_rule_test(rule).ok_or_else(|| {
            StructureError::new(std::io::Error::other("invalid rule test handle"))
        })?;
        let engine_state = self.rule.worker.index.to_engine(state).map_err(|e| {
            StructureError::new(std::io::Error::other(format!("invalid block state: {e:?}")))
        })?;
        match self
            .inner
            .test_rule(&engine_rule, engine_state, pos, random)
        {
            Ok(matched) => Ok(matched),
            Err(err) => {
                let mut slot = self.nested_error.borrow_mut();
                if slot.is_none() {
                    *slot = Some(err.clone());
                }
                Err(StructureError::new(std::io::Error::other(err)))
            }
        }
    }
}

impl ModRuleTest for ModRule {
    fn test(
        &self,
        context: &dyn ModRuleTestContext,
        state: BlockStateId,
        position: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, String> {
        let api_state = self.worker.index.to_api(state).unwrap_or(self.worker.air);
        let view = RuleView {
            rule: self,
            inner: context,
            nested_error: RefCell::new(None),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.inner.test(&view, api_state, position, random)
        }))
        .map_err(|payload| {
            format_panic(
                "rule test",
                self.mod_id,
                &self.type_id,
                "evaluation",
                payload,
            )
        })?;
        if let Some(err) = view.nested_error.into_inner() {
            return Err(err);
        }
        Ok(result)
    }
}

struct PosRuleDecoding<'a> {
    scope: &'a PosRuleTestDecodeScope,
}
impl PosRuleTestDecodeContext for PosRuleDecoding<'_> {
    fn pos_rule_test(&self, config: &Tag) -> Result<PosRuleTestRef, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let rule = self
            .scope
            .decode_pos_rule_test(&value)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let mod_rule = match &rule {
            EnginePosRule::Mod(inner) => {
                let inner: &dyn Any = &**inner;
                inner.downcast_ref::<ModPosRule>().map(|r| r.inner.clone())
            }
            _ => None,
        };
        Ok(mod_api::__private::pos_rule_test_ref(
            Arc::new(rule),
            mod_rule,
        ))
    }
}

struct PosRuleTypeAdapter {
    mod_id: &'static str,
    type_id: String,
    inner: Arc<dyn PosRuleTestType>,
}
impl ModPosRuleTestType for PosRuleTypeAdapter {
    fn decode(
        &self,
        config: &serde_json::Value,
        scope: &PosRuleTestDecodeScope,
    ) -> Result<Arc<dyn ModPosRuleTest>, String> {
        let tag = config_to_tag(config)?;
        let context = PosRuleDecoding { scope };
        let decoded = catch_unwind(AssertUnwindSafe(|| self.inner.decode(&context, &tag)))
            .map_err(|payload| {
                format_panic(
                    "position rule test",
                    self.mod_id,
                    &self.type_id,
                    "decode",
                    payload,
                )
            })?
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(ModPosRule {
            mod_id: self.mod_id,
            type_id: self.type_id.clone(),
            inner: decoded,
        }))
    }
}

#[derive(Debug)]
struct ModPosRule {
    mod_id: &'static str,
    type_id: String,
    inner: Arc<dyn PosRuleTest>,
}

struct PosRuleView<'a> {
    inner: &'a dyn ModPosRuleTestContext,
    nested_error: RefCell<Option<String>>,
}
impl PosRuleTestContext for PosRuleView<'_> {
    fn test_pos_rule(
        &self,
        rule: &PosRuleTestRef,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, StructureError> {
        let engine_rule = engine_pos_rule_test(rule).ok_or_else(|| {
            StructureError::new(std::io::Error::other("invalid position rule test handle"))
        })?;
        match self.inner.test_pos_rule(
            &engine_rule,
            in_template_pos,
            world_pos,
            world_reference,
            random,
        ) {
            Ok(matched) => Ok(matched),
            Err(err) => {
                let mut slot = self.nested_error.borrow_mut();
                if slot.is_none() {
                    *slot = Some(err.clone());
                }
                Err(StructureError::new(std::io::Error::other(err)))
            }
        }
    }
}

impl ModPosRuleTest for ModPosRule {
    fn test(
        &self,
        context: &dyn ModPosRuleTestContext,
        in_template_pos: [i32; 3],
        world_pos: [i32; 3],
        world_reference: [i32; 3],
        random: &mut dyn RandomSource,
    ) -> Result<bool, String> {
        let view = PosRuleView {
            inner: context,
            nested_error: RefCell::new(None),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.inner
                .test(&view, in_template_pos, world_pos, world_reference, random)
        }))
        .map_err(|payload| {
            format_panic(
                "position rule test",
                self.mod_id,
                &self.type_id,
                "evaluation",
                payload,
            )
        })?;
        if let Some(err) = view.nested_error.into_inner() {
            return Err(err);
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "native_rule_tests/tests.rs"]
mod tests;

/// Registered API rule test declarations become engine adapters once.
pub fn installed_rule_test_types() -> ModRuleTestTypes {
    mod_api::__private::rule_test_type_declarations()
        .into_iter()
        .map(|(id, mod_id, inner)| {
            let ty: Arc<dyn ModRuleTestType> = Arc::new(RuleTypeAdapter {
                mod_id,
                type_id: id.clone(),
                inner,
            });
            (id, ty)
        })
        .collect()
}

/// Registered API position rule test declarations become engine adapters once.
pub fn installed_pos_rule_test_types() -> ModPosRuleTestTypes {
    mod_api::__private::pos_rule_test_type_declarations()
        .into_iter()
        .map(|(id, mod_id, inner)| {
            let ty: Arc<dyn ModPosRuleTestType> = Arc::new(PosRuleTypeAdapter {
                mod_id,
                type_id: id.clone(),
                inner,
            });
            (id, ty)
        })
        .collect()
}
