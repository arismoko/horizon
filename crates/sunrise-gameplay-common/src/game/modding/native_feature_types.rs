//! Native worker adapter for API-owned feature behaviours: the engine's `ModFeatureType` around a
//! mod's `FeatureType`, over the caller's own world and random.
use super::native_worldgen::WorkerResources;
use mod_api::__private::game::CallbackOwner;
use mod_api::{
    BiomeKey, BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, DatapackEntry, DatapackField,
    FeatureContext, FeatureType, HeightmapType, RandomSource, TagKey,
};
use std::any::Any;
use std::sync::Arc;
use sunrise_core::nbt::{Compound, Tag};
use sunrise_world::registry::BlockRegistry;
use sunrise_world::worldgen::mod_features::{
    ModFeatureConfig, ModFeatureEntry, ModFeatureRandom, ModFeatureTarget, ModFeatureType,
    ModFeatureTypes,
};

fn engine_heightmap(kind: HeightmapType) -> sunrise_world::heightmap::HeightmapType {
    use sunrise_world::heightmap::HeightmapType as Engine;
    match kind {
        HeightmapType::WorldSurfaceWg => Engine::WorldSurfaceWg,
        HeightmapType::WorldSurface => Engine::WorldSurface,
        HeightmapType::OceanFloorWg => Engine::OceanFloorWg,
        HeightmapType::OceanFloor => Engine::OceanFloor,
        HeightmapType::MotionBlocking => Engine::MotionBlocking,
        HeightmapType::MotionBlockingNoLeaves => Engine::MotionBlockingNoLeaves,
        _ => Engine::WorldSurfaceWg,
    }
}

/// The caller's random, as a [`RandomSource`].
struct Draws<'a>(&'a mut dyn ModFeatureRandom);
impl RandomSource for Draws<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        CallbackOwner::call_engine(|| self.0.int(bound))
    }
    fn next_long(&mut self) -> i64 {
        CallbackOwner::call_engine(|| self.0.long())
    }
    fn next_float(&mut self) -> f32 {
        CallbackOwner::call_engine(|| self.0.float())
    }
    fn next_double(&mut self) -> f64 {
        CallbackOwner::call_engine(|| self.0.double())
    }
    fn next_boolean(&mut self) -> bool {
        CallbackOwner::call_engine(|| self.0.boolean())
    }
    fn next_int(&mut self) -> i32 {
        CallbackOwner::call_engine(|| self.0.int_unbounded())
    }
    fn next_gaussian(&mut self) -> f64 {
        CallbackOwner::call_engine(|| self.0.gaussian())
    }
}

/// The engine's target, as a [`FeatureContext`].
struct Placing<'a> {
    target: &'a mut dyn ModFeatureTarget,
    mod_id: &'static str,
    worker: &'static WorkerResources,
    registry: &'static BlockRegistry,
    random: Draws<'a>,
    config: &'a DatapackEntry,
}

impl FeatureContext for Placing<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn config(&self) -> &DatapackEntry {
        self.config
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        &mut self.random
    }
    fn min_y(&mut self) -> Option<i32> {
        CallbackOwner::call_engine(|| self.target.min_y())
    }
    fn max_y(&mut self) -> Option<i32> {
        CallbackOwner::call_engine(|| self.target.max_y())
    }
    fn state(&self, block: BlockKey) -> BlockState {
        CallbackOwner::call_engine(|| self.worker.state(block))
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        CallbackOwner::call_engine(|| self.worker.state_with(self.registry, block, properties))
    }
    fn is_air(&self, state: BlockState) -> bool {
        CallbackOwner::call_engine(|| self.worker.is_air(state))
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState> {
        CallbackOwner::call_engine(|| {
            let state = self.target.block(pos)?;
            self.worker.index.to_api(state).ok()
        })
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: TagKey) -> Option<bool> {
        CallbackOwner::call_engine(|| {
            mod_api::__private::with_tag_name(tag, |name| self.target.block_in_tag(pos, name?))
        })
    }
    fn ensure_can_write(&mut self, pos: [i32; 3]) -> bool {
        CallbackOwner::call_engine(|| self.target.ensure_can_write(pos))
    }
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey> {
        self.target.biome(pos)
    }
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool {
        CallbackOwner::call_engine(|| match self.worker.index.to_engine(state) {
            Ok(state) => self.target.set_block(pos, state, flags.bits()),
            Err(error) => {
                log::error!(
                    "mod {}: setting worldgen block at {pos:?} failed: {error}",
                    self.mod_id
                );
                false
            }
        })
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        CallbackOwner::call_engine(|| self.target.height(engine_heightmap(kind), x, z))
    }
    fn block_entity(&mut self, pos: [i32; 3], mut nbt: Compound) -> bool {
        CallbackOwner::call_engine(|| {
            let Some(block) = self
                .target
                .block(pos)
                .and_then(|state| self.registry.state(state))
            else {
                log::error!(
                    "mod {}: cannot set worldgen block entity at {pos:?}: no readable block",
                    self.mod_id
                );
                return false;
            };
            for (axis, value) in ["x", "y", "z"].into_iter().zip(pos) {
                nbt.insert(axis.into(), Tag::Int(value));
            }
            let tag = Tag::Compound(nbt);
            if sunrise_sim::save::decode_block_entity_for_block(&tag, &block.name).is_none() {
                let id = tag.get("id").filter(|id| matches!(id, Tag::String(_)));
                log::error!(
                    "mod {}: invalid worldgen block entity at {pos:?}: id {id:?} for {}",
                    self.mod_id,
                    block.name,
                );
                return false;
            }
            let Tag::Compound(nbt) = tag else {
                unreachable!("constructed as a compound")
            };
            self.target.set_block_entity(pos, nbt)
        })
    }
}

/// The adapter: the game's `ModFeatureType` around a mod's `FeatureType`.
struct Adapter {
    owner: CallbackOwner,
    inner: Arc<dyn FeatureType>,
    schema: Vec<DatapackField>,
}

impl ModFeatureType for Adapter {
    fn prepare_config(&self, config: &serde_json::Value) -> Result<ModFeatureConfig, String> {
        let entry = mod_api::__private::decode_feature_config(&self.schema, config)?;
        Ok(Arc::new(entry))
    }
    fn place(
        &self,
        target: &mut dyn ModFeatureTarget,
        random: &mut dyn ModFeatureRandom,
        origin: [i32; 3],
        config: &(dyn Any + Send + Sync),
    ) -> Result<bool, String> {
        let Some(registry) = target.registry() else {
            return Err("this world has no block registry for mod features".to_owned());
        };
        let config = config
            .downcast_ref::<DatapackEntry>()
            .ok_or_else(|| "config was not decoded by this adapter".to_owned())?;
        let worker = WorkerResources::get(registry).map_err(|error| error.to_string())?;
        let mut context = Placing {
            target,
            mod_id: self.owner.id(),
            worker,
            registry,
            random: Draws(random),
            config,
        };
        // The owner reports panics and phase refusals itself; don't report them again as
        // returned FeatureErrors. Only the mod's typed result reaches the placement caller.
        self.owner
            .call("feature type", "FeatureType::place", || {
                // The error's text and its destructor run here, inside the retained boundary.
                self.inner
                    .place(&mut context, origin)
                    .map_err(|error| mod_api::__private::game::error_chain(&error))
            })
            .unwrap_or(Ok(false))
    }
}

#[cfg(test)]
#[path = "native_feature_types/tests.rs"]
mod tests;

/// Registered API declarations become engine adapters once, not once per chunk.
pub fn installed_feature_types() -> ModFeatureTypes {
    mod_api::__private::feature_type_declarations()
        .into_iter()
        .map(|(id, mod_id, schema, inner)| {
            let ty = Arc::new(Adapter {
                owner: CallbackOwner::for_mod(mod_id),
                inner,
                schema,
            });
            (id, Arc::new(ModFeatureEntry { mod_id, ty }))
        })
        .collect()
}
