//! The dropped items' own moves, worked out ahead of the item loop on the tick's helper threads
//! while the parallel tick is on (`sunrise_client::parallel`).
//!
//! An item's move (`ItemEntity.tick` up to its merge) reads only itself, the level's blocks, the
//! obstacle boxes and the border, and the item loop changes none of them: so an item's move worked
//! out on a copy before the loop is the move its turn would make. Its escape from inside a block
//! draws the item's own random, which the copy carries. A merge before its turn can change its
//! count, age and pickup delay: such an item moves at its turn instead. With a native mod loaded
//! (its hooks see every item) or a mod listening to the round, every item moves at its turn.
use super::*;
use super::{entity::Entity as _, native_ticks};
use sunrise_core::geometry::{Aabb, Vec3, border::BorderGeometry};
use sunrise_items::blocks::BlockFacts;
use sunrise_level::world::World;
use sunrise_sim::{
    entities::{ItemEntity, ItemSplash, ItemStep, ObstacleGrid},
    motion::MotionResources,
};
use sunrise_world::{block_collision::BlockCollisionRegistry, registry::BlockRegistry};

/// From this many items their moves go to the helper threads.
const MIN_ITEMS: usize = 256;

/// What an item's own move reads.
pub(in crate::game) struct ItemTables<'a> {
    world: &'a World,
    facts: &'a BlockFacts,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    registry: &'a BlockRegistry,
    border: &'a BorderGeometry,
    context: &'a sunrise_world::native_load_context::LoopContext,
    side: sunrise_gameplay_common::game::native_surface::GameSide<'a>,
}

impl<'w> crate::game::ServerGame<'w> {
    pub(in crate::game::server_side) fn item_tables(&self) -> ItemTables<'_> {
        ItemTables {
            world: &self.common.world,
            facts: self.common.res.facts(),
            collision: self.common.collision,
            motion: self.common.res.motion(),
            registry: self.common.pack.assets.registry,
            border: self.common.border.geometry(),
            context: &self.common.loop_context,
            side: game_side!(self),
        }
    }
}

impl ItemTables<'_> {
    /// `ItemEntity.tick` up to its move's vertical contact: the fluids, the water splash, the move
    /// (pushed out at `escape_speed` when inside a block) and its fall.
    pub(in crate::game::server_side) fn move_item(
        &self,
        item: &mut ItemEntity,
        escape_speed: Option<f32>,
        entity_boxes: &[Aabb],
        splash: &mut impl FnMut(ItemSplash),
    ) -> Option<ItemStep> {
        // Entity.baseTick splashes before moving, except on the first tick, at
        // the motion before the currents push it.
        let before = item.velocity;
        let heights = item.push_by_fluid(self.world, &self.motion.fluids, self.context);
        item.water_entry(heights.0 > 0.0, item.ticks == 0, before, splash);
        item.lava_fall(item.ticks > 0 && heights.1 > 0.0);
        let mut step = item.tick_move_before_block_effects(
            self.world,
            self.facts,
            self.collision,
            escape_speed.is_some(),
            escape_speed,
            heights,
            entity_boxes,
            Some(self.border),
            Some(&self.motion.blocks),
            self.context,
        );
        item.check_fall(&mut step, |fall, from, moved| {
            if fall == 0.0 || moved.length_squared() < 1.0 {
                return fall;
            }
            let reset = &self.motion.fall_reset;
            let hit = sunrise_sim::entities::fall_reset_hit(
                self.world,
                &self.motion.fluids,
                |p| reset.contains(&*self.facts.get(self.world.block(p.x, p.y, p.z)).name),
                from,
                moved,
            );
            if hit { 0.0 } else { fall }
        });
        if let Some(response) = item.move_restitution(
            sunrise_sim::motion::BodyMotion {
                world: self.world,
                collision: self.collision,
                facts: self.facts,
                blocks: Some(&self.motion.blocks),
                context: self.context,
                side: None,
            },
            step,
        ) {
            // A real bounce has synchronous level effects. Leave that move to its own turn.
            if response.bounce.is_some() {
                return None;
            }
            item.velocity = response.velocity;
        }
        Some(step)
    }
    /// The rest of the move after its vertical contact: block effects, bubble columns, settling
    /// and the water entry. Whether the item lives on.
    pub(in crate::game::server_side) fn settle_item(
        &self,
        item: &mut ItemEntity,
        no_physics: bool,
        step: ItemStep,
        splash: &mut impl FnMut(ItemSplash),
    ) -> bool {
        if !no_physics {
            let motion = sunrise_sim::motion::BodyMotion {
                world: self.world,
                collision: self.collision,
                facts: self.facts,
                blocks: Some(&self.motion.blocks),
                context: self.context,
                side: None,
            };
            item.apply_move_and_vanilla_step(motion, step);
        }
        item.finish_move_block_effects(self.world, self.facts, step);
        item.push_by_bubble_column(self.world, self.registry, self.facts, self.collision);
        let alive = item.tick_settle(
            self.world,
            self.registry,
            self.facts,
            step,
            self.context,
            &self.side,
        );
        // ItemEntity.tick refreshes immersion and splashes on water entry.
        let before = item.velocity;
        let (water, _) = item.push_by_fluid(self.world, &self.motion.fluids, self.context);
        item.water_entry(water > 0.0, false, before, splash);
        alive
    }
}

/// `getEntityCollisions(entity, bounds.expandTowards(movement))`: the box around an item that its
/// move reads others' boxes in (a block of slack for the next velocity).
pub(in crate::game::server_side) fn reach(item: &ItemEntity) -> Aabb {
    let v = item.velocity;
    item.bounds()
        .inflate(Vec3::new(v.x.abs() + 1.0, v.y.abs() + 1.0, v.z.abs() + 1.0))
}

/// An item's move as its turn makes it: without physics (inside a block), alive, discarded, and
/// its splashes.
pub(in crate::game) type Moved = (bool, bool, bool, Vec<ItemSplash>);

/// One item's move, worked out on a copy.
pub(in crate::game) struct Ahead {
    moved: ItemEntity,
    result: Moved,
    /// What a merge before its turn can change: its count, age and pickup delay.
    started: (u32, i32, i32),
}

fn started(item: &ItemEntity) -> (u32, i32, i32) {
    (item.stack.count, item.age, item.pickup_delay)
}

/// `item`'s whole move on a server with no native mod, from the tables alone.
fn move_alone(
    tables: &ItemTables,
    obstacles: &ObstacleGrid,
    item: &mut ItemEntity,
    found: &mut Vec<u32>,
    near: &mut Vec<Aabb>,
) -> Option<Moved> {
    if item.fire_ticks != 0 {
        return None;
    }
    obstacles.near_shared(reach(item), found, near);
    let no_physics = item.inside_block(tables.world, tables.collision, near, Some(tables.border));
    // `moveTowardsClosestSpace` draws the speed from the item's own random.
    let escape_speed = no_physics.then(|| item.random.next_float() * 0.2 + 0.1);
    let mut splashes = Vec::new();
    let step = tables.move_item(item, escape_speed, near, &mut |s| splashes.push(s))?;
    if step.vertical_contact.is_some() && item.stack.count == 0 {
        return Some((no_physics, false, true, splashes));
    }
    // Contact effects need the owning level. The movement envelope is enough to
    // rule them out; the real sweep, when needed, tests the individual shapes.
    if !no_physics {
        let area = item
            .bounds()
            .expand_towards(item.previous.subtract(item.position));
        if !sunrise_sim::block_contact::box_cells(area, |pos| {
            let state = tables.world.block(pos.x, pos.y, pos.z);
            !matches!(
                &*tables.facts.get(state).name,
                "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:lava_cauldron"
                    | "minecraft:cactus"
            ) && !matches!(
                tables.motion.fluids.fluid(state).kind(),
                sunrise_world::fluid::FluidKind::Lava
                    | sunrise_world::fluid::FluidKind::FlowingLava
            )
        }) {
            return None;
        }
    }
    let alive = tables.settle_item(item, no_physics, step, &mut |s| splashes.push(s));
    Some((no_physics, alive, false, splashes))
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// A test's choice of the path, over the parallel tick's switch.
    pub static AHEAD: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    /// The moves a test's tick put in place, and those a merge made it refuse.
    pub static TAKEN: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

/// Whether the moves are worked out ahead: while the parallel tick is on.
fn ahead() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(ahead) = AHEAD.with(std::cell::Cell::get) {
        return ahead;
    }
    sunrise_sim::parallel::lanes() > 1
}

/// The moves worked out ahead, by the item's place in the loop.
pub struct Moves {
    planned: Vec<Option<Ahead>>,
    revision: u64,
}

impl Moves {
    /// The moves of the items that will tick (`ticks`), on the helper threads; none while the
    /// parallel tick is off, for few items, or with a mod that could see an item's move.
    pub fn work_out(
        game: &crate::game::ServerGame<'_>,
        items: &[ItemEntity],
        obstacles: &ObstacleGrid,
        round: &native_ticks::TickRound,
        ticks: impl Fn(&ItemEntity) -> bool,
    ) -> Self {
        if !ahead()
            || items.len() < MIN_ITEMS
            || round.listening()
            || !mod_api::__private::mods().is_empty()
        {
            return Self {
                planned: Vec::new(),
                revision: game.common.world.revision,
            };
        }
        let todo: Vec<usize> = (0..items.len()).filter(|&i| ticks(&items[i])).collect();
        let tables = game.item_tables();
        let done = sunrise_sim::parallel::par_map(&todo, MIN_ITEMS / 4, |part, out| {
            let (mut found, mut near) = (Vec::new(), Vec::new());
            for &i in part {
                let mut moved = items[i].clone();
                let Some(result) =
                    move_alone(&tables, obstacles, &mut moved, &mut found, &mut near)
                else {
                    continue;
                };
                let started = started(&items[i]);
                out.push((
                    i,
                    Ahead {
                        moved,
                        result,
                        started,
                    },
                ));
            }
        });
        let mut moves: Vec<Option<Ahead>> = (0..items.len()).map(|_| None).collect();
        for (i, a) in done {
            moves[i] = Some(a);
        }
        Self {
            planned: moves,
            revision: game.common.world.revision,
        }
    }
    /// Take the planned move only if the world revision and item state still match.
    pub fn take(
        &mut self,
        i: usize,
        item: &mut ItemEntity,
        revision: u64,
        again: impl FnOnce(&mut ItemEntity) -> Moved,
    ) -> Option<Moved> {
        let a = self.planned.get_mut(i)?.take()?;
        let fits = self.revision == revision && a.started == started(item);
        #[cfg(any(test, feature = "test-support"))]
        TAKEN.with(|t| {
            let (taken, refused) = t.get();
            t.set((taken + usize::from(fits), refused + usize::from(!fits)));
        });
        if !fits {
            return None;
        }
        if sunrise_sim::parallel::parallel_tick() == 2 {
            let mut serial = item.clone();
            let expected = again(&mut serial);
            check(item.id, &serial, &expected, &a.moved, &a.result);
        }
        *item = a.moved;
        Some(a.result)
    }
}

/// `SUNRISE_PARALLEL_TICK=verify`: an item's move worked out ahead against the one at its turn.
fn check(id: u32, serial: &ItemEntity, expected: &Moved, ahead: &ItemEntity, got: &Moved) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHECKED: AtomicU64 = AtomicU64::new(0);
    static DIFFERED: AtomicU64 = AtomicU64::new(0);
    if format!("{serial:?} {expected:?}") != format!("{ahead:?} {got:?}") {
        DIFFERED.fetch_add(1, Ordering::Relaxed);
        log::error!("parallel tick: item {id}'s move worked out ahead differs from its turn's");
    }
    let n = CHECKED.fetch_add(1, Ordering::Relaxed) + 1;
    if n == 1 || n.is_multiple_of(10_000) {
        log::info!(
            "parallel tick: {n} item moves worked out ahead checked, {} differed",
            DIFFERED.load(Ordering::Relaxed)
        );
    }
}
