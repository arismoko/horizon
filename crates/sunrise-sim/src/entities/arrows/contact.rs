// Included in arrows.rs to retain its private scope.
/// `AbstractArrow.tick`'s block phase for a thrown trident: the zero-width
/// `level().clip(new ClipContext(position(), position().add(movement),
/// ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, this))`, then
/// `AbstractArrow.onHitBlock`'s `position().subtract(signum * 0.05)`. Returns
/// the position to stop at (pull-back included), the block that was hit and
/// the normal of the face it was hit on.
pub fn clip_block_stop(
    world: &World,
    facts: &BlockFacts,
    collision: &BlockCollisionRegistry<'_>,
    from: Vec3,
    movement: Vec3,
) -> Option<(Vec3, BlockPos, [i32; 3])> {
    let (location, block, normal) = clip_block_hit(world, facts, collision, from, movement)?;
    Some((back_off(location, movement), block, normal))
}
/// The clip of `clip_block_stop` before the back-off: the hit location (what a
/// `BlockHitResult` carries), the block and the normal of the face.
pub fn clip_block_hit(
    world: &World,
    facts: &BlockFacts,
    collision: &BlockCollisionRegistry<'_>,
    from: Vec3,
    movement: Vec3,
) -> Option<(Vec3, BlockPos, [i32; 3])> {
    let to = Vec3::new(
        from.x + movement.x,
        from.y + movement.y,
        from.z + movement.z,
    );
    let mut scene = SceneCollisions {
        world,
        binding: collision,
        subject: None,
    };
    // CollisionContext.of(projectile): scaffolding is open from below.
    let context = sunrise_world::collision_context::CollisionContext::position(from.y);
    let hit = sunrise_core::geometry::traverse_blocks(from, to, 1024, |p: BlockPos| {
        let state = world.block(p.x, p.y, p.z);
        if facts.is_air(state) {
            return None;
        }
        let shape = collision
            .collision_shape_with_context(&mut scene, state, p, &context)
            .ok()?;
        shape.clip(from, to, p)
    })
    .ok()
    .flatten()?;
    Some((hit.location, hit.block, hit.direction.normal()))
}
/// `AbstractArrow.onHitBlock`'s back-off: vanilla scales `(double) 0.05f` by
/// `Math.signum` per axis, whose value at zero is zero (not Rust's -1.0).
pub fn back_off(location: Vec3, movement: Vec3) -> Vec3 {
    fn signum(v: f64) -> f64 {
        if v > 0.0 {
            1.0
        } else if v < 0.0 {
            -1.0
        } else {
            0.0
        }
    }
    let offset = f64::from(0.05f32);
    Vec3::new(
        location.x - signum(movement.x) * offset,
        location.y - signum(movement.y) * offset,
        location.z - signum(movement.z) * offset,
    )
}
fn bounds(at: Vec3) -> Aabb {
    // EntityDimensions.makeBoundingBox: half the float width.
    let w = f64::from(WIDTH / 2.0);
    Aabb::new(
        Vec3::new(at.x - w, at.y, at.z - w),
        Vec3::new(at.x + w, at.y + f64::from(HEIGHT), at.z + w),
    )
}
/// The bounding box of an arrow, or a thrown trident, at `at`.
pub fn arrow_bounds(at: Vec3) -> Aabb {
    bounds(at)
}
/// The blocks an arrow that moved from `from` to `to` was inside
/// (`Entity.checkInsideBlocks`: the sweep of its box, 16 steps at most), for
/// the blocks that react to it (`entityInside`).
pub fn blocks_inside(from: Vec3, to: Vec3) -> Vec<BlockPos> {
    let mut cells = ArrowCells(Vec::new());
    let _ = crate::block_contact::sweep_inside_blocks(
        &mut cells,
        from,
        to,
        &mut crate::fast_hash::FxHashSet::default(),
        &mut NoSteps,
        16,
    );
    cells.0
}
/// A collector that queues nothing: the cells alone are asked for.
struct NoSteps;
impl crate::block_contact::StepCollector for NoSteps {
    fn advance_step(&mut self, _: i32) {}
}
/// The shared walk over every cell an arrow's box sweeps, air included, each
/// the whole block, for the blocks that react to it.
struct ArrowCells(Vec<BlockPos>);
impl crate::block_contact::InsideWalk for ArrowCells {
    type Effects = NoSteps;
    type Error = std::convert::Infallible;
    const DEFLATE: f64 = 1.0e-5_f32 as f64;
    fn bounds_at(&self, at: Vec3) -> Aabb {
        bounds(at)
    }
    fn is_alive(&self) -> bool {
        true
    }
    fn block(&self, _: BlockPos) -> BlockStateId {
        BlockStateId(0)
    }
    fn is_air(&self, _: BlockStateId) -> bool {
        // Every swept cell is reported, whatever is in it.
        false
    }
    fn inside_shape(&mut self, _: BlockStateId, _: BlockPos) -> crate::block_contact::InsideShape {
        crate::block_contact::InsideShape::Full
    }
    fn entity_inside(
        &mut self,
        at: BlockPos,
        _: BlockStateId,
        _: i32,
        _: bool,
        _: &mut NoSteps,
    ) -> Result<(), std::convert::Infallible> {
        self.0.push(at);
        Ok(())
    }
}
/// `Projectile.lerpRotation`.
pub fn lerp_rotation(mut old: f32, new: f32) -> f32 {
    while new - old < -180.0 {
        old -= 360.0;
    }
    while new - old >= 180.0 {
        old += 360.0;
    }
    old + 0.2 * (new - old)
}
/// `(float)(Mth.atan2(y, x) * 180.0F / (float)Math.PI)`: compiled as one
/// multiplication by the folded float constant.
fn degrees(y: f64, x: f64) -> f32 {
    sunrise_core::geometry::trig::atan2_degrees(y, x)
}
fn signum(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}
