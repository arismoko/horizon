//! Dual-role block collision box scans shared by server and client levels.
use mod_api::{__private::voxel_shape_from_core, VoxelShape};
use std::ops::ControlFlow;
use sunrise_core::geometry::{Aabb, Vec3};
use sunrise_level::collisions::SceneCollisions;
use sunrise_world::block_collision_scan::BlockCollisionScan;
use sunrise_world::collision_context::CollisionContext;

pub fn aabb_of(b: [f64; 6]) -> Option<Aabb> {
    (b.iter().all(|v| v.is_finite()) && b[0] <= b[3] && b[1] <= b[4] && b[2] <= b[5])
        .then(|| Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])))
}

pub trait NativeBotsCommonGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// `CollisionGetter.getBlockCollisions` with an empty context: no entities or border.
    /// The shared `BlockCollisions` cursor skips missing chunks without loading them.
    fn level_visit_block_collisions(
        &self,
        level: mod_api::LevelRef,
        aabb: [f64; 6],
        visit: &mut dyn FnMut(&VoxelShape) -> ControlFlow<()>,
    ) -> Option<ControlFlow<()>> {
        let bounds = aabb_of(aabb)?;
        let mut scene = SceneCollisions {
            world: self.native_level_view_world(level)?,
            binding: self.collision,
            subject: None,
        };
        let scan = BlockCollisionScan::new(
            &mut scene,
            CollisionContext::empty(false),
            bounds,
            false,
            usize::MAX,
        )
        .ok()?;
        for hit in scan {
            let shape = voxel_shape_from_core(hit.ok()?.shape);
            if let ControlFlow::Break(()) = visit(&shape) {
                return Some(ControlFlow::Break(()));
            }
        }
        Some(ControlFlow::Continue(()))
    }

    /// `CollisionGetter.noBlockCollision(null, aabb)` negated, without moving full cubes.
    fn level_has_collisions(&self, level: mod_api::LevelRef, aabb: [f64; 6]) -> Option<bool> {
        let bounds = aabb_of(aabb)?;
        let mut scene = SceneCollisions {
            world: self.native_level_view_world(level)?,
            binding: self.collision,
            subject: None,
        };
        BlockCollisionScan::any_collision(
            &mut scene,
            CollisionContext::empty(false),
            bounds,
            false,
            usize::MAX,
        )
        .ok()
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeBotsCommonGameExt1<'__sg> for G {}

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
