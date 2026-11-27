//! Native worker adapters for API-owned placement modifier types: the engine's
//! `ModPlacementModifierType` and `ModPlacementModifier` around a mod's
//! `PlacementModifierType` and `PlacementModifier`.
use super::api_values::block_state::Index;
use super::native_structure_types::engine_heightmap;
use super::native_worldgen::WorkerResources;
use mod_api::{
    BiomeKey, BlockKey, BlockMetadata, BlockState, FeatureError, HeightmapType,
    PlacementModifier as ApiPlacementModifier, PlacementModifierContext,
    PlacementModifierDecodeContext, PlacementModifierRef, PlacementModifierType, RandomSource,
};
use std::sync::Arc;
use sunrise_mod_runtime::block_definitions::Blocks;
use sunrise_world::registry::BlockRegistry;
use sunrise_world::worldgen::placement::{
    PlacementModifier, PlacementQueryError,
    mod_placements::{
        ModPlacementContext, ModPlacementModifier, ModPlacementModifierType,
        ModPlacementModifierTypes, PlacementDecodeScope,
    },
};

fn modifier_ref(modifier: Arc<PlacementModifier>) -> PlacementModifierRef {
    let mod_modifier = match &*modifier {
        PlacementModifier::Mod(inner) => {
            let inner: &dyn std::any::Any = &**inner;
            inner.downcast_ref::<ModModifier>().map(|m| m.inner.clone())
        }
        _ => None,
    };
    mod_api::__private::placement_modifier_ref(modifier, mod_modifier)
}

fn engine_modifier(handle: &PlacementModifierRef) -> Option<Arc<PlacementModifier>> {
    mod_api::__private::placement_modifier_payload(handle)
        .clone()
        .downcast::<PlacementModifier>()
        .ok()
}

/// Decode context over the engine's placement modifier codec.
struct Decoding<'a> {
    scope: &'a PlacementDecodeScope<'a>,
    registry: &'a BlockRegistry,
    worker: &'a WorkerResources,
}
impl PlacementModifierDecodeContext for Decoding<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker.state_with(self.registry, block, properties)
    }
    fn modifier(
        &self,
        config: &sunrise_core::nbt::Tag,
    ) -> Result<PlacementModifierRef, FeatureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| FeatureError::new(std::io::Error::other(e.to_string())))?;
        let modifier = self
            .scope
            .decode_modifier(&value.to_string())
            .map_err(|e| FeatureError::new(std::io::Error::other(e.to_string())))?;
        Ok(modifier_ref(Arc::new(modifier)))
    }
}

/// Placement context over the engine's `ModPlacementContext`.
struct Placing<'a> {
    inner: &'a mut dyn ModPlacementContext,
    registry: Option<&'static BlockRegistry>,
    worker: Option<&'static WorkerResources>,
    fallback_index: &'a Index,
    fallback_metadata: &'static Blocks,
    fallback_air: BlockState,
    /// The engine's own error from the last nested modifier that failed, so a
    /// mod passing that failure on keeps its kind (the work limit above all).
    failure: Option<PlacementQueryError>,
}
impl Placing<'_> {
    fn index(&self) -> &Index {
        self.worker
            .map_or(self.fallback_index, |worker| &worker.index)
    }
    fn metadata_ref(&self) -> &'static Blocks {
        self.worker
            .map_or(self.fallback_metadata, |worker| worker.metadata)
    }
    fn air(&self) -> BlockState {
        self.worker.map_or(self.fallback_air, |worker| worker.air)
    }
}
impl PlacementModifierContext for Placing<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.metadata_ref()
    }
    fn state(&self, block: BlockKey) -> BlockState {
        BlockState::of(self.metadata_ref(), block, &[]).unwrap_or_else(|_| self.air())
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        if let (Some(worker), Some(registry)) = (self.worker, self.registry) {
            worker.state_with(registry, block, properties)
        } else {
            let metadata = self.metadata_ref();
            let mut state = BlockState::of(metadata, block, &[]).unwrap_or_else(|_| self.air());
            for &(key, value) in properties {
                if let Ok(next) = state.with(metadata, key, value) {
                    state = next;
                }
            }
            state
        }
    }
    fn is_air(&self, state: BlockState) -> bool {
        if let Some(worker) = self.worker {
            worker.is_air(state)
        } else {
            self.index().to_engine(state).is_ok()
                && matches!(
                    self.metadata_ref().name(state.block()),
                    Ok("minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
                )
        }
    }
    fn world_seed(&mut self) -> Option<i64> {
        self.inner.world_seed()
    }
    fn min_y(&mut self) -> Option<i32> {
        self.inner.min_y()
    }
    fn max_y(&mut self) -> Option<i32> {
        self.inner.max_y()
    }
    fn sea_level(&mut self) -> Option<i32> {
        self.inner.sea_level()
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState> {
        self.inner
            .block(pos)
            .and_then(|state| self.index().to_api(state).ok())
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        self.inner.block_in_tag(pos, tag)
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        self.inner.height(engine_heightmap(kind), x, z)
    }
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey> {
        self.inner.biome(pos)
    }
    fn apply_modifier(
        &mut self,
        modifier: &PlacementModifierRef,
        random: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), FeatureError>,
    ) -> Result<(), FeatureError> {
        let modifier = engine_modifier(modifier).ok_or_else(|| {
            FeatureError::new(std::io::Error::other("invalid placement modifier handle"))
        })?;
        let mut refused = None;
        let result = self
            .inner
            .apply_modifier(&modifier, random, origin, &mut |pos| {
                output(pos).map_err(|e| {
                    let failed = PlacementQueryError::Failed(e.to_string());
                    refused = Some(e);
                    failed
                })
            });
        match (result, refused) {
            (Ok(()), _) => Ok(()),
            (Err(_), Some(refused)) => Err(refused),
            (Err(e), None) => {
                let message = FeatureError::new(std::io::Error::other(e.to_string()));
                self.failure = Some(e);
                Err(message)
            }
        }
    }
}

/// The engine's placement modifier type around a mod's.
struct ModifierType {
    inner: Arc<dyn PlacementModifierType>,
}
impl ModPlacementModifierType for ModifierType {
    fn decode(
        &self,
        config: &serde_json::Value,
        scope: &PlacementDecodeScope<'_>,
    ) -> Result<Arc<dyn ModPlacementModifier>, String> {
        let registry = scope
            .block_registry()
            .ok_or_else(|| "placement modifier decode requires a block registry".to_owned())?;
        let worker = WorkerResources::get(registry).map_err(|e| e.to_string())?;
        let config = sunrise_core::json_nbt::json_to_tag(config).map_err(|e| e.to_string())?;
        let context = Decoding {
            scope,
            registry,
            worker,
        };
        let inner = self
            .inner
            .decode(&context, &config)
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(ModModifier {
            inner,
            index: worker.index.clone(),
            metadata: worker.metadata,
            air: worker.air,
        }))
    }
}

/// The engine's placement modifier around a mod's.
struct ModModifier {
    inner: Arc<dyn ApiPlacementModifier>,
    index: Arc<Index>,
    metadata: &'static Blocks,
    air: BlockState,
}
impl std::fmt::Debug for ModModifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModModifier")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}
impl ModPlacementModifier for ModModifier {
    fn modify(
        &self,
        context: &mut dyn ModPlacementContext,
        random: &mut dyn RandomSource,
        origin: [i32; 3],
        output: &mut dyn FnMut([i32; 3]) -> Result<(), PlacementQueryError>,
    ) -> Result<(), PlacementQueryError> {
        let registry = context.block_registry();
        let worker = registry.and_then(|r| WorkerResources::get(r).ok());
        let mut placing = Placing {
            inner: context,
            registry,
            worker,
            fallback_index: &self.index,
            fallback_metadata: self.metadata,
            fallback_air: self.air,
            failure: None,
        };
        let mut refused = None;
        let result = self.inner.modify(&mut placing, random, origin, &mut |pos| {
            output(pos).map_err(|e| {
                let message = FeatureError::new(std::io::Error::other(e.to_string()));
                refused = Some(e);
                message
            })
        });
        // A mod's error that passes on the engine's own keeps the engine's kind.
        result.map_err(|e| {
            refused
                .or(placing.failure)
                .unwrap_or(PlacementQueryError::Failed(e.to_string()))
        })
    }
}

#[cfg(test)]
#[path = "native_placement_modifiers/tests.rs"]
mod tests;

/// Registered API declarations become engine adapters once.
pub fn installed_placement_modifier_types() -> ModPlacementModifierTypes {
    mod_api::__private::placement_modifier_type_declarations()
        .into_iter()
        .map(|(id, inner)| {
            let ty: Arc<dyn ModPlacementModifierType> = Arc::new(ModifierType { inner });
            (id, ty)
        })
        .collect()
}
