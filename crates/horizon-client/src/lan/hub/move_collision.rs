//! The collision part of `ServerGamePacketListenerImpl.handlePlayerPositionChange`, through the
//! guest's collision exclusions (#3717): a mod's boxes in which the level's blocks don't stop the
//! player are the same boxes its client moves it through.
use super::{Guest, Hub, TickCtx};
use crate::collisions::SceneCollisions;
use horizon_core::geometry::{Aabb, ShapeOp, Vec3, VoxelShape};
use horizon_world::{
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld},
    collision_context::CollisionContext,
    collision_exclusions::Excluding,
    entity_obstruction::EntityIdentity,
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
};
use std::io;

impl Hub {
    /// The collision part of handlePlayerPositionChange: whether the move
    /// counts as moved wrongly (logged), and whether it is refused (the
    /// player is sent back where they were).
    pub(super) fn collide(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> (bool, bool, bool) {
        let g = &self.guests[i];
        // noPhysics (spectators) and sleepers move freely.
        let Some(binding) = ctx.collision else {
            return (false, false, false);
        };
        if g.spectator() || g.sleeping {
            return (false, false, false);
        }
        let mut scene = SceneCollisions {
            world: ctx.world,
            binding,
            subject: None,
        };
        // A Player-cause move through the move gate (#2414); the gate records the movement
        // unchanged and its delta is what the collision resolves.
        let gate = crate::move_gate::armed(
            &self.loop_context,
            g.entity_id as u32,
            "minecraft:player",
            horizon_world::entity_move::MoverType::Player,
            Vec3::new(
                to[0] - g.position[0],
                to[1] - g.position[1],
                to[2] - g.position[2],
            ),
        );
        let movement = gate.delta();
        // A move with no exclusions pays this one check.
        if g.collision_exclusions.is_empty() {
            return check_move(g, to, movement, &mut scene);
        }
        let boxes = g.collision_exclusions.boxes();
        check_move(
            g,
            to,
            movement,
            &mut Excluding {
                world: &mut scene,
                boxes,
            },
        )
    }
}

/// [`Hub::collide`] against `world`'s blocks.
fn check_move<W>(g: &Guest, to: [f64; 3], movement: Vec3, world: &mut W) -> (bool, bool, bool)
where
    W: BlockCollisionWorld<Error = io::Error> + EntityMovementWorld<Error = io::Error>,
{
    // getBoundingBox: the box of the body's pose (swimming and crawling
    // are 0.6 tall), so a swimmer under a ceiling isn't sent back.
    let d = g.dimensions();
    let (half, height) = (f64::from(d.width) / 2.0, f64::from(d.height));
    let at = |p: [f64; 3]| {
        Aabb::new(
            Vec3::new(p[0] - half, p[1], p[2] - half),
            Vec3::new(p[0] + half, p[1] + height, p[2] + half),
        )
    };
    let (old, new) = (at(g.position), at(to));
    // player.move, then how far from the target it ended (vanilla drops
    // the vertical part: `yDist > -0.5 || yDist < 0.5` always holds).
    // The mover's own sneaking, as CollisionContext.of(player): a sneaking
    // player descends through scaffolding.
    let context = CollisionContext::position(g.position[1]).with_descending(g.sneaking);
    let done = resolve_world_movement(
        world,
        MovementRequest {
            entity: EntityIdentity(0),
            context,
            bounds: old,
            movement,
            max_step: 0.6,
            on_ground: g.on_ground,
            candidate_budget: 1_000_000,
        },
    )
    .ok();
    let fail = !g.creative()
        && done.is_some_and(|done| {
            let (dx, dz) = (movement.x - done.x, movement.z - done.z);
            dx * dx + dz * dz > 0.0625
        });
    // Entity.move: verticalCollisionBelow, the player "stands on something".
    let stands = done.is_some_and(|done| movement.y < 0.0 && done.y != movement.y);
    if fail {
        let unobstructed = no_world_collision(
            world,
            CollisionCheckRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(old.min_position().y),
                bounds: old,
                candidate_budget: 1_000_000,
            },
        );
        if unobstructed.unwrap_or(true) {
            return (true, true, stands);
        }
    }
    // isEntityCollidingWithAnythingNew: a block the old box did not
    // already touch.
    let shrink = f64::from(1.0E-5_f32);
    let shrink = Vec3::new(shrink, shrink, shrink);
    let Ok(old_shape) = VoxelShape::from_box(old.deflate(shrink)) else {
        return (fail, false, stands);
    };
    let Ok(scan) = BlockCollisionScan::new(world, context, new.deflate(shrink), false, 1 << 20)
    else {
        return (fail, false, stands);
    };
    for hit in scan {
        let Ok(hit) = hit else { break };
        if !hit
            .shape
            .join_is_not_empty(&old_shape, ShapeOp::AND)
            .unwrap_or(true)
        {
            return (fail, true, stands);
        }
    }
    (fail, false, stands)
}
