//! Synchronous mod-block contacts from the server's simulation code.
use crate::health::DamageLevel;
use std::{
    cell::{Cell, RefCell},
    sync::OnceLock,
};
use sunrise_core::{
    geometry::{Aabb, Axis, Direction, SweepCursor, Vec3},
    position::BlockPos,
};
use sunrise_world::{level::LandingWrite, registry::BlockStateId};

/// `getEntityInsideCollisionShape` as the shared walk sees it: the whole block, boxes
/// to sweep the move against, or nothing. This is the one point #1011 adds the Part's
/// own shape at.
#[derive(Clone, Debug)]
pub enum InsideShape {
    /// `Shapes.block()`: every move through the block is inside it.
    Full,
    /// The shape's boxes, in world space at the block, swept along the move.
    Boxes(Vec<sunrise_core::geometry::Aabb>),
    /// An empty shape: nothing is inside it.
    Empty,
    /// A shape every state of the block shares, in block space: each box is moved to
    /// the block and swept in turn, with nothing collected.
    Fixed(&'static sunrise_core::geometry::VoxelShape),
}

/// `AbstractCauldronBlock.SHAPE`: the cauldron's hollow shell, shared across
/// lava, water, and powder-snow cauldrons.
fn cauldron_shell_shape() -> &'static sunrise_core::geometry::VoxelShape {
    use sunrise_core::geometry::{ShapeOp, VoxelShape};
    static SHELL: std::sync::OnceLock<VoxelShape> = std::sync::OnceLock::new();
    SHELL.get_or_init(|| {
        let cuboid = |x0, y0, z0, x1, y1, z1| {
            VoxelShape::from_box(Aabb::new(
                Vec3::new(x0 / 16.0, y0 / 16.0, z0 / 16.0),
                Vec3::new(x1 / 16.0, y1 / 16.0, z1 / 16.0),
            ))
            .expect("valid cauldron box")
        };
        let mut solid = VoxelShape::block();
        // AbstractCauldronBlock.SHAPE: the openings under its feet
        // and its hollow centre are cut out of the unit block.
        for hole in [
            cuboid(0.0, 0.0, 4.0, 16.0, 3.0, 12.0),
            cuboid(4.0, 0.0, 0.0, 12.0, 3.0, 16.0),
            cuboid(2.0, 0.0, 2.0, 14.0, 3.0, 14.0),
            cuboid(2.0, 4.0, 2.0, 14.0, 16.0, 14.0),
        ] {
            solid = solid
                .join(&hole, ShapeOp::ONLY_FIRST)
                .expect("valid cauldron cutout");
        }
        solid
    })
}

fn cauldron_filled_shape_for_top(top: f64) -> sunrise_core::geometry::VoxelShape {
    use sunrise_core::geometry::{ShapeOp, VoxelShape};
    let fill = VoxelShape::from_box(Aabb::new(
        Vec3::new(2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0),
        Vec3::new(14.0 / 16.0, top / 16.0, 14.0 / 16.0),
    ))
    .expect("valid cauldron fill");
    cauldron_shell_shape()
        .join(&fill, ShapeOp::OR)
        .expect("valid filled cauldron")
}

/// `LavaCauldronBlock.FILLED_SHAPE`: `Shapes.or(AbstractCauldronBlock.SHAPE,
/// Block.column(12, 4, 15))`, the cauldron's solid shape plus its lava. It is the
/// block's `getEntityInsideCollisionShape` whatever the collision tables hold.
pub(crate) fn lava_cauldron_filled_shape() -> &'static sunrise_core::geometry::VoxelShape {
    static SHAPE: std::sync::OnceLock<sunrise_core::geometry::VoxelShape> =
        std::sync::OnceLock::new();
    SHAPE.get_or_init(|| cauldron_filled_shape_for_top(15.0))
}

/// `LayeredCauldronBlock.FILLEDSLOT_SHAPES`: precomputed fixed shapes for levels 1..=3
/// (`top = 6 + 3 * level`, i.e. 9, 12, 15), avoiding per-contact `Vec<Aabb>` allocation.
fn layered_cauldron_filled_shape(level: u8) -> Option<&'static sunrise_core::geometry::VoxelShape> {
    static SHAPES: std::sync::OnceLock<[sunrise_core::geometry::VoxelShape; 3]> =
        std::sync::OnceLock::new();
    let idx = usize::from(level.checked_sub(1)?);
    SHAPES
        .get_or_init(|| {
            [
                cauldron_filled_shape_for_top(9.0),
                cauldron_filled_shape_for_top(12.0),
                cauldron_filled_shape_for_top(15.0),
            ]
        })
        .get(idx)
}

/// The filled inside-collision shape of a lava or layered cauldron: its shell
/// and the contents up to that level's surface (vanilla `FILLED_SHAPE(S)`).
pub fn cauldron_inside_shape(
    collision: &sunrise_world::block_collision::BlockCollisionRegistry<'_>,
    state: BlockStateId,
    definition: &sunrise_world::chunk::BlockState,
    pos: BlockPos,
) -> InsideShape {
    let top = match definition.name.as_str() {
        // `LavaCauldronBlock.getEntityInsideCollisionShape`: its one fixed shape.
        "minecraft:lava_cauldron" => return InsideShape::Fixed(lava_cauldron_filled_shape()),
        "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
            let Some(level) = definition
                .properties
                .get("level")
                .and_then(|value| value.parse::<u8>().ok())
            else {
                return InsideShape::Empty;
            };
            if let Some(fixed) = layered_cauldron_filled_shape(level) {
                return InsideShape::Fixed(fixed);
            }
            6.0 + 3.0 * f64::from(level)
        }
        _ => return InsideShape::Full,
    };
    let offset = Vec3::new(f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
    let shell = collision.static_shape(state).ok().flatten();
    let mut boxes: Vec<_> = shell
        .into_iter()
        .flat_map(|shape| shape.boxes())
        .map(|bounds| bounds.move_by(offset))
        .collect();
    boxes.push(Aabb::new(
        Vec3::new(2.0 / 16.0, 4.0 / 16.0, 2.0 / 16.0).add_vector(offset),
        Vec3::new(14.0 / 16.0, top / 16.0, 14.0 / 16.0).add_vector(offset),
    ));
    InsideShape::Boxes(boxes)
}

/// The fire, cauldron and powder snow effects that run in vanilla's ordered
/// collector, rather than immediately inside `Block.entityInside`.
pub fn queue_block_effects(
    effects: &mut mod_api::InsideEffects,
    block: &str,
    state: BlockStateId,
    pos: BlockPos,
) {
    use mod_api::{__private::BuiltinInsideAction as Action, InsideEffect as Effect};
    match block {
        "minecraft:fire" | "minecraft:soul_fire" => {
            effects.apply(Effect::ClearFreeze);
            effects.apply(Effect::FireIgnite);
            let damage = if block == "minecraft:fire" { 1.0 } else { 2.0 };
            effects.run_after_builtin(Effect::FireIgnite, Action::FireHurt(damage));
        }
        "minecraft:lava_cauldron" => {
            queue_fluid_effects(effects, sunrise_world::fluid::FluidKind::Lava)
        }
        "minecraft:water_cauldron" | "minecraft:powder_snow_cauldron" => {
            effects.run_before_builtin(
                Effect::Extinguish,
                Action::LayeredCauldronExtinguish {
                    pos: [pos.x, pos.y, pos.z],
                    state: state.0,
                },
            );
            effects.apply(Effect::Extinguish);
        }
        "minecraft:powder_snow" => {
            effects.run_before_builtin(
                Effect::Extinguish,
                Action::PowderSnowMelt([pos.x, pos.y, pos.z]),
            );
            effects.apply(Effect::Freeze);
            effects.apply(Effect::Extinguish);
        }
        _ => {}
    }
}

/// `LavaFluid.entityInside` and `WaterFluid.entityInside` at the fluid's
/// position in the walk. Lava shares its slots with a filled lava cauldron.
pub fn queue_fluid_effects(
    effects: &mut mod_api::InsideEffects,
    kind: sunrise_world::fluid::FluidKind,
) {
    use mod_api::{__private::BuiltinInsideAction as Action, InsideEffect as Effect};
    use sunrise_world::fluid::FluidKind;
    match kind {
        FluidKind::Water | FluidKind::FlowingWater => effects.apply(Effect::Extinguish),
        FluidKind::Lava | FluidKind::FlowingLava => {
            effects.apply(Effect::ClearFreeze);
            effects.apply(Effect::LavaIgnite);
            effects.run_after_builtin(Effect::LavaIgnite, Action::LavaHurt);
        }
        _ => {}
    }
}

/// The collector a walk queues its per-step effects into. `advanceStep` is the only
/// call the shared walk makes on it; the kind's own moments queue the effects.
pub trait StepCollector {
    /// `InsideBlockEffectApplier.StepBasedCollector.advanceStep`.
    fn advance_step(&mut self, step: i32);
}

impl StepCollector for mod_api::InsideEffects {
    fn advance_step(&mut self, step: i32) {
        mod_api::InsideEffects::advance_step(self, step);
    }
}

/// One entity kind's view of `Entity.checkInsideBlocks`: what the walk needs from the
/// kind, and the kind's own `entityInside` moments, engine blocks and fluid, which run
/// mid-walk where vanilla runs them and queue into the shared collector.
pub trait InsideWalk {
    type Error;
    /// The collector the kind's moments queue into.
    type Effects: StepCollector;
    /// The epsilon `makeBoundingBox(to).deflate(..)` shrinks the target box by before
    /// the cells are enumerated. Vanilla's is `1.0E-5F`; a kind that always widened it
    /// from `f32` keeps that, the others keep the `f64` they always used.
    const DEFLATE: f64 = 1.0e-5;
    /// `makeBoundingBox(at)`: the entity's box at a position.
    fn bounds_at(&self, at: sunrise_core::geometry::Vec3) -> sunrise_core::geometry::Aabb;
    /// `isAlive`: the walk stops the moment the entity dies.
    fn is_alive(&self) -> bool;
    /// `level.getBlockState(pos)`.
    fn block(&self, at: BlockPos) -> BlockStateId;
    /// `state.isAir()`, read as the sweep visits.
    fn is_air(&self, state: BlockStateId) -> bool;
    /// `state.getEntityInsideCollisionShape(level, pos, this)` for the walk.
    fn inside_shape(&mut self, state: BlockStateId, at: BlockPos) -> InsideShape;
    /// `collidedWithFluid(state.getFluidState(), pos, from, to)`.
    fn collided_with_fluid(
        &self,
        _at: BlockPos,
        _from: sunrise_core::geometry::Vec3,
        _to: sunrise_core::geometry::Vec3,
    ) -> bool {
        false
    }
    /// `advanceStep` → `state.entityInside`, then `onInsideBlock`: the kind's own
    /// per-block effects and the Part's contact, queueing into `effects`.
    fn entity_inside(
        &mut self,
        at: BlockPos,
        state: BlockStateId,
        step: i32,
        precise: bool,
        effects: &mut Self::Effects,
    ) -> Result<(), Self::Error>;
    /// `advanceStep` → `fluidState.entityInside`.
    fn fluid_inside(
        &mut self,
        _at: BlockPos,
        _step: i32,
        _effects: &mut Self::Effects,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Which of a candidate cell's two shapes the move touches. Both are decided before
/// the cell is marked visited, as `Entity.checkInsideBlocks` decides them: a cell
/// neither touches stays unvisited, so a later segment can still reach it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contacted {
    /// The block's inside shape (`collidedWithShapeMovingFrom`, or the full block).
    pub block: bool,
    /// The fluid's shape (`collidedWithFluid`).
    pub fluid: bool,
}

impl Contacted {
    /// Whether either shape was touched: the cell is visited.
    pub fn any(self) -> bool {
        self.block || self.fluid
    }
}

/// The one contact decision every traversal makes for a non-air candidate `state` at
/// `at`, for the move from `from` to `to`: the block's inside shape, then the fluid's,
/// independently. A stationary pass gives `from == to`.
pub fn contact_at<W: InsideWalk>(
    walk: &mut W,
    state: BlockStateId,
    at: BlockPos,
    from: sunrise_core::geometry::Vec3,
    to: sunrise_core::geometry::Vec3,
) -> Contacted {
    let block = match walk.inside_shape(state, at) {
        InsideShape::Full => true,
        InsideShape::Empty => false,
        InsideShape::Boxes(boxes) => walk
            .bounds_at(from)
            .collided_along_vector(to.subtract(from), &boxes),
        InsideShape::Fixed(shape) => {
            let (bounds, offset) = (walk.bounds_at(from), Vec3::from_block(at));
            shape
                .boxes()
                .iter()
                .any(|b| bounds.collided_along_vector(to.subtract(from), &[b.move_by(offset)]))
        }
    };
    Contacted {
        block,
        fluid: walk.collided_with_fluid(at, from, to),
    }
}

/// `Entity.checkInsideBlocks(from, to, …)`: the blocks the box at `to` sweeps through
/// from `from`, in the walk's order, each visited once, stopping when the entity dies
/// or the iteration budget runs out. Returns the iterations used plus one. Air is
/// passed over; a block is inside when [`contact_at`] says its shape or its fluid is
/// touched, and the step advances before the block's `entityInside` and again before
/// the fluid's, as vanilla does.
pub fn sweep_inside_blocks<W: InsideWalk>(
    walk: &mut W,
    from: sunrise_core::geometry::Vec3,
    to: sunrise_core::geometry::Vec3,
    visited: &mut crate::fast_hash::FxHashSet<i64>,
    effects: &mut W::Effects,
    max_iterations: i32,
) -> Result<i32, W::Error> {
    let mut segment = SweepSegment::new(walk, from, to, max_iterations);
    segment.run(walk, visited, effects, |_| false)?;
    Ok(segment.iteration + 1)
}

#[derive(Debug)]
struct SweepSegment {
    from: Vec3,
    to: Vec3,
    deflated: Aabb,
    moved_far: bool,
    cells: Option<SweepCursor>,
    iteration: i32,
    max_iterations: i32,
}

impl SweepSegment {
    fn new<W: InsideWalk>(walk: &W, from: Vec3, to: Vec3, max_iterations: i32) -> Self {
        let epsilon = Vec3::new(W::DEFLATE, W::DEFLATE, W::DEFLATE);
        let deflated = walk.bounds_at(to).deflate(epsilon);
        // Mth.square(0.9999900000002526): a long move counts every block it crosses as precise.
        let far = 0.999_990_000_000_252_6_f64;
        Self {
            from,
            to,
            deflated,
            moved_far: from.distance_squared(to) > far * far,
            // As before, a geometry error ends the segment after its valid prefix.
            cells: SweepCursor::new(from, to, deflated, None).ok(),
            iteration: 0,
            max_iterations,
        }
    }

    fn run<W: InsideWalk>(
        &mut self,
        walk: &mut W,
        visited: &mut crate::fast_hash::FxHashSet<i64>,
        effects: &mut W::Effects,
        halt: impl Fn(&W) -> bool,
    ) -> Result<bool, W::Error> {
        while walk.is_alive() {
            let Some(Ok((p, step))) = self.cells.as_mut().and_then(Iterator::next) else {
                break;
            };
            if step >= self.max_iterations {
                break;
            }
            self.iteration = step;
            let state = walk.block(p);
            if walk.is_air(state) {
                continue;
            }
            let contact = contact_at(walk, state, p, self.from, self.to);
            if !contact.any() || !visited.insert(p.pack()) {
                continue;
            }
            if contact.block {
                let precise = self.moved_far || self.deflated.intersects(Aabb::block(p));
                effects.advance_step(step);
                walk.entity_inside(p, state, step, precise, effects)?;
            }
            if contact.fluid {
                effects.advance_step(step);
                walk.fluid_inside(p, step, effects)?;
            }
            if halt(walk) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// The cells a box covers, x then y then z, each floor to floor inclusive: the
/// stationary enumeration (`BlockPos.betweenClosed`) the box-cell passes share.
/// `each` returns whether to go on; the result is whether every cell was seen.
pub fn box_cells(
    area: sunrise_core::geometry::Aabb,
    mut each: impl FnMut(BlockPos) -> bool,
) -> bool {
    let (lo, hi) = (area.min_position(), area.max_position());
    for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
        for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
            for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                if !each(BlockPos::new(x, y, z)) {
                    return false;
                }
            }
        }
    }
    true
}

/// A stationary contact pass at `at` over the cells of `area` (the box at `at`,
/// already deflated), on the shared decision: each non-air cell [`contact_at`]
/// touches is visited once, its block moment at step 0 and rough, then its fluid
/// moment. No step advances: the pass shares the walk's first step, as the kind's
/// own pass always did.
pub fn visit_box_cells<W: InsideWalk>(
    walk: &mut W,
    area: sunrise_core::geometry::Aabb,
    at: sunrise_core::geometry::Vec3,
    visited: &mut crate::fast_hash::FxHashSet<i64>,
    effects: &mut W::Effects,
) -> Result<(), W::Error> {
    BoxCellSweep::new(area, at).run(walk, visited, effects, |_| false)?;
    Ok(())
}

/// The exact next stationary cell, so a hurt need not replay previous contacts.
#[derive(Debug)]
pub(crate) struct BoxCellSweep {
    lo: BlockPos,
    hi: BlockPos,
    at: Vec3,
    next: Option<BlockPos>,
}

impl BoxCellSweep {
    pub(crate) fn new(area: Aabb, at: Vec3) -> Self {
        let at_cell =
            |p: Vec3| BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
        let lo = at_cell(area.min_position());
        let hi = at_cell(area.max_position());
        let next = (lo.x <= hi.x && lo.y <= hi.y && lo.z <= hi.z).then_some(lo);
        Self { lo, hi, at, next }
    }

    pub(crate) fn run<W: InsideWalk>(
        &mut self,
        walk: &mut W,
        visited: &mut crate::fast_hash::FxHashSet<i64>,
        effects: &mut W::Effects,
        halt: impl Fn(&W) -> bool,
    ) -> Result<bool, W::Error> {
        while let Some(p) = self.next {
            self.next = if p.z < self.hi.z {
                Some(BlockPos::new(p.x, p.y, p.z + 1))
            } else if p.y < self.hi.y {
                Some(BlockPos::new(p.x, p.y + 1, self.lo.z))
            } else if p.x < self.hi.x {
                Some(BlockPos::new(p.x + 1, self.lo.y, self.lo.z))
            } else {
                None
            };
            let state = walk.block(p);
            if walk.is_air(state) {
                continue;
            }
            let contact = contact_at(walk, state, p, self.at, self.at);
            if !contact.any() || !visited.insert(p.pack()) {
                continue;
            }
            if contact.block {
                walk.entity_inside(p, state, 0, false, effects)?;
            }
            if contact.fluid {
                walk.fluid_inside(p, 0, effects)?;
            }
            if halt(walk) {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// `Entity.checkInsideBlocks(movements, collector)`: every movement of a tick, each
/// axis of a recorded original in `axisStepOrder` within its own 16-iteration budget,
/// then the target alone when the budget ran out, over one visited set, ending with
/// `applyAndClear`'s flush left to the kind's own tick.
pub fn sweep_movements<W: InsideWalk>(
    walk: &mut W,
    movements: &[sunrise_world::entity_move::MovementRecord],
    visited: &mut crate::fast_hash::FxHashSet<i64>,
    effects: &mut W::Effects,
) -> Result<(), W::Error> {
    MovementSweep::default().run(walk, movements, visited, effects, |_| false)?;
    Ok(())
}

/// Walk state is made of values, not borrows of the level or the moving entity.
#[derive(Debug, Default)]
pub(crate) struct MovementSweep {
    row: usize,
    axis: usize,
    from: Option<Vec3>,
    budget: i32,
    segment: Option<SweepSegment>,
}

impl MovementSweep {
    pub(crate) fn run<W: InsideWalk>(
        &mut self,
        walk: &mut W,
        movements: &[sunrise_world::entity_move::MovementRecord],
        visited: &mut crate::fast_hash::FxHashSet<i64>,
        effects: &mut W::Effects,
        halt: impl Fn(&W) -> bool,
    ) -> Result<bool, W::Error> {
        loop {
            if let Some(segment) = self.segment.as_mut() {
                if !segment.run(walk, visited, effects, &halt)? {
                    return Ok(false);
                }
                // The target-box fallback runs after advancing the row and spends no budget.
                if self.from.is_some() {
                    self.budget -= segment.iteration + 1;
                }
                self.segment = None;
            }
            let Some(movement) = movements.get(self.row) else {
                return Ok(true);
            };
            let mut from = match self.from {
                Some(from) => from,
                None => {
                    self.axis = 0;
                    self.budget = 16;
                    movement.from
                }
            };
            let delta = movement.to.subtract(movement.from);
            let to = match movement.original {
                Some(original) if delta.length_squared() > 0.0 => {
                    let axes = Axis::step_order(original);
                    let mut to = None;
                    while let Some(&axis) = axes.get(self.axis) {
                        self.axis += 1;
                        let step = delta.component(axis);
                        if step != 0.0 {
                            let direction = match axis {
                                Axis::X => Direction::East,
                                Axis::Y => Direction::Up,
                                Axis::Z => Direction::South,
                            };
                            to = Some(from.relative(direction, step));
                            break;
                        }
                    }
                    to
                }
                _ if self.axis == 0 => {
                    self.axis = 1;
                    from = movement.from;
                    Some(movement.to)
                }
                _ => None,
            };
            if let Some(to) = to {
                self.segment = Some(SweepSegment::new(walk, from, to, self.budget));
                self.from = Some(to);
            } else {
                if self.budget <= 0 {
                    self.segment = Some(SweepSegment::new(walk, movement.to, movement.to, 1));
                }
                self.row += 1;
                self.from = None;
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Contact<'a> {
    pub entity: i32,
    pub kind: &'a str,
    pub state: BlockStateId,
    pub position: BlockPos,
}

/// The body and exact retained contact used by `Block.stepOn`.
#[derive(Clone, Copy)]
pub struct StepContact<'a> {
    pub contact: Contact<'a>,
    pub carefully: bool,
    pub bounds: sunrise_core::geometry::Aabb,
}

pub enum StepLevel<'a, 'world> {
    Server(&'a mut DamageLevel<'world>),
    Client(crate::motion::FrictionLevel<'a>),
}

/// Where the contacts are answered: the game binds these to the hooks of the block
/// parts, and says which block states have a part for each.
pub struct Hooks {
    pub step_listens: fn(BlockStateId) -> bool,
    /// `Block.stepOn` for the block's subscribed Parts; returns only checked outputs.
    pub step_on: fn(StepLevel<'_, '_>, StepContact<'_>, &mut [f64; 3]) -> Vec<mod_api::EntityWrite>,
    pub any_inside: fn() -> bool,
    pub inside_listens: fn(BlockStateId) -> bool,
    pub pass_listens: fn(BlockStateId) -> bool,
    pub fall_listens: fn(BlockStateId) -> bool,
    pub bounce_listens: fn(BlockStateId) -> bool,
    /// `Block.fallOn`: change the distance and multiplier before calculating damage.
    pub fall_on:
        fn(&str, &mut DamageLevel<'_>, Contact<'_>, &mut f64, &mut f32, &mut Vec<LandingWrite>),
    /// `Block.bounceOn`: change `movement`, the movement the bounce leaves the entity.
    pub bounce_on:
        fn(&str, &mut DamageLevel<'_>, Contact<'_>, f64, &mut sunrise_core::geometry::Vec3),
    /// `Block.entityInside` told mid-walk: generated dispatch isolates each
    /// part into a staged collector and absorbs it on return.
    pub inside_fire: fn(
        &mut DamageLevel<'_>,
        &'static str,
        i32,
        &str,
        BlockStateId,
        BlockPos,
        bool,
        &mut mod_api::InsideEffects,
    ),
    /// Convert an engine block state id to the mod API's `BlockState`.
    pub state_to_api: fn(BlockStateId) -> Option<mod_api::BlockState>,
    /// Convert a mod API `BlockState` to an engine block state id.
    pub state_to_engine: fn(mod_api::BlockState) -> Option<BlockStateId>,
}

pub static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// Convert an engine block state id to a mod API `BlockState` through the installed hooks.
pub fn state_to_api(id: BlockStateId) -> Option<mod_api::BlockState> {
    HOOKS.get().and_then(|hooks| (hooks.state_to_api)(id))
}

/// Convert a mod API `BlockState` to an engine block state id through the installed hooks.
pub fn state_to_engine(state: mod_api::BlockState) -> Option<BlockStateId> {
    HOOKS.get().and_then(|hooks| (hooks.state_to_engine)(state))
}

/// Where `IBlockExtension.getBounceRestitution` is answered, on both sides: the game says
/// which block states have a part for it and binds the part's answer.
pub struct RestitutionHooks {
    pub listens: fn(BlockStateId) -> bool,
    /// Change `factor`, the restitution of `Contact`'s block, for the entity landing on it.
    pub restitution: fn(crate::motion::FrictionLevel<'_>, Contact<'_>, &mut f32),
}

pub static RESTITUTION_HOOKS: OnceLock<RestitutionHooks> = OnceLock::new();

/// What a contact asked to keep until the entity's batch ends, delivered in
/// the order it happened: landing writes (`Block.fallOn`) and minecart passes.
/// `entity_inside` callbacks now apply their level writes directly at flush.
#[derive(Clone, Debug, PartialEq)]
pub enum Pending {
    Landed(Landed),
    MinecartPass(MinecartPass),
}

/// An entity that landed on a block whose Part asked for writes (`Block.fallOn`).
#[derive(Clone, Debug, PartialEq)]
pub struct Landed {
    pub state: BlockStateId,
    pub position: BlockPos,
    pub writes: Vec<LandingWrite>,
}

/// A minecart on a rail whose Part listens to `minecart_pass`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MinecartPass {
    pub entity: i32,
    pub state: BlockStateId,
    pub position: BlockPos,
}

thread_local! {
    static LEVEL: Cell<Option<&'static str>> = const { Cell::new(None) };
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
}

/// Scope authority to this thread and restore it even on an error or panic.
/// Nested guest actions may temporarily enter a different dimension. Only the tick's own
/// thread enters a level (`tick`), so a level never changes under a tick phase.
pub fn in_level<R>(
    _tick: crate::parallel::TickThread,
    dimension: Option<&'static str>,
    f: impl FnOnce() -> R,
) -> R {
    struct Restore(Option<&'static str>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEVEL.set(self.0);
        }
    }
    let _restore = Restore(LEVEL.replace(dimension));
    f()
}

/// The level this thread's contacts answer in. A tick phase's work is refused one, by name:
/// the level belongs to the tick's thread, and a phase that needs it reads it through its view.
fn tick_level(read: &'static str) -> Option<&'static str> {
    if sunrise_core::tick_phase::in_tick_phase() {
        static REFUSED: std::sync::Once = std::sync::Once::new();
        REFUSED.call_once(|| {
            log::error!(
                "block contact `{read}` refused inside a tick phase's work: the current level belongs to the tick's thread"
            );
        });
        debug_assert!(false, "block contact `{read}` read inside a tick phase");
        return None;
    }
    LEVEL.get()
}

/// Prefer checking this before lending the level, so an unmodded step doesn't
/// borrow the random stream or prepare a host.
pub fn step_listens(state: BlockStateId) -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.step_listens)(state))
}

/// `Block.stepOn`: give the contacted state the simulating body's current movement.
/// The body consumes the entity writes before its inside scan, never at the batch drain.
pub fn step_on(
    level: StepLevel<'_, '_>,
    step: StepContact<'_>,
    movement: &mut sunrise_core::geometry::Vec3,
) -> Vec<mod_api::EntityWrite> {
    let Some(hooks) = HOOKS.get() else {
        return Vec::new();
    };
    let mut answer = movement.components();
    let writes = (hooks.step_on)(level, step, &mut answer);
    let [x, y, z] = answer;
    *movement = sunrise_core::geometry::Vec3::new(x, y, z);
    writes
}

/// The dimension the current thread's contacts answer in, as `inside` and the
/// landing routes read it: the server's tick set it around the entity passes.
pub fn current_level() -> Option<&'static str> {
    tick_level("current_level")
}

/// Whether a part of the block `state` answers `fall_on`.
pub fn fall_listens(state: BlockStateId) -> bool {
    HOOKS.get().is_some_and(|hooks| (hooks.fall_listens)(state))
}

/// `Block.fallOn`: the fall inputs after the block Part's answer. Invalid numeric answers are ignored.
/// Requested writes remain queued until the entity batch ends.
pub fn fall(
    level: &mut DamageLevel<'_>,
    contact: Contact<'_>,
    distance: f64,
    multiplier: f32,
) -> (f64, f32) {
    let (Some(dimension), Some(hooks)) = (tick_level("fall"), HOOKS.get()) else {
        return (distance, multiplier);
    };
    if !(hooks.fall_listens)(contact.state) {
        return (distance, multiplier);
    }
    let mut answer_distance = distance;
    let mut answer = multiplier;
    let mut writes = Vec::new();
    (hooks.fall_on)(
        dimension,
        level,
        contact,
        &mut answer_distance,
        &mut answer,
        &mut writes,
    );
    landed(contact.state, contact.position, writes);
    (
        if answer_distance.is_finite() {
            answer_distance
        } else {
            distance
        },
        accepted_damage(answer, multiplier),
    )
}

/// A part's multiplier answer: negative is zero, and a non-finite one is ignored.
fn accepted_damage(answer: f32, kept: f32) -> f32 {
    if answer.is_finite() {
        answer.max(0.0)
    } else {
        kept
    }
}

/// A contact Part's movement answer, ignored unless every component is finite.
fn accepted_movement(
    answer: sunrise_core::geometry::Vec3,
    kept: sunrise_core::geometry::Vec3,
) -> sunrise_core::geometry::Vec3 {
    if answer.x.is_finite() && answer.y.is_finite() && answer.z.is_finite() {
        answer
    } else {
        kept
    }
}

/// `Block.bounceOn`: the movement a bounce off `contact`'s block leaves, after any part of
/// that block changed it. A non-finite answer is ignored.
pub fn bounce(
    level: &mut DamageLevel<'_>,
    contact: Contact<'_>,
    fall_distance: f64,
    movement: sunrise_core::geometry::Vec3,
) -> sunrise_core::geometry::Vec3 {
    let (Some(dimension), Some(hooks)) = (tick_level("bounce"), HOOKS.get()) else {
        return movement;
    };
    if !(hooks.bounce_listens)(contact.state) {
        return movement;
    }
    let mut answer = movement;
    (hooks.bounce_on)(dimension, level, contact, fall_distance, &mut answer);
    accepted_movement(answer, movement)
}

/// `Entity.getBlockBounciness`' `Block.getBounceRestitution` for the entity landing on
/// `contact`'s block, whose own restitution is `own`, after any part of that block changed it
/// (`IBlockExtension.getBounceRestitution`) in `level`, on either side. A non-finite answer is
/// ignored: it would make the entity's velocity not a number.
pub fn bounce_restitution(
    level: crate::motion::FrictionLevel<'_>,
    contact: Contact<'_>,
    own: f32,
) -> f32 {
    let Some(hooks) = RESTITUTION_HOOKS.get() else {
        return own;
    };
    if !(hooks.listens)(contact.state) {
        return own;
    }
    let mut answer = own;
    (hooks.restitution)(level, contact, &mut answer);
    if answer.is_finite() { answer } else { own }
}

/// Keep the writes a landing on the block `state` at `position` asked for until the
/// entity's batch ends; nothing is kept when none were asked.
pub fn landed(state: BlockStateId, position: BlockPos, writes: Vec<LandingWrite>) {
    if !writes.is_empty() {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::Landed(Landed {
                state,
                position,
                writes,
            }));
        });
    }
}

/// Note a minecart on the rail `state` at `position`, if a part there listens; a world
/// without such a part pays one test per rail step.
pub fn minecart_pass(entity: i32, state: BlockStateId, position: BlockPos) {
    if tick_level("minecart_pass").is_some()
        && HOOKS.get().is_some_and(|hooks| (hooks.pass_listens)(state))
    {
        PENDING.with_borrow_mut(|pending| {
            pending.push(Pending::MinecartPass(MinecartPass {
                entity,
                state,
                position,
            }));
        });
    }
}

/// The contacts noted since the last call, in the order they happened.
pub fn take_pending() -> Vec<Pending> {
    PENDING.with_borrow_mut(std::mem::take)
}

/// `Entity.checkInsideBlocks`' `isPrecise` for the block at `position`: the
/// move from `from` to `to` was long, or the box at `to` touches the block.
pub fn is_precise(
    from: sunrise_core::geometry::Vec3,
    to: sunrise_core::geometry::Vec3,
    at_target: sunrise_core::geometry::Aabb,
    position: BlockPos,
) -> bool {
    let far = 0.999_990_000_000_252_6_f64;
    from.distance_squared(to) > far * far
        || at_target.intersects(sunrise_core::geometry::Aabb::block(position))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// `checkInsideBlocks`' `isPrecise`: a long move, or the box at the target
    /// touching the block.
    #[test]
    fn precision_follows_the_move_length_and_the_box() {
        use sunrise_core::geometry::{Aabb, Vec3};
        let at = Aabb::new(Vec3::new(2.25, 2.0, 3.25), Vec3::new(2.75, 2.5, 3.75));
        let block = BlockPos::new(2, 2, 3);
        let from = Vec3::new(2.5, 2.0, 3.5);
        assert!(is_precise(from, from, at, block));
        assert!(!is_precise(from, from, at, BlockPos::new(1, 2, 3)));
        assert!(is_precise(
            Vec3::new(0.5, 2.0, 3.5),
            from,
            at,
            BlockPos::new(1, 2, 3)
        ));
    }

    /// A fall answer is clamped at zero and a non-finite one ignored; a bounce answer is
    /// used unless one component is non-finite.
    #[test]
    fn landing_answers_are_checked() {
        use sunrise_core::geometry::Vec3;
        assert_eq!(accepted_damage(-2.0, 3.0), 0.0);
        assert_eq!(accepted_damage(5.0, 3.0), 5.0);
        assert_eq!(accepted_damage(f32::NAN, 3.0), 3.0);
        let kept = Vec3::new(0.0, 0.8, 0.0);
        assert_eq!(
            accepted_movement(Vec3::new(0.0, 0.4, 0.0), kept),
            Vec3::new(0.0, 0.4, 0.0)
        );
        assert_eq!(
            accepted_movement(Vec3::new(0.0, f64::INFINITY, 0.0), kept),
            kept
        );
    }

    /// A walk over a named grid: what it visited, in the walk's order, block moments
    /// and fluid moments apart.
    #[derive(Default)]
    struct Grid {
        blocks: std::collections::HashMap<[i32; 3], (BlockStateId, InsideShape, bool)>,
        fluid: std::collections::HashSet<[i32; 3]>,
        alive: bool,
        visits: Vec<(i32, [i32; 3], &'static str)>,
    }
    const STONE: BlockStateId = BlockStateId(1);
    impl Grid {
        fn cell(&mut self, at: [i32; 3], shape: InsideShape, fluid: bool) -> &mut Self {
            self.blocks.insert(at, (STONE, shape, fluid));
            if fluid {
                self.fluid.insert(at);
            }
            self
        }
    }
    impl super::InsideWalk for Grid {
        type Error = ();
        type Effects = mod_api::InsideEffects;
        fn bounds_at(&self, at: sunrise_core::geometry::Vec3) -> sunrise_core::geometry::Aabb {
            sunrise_core::geometry::Aabb::new(
                at.subtract(sunrise_core::geometry::Vec3::new(0.3, 0.0, 0.3)),
                at.add_vector(sunrise_core::geometry::Vec3::new(0.3, 1.8, 0.3)),
            )
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
        fn block(&self, at: BlockPos) -> BlockStateId {
            self.blocks
                .get(&[at.x, at.y, at.z])
                .map_or(BlockStateId(0), |(state, _, _)| *state)
        }
        fn is_air(&self, state: BlockStateId) -> bool {
            state == BlockStateId(0)
        }
        fn inside_shape(&mut self, state: BlockStateId, at: BlockPos) -> InsideShape {
            match self.blocks.get(&[at.x, at.y, at.z]) {
                Some((block, shape, _)) if *block == state => shape.clone(),
                _ => InsideShape::Full,
            }
        }
        fn collided_with_fluid(
            &self,
            at: BlockPos,
            _: sunrise_core::geometry::Vec3,
            _: sunrise_core::geometry::Vec3,
        ) -> bool {
            self.fluid.contains(&[at.x, at.y, at.z])
        }
        fn entity_inside(
            &mut self,
            at: BlockPos,
            _: BlockStateId,
            step: i32,
            precise: bool,
            _: &mut mod_api::InsideEffects,
        ) -> Result<(), ()> {
            self.visits.push((
                step,
                [at.x, at.y, at.z],
                if precise { "precise" } else { "rough" },
            ));
            Ok(())
        }
        fn fluid_inside(
            &mut self,
            at: BlockPos,
            step: i32,
            _: &mut mod_api::InsideEffects,
        ) -> Result<(), ()> {
            self.visits.push((step, [at.x, at.y, at.z], "fluid"));
            Ok(())
        }
    }

    fn records(
        from_to: [([f64; 3], [f64; 3]); 2],
    ) -> [sunrise_world::entity_move::MovementRecord; 2] {
        from_to.map(|(from, to)| sunrise_world::entity_move::MovementRecord {
            from: sunrise_core::geometry::Vec3::new(from[0], from[1], from[2]),
            to: sunrise_core::geometry::Vec3::new(to[0], to[1], to[2]),
            original: None,
        })
    }

    fn walk(grid: &mut Grid, movements: &[sunrise_world::entity_move::MovementRecord]) {
        let mut visited = crate::fast_hash::FxHashSet::default();
        super::sweep_movements(grid, movements, &mut visited, &mut Default::default()).unwrap();
    }

    /// The walk visits a straight move's blocks in move order, one step each, `precise`
    /// when the box at the target touches the block.
    #[test]
    fn a_straight_move_visits_in_order() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        for x in 1..=3 {
            grid.cell([x, 64, 1], InsideShape::Full, false);
        }
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
            ]),
        );
        // The move crosses three blocks, so it is `movedFar`: every block is precise.
        assert_eq!(
            grid.visits,
            vec![
                (1, [1, 64, 1], "precise"),
                (2, [2, 64, 1], "precise"),
                (3, [3, 64, 1], "precise"),
            ]
        );
    }

    #[test]
    fn a_suspended_move_resumes_with_live_block_states() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Full, true);
        grid.cell([3, 64, 1], InsideShape::Full, false);
        let movement = sunrise_world::entity_move::MovementRecord {
            from: Vec3::new(0.5, 64.0, 1.5),
            to: Vec3::new(3.5, 64.0, 2.5),
            original: Some(Vec3::new(3.0, 0.0, 1.0)),
        };
        let mut sweep = super::MovementSweep::default();
        let mut visited = crate::fast_hash::FxHashSet::default();
        let mut effects = mod_api::InsideEffects::default();
        assert!(
            !sweep
                .run(&mut grid, &[movement], &mut visited, &mut effects, |_| true)
                .unwrap()
        );
        assert_eq!(
            grid.visits,
            [(1, [1, 64, 1], "precise"), (1, [1, 64, 1], "fluid")]
        );

        grid.cell([2, 64, 1], InsideShape::Full, false);
        grid.blocks.remove(&[3, 64, 1]);
        grid.cell([3, 64, 2], InsideShape::Full, false);
        assert!(
            sweep
                .run(&mut grid, &[movement], &mut visited, &mut effects, |_| {
                    false
                })
                .unwrap()
        );
        assert_eq!(
            grid.visits,
            [
                (1, [1, 64, 1], "precise"),
                (1, [1, 64, 1], "fluid"),
                (2, [2, 64, 1], "precise"),
                (1, [3, 64, 2], "precise"),
            ]
        );
    }

    /// A recorded original walks its axes in `axisStepOrder`: y first, then the longer
    /// horizontal axis, with a fresh step per axis.
    #[test]
    fn an_axis_original_walks_y_first_then_the_longer_axis() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Full, false);
        grid.cell([1, 65, 1], InsideShape::Full, false);
        grid.cell([1, 66, 2], InsideShape::Full, false);
        let record = sunrise_world::entity_move::MovementRecord {
            from: sunrise_core::geometry::Vec3::new(1.5, 64.5, 1.5),
            to: sunrise_core::geometry::Vec3::new(1.5, 66.5, 2.5),
            original: Some(sunrise_core::geometry::Vec3::new(0.0, 2.0, 1.0)),
        };
        let mut visited = crate::fast_hash::FxHashSet::default();
        super::sweep_movements(&mut grid, &[record], &mut visited, &mut Default::default())
            .unwrap();
        assert_eq!(
            grid.visits,
            vec![
                (0, [1, 64, 1], "precise"),
                (0, [1, 65, 1], "precise"),
                (1, [1, 66, 2], "precise")
            ]
        );
    }

    /// Air is passed over, an empty shape is never inside, boxes sweep the move, and
    /// one visited set covers every movement of the tick.
    #[test]
    fn shapes_decide_inside_and_visited_is_shared() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Empty, false);
        // A one-high top slab shape the standing box only reaches at the end.
        grid.cell(
            [2, 64, 1],
            InsideShape::Boxes(vec![sunrise_core::geometry::Aabb::new(
                sunrise_core::geometry::Vec3::new(2.0, 64.9, 1.0),
                sunrise_core::geometry::Vec3::new(3.0, 65.0, 2.0),
            )]),
            false,
        );
        grid.cell([3, 64, 1], InsideShape::Full, false);
        // The way back crosses both again over the same visited set: nothing new.
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [3.5, 64.0, 1.5]),
                ([3.5, 64.0, 1.5], [0.5, 64.0, 1.5]),
            ]),
        );
        assert_eq!(
            grid.visits,
            vec![(2, [2, 64, 1], "precise"), (3, [3, 64, 1], "precise")]
        );
    }

    /// The block's moment and the fluid's each get their own step advance, block first,
    /// and a dead entity stops the walk.
    #[test]
    fn fluid_advances_after_the_block_and_death_stops_the_walk() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        grid.cell([1, 64, 1], InsideShape::Full, true);
        grid.cell([2, 64, 1], InsideShape::Full, false);
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
            ]),
        );
        assert_eq!(
            grid.visits,
            vec![
                (1, [1, 64, 1], "precise"),
                (1, [1, 64, 1], "fluid"),
                (2, [2, 64, 1], "precise"),
            ]
        );
        grid.alive = false;
        grid.visits.clear();
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [2.5, 64.0, 1.5]),
            ]),
        );
        assert!(grid.visits.is_empty(), "a dead entity is not walked");
    }

    /// A move over the 16-iteration budget ends with the target checked alone.
    #[test]
    fn an_exhausted_budget_checks_the_target_alone() {
        let mut grid = Grid {
            alive: true,
            ..Grid::default()
        };
        for x in 1..=20 {
            grid.cell([x, 64, 1], InsideShape::Full, false);
        }
        walk(
            &mut grid,
            &records([
                ([0.5, 64.0, 1.5], [20.5, 64.0, 1.5]),
                ([0.5, 64.0, 1.5], [20.5, 64.0, 1.5]),
            ]),
        );
        // The budget ran out mid-move: fifteen cells on the way, then the target
        // alone, and nothing between them.
        assert_eq!(grid.visits.len(), 16);
        assert_eq!(grid.visits[0], (1, [1, 64, 1], "precise"));
        assert_eq!(grid.visits[14], (15, [15, 64, 1], "precise"));
        assert_eq!(grid.visits[15], (0, [20, 64, 1], "precise"));
        assert!(
            !grid
                .visits
                .iter()
                .any(|&(_, p, _)| (16..20).contains(&p[0])),
            "nothing between the budget and the target is visited"
        );
    }

    #[test]
    fn layered_cauldron_inside_shape_uses_static_fixed_shapes_for_levels_1_to_3() {
        for level in 1u8..=3 {
            let fixed = layered_cauldron_filled_shape(level).expect("cached shape for level");
            assert!(
                !fixed.boxes().is_empty(),
                "level {level} cauldron shape must have boxes"
            );
            let max_y = fixed
                .boxes()
                .iter()
                .map(|b| b.max_position().y)
                .fold(0.0_f64, f64::max);
            assert_eq!(max_y, 1.0, "shell top stays at 1.0 for level {level}");
        }
        assert!(layered_cauldron_filled_shape(0).is_none());
        assert!(layered_cauldron_filled_shape(4).is_none());
    }
}

#[cfg(test)]
#[path = "block_contact_parity_tests.rs"]
mod parity_tests;
