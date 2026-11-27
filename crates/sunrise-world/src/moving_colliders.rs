//! The moving colliders solid to every mover of the level being ticked, as the game placed them
//! when that level's tick began (`Game::placed_moving_colliders`).
//!
//! Every physical move resolves through `SceneCollisions`, so this is the one place a collider
//! enters entity collision, whatever the mover (the player, a mob, an item, a vehicle). The
//! game runs on one thread per world (the server thread, the window), so the placed colliders
//! are that thread's own, and a [`Scope`] sets them for one level's tick only. Outside a scope,
//! and in a level with none, a query answers at once with nothing.
//!
//! The level's colliders are indexed by their world bounds in a coarse grid, so a mover far
//! from every collider pays one lookup however many the level holds.
use crate::entity_obstruction::EntityIdentity;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{Arc, OnceLock},
};
use sunrise_core::geometry::{
    Aabb, Axis, Vec3,
    carried::FrameLog,
    oriented::{ColliderBoxes, OrientedCollider},
};

/// Side of an index cell, in blocks.
const CELL: f64 = 16.0;
/// A collider covering more cells than this is not stored per cell; every query tests it.
const BIG_CELLS: i64 = 64;

/// One collider entity, placed where it stood at the start of the pass.
pub struct Placed {
    /// The collider's entity.
    pub id: u32,
    pub collider: OrientedCollider,
    /// The collider's own entity and every entity in its vehicle stack: the root vehicle and
    /// each passenger at any depth, those the collider's entity rides with included. The mover
    /// whose identity is among them does not collide with it (`isPassengerOfSameVehicle`).
    pub exempt: Vec<u32>,
}

/// The level's colliders with their world bounds and the grid over them.
#[derive(Default)]
struct Index {
    placed: Vec<(Aabb, Placed)>,
    cells: HashMap<[i32; 3], Vec<u32>>,
    big: Vec<u32>,
    /// The box holding every collider.
    all: Option<Aabb>,
}

fn cells_of(b: Aabb) -> ([i64; 3], [i64; 3]) {
    let f = |v: f64| (v / CELL).floor() as i64;
    (
        Axis::ALL.map(|a| f(b.min(a))),
        Axis::ALL.map(|a| f(b.max(a))),
    )
}

impl Index {
    fn new(placed: Vec<Placed>) -> Self {
        let mut index = Self::default();
        for p in placed {
            let Some(bounds) = p.collider.world_bounds() else {
                continue;
            };
            let at = index.placed.len() as u32;
            index.all = Some(index.all.map_or(bounds, |all| all.union_bounds(bounds)));
            let (lo, hi) = cells_of(bounds);
            let count = (0..3).map(|i| hi[i] - lo[i] + 1).product::<i64>();
            if count > BIG_CELLS {
                index.big.push(at);
            } else {
                for x in lo[0]..=hi[0] {
                    for y in lo[1]..=hi[1] {
                        for z in lo[2]..=hi[2] {
                            index
                                .cells
                                .entry([x as i32, y as i32, z as i32])
                                .or_default()
                                .push(at);
                        }
                    }
                }
            }
            index.placed.push((bounds, p));
        }
        index
    }
}

/// A level's placed colliders, indexed once and shared: the game keeps one per level and
/// builds a new one only when a collider, its place or its vehicle stack changed.
#[derive(Clone)]
pub struct Prepared(Arc<Index>);

impl Prepared {
    pub fn new(placed: Vec<Placed>) -> Self {
        Self(Arc::new(Index::new(placed)))
    }

    /// How many colliders the level holds.
    pub fn len(&self) -> usize {
        self.0.placed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.placed.is_empty()
    }

    /// The entities exempt from the `at`th collider.
    pub fn exempt(&self, at: usize) -> Option<&[u32]> {
        self.0.placed.get(at).map(|(_, p)| p.exempt.as_slice())
    }

    /// The colliders, in the order they were placed.
    pub fn placed(&self) -> impl Iterator<Item = &Placed> {
        self.0.placed.iter().map(|(_, p)| p)
    }

    /// Calls `f` with each collider whose world bounds meet `area`, through the grid: a query far
    /// from every collider costs one lookup.
    pub fn meeting(&self, area: Aabb, mut f: impl FnMut(&Placed)) {
        visit(&self.0, area, 0, |p| {
            f(p);
            false
        });
    }

    /// No collider at all, without building anything.
    pub fn empty() -> Self {
        static EMPTY: OnceLock<Arc<Index>> = OnceLock::new();
        Self(EMPTY.get_or_init(Arc::default).clone())
    }
}

struct ScopedColliders {
    level: Option<mod_api::LevelRef>,
    index: Arc<Index>,
}

thread_local! {
    static PLACED: RefCell<ScopedColliders> = RefCell::new(ScopedColliders {
        level: None,
        index: Prepared::empty().0,
    });
    /// The entity whose move is being resolved, for the moves that do not say who they are.
    static MOVER: Cell<u64> = const { Cell::new(0) };
    static SHOVE: Cell<Option<(u64, u32)>> = const { Cell::new(None) };
    static SCRATCH: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// The age the level's colliders are queried at (`at_age`), None for where they stand now.
    static AGE: Cell<Option<f64>> = const { Cell::new(None) };
}

/// The colliders placed for the level being ticked, for as long as the guard lives; the
/// colliders of the scope it replaced come back when it drops, so levels nest and nothing
/// outlives its tick. The level's tick holds one (like `block_contact::in_level`), and so
/// does a client's tick of the level it shows.
pub struct Scope {
    before: ScopedColliders,
}

impl Scope {
    pub fn enter(placed: Prepared) -> Self {
        Self::enter_level(None, placed)
    }

    /// Installs a new snapshot for the named level (e.g. at its tick or carry pass).
    pub fn enter_level(level: Option<mod_api::LevelRef>, placed: Prepared) -> Self {
        Self {
            before: PLACED.with_borrow_mut(|current| {
                std::mem::replace(
                    current,
                    ScopedColliders {
                        level,
                        index: placed.0,
                    },
                )
            }),
        }
    }

    /// The snapshot installed for the level's tick, for its end-of-tick displacement pass.
    pub fn placed() -> Prepared {
        PLACED.with_borrow(|current| Prepared(current.index.clone()))
    }

    /// Whether the named level already has a snapshot in scope.
    pub fn covers(level: mod_api::LevelRef) -> bool {
        PLACED.with_borrow(|current| current.level == Some(level))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        PLACED.with_borrow_mut(|current| std::mem::swap(current, &mut self.before));
    }
}

/// Runs `f` with `placed` as the colliders of the level it ticks.
pub fn in_level<R>(placed: Prepared, f: impl FnOnce() -> R) -> R {
    let _scope = Scope::enter(placed);
    f()
}

/// Runs `f` as the move of entity `id`: the collision entries that are not told which entity
/// moves take it from here, so the entity is never stopped by a collider of its own vehicle
/// stack. The move gate wraps every body move in this.
pub fn as_mover<R>(id: u32, f: impl FnOnce() -> R) -> R {
    let before = MOVER.with(|m| m.replace(u64::from(id)));
    let result = f();
    MOVER.with(|m| m.set(before));
    result
}

/// Keeps the collider giving this shove out of the pushed body's movement query. Like a
/// piston's scoped noclip, it cannot stop its own push. Other bodies and overlap queries still
/// see it, even if a move listener makes another query during the shove.
#[must_use]
pub struct Shove(Option<(u64, u32)>);

impl Shove {
    pub fn enter(mover: u32, source: u32) -> Self {
        Self(SHOVE.with(|active| active.replace(Some((u64::from(mover), source)))))
    }
}

impl Drop for Shove {
    fn drop(&mut self) {
        SHOVE.with(|active| active.set(self.0));
    }
}

/// Calls `f` with each collider of `index` meeting `area` that `mover` collides with, until it
/// answers true. `mover` 0 is exempt from nothing.
fn visit(index: &Index, area: Aabb, mover: u64, mut f: impl FnMut(&Placed) -> bool) {
    let Some(all) = index.all else { return };
    if !all.intersects(area) {
        return;
    }
    let mut found = SCRATCH.with(|s| std::mem::take(&mut *s.borrow_mut()));
    found.clear();
    let (lo, hi) = cells_of(area);
    let count = (0..3).map(|i| hi[i] - lo[i] + 1).product::<i64>();
    if count > index.cells.len() as i64 {
        found.extend(0..index.placed.len() as u32);
    } else {
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(list) = index.cells.get(&[x as i32, y as i32, z as i32]) {
                        found.extend_from_slice(list);
                    }
                }
            }
        }
        found.extend_from_slice(&index.big);
    }
    found.sort_unstable();
    found.dedup();
    for &at in &found {
        let (world, p) = &index.placed[at as usize];
        if world.intersects(area)
            && (mover == 0 || !p.exempt.iter().any(|id| u64::from(*id) == mover))
            && f(p)
        {
            break;
        }
    }
    SCRATCH.with(|s| *s.borrow_mut() = found);
}

/// The entity an unnamed query is made for: the move [`as_mover`] names when `entity` is 0.
fn mover_of(entity: EntityIdentity) -> u64 {
    if entity.0 == 0 {
        MOVER.with(Cell::get)
    } else {
        entity.0
    }
}

/// Calls `f` with each collider of the level meeting `area` that `mover` collides with, until it
/// answers true: where the colliders stand now, or, inside [`at_age`], where they stood.
fn visit_scene(area: Aabb, mover: u64, mut f: impl FnMut(u32, &OrientedCollider) -> bool) {
    if let Some(age) = AGE.with(Cell::get)
        && let Some(history) = HISTORY.with(|cell| cell.borrow().clone())
    {
        let history = history.borrow();
        for (&id, tracked) in &history.tracked {
            if !tracked.swept.is_some_and(|swept| swept.intersects(area))
                || (mover != 0 && tracked.exempt.iter().any(|id| u64::from(*id) == mover))
            {
                continue;
            }
            let Some(frame) = tracked.log.frame_at(history.now, age) else {
                continue;
            };
            let collider = OrientedCollider::new(frame, tracked.boxes.clone());
            if collider.world_bounds().is_some_and(|w| w.intersects(area)) && f(id, &collider) {
                return;
            }
        }
        return;
    }
    PLACED.with_borrow(|current| visit(&current.index, area, mover, |p| f(p.id, &p.collider)));
}

/// The entity whose move [`as_mover`] names, or 0 outside one.
pub fn mover() -> u64 {
    MOVER.with(Cell::get)
}

/// Appends to `out` the colliders meeting `bounds` that `entity` collides with. An identity of
/// 0 is the move [`as_mover`] names; with none named (a ray, a projectile's path) it is exempt
/// from nothing and meets every collider.
pub fn near(entity: EntityIdentity, bounds: Aabb, out: &mut Vec<OrientedCollider>) {
    // EntityGetter.getEntityCollisions expands its query by 1e-7.
    let area = bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
    let mover = mover_of(entity);
    visit_scene(area, mover, |id, collider| {
        if SHOVE.with(Cell::get) != Some((mover, id)) {
            out.push(collider.clone());
        }
        false
    });
}

/// Whether a collider box `entity` collides with overlaps `bounds` (the thin box at a body's
/// eyes, for the in-wall damage a block's suffocation gives). Allocates nothing.
pub fn overlaps(entity: EntityIdentity, bounds: Aabb) -> bool {
    let area = bounds.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
    let mut hit = false;
    visit_scene(area, mover_of(entity), |_, collider| {
        hit = collider.intersects(bounds);
        hit
    });
    hit
}

/// Whether the entity box `new` is in a box of a collider `entity` collides with that the box
/// `old` was not already in, box by box: a body that already overlaps one box may not walk into
/// another, of the same collider or of another. The entity-collision half of vanilla's
/// "colliding with anything new". Allocates nothing.
pub fn new_collision(entity: EntityIdentity, old: Aabb, new: Aabb) -> bool {
    let area = new.inflate(Vec3::new(1.0e-7, 1.0e-7, 1.0e-7));
    let mut found = false;
    visit_scene(area, mover_of(entity), |_, collider| {
        collider.overlapping(new, |index| {
            found = !collider.overlaps_box(index, old);
            found
        });
        found
    });
    found
}

/// One collider's recent frames, with the boxes it had when they were recorded.
struct Tracked {
    log: FrameLog,
    boxes: Arc<ColliderBoxes>,
    /// Everywhere the collider has been over the log, for finding what a position can have
    /// stood on.
    swept: Option<Aabb>,
    /// The collider's own entity and its vehicle stack (`Placed::exempt`).
    exempt: Vec<u32>,
}

/// The recent frames of a level's colliders, for judging a player a collider carries
/// (`sunrise_core::geometry::carried`). The game keeps one per level and records the level's
/// colliders into it at the start of each of the level's ticks; only a server needs it.
#[derive(Default)]
pub struct History {
    now: u64,
    /// By entity id, so a walk over them is in id order.
    tracked: BTreeMap<u32, Tracked>,
}

impl History {
    /// Records the colliders as the level's tick began. A collider that stood still adds
    /// nothing; one that is gone is forgotten.
    pub fn record(&mut self, placed: &Prepared) {
        self.now += 1;
        let now = self.now;
        for p in placed.placed() {
            let entry = self.tracked.entry(p.id).or_insert_with(|| Tracked {
                log: FrameLog::default(),
                boxes: p.collider.boxes.clone(),
                swept: None,
                exempt: p.exempt.clone(),
            });
            if entry.exempt != p.exempt {
                entry.exempt.clone_from(&p.exempt);
            }
            let new_boxes = !Arc::ptr_eq(&entry.boxes, &p.collider.boxes);
            if new_boxes {
                entry.boxes = p.collider.boxes.clone();
            }
            if entry.log.record(now, p.collider.frame) || new_boxes || entry.swept.is_none() {
                entry.swept = entry
                    .boxes
                    .bounds()
                    .and_then(|local| entry.log.swept(local));
            }
        }
        if self.tracked.len() != placed.len() {
            let ids: std::collections::HashSet<u32> = placed.placed().map(|p| p.id).collect();
            self.tracked.retain(|id, _| ids.contains(id));
        }
    }

    /// The tick of the newest frames: age 0.
    pub fn now(&self) -> u64 {
        self.now
    }

    pub fn is_empty(&self) -> bool {
        self.tracked.is_empty()
    }

    /// A collider's frames and boxes.
    pub fn get(&self, id: u32) -> Option<(&FrameLog, &ColliderBoxes)> {
        self.tracked.get(&id).map(|t| (&t.log, &*t.boxes))
    }

    /// Calls `f` with each collider that has been in `area` at some tick of the log, in entity
    /// id order, until it answers true.
    pub fn near(&self, area: Aabb, mut f: impl FnMut(u32) -> bool) {
        for (id, tracked) in &self.tracked {
            if tracked.swept.is_some_and(|s| s.intersects(area)) && f(*id) {
                return;
            }
        }
    }
}

thread_local! {
    static HISTORY: RefCell<Option<Rc<RefCell<History>>>> = const { RefCell::new(None) };
}

/// The history of the level being ticked, for as long as the guard lives (like [`Scope`]).
pub struct HistoryScope {
    before: Option<Rc<RefCell<History>>>,
}

impl HistoryScope {
    pub fn enter(history: Rc<RefCell<History>>) -> Self {
        Self::replace(Some(history))
    }

    /// Makes `history` the one in scope, `None` meaning a level that keeps none: the guard
    /// then puts back whatever was in scope before.
    pub fn replace(history: Option<Rc<RefCell<History>>>) -> Self {
        Self {
            before: HISTORY.with(|cell| std::mem::replace(&mut *cell.borrow_mut(), history)),
        }
    }
}

impl Drop for HistoryScope {
    fn drop(&mut self) {
        let before = self.before.take();
        HISTORY.with(|cell| *cell.borrow_mut() = before);
    }
}

/// Calls `f` with the level's history; `None` outside a level that keeps one.
pub fn with_history<R>(f: impl FnOnce(&History) -> R) -> Option<R> {
    let history = HISTORY.with(|cell| cell.borrow().clone())?;
    let history = history.borrow();
    Some(f(&history))
}

/// Runs `f` with the level's colliders where they stood `age` ticks before the tick began (a
/// fractional age is interpolated): the world a client saw, which is what a carried player's
/// move is checked in. Outside a level with a history it just runs `f`.
pub fn at_age<R>(age: f64, f: impl FnOnce() -> R) -> R {
    // The colliders are read from the history at the age, a collider at a time, as the query
    // meets them: nothing is built.
    let before = AGE.with(|cell| cell.replace(Some(age)));
    let result = f();
    AGE.with(|cell| cell.set(before));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use sunrise_core::geometry::oriented::{ColliderBoxes, Frame};

    fn block_at(x: f64, exempt: &[u32]) -> Placed {
        let boxes = ColliderBoxes::new(vec![Aabb::new(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        )]);
        Placed {
            id: 1,
            collider: OrientedCollider::new(
                Frame::translation(Vec3::new(x, 0.0, 0.0)),
                Arc::new(boxes),
            ),
            exempt: exempt.to_vec(),
        }
    }

    fn found(entity: u64, around: f64) -> usize {
        let mut out = Vec::new();
        let area = Aabb::new(
            Vec3::new(around - 0.5, 0.0, 0.0),
            Vec3::new(around + 1.5, 1.0, 1.0),
        );
        near(EntityIdentity(entity), area, &mut out);
        out.len()
    }

    #[test]
    fn a_query_meets_the_colliders_near_it_and_not_those_exempt_or_far() {
        in_level(
            Prepared::new(vec![
                block_at(0.0, &[7, 8]),
                block_at(100.0, &[]),
                block_at(2_000.0, &[]),
            ]),
            || {
                assert_eq!(found(5, 0.0), 1, "an outsider meets the near one");
                assert_eq!(found(7, 0.0), 0, "the collider's own stack does not");
                assert_eq!(found(8, 0.0), 0);
                assert_eq!(found(7, 100.0), 1, "another collider is not exempt for it");
                assert_eq!(found(5, 50.0), 0, "nothing in reach");
                assert_eq!(found(0, 0.0), 1, "no mover named: exempt from nothing");
                as_mover(7, || {
                    assert_eq!(found(0, 0.0), 0, "the named mover is exempt")
                });
                assert_eq!(found(0, 0.0), 1, "the name ends with the move");
            },
        );
        assert_eq!(found(5, 0.0), 0, "outside a scope there is none");
    }

    /// A thin box at a body's eyes inside a collider box is in it; outside, or in its own
    /// stack's, it is not.
    #[test]
    fn an_eye_inside_a_box_is_in_the_wall_unless_it_is_the_stacks_own() {
        let eye = Aabb::new(Vec3::new(0.5, 0.5, 0.5), Vec3::new(0.7, 0.500_001, 0.7));
        let outside = Aabb::new(Vec3::new(5.0, 0.5, 0.5), Vec3::new(5.2, 0.500_001, 0.7));
        in_level(Prepared::new(vec![block_at(0.0, &[7])]), || {
            assert!(overlaps(EntityIdentity(5), eye));
            assert!(!overlaps(EntityIdentity(5), outside));
            assert!(!overlaps(EntityIdentity(7), eye));
        });
    }

    #[test]
    fn scopes_nest_and_restore() {
        in_level(Prepared::new(vec![block_at(0.0, &[])]), || {
            in_level(Prepared::empty(), || assert_eq!(found(5, 0.0), 0));
            assert_eq!(found(5, 0.0), 1);
        });
    }
}
