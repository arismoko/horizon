//! Native worker adapters for API-owned structure processor types: the engine's
//! `ModStructureProcessorType` and `ModStructureProcessor` around a mod's
//! `StructureProcessorType` and `StructureProcessor`.
use super::native_pool_elements::api_rotation;
use super::native_structure_types::engine_heightmap;
use super::native_worldgen::WorkerResources;
use mod_api::{
    BiomeKey, BlockKey, BlockMetadata, BlockState, BlockUpdateFlags, HeightmapType, LiquidSettings,
    ProcessorContext, ProcessorDecodeContext, ProcessorFinalizeContext, RandomSource,
    StructureBlockInfo, StructureError, StructureMirror, StructurePlaceSettings,
    StructureProcessor, StructureProcessorListRef, StructureProcessorRef, StructureProcessorType,
};
use std::sync::Arc;
use sunrise_world::worldgen::{
    block_feature::FeatureBlockBindings,
    placement::PlacementQueryError,
    structure::{
        BlockInfo, Mirror, StructureResources,
        mod_processors::{
            ModProcessorContext, ModProcessorFinalizeContext, ModStructureProcessor,
            ModStructureProcessorType, ModStructureProcessorTypes, PlaceSettingsSnapshot,
        },
        processor::{Processor, ProcessorList},
    },
};

fn api_mirror(mirror: Mirror) -> StructureMirror {
    match mirror {
        Mirror::None => StructureMirror::None,
        Mirror::LeftRight => StructureMirror::LeftRight,
        Mirror::FrontBack => StructureMirror::FrontBack,
    }
}

fn api_settings(snapshot: PlaceSettingsSnapshot) -> StructurePlaceSettings {
    let mut settings = StructurePlaceSettings::new(api_rotation(snapshot.rotation));
    settings.mirror = api_mirror(snapshot.mirror);
    settings.pivot = snapshot.pivot;
    settings.bounding_box = snapshot.bounding_box;
    settings.liquid = if snapshot.waterlogging {
        LiquidSettings::ApplyWaterlogging
    } else {
        LiquidSettings::IgnoreWaterlogging
    };
    settings.known_shape = snapshot.known_shape;
    settings.ignore_entities = snapshot.ignore_entities;
    settings.finalize_entities = snapshot.finalize_entities;
    settings
}

fn api_block_info(worker: &WorkerResources, info: &BlockInfo) -> StructureBlockInfo {
    let state = worker.index.to_api(info.state).unwrap_or(worker.air);
    StructureBlockInfo::new(info.pos, state, info.nbt.as_deref().cloned())
}

fn engine_block_info(
    worker: &WorkerResources,
    info: StructureBlockInfo,
) -> Result<BlockInfo, PlacementQueryError> {
    let state = worker
        .index
        .to_engine(info.state)
        .map_err(|e| PlacementQueryError::Failed(format!("invalid block state: {e:?}")))?;
    Ok(BlockInfo {
        pos: info.pos,
        state,
        nbt: info.nbt.map(Box::new),
    })
}

fn processor_ref(processor: Arc<Processor>) -> StructureProcessorRef {
    let mod_processor = match &*processor {
        Processor::Mod(inner) => {
            let inner: &dyn std::any::Any = &**inner;
            inner
                .downcast_ref::<ModProcessor>()
                .map(|p| p.inner.clone())
        }
        _ => None,
    };
    mod_api::__private::structure_processor_ref(processor, mod_processor)
}

fn engine_processor(handle: &StructureProcessorRef) -> Option<Arc<Processor>> {
    mod_api::__private::structure_processor_payload(handle)
        .clone()
        .downcast::<Processor>()
        .ok()
}

fn processor_list_ref(list: Arc<ProcessorList>) -> StructureProcessorListRef {
    mod_api::__private::structure_processor_list_ref(list)
}

fn engine_processor_list(handle: &StructureProcessorListRef) -> Option<Arc<ProcessorList>> {
    mod_api::__private::structure_processor_list_payload(handle)
        .clone()
        .downcast::<ProcessorList>()
        .ok()
}

/// Decode context over the engine's processor and processor list codecs.
struct Decoding<'a> {
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    worker: &'a WorkerResources,
}
impl ProcessorDecodeContext for Decoding<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.resources.registry, block, properties)
    }
    fn processor(
        &self,
        config: &sunrise_core::nbt::Tag,
    ) -> Result<StructureProcessorRef, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let processor = Processor::decode(&value, self.resources, self.bindings)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        Ok(processor_ref(Arc::new(processor)))
    }
    fn processor_list(
        &self,
        config: &sunrise_core::nbt::Tag,
    ) -> Result<StructureProcessorListRef, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let list = ProcessorList::decode_value(&value, self.resources, self.bindings)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        Ok(processor_list_ref(Arc::new(list)))
    }
}

/// Shared adapter implementing `ProcessorContext` for both `process_block` and
/// `finalize_processing`, and `ProcessorFinalizeContext` when `C` supports
/// writes.
struct ProcessView<'a, C: ?Sized> {
    inner: &'a mut C,
    worker: &'a WorkerResources,
}
impl<C: ModProcessorContext + ?Sized> ProcessorContext for ProcessView<'_, C> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn offset(&self) -> [i32; 3] {
        self.inner.offset()
    }
    fn pivot(&self) -> [i32; 3] {
        self.inner.pivot()
    }
    fn settings(&self) -> StructurePlaceSettings {
        api_settings(self.inner.settings())
    }
    fn random(&mut self) -> &mut dyn RandomSource {
        self.inner.random()
    }
    fn random_at(&mut self, pos: [i32; 3]) -> &mut dyn RandomSource {
        self.inner.random_at(pos)
    }
    fn state(&self, block: BlockKey) -> BlockState {
        self.worker.state(block)
    }
    fn state_with(&self, block: BlockKey, properties: &[(&str, &str)]) -> BlockState {
        self.worker
            .state_with(self.inner.resources().registry, block, properties)
    }
    fn is_air(&self, state: BlockState) -> bool {
        self.worker.is_air(state)
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
            .and_then(|state| self.worker.index.to_api(state).ok())
    }
    fn block_in_tag(&mut self, pos: [i32; 3], tag: &str) -> Option<bool> {
        self.inner.block_in_tag(pos, tag)
    }
    fn state_in_tag(&mut self, state: BlockState, tag: &str) -> Option<bool> {
        let state = self.worker.index.to_engine(state).ok()?;
        self.inner.state_in_tag(state, tag)
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Option<i32> {
        self.inner.height(engine_heightmap(kind), x, z)
    }
    fn biome(&mut self, pos: [i32; 3]) -> Option<BiomeKey> {
        self.inner.biome(pos)
    }
    fn is_water(&mut self, pos: [i32; 3]) -> Option<bool> {
        self.inner.is_water(pos)
    }
    fn can_occlude(&mut self, state: BlockState) -> Option<bool> {
        let state = self.worker.index.to_engine(state).ok()?;
        self.inner.can_occlude(state)
    }
    fn apply_processor(
        &mut self,
        processor: &StructureProcessorRef,
        original: &StructureBlockInfo,
        current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError> {
        let processor = engine_processor(processor).ok_or_else(|| {
            StructureError::new(std::io::Error::other("invalid structure processor handle"))
        })?;
        let original = engine_block_info(self.worker, original.clone())
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let current = engine_block_info(self.worker, current)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let out = self
            .inner
            .apply_processor(&processor, &original, current)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        Ok(out.as_ref().map(|info| api_block_info(self.worker, info)))
    }
    fn apply_processor_list(
        &mut self,
        list: &StructureProcessorListRef,
        original: &StructureBlockInfo,
        current: StructureBlockInfo,
    ) -> Result<Option<StructureBlockInfo>, StructureError> {
        let list = engine_processor_list(list).ok_or_else(|| {
            StructureError::new(std::io::Error::other(
                "invalid structure processor list handle",
            ))
        })?;
        let original = engine_block_info(self.worker, original.clone())
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let current = engine_block_info(self.worker, current)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        let out = self
            .inner
            .apply_processor_list(&list, &original, current)
            .map_err(|e| StructureError::new(std::io::Error::other(e.to_string())))?;
        Ok(out.as_ref().map(|info| api_block_info(self.worker, info)))
    }
}
impl<C: ModProcessorFinalizeContext + ?Sized> ProcessorFinalizeContext for ProcessView<'_, C> {
    fn set_block(&mut self, pos: [i32; 3], state: BlockState, flags: BlockUpdateFlags) -> bool {
        let Ok(state) = self.worker.index.to_engine(state) else {
            return false;
        };
        self.inner.set_block(pos, state, flags.bits())
    }
}

/// The engine's processor type around a mod's.
struct ProcessorType {
    inner: Arc<dyn StructureProcessorType>,
}
impl ModStructureProcessorType for ProcessorType {
    fn decode(
        &self,
        config: &serde_json::Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Arc<dyn ModStructureProcessor>, String> {
        let worker = WorkerResources::get(resources.registry).map_err(|e| e.to_string())?;
        let config = sunrise_core::json_nbt::json_to_tag(config).map_err(|e| e.to_string())?;
        let context = Decoding {
            resources,
            bindings,
            worker,
        };
        let inner = self
            .inner
            .decode(&context, &config)
            .map_err(|e| e.to_string())?;
        Ok(Arc::new(ModProcessor { inner }))
    }
}

/// The engine's processor around a mod's.
#[derive(Debug)]
struct ModProcessor {
    inner: Arc<dyn StructureProcessor>,
}
impl ModStructureProcessor for ModProcessor {
    fn process_block(
        &self,
        context: &mut dyn ModProcessorContext,
        original: &BlockInfo,
        current: BlockInfo,
    ) -> Result<Option<BlockInfo>, PlacementQueryError> {
        let worker = WorkerResources::get(context.resources().registry)?;
        let original = api_block_info(worker, original);
        let current = api_block_info(worker, &current);
        let mut view = ProcessView {
            inner: context,
            worker,
        };
        let out = self
            .inner
            .process_block(&mut view, &original, current)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        out.map(|info| engine_block_info(worker, info)).transpose()
    }
    fn finalize_processing(
        &self,
        context: &mut dyn ModProcessorFinalizeContext,
        original: &[BlockInfo],
        processed: Vec<BlockInfo>,
    ) -> Result<Vec<BlockInfo>, PlacementQueryError> {
        let worker = WorkerResources::get(context.resources().registry)?;
        let original: Vec<_> = original
            .iter()
            .map(|info| api_block_info(worker, info))
            .collect();
        let processed: Vec<_> = processed
            .iter()
            .map(|info| api_block_info(worker, info))
            .collect();
        let mut view = ProcessView {
            inner: context,
            worker,
        };
        let out = self
            .inner
            .finalize_processing(&mut view, &original, processed)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))?;
        out.into_iter()
            .map(|info| engine_block_info(worker, info))
            .collect()
    }
    fn evaluates_entire_piece_state(&self) -> bool {
        self.inner.evaluates_entire_piece_state()
    }
}

#[cfg(test)]
#[path = "native_structure_processors/tests.rs"]
mod tests;

/// Registered API declarations become engine adapters once.
pub fn installed_structure_processor_types() -> ModStructureProcessorTypes {
    mod_api::__private::structure_processor_type_declarations()
        .into_iter()
        .map(|(id, inner)| {
            let ty: Arc<dyn ModStructureProcessorType> = Arc::new(ProcessorType { inner });
            (id, ty)
        })
        .collect()
}
