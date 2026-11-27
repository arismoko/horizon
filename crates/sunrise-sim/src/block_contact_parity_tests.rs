//! #1011 part 1: every mover's contact traversal now runs through the shared
//! decision ([`super::contact_at`]) and its entries ([`super::sweep_inside_blocks`],
//! [`super::sweep_movements`], [`super::visit_box_cells`], [`super::box_cells`]).
//! Each test below keeps the traversal a mover ran before the change, verbatim
//! in its decisions, as the oracle, and compares its trace with the shared
//! sweep's over the same seeded grids and moves: the step advances, the block
//! moments (with `precise`), the fluid moments, their order and the visited set.
use super::{
    InsideShape, InsideWalk, StepCollector, box_cells, sweep_inside_blocks, sweep_movements,
    visit_box_cells,
};
use crate::fast_hash::FxHashSet;
use std::collections::HashMap;
use sunrise_core::{
    geometry::{Aabb, Axis, Direction, Vec3, sweep_intersected_blocks},
    position::BlockPos,
};
use sunrise_world::{entity_move::MovementRecord, registry::BlockStateId};

/// One thing a traversal did, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ev {
    /// `advanceStep(step)`.
    Advance(i32),
    /// The block's `entityInside`: step, cell, `isPrecise`.
    Block(i32, [i32; 3], bool),
    /// A fire block's own effect, after its moment (the arrow's).
    Fire([i32; 3]),
    /// The fluid's `entityInside`: step, cell.
    Fluid(i32, [i32; 3]),
}

/// The collector the comparison walks queue into: the whole trace.
#[derive(Default, Debug, PartialEq)]
struct Trace(Vec<Ev>);

impl StepCollector for Trace {
    fn advance_step(&mut self, step: i32) {
        self.0.push(Ev::Advance(step));
    }
}

/// A seeded generator, so a failing case can be replayed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
}

/// A cell of the grid: its inside shape, whether its fluid collides, whether it
/// is a fire (or, for the flower pass, a flower).
#[derive(Clone, Debug)]
struct Cell {
    shape: InsideShape,
    fluid: bool,
    fire: bool,
}

/// The level the walks read: non-air cells, everything else air.
#[derive(Default)]
struct Grid {
    cells: HashMap<[i32; 3], Cell>,
}

impl Grid {
    /// A seeded grid over x, y, z in -2..12, about a third of it non-air. With
    /// `full_only`, every block is the whole block and no fluid collides, the
    /// shape the mob, item, projectile and arrow walks give every block.
    fn seeded(rng: &mut Rng, full_only: bool) -> Self {
        let mut cells = HashMap::new();
        for x in -2..12 {
            for y in -2..12 {
                for z in -2..12 {
                    if !rng.chance(0.33) {
                        continue;
                    }
                    let shape = if full_only {
                        InsideShape::Full
                    } else {
                        match rng.next() % 20 {
                            0..=9 => InsideShape::Full,
                            10..=12 => InsideShape::Empty,
                            _ => {
                                let base = Vec3::new(f64::from(x), f64::from(y), f64::from(z));
                                let (a, b) = (rng.range(0.0, 0.9), rng.range(0.0, 0.9));
                                let lo = Vec3::new(a, b, rng.range(0.0, 0.5));
                                let hi = Vec3::new(
                                    rng.range(a + 0.05, 1.0),
                                    rng.range(b + 0.05, 1.0),
                                    rng.range(0.55, 1.0),
                                );
                                InsideShape::Boxes(vec![Aabb::new(
                                    base.add_vector(lo),
                                    base.add_vector(hi),
                                )])
                            }
                        }
                    };
                    let fluid = !full_only && rng.chance(0.25);
                    let fire = rng.chance(0.2);
                    cells.insert([x, y, z], Cell { shape, fluid, fire });
                }
            }
        }
        Self { cells }
    }
    fn get(&self, p: BlockPos) -> Option<&Cell> {
        self.cells.get(&[p.x, p.y, p.z])
    }
    fn state(&self, p: BlockPos) -> BlockStateId {
        BlockStateId(u32::from(self.get(p).is_some()) as _)
    }
}

/// An entity box about `at`: `width` wide, `height` tall.
fn box_at(at: Vec3, width: f64, height: f64) -> Aabb {
    let h = width / 2.0;
    Aabb::new(
        Vec3::new(at.x - h, at.y, at.z - h),
        Vec3::new(at.x + h, at.y + height, at.z + h),
    )
}

/// The shared walk over the grid, for a kind of a given box and deflate.
struct GridWalk<'g, const ARROW: bool> {
    grid: &'g Grid,
    width: f64,
    height: f64,
}

impl<const ARROW: bool> InsideWalk for GridWalk<'_, ARROW> {
    type Error = ();
    type Effects = Trace;
    const DEFLATE: f64 = if ARROW { 1.0e-5_f32 as f64 } else { 1.0e-5 };
    fn bounds_at(&self, at: Vec3) -> Aabb {
        box_at(at, self.width, self.height)
    }
    fn is_alive(&self) -> bool {
        true
    }
    fn block(&self, at: BlockPos) -> BlockStateId {
        self.grid.state(at)
    }
    fn is_air(&self, state: BlockStateId) -> bool {
        state == BlockStateId(0)
    }
    fn inside_shape(&mut self, _: BlockStateId, at: BlockPos) -> InsideShape {
        self.grid
            .get(at)
            .map_or(InsideShape::Empty, |c| c.shape.clone())
    }
    fn collided_with_fluid(&self, at: BlockPos, _: Vec3, _: Vec3) -> bool {
        self.grid.get(at).is_some_and(|c| c.fluid)
    }
    fn entity_inside(
        &mut self,
        at: BlockPos,
        _: BlockStateId,
        step: i32,
        precise: bool,
        trace: &mut Trace,
    ) -> Result<(), ()> {
        trace.0.push(Ev::Block(step, [at.x, at.y, at.z], precise));
        if ARROW && self.grid.get(at).is_some_and(|c| c.fire) {
            trace.0.push(Ev::Fire([at.x, at.y, at.z]));
        }
        Ok(())
    }
    fn fluid_inside(&mut self, at: BlockPos, step: i32, trace: &mut Trace) -> Result<(), ()> {
        trace.0.push(Ev::Fluid(step, [at.x, at.y, at.z]));
        Ok(())
    }
}

const FAR: f64 = 0.999_990_000_000_252_6_f64;

/// The shared sweep as it was before the decision was split out (the player's,
/// the mob's main walk, the item's and the projectile's): the shape decided in
/// the loop, then the fluid, both before the visited insert.
fn old_sweep(
    walk: &mut GridWalk<'_, false>,
    from: Vec3,
    to: Vec3,
    visited: &mut FxHashSet<i64>,
    trace: &mut Trace,
    max_iterations: i32,
) -> i32 {
    let deflated = walk
        .bounds_at(to)
        .deflate(Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
    let moved_far = from.distance_squared(to) > FAR * FAR;
    let mut iterations = 0;
    let mut hits = Vec::new();
    let _ = sweep_intersected_blocks(from, to, deflated, 1_000_000, |p, iteration| {
        if !walk.is_alive() || iteration >= max_iterations {
            return false;
        }
        iterations = iteration;
        let state = walk.block(p);
        if !walk.is_air(state) {
            hits.push((iteration, p, state));
        }
        true
    });
    for (step, p, state) in hits {
        let inside_block = match walk.inside_shape(state, p) {
            InsideShape::Full => true,
            InsideShape::Empty => false,
            InsideShape::Boxes(boxes) => walk
                .bounds_at(from)
                .collided_along_vector(to.subtract(from), &boxes),
            // Entity.collidedWithShapeMovingFrom over the shape's boxes moved to the block.
            InsideShape::Fixed(shape) => {
                let boxes: Vec<_> = shape.moved(Vec3::from_block(p)).boxes().to_vec();
                walk.bounds_at(from)
                    .collided_along_vector(to.subtract(from), &boxes)
            }
        };
        let inside_fluid = walk.collided_with_fluid(p, from, to);
        if !(inside_block || inside_fluid) || !visited.insert(p.pack()) {
            continue;
        }
        if inside_block {
            let precise = moved_far || deflated.intersects(Aabb::block(p));
            trace.0.push(Ev::Advance(step));
            let _ = walk.entity_inside(p, state, step, precise, trace);
        }
        if inside_fluid {
            trace.0.push(Ev::Advance(step));
            let _ = walk.fluid_inside(p, step, trace);
        }
    }
    iterations + 1
}

/// `sweep_movements` as it was, over [`old_sweep`].
fn old_movements(
    walk: &mut GridWalk<'_, false>,
    movements: &[MovementRecord],
    visited: &mut FxHashSet<i64>,
    trace: &mut Trace,
) {
    for movement in movements {
        let mut from = movement.from;
        let delta = movement.to.subtract(movement.from);
        let mut budget = 16;
        match movement.original {
            Some(original) if delta.length_squared() > 0.0 => {
                for axis in Axis::step_order(original) {
                    let step = delta.component(axis);
                    if step != 0.0 {
                        let to = from.relative(
                            match axis {
                                Axis::X => Direction::East,
                                Axis::Y => Direction::Up,
                                Axis::Z => Direction::South,
                            },
                            step,
                        );
                        budget -= old_sweep(walk, from, to, visited, trace, budget);
                        from = to;
                    }
                }
            }
            _ => budget -= old_sweep(walk, movement.from, movement.to, visited, trace, 16),
        }
        if budget <= 0 {
            old_sweep(walk, movement.to, movement.to, visited, trace, 1);
        }
    }
}

/// The arrow's own `checkInsideBlocks` as it was (`ArrowScene::check_inside_blocks`):
/// every swept cell kept, air skipped after, the whole block for every block,
/// one advance before the visit and the fire's effect, another before the fluid.
fn old_arrow_sweep(
    grid: &Grid,
    from: Vec3,
    to: Vec3,
    visited: &mut std::collections::HashSet<BlockPos>,
    trace: &mut Trace,
    max_iterations: i32,
) -> i32 {
    let e = f64::from(1.0e-5f32);
    let deflated = box_at(to, 0.5, 0.5).deflate(Vec3::new(e, e, e));
    let far = from.distance_squared(to) > FAR * FAR;
    let mut hits = Vec::new();
    let mut iterations = 0;
    let _ = sweep_intersected_blocks(from, to, deflated, 1_000_000, |p, iteration| {
        if iteration >= max_iterations {
            return false;
        }
        iterations = iteration;
        hits.push((p, iteration));
        true
    });
    for (p, step) in hits {
        let Some(cell) = grid.get(p) else {
            continue;
        };
        let inside_block = true;
        let inside_fluid = cell.fluid;
        if !(inside_block || inside_fluid) || !visited.insert(p) {
            continue;
        }
        trace.0.push(Ev::Advance(step));
        let precise = far
            || deflated.intersects(Aabb::new(
                Vec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)),
                Vec3::new(f64::from(p.x + 1), f64::from(p.y + 1), f64::from(p.z + 1)),
            ));
        trace.0.push(Ev::Block(step, [p.x, p.y, p.z], precise));
        if cell.fire {
            trace.0.push(Ev::Fire([p.x, p.y, p.z]));
        }
        if inside_fluid {
            trace.0.push(Ev::Advance(step));
            trace.0.push(Ev::Fluid(step, [p.x, p.y, p.z]));
        }
    }
    iterations + 1
}

/// A seeded move inside the grid: short, axis-ish, or long enough to run the
/// 16-iteration budget out, with or without a recorded original.
fn seeded_move(rng: &mut Rng) -> MovementRecord {
    let from = Vec3::new(
        rng.range(1.0, 9.0),
        rng.range(1.0, 9.0),
        rng.range(1.0, 9.0),
    );
    let reach = match rng.next() % 4 {
        0 => 0.0,
        1 => 0.6,
        2 => 2.5,
        _ => 9.0,
    };
    let delta = Vec3::new(
        rng.range(-reach, reach),
        rng.range(-reach, reach),
        rng.range(-reach, reach),
    );
    let original = rng.chance(0.5).then(|| {
        Vec3::new(
            delta.x * rng.range(0.5, 2.0),
            delta.y * rng.range(0.5, 2.0),
            delta.z * rng.range(0.5, 2.0),
        )
    });
    MovementRecord {
        from,
        to: from.add_vector(delta),
        original,
    }
}

const CASES: u64 = 400;

/// Player, mob main walk, item and projectile: the movements of a tick (with
/// and without originals, over one visited set) give the same trace and the same
/// visited set through the shared decision as through the walk it replaced, for
/// whole, empty and boxed shapes and colliding fluids.
#[test]
fn movement_walks_match_the_walk_they_replaced() {
    let mut rng = Rng(0x1011_0001);
    let mut nonempty = 0;
    for case in 0..CASES {
        let grid = Grid::seeded(&mut rng, false);
        let movements: Vec<_> = (0..1 + rng.next() % 3)
            .map(|_| seeded_move(&mut rng))
            .collect();
        let (width, height) = [(0.6, 1.8), (0.25, 0.25), (0.98, 0.7)][(case % 3) as usize];
        let mut walk = GridWalk::<false> {
            grid: &grid,
            width,
            height,
        };
        let (mut old_visited, mut old_trace) = (FxHashSet::default(), Trace::default());
        old_movements(&mut walk, &movements, &mut old_visited, &mut old_trace);
        let (mut visited, mut trace) = (FxHashSet::default(), Trace::default());
        sweep_movements(&mut walk, &movements, &mut visited, &mut trace).unwrap();
        assert_eq!(trace, old_trace, "case {case}: {movements:?}");
        assert_eq!(visited, old_visited, "case {case}");
        nonempty += usize::from(!trace.0.is_empty());
    }
    assert!(nonempty > CASES as usize / 2, "the cases reach blocks");
}

/// Arrows: `apply_block_effects`' two calls of its own walk (16 iterations, then
/// the target alone when the budget ran out) give the same advances, visits, fire
/// effects and fluid moments as the shared `sweep_movements` with the arrow's
/// `f32` deflate, whole blocks and its fluid.
#[test]
fn the_arrow_walk_matches_the_one_it_replaced() {
    let mut rng = Rng(0x1011_0002);
    for case in 0..CASES {
        let grid = Grid::seeded(&mut rng, true);
        // The arrow's fluid collides on its own, whole blocks or not.
        let mut grid = grid;
        for cell in grid.cells.values_mut() {
            cell.fluid = rng.chance(0.25);
        }
        let movement = MovementRecord {
            original: None,
            ..seeded_move(&mut rng)
        };
        let (mut old_visited, mut old_trace) = (Default::default(), Trace::default());
        let used = old_arrow_sweep(
            &grid,
            movement.from,
            movement.to,
            &mut old_visited,
            &mut old_trace,
            16,
        );
        if 16 - used <= 0 {
            old_arrow_sweep(
                &grid,
                movement.to,
                movement.to,
                &mut old_visited,
                &mut old_trace,
                1,
            );
        }
        let mut walk = GridWalk::<true> {
            grid: &grid,
            width: 0.5,
            height: 0.5,
        };
        let (mut visited, mut trace) = (FxHashSet::default(), Trace::default());
        sweep_movements(&mut walk, &[movement], &mut visited, &mut trace).unwrap();
        assert_eq!(trace, old_trace, "case {case}: {movement:?}");
        let old: FxHashSet<i64> = old_visited.iter().map(|p| p.pack()).collect();
        assert_eq!(visited, old, "case {case}");
    }
}

/// Pressure plates and tripwires: `arrows::blocks_inside`, now on the shared
/// walk, lists the same cells in the same order as its own sweep did.
#[test]
fn the_arrow_cells_match_the_sweep_they_replaced() {
    let mut rng = Rng(0x1011_0003);
    for case in 0..CASES {
        let m = seeded_move(&mut rng);
        let e = f64::from(1.0e-5f32);
        let deflated = crate::entities::arrows::arrow_bounds(m.to).deflate(Vec3::new(e, e, e));
        let mut old = Vec::new();
        let _ = sweep_intersected_blocks(m.from, m.to, deflated, 1_000_000, |p, iteration| {
            if iteration >= 16 {
                return false;
            }
            old.push(p);
            true
        });
        assert_eq!(
            crate::entities::arrows::blocks_inside(m.from, m.to),
            old,
            "case {case}: {m:?}"
        );
    }
}

/// The mob's stationary pass over its box's cells at the tick's start, then its
/// walk: the old cell loop (non-air, visited, the block's moment at step 0,
/// rough, no advance) and [`visit_box_cells`] give the same trace and leave the
/// same visited set for the walk after.
#[test]
fn the_mob_start_pass_matches_the_loop_it_replaced() {
    let mut rng = Rng(0x1011_0004);
    for case in 0..CASES {
        let grid = Grid::seeded(&mut rng, true);
        let movement = MovementRecord {
            original: None,
            ..seeded_move(&mut rng)
        };
        let mut walk = GridWalk::<false> {
            grid: &grid,
            width: 0.6,
            height: 1.95,
        };
        let start_box = walk
            .bounds_at(movement.from)
            .deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (mut old_visited, mut old_trace) = (FxHashSet::default(), Trace::default());
        let (lo, hi) = (start_box.min_position(), start_box.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let at = BlockPos::new(x, y, z);
                    if grid.get(at).is_some() && old_visited.insert(at.pack()) {
                        old_trace.0.push(Ev::Block(0, [x, y, z], false));
                    }
                }
            }
        }
        old_movements(&mut walk, &[movement], &mut old_visited, &mut old_trace);
        let (mut visited, mut trace) = (FxHashSet::default(), Trace::default());
        visit_box_cells(
            &mut walk,
            start_box,
            movement.from,
            &mut visited,
            &mut trace,
        )
        .unwrap();
        sweep_movements(&mut walk, &[movement], &mut visited, &mut trace).unwrap();
        assert_eq!(trace, old_trace, "case {case}: {movement:?}");
        assert_eq!(visited, old_visited, "case {case}");
    }
}

/// The mob's flower pass: its own whole-path sweep (no budget, no visited set,
/// each flower's effect where the sweep meets it) and the shared sweep with the
/// pass's 2^22 budget meet the same flowers in the same order.
#[test]
fn the_mob_flower_pass_matches_the_sweep_it_replaced() {
    let mut rng = Rng(0x1011_0005);
    for case in 0..CASES {
        let grid = Grid::seeded(&mut rng, true);
        let m = seeded_move(&mut rng);
        let mut walk = GridWalk::<false> {
            grid: &grid,
            width: 0.6,
            height: 1.95,
        };
        let box_at_end = walk
            .bounds_at(m.to)
            .deflate(Vec3::new(1.0e-5, 1.0e-5, 1.0e-5));
        let mut old = Vec::new();
        let _ = sweep_intersected_blocks(m.from, m.to, box_at_end, 1 << 22, |p, _| {
            if grid.get(p).is_some_and(|c| c.fire) {
                old.push([p.x, p.y, p.z]);
            }
            true
        });
        let mut trace = Trace::default();
        sweep_inside_blocks(
            &mut walk,
            m.from,
            m.to,
            &mut FxHashSet::default(),
            &mut trace,
            1 << 22,
        )
        .unwrap();
        let new: Vec<_> = trace
            .0
            .iter()
            .filter_map(|e| match *e {
                Ev::Block(_, p, _) if grid.cells[&p].fire => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(new, old, "case {case}: {m:?}");
    }
}

/// The item's and the stand's box-cell passes (fire, cactus, lava cauldron,
/// cobweb and powder snow, honey): [`box_cells`] enumerates x, then y, then z,
/// floor to floor, as their own loops did, and stops where they returned.
#[test]
fn box_cells_match_the_loops_they_replaced() {
    let mut rng = Rng(0x1011_0006);
    for case in 0..CASES {
        let at = Vec3::new(
            rng.range(-3.0, 3.0),
            rng.range(-3.0, 3.0),
            rng.range(-3.0, 3.0),
        );
        let b = box_at(at, rng.range(0.1, 2.5), rng.range(0.1, 2.5))
            .deflate(Vec3::new(1e-5, 1e-5, 1e-5));
        let (lo, hi) = (b.min_position(), b.max_position());
        let mut old = Vec::new();
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    old.push(BlockPos::new(x, y, z));
                }
            }
        }
        let mut new = Vec::new();
        assert!(box_cells(b, |p| {
            new.push(p);
            true
        }));
        assert_eq!(new, old, "case {case}");
        // A pass that stops at a cell sees nothing after it.
        let stop = (rng.next() as usize) % old.len();
        let mut seen = Vec::new();
        assert!(!box_cells(b, |p| {
            seen.push(p);
            seen.len() <= stop
        }));
        assert_eq!(seen, old[..=stop], "case {case}");
    }
}
