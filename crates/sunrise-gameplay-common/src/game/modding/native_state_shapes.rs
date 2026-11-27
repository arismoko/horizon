//! Shape queries for a supplied state without replacing the block in the world.

pub use mod_api::BlockShapeKind;
use std::ops::ControlFlow;
use sunrise_core::{geometry::VoxelShape, position::BlockPos};
use sunrise_items::blocks::VisualShape;
use sunrise_level::{collisions::SceneCollisions, world::World};
use sunrise_level::{fast_hash::FxHashMap, shulker_box::Shulker};
use sunrise_world::{
    block_collision::BlockCollisionRegistry,
    collision::ShapeChannel,
    collision_context::{CollisionContext, EntityContextKind},
    registry::BlockStateId,
};

/// Common empty-context `BlockStateBase.getCollisionShape` query, including contextual
/// and block-entity-published shapes. The returned grid shares its backing storage.
pub fn collision_shape(
    collision: &BlockCollisionRegistry<'_>,
    world: &World,
    state: BlockStateId,
    pos: [i32; 3],
) -> VoxelShape {
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    collision
        .collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(pos[0], pos[1], pos[2]),
            &CollisionContext::empty(false),
        )
        .unwrap_or_else(|_| VoxelShape::empty())
}

/// The geometry chosen by selection, before its column offset is applied.
#[derive(Clone, Copy)]
pub enum Outline<'a> {
    Published(&'a VoxelShape),
    Declared(&'a VoxelShape, EntityContextKind),
    Dynamic([f64; 6]),
    Default(&'a sunrise_items::blocks::StateFacts),
}

pub fn outline_offset(
    facts: &sunrise_items::blocks::BlockFacts,
    state: BlockStateId,
    x: i32,
    z: i32,
    follows_offset: bool,
) -> [f64; 3] {
    if follows_offset {
        facts
            .states
            .get(state.0 as usize)
            .map_or([0.0; 3], |sf| sf.offset_at(x, z))
    } else {
        [0.0; 3]
    }
}

pub trait NativeStateShapesGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn selected_outline<'a>(
        &'a self,
        world: &'a World,
        state: BlockStateId,
        shulkers: &FxHashMap<[i32; 3], Shulker>,
        pos: [i32; 3],
        player_context: bool,
    ) -> (Outline<'a>, [f64; 3])
    where
        '__sg: 'a,
    {
        let facts = self.res.facts().get(state);
        let default_offset = || {
            outline_offset(
                self.res.facts(),
                state,
                pos[0],
                pos[2],
                facts.outline_offset,
            )
        };
        if let Some(shape) =
            sunrise_level::collisions::published_outline(world, self.collision, state, pos)
        {
            // Published outlines already followed the template's outline offset before
            // declarations added their own channels.
            return (Outline::Published(shape), default_offset());
        }
        let cached = self.collision.cached_shapes();
        let kind = if player_context {
            self.outline_context_kind()
        } else {
            EntityContextKind::Other
        };
        if let Some(shape) = cached.context_outline_shape(state, kind) {
            return (
                Outline::Declared(shape, kind),
                cached.shape_offset(state, ShapeChannel::ContextOutline, pos),
            );
        }
        if let Some(bounds) = self.context_outline(pos, state, player_context, shulkers) {
            return (Outline::Dynamic(bounds), [0.0; 3]);
        }
        (Outline::Default(facts), default_offset())
    }

    /// The same outline traversal without allocating a box list.
    fn outline_boxes_iter<'a>(
        &'a self,
        world: &'a World,
        state: BlockStateId,
        shulkers: &FxHashMap<[i32; 3], Shulker>,
        pos: [i32; 3],
        player_context: bool,
    ) -> impl Iterator<Item = [f64; 6]> + 'a
    where
        '__sg: 'a,
    {
        let (outline, offset) = self.selected_outline(world, state, shulkers, pos, player_context);
        let base = std::array::from_fn(|i| f64::from(pos[i]) + offset[i]);
        outline_boxes(outline, base)
    }

    fn visit_state_shape(
        &self,
        world: &World,
        shulkers: &FxHashMap<[i32; 3], Shulker>,
        state: BlockStateId,
        pos: [i32; 3],
        kind: BlockShapeKind,
        visit: &mut dyn FnMut([f64; 6]) -> ControlFlow<()>,
    ) -> ControlFlow<()> {
        match kind {
            BlockShapeKind::Outline => {
                let (outline, offset) = self.selected_outline(world, state, shulkers, pos, false);
                // Stay block-local; translating to world space and back loses precision.
                outline_boxes(outline, offset).try_for_each(visit)
            }
            BlockShapeKind::Collision => mod_api::__private::voxel_shape_from_core(
                collision_shape(self.collision, world, state, pos),
            )
            .boxes()
            .try_for_each(visit),
            // `BlockStateBase.getVisualShape`, from the same table `ClipBlock::Visual` reads.
            BlockShapeKind::Visual => {
                let kind = match self.res.facts().get(state).visual_shape() {
                    VisualShape::Collision => BlockShapeKind::Collision,
                    VisualShape::Outline => BlockShapeKind::Outline,
                    VisualShape::Empty => return ControlFlow::Continue(()),
                    VisualShape::Block => return visit([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]),
                };
                self.visit_state_shape(world, shulkers, state, pos, kind, visit)
            }
            _ => unreachable!("BlockShapeKind: unhandled variant"),
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeStateShapesGameExt1<'__sg> for G {}

pub(in crate::game) fn outline_boxes(
    outline: Outline<'_>,
    base: [f64; 3],
) -> impl Iterator<Item = [f64; 6]> + '_ {
    let (voxel, dynamic, defaults) = match outline {
        Outline::Published(shape) | Outline::Declared(shape, _) => (shape.boxes(), None, &[][..]),
        Outline::Dynamic(bounds) => (&[][..], Some(bounds), &[][..]),
        Outline::Default(facts) => (&[][..], None, &facts.outline[..]),
    };
    voxel
        .iter()
        .map(|b| {
            let min = b.min_position();
            let max = b.max_position();
            [min.x, min.y, min.z, max.x, max.y, max.z]
        })
        .chain(dynamic)
        .chain(defaults.iter().copied())
        .map(move |b| std::array::from_fn(|i| base[i % 3] + b[i]))
}

use crate::game::GameGameExt1 as _;
