//! `LocalPlayer.moveTowardsClosestSpace`: the push out of a wall the player's body is in.
use sunrise_core::geometry::{Aabb, Axis, ShapeOp, Vec3, VoxelShape};
use sunrise_core::position::BlockPos;
use sunrise_level::{collisions::SceneCollisions, world::World};
use sunrise_sim::motion::PlayerMotion;
use sunrise_world::{block_collision::BlockCollisionRegistry, suffocation::SuffocationRegistry};

/// `LocalPlayer.moveTowardsClosestSpace` at the body's four corners (aiStep, unless a spectator or a
/// passenger): a corner inside a suffocating block sets a 0.1 speed towards the nearest side that is
/// free, on that side's axis; a later corner overwrites an earlier one. `suffocatesAt` is
/// `collidesWithSuffocatingBlock` over the corner's column at the body's height: a block whose
/// `isSuffocating` holds (a closed shulker box, ice, soul sand...) and whose collision shape meets
/// it, outside the player's block collision exclusions (#3717).
pub fn closest_space_nudge(
    p: &PlayerMotion,
    world: &World,
    collision: &BlockCollisionRegistry<'_>,
    suffocation: &SuffocationRegistry<'_>,
) -> [Option<f64>; 2] {
    let mut out = [None; 2];
    let body = p.bounds();
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    let mut suffocates_at = |x: i32, z: i32| {
        // `suffocatesAt`'s test area, deflated by 1e-7; the cells just above and below reach it
        // only with a shape taller than its cell (a fence, a wall).
        let (lo, hi) = (body.min(Axis::Y) + 1.0e-7, body.max(Axis::Y) - 1.0e-7);
        let area = Aabb::new(
            Vec3::new(f64::from(x) + 1.0e-7, lo, f64::from(z) + 1.0e-7),
            Vec3::new(f64::from(x + 1) - 1.0e-7, hi, f64::from(z + 1) - 1.0e-7),
        );
        let Ok(area_shape) = VoxelShape::from_box(area) else {
            return false;
        };
        ((lo.floor() as i32 - 1)..=(hi.floor() as i32 + 1)).any(|y| {
            let at = [x, y, z];
            if p.collision_exclusions.spares(BlockPos::new(x, y, z)) {
                return false;
            }
            let state = world.block(x, y, z);
            suffocation
                .is_suffocating(&mut scene, state, at)
                .unwrap_or(false)
                && collision
                    .collision_shape(&mut scene, state, at)
                    .ok()
                    .is_some_and(|shape| {
                        shape
                            .moved(Vec3::new(f64::from(x), f64::from(y), f64::from(z)))
                            .join_is_not_empty(&area_shape, ShapeOp::AND)
                            .unwrap_or(false)
                    })
        })
    };
    let side = f64::from(p.dimensions.width) * 0.35;
    let (px, pz) = (p.position.x, p.position.z);
    for (x, z) in [
        (px - side, pz + side),
        (px - side, pz - side),
        (px + side, pz - side),
        (px + side, pz + side),
    ] {
        let (bx, bz) = (x.floor() as i32, z.floor() as i32);
        if !suffocates_at(bx, bz) {
            continue;
        }
        let (xd, zd) = (x - f64::from(bx), z - f64::from(bz));
        // WEST, EAST, NORTH, SOUTH: (axis, step, distance to that edge).
        let mut closest = 1.797_693_134_862_315_7e308; // Double.MAX_VALUE
        let mut chosen = None;
        for (axis, step, distance) in [(0, -1, xd), (0, 1, 1.0 - xd), (1, -1, zd), (1, 1, 1.0 - zd)]
        {
            let (nx, nz) = if axis == 0 {
                (bx + step, bz)
            } else {
                (bx, bz + step)
            };
            if distance < closest && !suffocates_at(nx, nz) {
                closest = distance;
                chosen = Some((axis, step));
            }
        }
        if let Some((axis, step)) = chosen {
            out[axis] = Some(0.1 * f64::from(step));
        }
    }
    out
}
