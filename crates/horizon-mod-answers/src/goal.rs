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
        pick(
            self.level,
            self.w.level.id(),
            from,
            [v.x, v.y, v.z],
            distance,
        )
    }
}

/// `PathNavigation.moveTo` for a mod goal's own mob: numbers the pathfinder does not take refuse
/// here as they refuse in the navigation handle. Writes the mob, reads the tick's world.
pub fn move_to(mob: &mut Mob, w: &MobWorld<'_>, at: [f64; 3], speed: f64) -> bool {
    let accepted = speed.is_finite()
        && at
            .iter()
            .all(|v| v.is_finite() && v.abs() <= 2_000_000_000.0);
    accepted && mob.nav_move_to(w, at, speed)
}

/// `Mob.getLookControl().setLookAt` for a mod goal's own mob; refuses non-finite coordinates.
pub fn look_at(mob: &mut Mob, at: [f64; 3]) {
    if at.iter().all(|v| v.is_finite()) {
        mob.look_at(at);
    }
}

/// A body that writes the level while the mob is lent: both are separate `&mut` borrows the
/// caller hands in, which is the whole requirement on the game's side. Places `state` in the
/// block under the mob's feet.
pub fn mob_places_block_below<L: engine_access::LevelWrite + LevelRead>(
    mob: &mut Mob,
    level: &mut L,
    dimension: &str,
    state: horizon_world::registry::BlockStateId,
) -> Result<bool, mod_api::LevelWriteError> {
    let level_ref = level
        .level_of(dimension)
        .ok_or(mod_api::LevelWriteError::Unavailable)?;
    let pos = [
        mob.position.x.floor() as i32,
        mob.position.y.floor() as i32 - 1,
        mob.position.z.floor() as i32,
    ];
    level.set_block(level_ref, pos, state, mod_api::BlockUpdateFlags::NONE)
}
