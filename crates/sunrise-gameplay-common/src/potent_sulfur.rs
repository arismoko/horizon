//! Potent sulfur's pass grid, geyser states, level-seed random and areas, which need no Game.
use sunrise_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::{PositionalRandom, RandomSource, XoroshiroRandom},
};
use sunrise_level::block_entities::Pos;
use sunrise_sim::{
    join_seq::{JoinList, ListStamp},
    mobs::{Mob, seen_grid::SeenGrid},
};

/// The mobs' boxes for one block entity pass, built when a geyser's gas first
/// looks for mobs and kept until the mob list's next write (a spawner's mob, a
/// launch), so every geyser of the pass reads its neighbours, not every mob.
#[derive(Default)]
pub struct PassMobs {
    grid: Option<(ListStamp, SeenGrid)>,
}
impl PassMobs {
    /// The list was written without moving, adding or removing a mob.
    pub fn wrote(&mut self, mobs: &JoinList<Mob>) {
        if let Some((stamp, _)) = &mut self.grid {
            *stamp = mobs.stamp();
        }
    }

    /// Forget the grid: mobs may have moved, been added or removed unseen.
    pub fn clear(&mut self) {
        self.grid = None;
    }

    /// The grid of the mobs' boxes, rebuilt when the list was written since it was built.
    pub fn grid_for(&mut self, mobs: &JoinList<Mob>) -> &SeenGrid {
        if self
            .grid
            .as_ref()
            .is_none_or(|(stamp, _)| *stamp != mobs.stamp())
        {
            self.grid = Some((mobs.stamp(), SeenGrid::new(mobs.iter().map(Mob::bounds))));
        }
        &self.grid.as_ref().expect("grid just built").1
    }
}

/// `PotentSulfurBlockEntity.GEYSER_SALT`.
pub const GEYSER_SALT: i64 = -904_011_478;
/// `PotentSulfurBlockEntity.EFFECT_APPLICATION_FREQUENCY_TICKS`.
pub const EFFECT_APPLICATION_FREQUENCY_TICKS: i64 = 10;
/// `PotentSulfurBlockEntity.EFFECT_DURATION_IN_TICKS`.
pub const EFFECT_DURATION_IN_TICKS: i32 = 80;
/// `PotentSulfurBlockEntity.PARTICLE_FREQUENCY_TICKS`.
pub const PARTICLE_FREQUENCY_TICKS: i64 = 20;
/// `PotentSulfurBlockEntity.SOUND_FREQUENCY_TICKS`.
pub const SOUND_FREQUENCY_TICKS: i64 = 40;
/// `PotentSulfurState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Geyser {
    Dry,
    Wet,
    Dormant,
    Erupting,
    Continuous,
}

/// `PotentSulfurBlockEntity.geyserPositional`: the random of the sulfur block
/// at `pos`, forked from the level seed and the geyser salt.
pub fn geyser_positional(seed: i64, pos: Pos) -> XoroshiroRandom {
    let mut base = XoroshiroRandom::new(seed ^ GEYSER_SALT);
    base.fork_positional()
        .at(BlockPos::new(pos.0, pos.1, pos.2))
}

/// `PotentSulfurBlockEntity.getNearbyLivingEntities`: the box of the source
/// block grown 2.5 blocks sideways.
pub fn nearby_living_area(source: [i32; 3]) -> Aabb {
    let [x, y, z] = source.map(f64::from);
    Aabb::new(
        Vec3::new(x - 2.5, y, z - 2.5),
        Vec3::new(x + 1.0 + 2.5, y + 1.0, z + 1.0 + 2.5),
    )
}

/// The box `PotentSulfurBlockEntity.LAUNCH_ENTITY_TICKER` pushes entities in:
/// the block above the sulfur, `expandTowards` the unobstructed height.
pub fn launch_area(pos: Pos, height: i32) -> Aabb {
    let (x, y, z) = (f64::from(pos.0), f64::from(pos.1 + 1), f64::from(pos.2));
    let up = f64::from(height - 1);
    let (min_y, max_y) = if up < 0.0 {
        (y + up, y + 1.0)
    } else {
        (y, y + 1.0 + up)
    };
    Aabb::new(Vec3::new(x, min_y, z), Vec3::new(x + 1.0, max_y, z + 1.0))
}
