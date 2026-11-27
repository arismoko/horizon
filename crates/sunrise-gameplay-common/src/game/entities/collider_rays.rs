use crate::game::ray_shapes::{ray_interaction_shape, ray_shape_from_boxes};
use std::io;
use sunrise_core::{
    geometry::{Vec3, VoxelShape},
    position::BlockPos,
};
use sunrise_items::blocks::VisualShape;
use sunrise_level::collisions::SceneCollisions;
use sunrise_world::{
    collision_context::CollisionContext,
    registry::BlockStateId,
    world_ray::{RayBlockMode, RayWorld},
};

/// Outline, visual and interaction shape tables for [`ColliderRays`] when clipping with
/// `RayBlockMode::Outline`, `RayBlockMode::Visual` or `BlockGetter.clipWithInteractionOverride`.
pub struct ClipData<'a, 'r> {
    pub facts: &'a sunrise_items::blocks::BlockFacts,
    pub behaviours: &'a sunrise_world::level::Behaviours,
    pub tables: &'a crate::game::native_block_tables::BlockTables<'r>,
    pub registry: &'r sunrise_world::registry::BlockRegistry,
}

/// A ray read a cell inside a column that is not loaded while [`ColliderRays::report_unloaded`]
/// is set. `clip` turns it into `ClipOutcome::Unloaded`.
#[derive(Debug)]
pub struct UnloadedColumn(pub BlockPos);

impl std::fmt::Display for UnloadedColumn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "column of {:?} is not loaded", self.0)
    }
}

impl std::error::Error for UnloadedColumn {}

/// `BlockGetter.clip` over a resident [`World`](sunrise_level::world::World): out-of-height
/// cells read as `world.air` (`BlockGetter.getBlockState`) and loads nothing. A missing column
/// reads as `world.air` too (`ClientChunkCache.getChunk(..., true)` on clients and viewed worlds,
/// and `PathNavigationRegion.getBlockState`/`getChunk` on immutable server views), unless
/// `report_unloaded` asks for the ray to stop there with [`UnloadedColumn`].
pub struct ColliderRays<'a, 'r> {
    pub scene: SceneCollisions<'a, 'r>,
    pub context: CollisionContext<'a>,
    pub clip_data: Option<ClipData<'a, 'r>>,
    pub report_unloaded: bool,
}
impl RayWorld for ColliderRays<'_, '_> {
    type Error = io::Error;
    type Block = BlockStateId;
    type Fluid = ();
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        let world = self.scene.world;
        if !world.shape.contains_y(p.y) {
            return Ok(world.air);
        }
        match world.chunks.get(&(p.x >> 4, p.z >> 4)) {
            Some(chunk) => Ok(if chunk.section_index(p.y).is_some() {
                chunk.block(p.x & 15, p.y, p.z & 15)
            } else {
                world.air
            }),
            None if self.report_unloaded => Err(io::Error::other(UnloadedColumn(p))),
            None => Ok(world.air),
        }
    }
    fn fluid_state(&mut self, _: BlockPos) -> io::Result<()> {
        Ok(())
    }
    fn block_shape(
        &mut self,
        state: BlockStateId,
        p: BlockPos,
        mode: RayBlockMode,
    ) -> io::Result<VoxelShape> {
        if mode == RayBlockMode::Visual {
            // `BlockStateBase.getVisualShape`: the overriding classes answer here,
            // every other block falls through to its collision shape below.
            let Some(data) = &self.clip_data else {
                return Err(io::Error::other("visual ray shape unavailable"));
            };
            match data.facts.get(state).visual_shape() {
                VisualShape::Empty => return Ok(VoxelShape::empty()),
                VisualShape::Block => return Ok(VoxelShape::block()),
                VisualShape::Outline => return self.block_shape(state, p, RayBlockMode::Outline),
                VisualShape::Collision => {}
            }
        }
        if mode == RayBlockMode::Outline {
            let Some(data) = &self.clip_data else {
                return Err(io::Error::other("outline ray shape unavailable"));
            };
            // A block entity's published shapes stand over the declared ones, a
            // dynamic template's included (#1666): the collider below reads them
            // through the binding, the outline here.
            let at = [p.x, p.y, p.z];
            let facts = data.facts.get(state);
            let offset = crate::game::native_state_shapes::outline_offset(
                data.facts,
                state,
                p.x,
                p.z,
                facts.outline_offset,
            );
            let offset = Vec3::new(offset[0], offset[1], offset[2]);
            if let Some(shape) = sunrise_level::collisions::published_outline(
                self.scene.world,
                self.scene.binding,
                state,
                at,
            ) {
                return Ok(if offset == Vec3::ZERO {
                    shape.clone()
                } else {
                    shape.moved(offset)
                });
            }
            let cached = self.scene.binding.cached_shapes();
            if let Some(shape) = cached.context_outline_shape(state, self.context.entity_kind()) {
                return Ok(cached.positioned_shape(
                    state,
                    sunrise_world::collision::ShapeChannel::ContextOutline,
                    at,
                    shape,
                ));
            }
            if data.behaviours.data(state).dynamic_shape {
                return Err(io::Error::other("dynamic outline shape unsupported"));
            }
            let shape = ray_shape_from_boxes(&facts.outline)?;
            return Ok(if offset == Vec3::ZERO {
                shape
            } else {
                shape.moved(offset)
            });
        }
        let binding = self.scene.binding;
        binding
            .collision_shape_with_context(&mut self.scene, state, p, &self.context)
            .map_err(|e| io::Error::other(format!("{e:?}")))
    }
    fn interaction_shape(&mut self, state: BlockStateId, p: BlockPos) -> io::Result<VoxelShape> {
        let cached = self.scene.binding.cached_shapes();
        if let Some(shape) = cached.interaction_shape(state) {
            return Ok(cached.positioned_shape(
                state,
                sunrise_world::collision::ShapeChannel::Interaction,
                [p.x, p.y, p.z],
                shape,
            ));
        }
        match &self.clip_data {
            Some(data) => ray_interaction_shape(data.registry, state),
            None => Ok(VoxelShape::empty()),
        }
    }
    fn fluid_is_source(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_is_empty(&mut self, _: ()) -> io::Result<bool> {
        Ok(true)
    }
    fn fluid_is_water(&mut self, _: ()) -> io::Result<bool> {
        Ok(false)
    }
    fn fluid_shape(&mut self, _: (), _: BlockPos) -> io::Result<VoxelShape> {
        Ok(VoxelShape::empty())
    }
}
