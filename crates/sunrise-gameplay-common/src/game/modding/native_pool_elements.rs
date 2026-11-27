//! Native worker adapters for API-owned pool element types: the engine's
//! `ModPoolElementType` and `ModPoolElement` around a mod's `PoolElementType`
//! and `PoolElement`, and the element reads a structure type or an element
//! makes during layout.
use super::native_structure_types::{Draws, Placing};
use super::native_worldgen::WorkerResources;
use crate::game::native_structure_values::engine_rotation;
use mod_api::{
    BlockMetadata, ElementContext, ElementDecodeContext, ElementPlacement, Identifier, JigsawBlock,
    JigsawJoint, PoolElementRef, PoolElementType, RandomSource, StructureError, StructureRotation,
};
use std::borrow::Cow;
use std::sync::Arc;
use sunrise_core::{bounding_box::BoundingBox, nbt::Compound};
use sunrise_world::worldgen::{
    block_feature::FeatureBlockBindings,
    placement::PlacementQueryError,
    structure::{
        Rotation, StructureResources,
        jigsaw::{ElementAt, Jigsaw, PoolElement, Projection},
        legacy::{PieceContext, PieceDraws},
        mod_elements::{ModPoolElement, ModPoolElementType, ModPoolElementTypes},
        processor::ProcessorRandom,
    },
};

/// The API's rotation for an engine one.
pub(super) fn api_rotation(rotation: Rotation) -> StructureRotation {
    match rotation {
        Rotation::None => StructureRotation::None,
        Rotation::Clockwise90 => StructureRotation::Clockwise90,
        Rotation::Clockwise180 => StructureRotation::Clockwise180,
        Rotation::Counterclockwise90 => StructureRotation::Counterclockwise90,
    }
}

pub(super) fn api_projection(projection: Projection) -> mod_api::Projection {
    match projection {
        Projection::Rigid => mod_api::Projection::Rigid,
        Projection::TerrainMatching => mod_api::Projection::TerrainMatching,
    }
}

pub(super) fn engine_projection(projection: mod_api::Projection) -> Projection {
    match projection {
        mod_api::Projection::Rigid => Projection::Rigid,
        mod_api::Projection::TerrainMatching => Projection::TerrainMatching,
    }
}

/// The handle a mod sees for one of the engine's elements.
pub(super) fn element_ref(element: Arc<PoolElement>) -> PoolElementRef {
    let mod_element = match &*element {
        PoolElement::Mod { element, .. } => {
            let element: &dyn std::any::Any = &**element;
            element
                .downcast_ref::<ModElement>()
                .map(|element| element.inner.clone())
        }
        _ => None,
    };
    let projection = api_projection(element.projection());
    let ground_level_delta = element.ground_level_delta();
    let empty = matches!(*element, PoolElement::Empty);
    let template_location = match &*element {
        PoolElement::Single { template, .. } => Identifier::parse(template.location()).ok(),
        _ => None,
    };
    mod_api::__private::pool_element_ref(
        element,
        mod_element,
        projection,
        ground_level_delta,
        empty,
        template_location,
    )
}

/// The engine's element behind a handle. Only the game builds handles, so
/// the payload is always one.
pub(super) fn engine_element(element: &PoolElementRef) -> Option<Arc<PoolElement>> {
    mod_api::__private::pool_element_payload(element)
        .clone()
        .downcast::<PoolElement>()
        .ok()
}

/// A mod's random, as the engine's piece draws.
struct ApiDraws<'a>(&'a mut dyn RandomSource);
impl ProcessorRandom for ApiDraws<'_> {
    fn float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn int(&mut self, bound: i32) -> i32 {
        self.0.next_int_bound(bound)
    }
    fn long(&mut self) -> i64 {
        self.0.next_long()
    }
    fn int_unbounded(&mut self) -> i32 {
        self.0.next_int()
    }
    fn double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
    fn gaussian(&mut self) -> f64 {
        self.0.next_gaussian()
    }
}
impl PieceDraws for ApiDraws<'_> {}

/// An engine jigsaw as the API's. An id the API can't parse reads as
/// `minecraft:empty`, the id vanilla's jigsaws default to.
fn api_jigsaw(jigsaw: Jigsaw<'_>) -> JigsawBlock {
    let id = |text: &str| {
        Identifier::parse(text).unwrap_or_else(|_| {
            Identifier::from_parts("minecraft", "empty").expect("a valid identifier")
        })
    };
    let mut block = JigsawBlock::new(
        jigsaw.pos,
        jigsaw.front,
        jigsaw.top,
        id(&jigsaw.pool),
        id(&jigsaw.target),
    );
    block.joint = if jigsaw.rollable {
        JigsawJoint::Rollable
    } else {
        JigsawJoint::Aligned
    };
    block.name = jigsaw.name.as_deref().map(id);
    block.placement_priority = jigsaw.placement_priority;
    block.selection_priority = jigsaw.selection_priority;
    block
}

/// An API jigsaw as the engine's.
pub(super) fn engine_jigsaw(jigsaw: JigsawBlock) -> Jigsaw<'static> {
    Jigsaw {
        pos: jigsaw.pos,
        front: jigsaw.front,
        top: jigsaw.top,
        rollable: jigsaw.joint == JigsawJoint::Rollable,
        name: jigsaw.name.map(|name| Cow::Owned(name.to_string())),
        pool: Cow::Owned(jigsaw.pool.to_string()),
        target: Cow::Owned(jigsaw.target.to_string()),
        placement_priority: jigsaw.placement_priority,
        selection_priority: jigsaw.selection_priority,
    }
}

/// The element reads, over the engine's elements.
pub(super) struct Elements<'a> {
    pub(super) resources: &'a StructureResources,
    pub(super) worker: &'a WorkerResources,
}
impl ElementContext for Elements<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn jigsaws(
        &self,
        element: &PoolElementRef,
        position: [i32; 3],
        rotation: StructureRotation,
        random: &mut dyn RandomSource,
    ) -> Vec<JigsawBlock> {
        let (Some(element), Some(rotation)) = (engine_element(element), engine_rotation(rotation))
        else {
            return Vec::new();
        };
        element
            .shuffled_jigsaws(self.resources, position, rotation, &mut ApiDraws(random))
            .into_iter()
            .map(api_jigsaw)
            .collect()
    }
    fn bounding_box(
        &self,
        element: &PoolElementRef,
        position: [i32; 3],
        rotation: StructureRotation,
    ) -> BoundingBox {
        match (engine_element(element), engine_rotation(rotation)) {
            (Some(element), Some(rotation)) => {
                element.bounding_box(self.resources, position, rotation)
            }
            _ => BoundingBox::new(position, position),
        }
    }
}

/// The decode context, over the engine's element codec.
struct Decoding<'a> {
    resources: &'a StructureResources,
    bindings: &'a FeatureBlockBindings<'a>,
    worker: &'a WorkerResources,
}
impl ElementDecodeContext for Decoding<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn element(&self, config: &sunrise_core::nbt::Tag) -> Result<PoolElementRef, StructureError> {
        let value = sunrise_core::json_nbt::tag_to_json(config)
            .map_err(|error| StructureError::new(std::io::Error::other(error.to_string())))?;
        let element = PoolElement::decode_element(&value, self.resources, self.bindings)
            .map_err(|error| StructureError::new(std::io::Error::other(error.to_string())))?;
        Ok(element_ref(Arc::new(element)))
    }
}

/// The engine's element type around a mod's.
struct ElementType {
    id: &'static str,
    mod_id: &'static str,
    inner: Arc<dyn PoolElementType>,
}
impl ModPoolElementType for ElementType {
    fn decode(
        &self,
        config: &serde_json::Value,
        resources: &StructureResources,
        bindings: &FeatureBlockBindings<'_>,
    ) -> Result<Arc<dyn ModPoolElement>, String> {
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
        Ok(Arc::new(ModElement {
            id: self.id,
            mod_id: self.mod_id,
            inner,
        }))
    }
}

/// The engine's element around a mod's.
#[derive(Debug)]
struct ModElement {
    id: &'static str,
    mod_id: &'static str,
    inner: Arc<dyn mod_api::PoolElement>,
}
impl ModPoolElement for ModElement {
    fn shuffled_jigsaws(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
        random: &mut dyn PieceDraws,
    ) -> Vec<Jigsaw<'static>> {
        let Ok(worker) = WorkerResources::get(resources.registry) else {
            return Vec::new();
        };
        let context = Elements { resources, worker };
        self.inner
            .shuffled_jigsaws(
                &context,
                position,
                api_rotation(rotation),
                &mut Draws(random),
            )
            .into_iter()
            .map(engine_jigsaw)
            .collect()
    }
    fn bounding_box(
        &self,
        resources: &StructureResources,
        position: [i32; 3],
        rotation: Rotation,
    ) -> BoundingBox {
        let Ok(worker) = WorkerResources::get(resources.registry) else {
            return BoundingBox::new(position, position);
        };
        let context = Elements { resources, worker };
        self.inner
            .bounding_box(&context, position, api_rotation(rotation))
    }
    fn place(
        &self,
        context: &mut PieceContext<'_>,
        at: ElementAt,
        projection: Projection,
    ) -> Result<bool, PlacementQueryError> {
        let mut placement = ElementPlacement::new(at.position, api_rotation(at.rotation));
        placement.reference = at.reference;
        placement.projection = api_projection(projection);
        placement.liquid = if at.waterlogging {
            mod_api::LiquidSettings::ApplyWaterlogging
        } else {
            mod_api::LiquidSettings::IgnoreWaterlogging
        };
        placement.keep_jigsaws = at.keep_jigsaws;
        let mut context = Placing::new(context, self.mod_id, self.id)?;
        self.inner
            .place(&mut context, &placement)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))
    }
    fn projection(&self) -> Projection {
        engine_projection(self.inner.projection())
    }
    fn ground_level_delta(&self) -> i32 {
        self.inner.ground_level_delta()
    }
    fn save(&self) -> Compound {
        self.inner.save()
    }
}

/// An API placement as the engine's; `None` for a rotation this build
/// doesn't know.
pub(super) fn engine_placement(placement: &ElementPlacement) -> Option<ElementAt> {
    Some(ElementAt {
        position: placement.position,
        reference: placement.reference,
        rotation: engine_rotation(placement.rotation)?,
        waterlogging: placement.liquid == mod_api::LiquidSettings::ApplyWaterlogging,
        keep_jigsaws: placement.keep_jigsaws,
    })
}

#[cfg(test)]
#[path = "native_pool_elements/tests.rs"]
mod tests;

/// Registered API declarations become engine adapters once.
pub fn installed_pool_element_types() -> ModPoolElementTypes {
    mod_api::__private::pool_element_type_declarations()
        .into_iter()
        .map(|(id, mod_id, inner)| {
            let static_id: &'static str = Box::leak(id.clone().into_boxed_str());
            let ty: Arc<dyn ModPoolElementType> = Arc::new(ElementType {
                id: static_id,
                mod_id,
                inner,
            });
            (id, ty)
        })
        .collect()
}
