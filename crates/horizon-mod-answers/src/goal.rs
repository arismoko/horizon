//! The goal-tick context: what a mod goal's reads run with while its mob is lent to the tick.
//!
//! The mob in hand, the tick's world and the level the game lends are three borrows the game
//! builds and hands in disjoint: nothing here reaches the game, and a body that needs an entity
//! and the level together uses `m`/`w` for the entity and `level` for the level.
use engine_access::LevelRead;
use horizon_client_lib::mobs::{Mob, MobWorld};
use horizon_world::view_direction::view_vector;
use mod_api::{ClipBlock, ClipFluid, ClipHit};

/// What a mod goal runs with.
pub struct GoalCx<'a, 'w, L: LevelRead> {
    pub m: &'a Mob,
    pub w: &'a MobWorld<'w>,
    pub level: &'a L,
}

/// `Entity.pick` from a point along a view vector: the block ray, outlines and no fluids, in the
/// level named `dimension`. None for a distance that is not finite and non-negative, a level
/// that is not loaded, or a ray the level cannot clip.
pub fn pick<L: LevelRead>(
    level: &L,
    dimension: &str,
    from: [f64; 3],
    view: [f64; 3],
    distance: f64,
) -> Option<ClipHit> {
    if !(distance.is_finite() && distance >= 0.0) {
        return None;
    }
    let to = std::array::from_fn(|i| from[i] + view[i] * distance);
    let level_ref = level.level_of(dimension)?;
    level.clip(level_ref, from, to, ClipBlock::Outline, ClipFluid::None)
}

impl<L: LevelRead> GoalCx<'_, '_, L> {
    /// What the ticking mob looks at within `distance` blocks of its eyes (`r_entity_pick`).
    pub fn pick_own(&self, distance: f64) -> Option<ClipHit> {
        let mob = self.m;
        let from = [mob.position.x, mob.eye_y(), mob.position.z];
        let v = view_vector(mob.head_pitch, mob.yaw);
        pick(self.level, self.w.level.id(), from, [v.x, v.y, v.z], distance)
    }
}
