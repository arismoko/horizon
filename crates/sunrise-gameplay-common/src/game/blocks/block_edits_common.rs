//! Common loaded-column heightmap queries shared by server block edits and client views.
use mod_api::Heightmap;
use sunrise_level::{
    chunk::{Chunk, LevelShape},
    world::World,
};
use sunrise_world::{
    heightmap::HeightmapType, registry::BlockStateId, worldgen::terrain::TerrainBlockStates,
};

/// A change `World::edit_runs` reported: position, previous and new state.
pub type Tracked = ((i32, i32, i32), u32, u32);

impl crate::game::CommonGame<'_> {
    /// One above the highest block of column (x, z) that counts on heightmap `h`, or the bottom of
    /// the column (also where it is not loaded).
    pub fn edit_height(&self, h: Heightmap, x: i32, z: i32) -> i32 {
        self.world_column_height(&self.world, h, x, z)
    }

    /// One above the highest block of column (x, z) in `world` that counts on heightmap `h`, or
    /// the bottom of the column (also where it is not loaded).
    pub fn world_column_height(&self, world: &World, h: Heightmap, x: i32, z: i32) -> i32 {
        let properties = &self.pack.assets.properties;
        column_height(
            world,
            h,
            (x, z),
            |s| self.res.facts().is_air(s),
            |t, s| t.is_opaque(properties.heightmap_state(s)),
        )
    }
}

/// The vertical span `below + above` for one column read, bounded by the level's build height
/// (`LevelHeightAccessor.getHeight`).
pub fn column_span(below: u32, above: u32, height: i32) -> Option<usize> {
    below
        .checked_add(above)
        .filter(|&per| per > 0 && i64::from(per) <= i64::from(height))
        .map(|per| per as usize)
}

/// One above the highest block of column `(x, z)` on heightmap `h`.
pub fn column_height(
    world: &World,
    h: Heightmap,
    (x, z): (i32, i32),
    air: impl Fn(BlockStateId) -> bool,
    counts: impl Fn(HeightmapType, BlockStateId) -> bool,
) -> i32 {
    // The chunk storage is padded past a shorter level (the Nether starts at 0, not -64): an
    // empty column's bottom is the level's own minimum.
    column_height_in_storage(world, h, (x, z), air, counts).max(world.shape.min_y)
}

fn column_height_in_storage(
    world: &World,
    h: Heightmap,
    (x, z): (i32, i32),
    air: impl Fn(BlockStateId) -> bool,
    counts: impl Fn(HeightmapType, BlockStateId) -> bool,
) -> i32 {
    if h == Heightmap::WorldSurface {
        return world.world_surface(x, z, air);
    }
    let top = world.motion_blocking(x, z, |s| counts(HeightmapType::MotionBlocking, s));
    let kind = match h {
        Heightmap::MotionBlockingNoLeaves => HeightmapType::MotionBlockingNoLeaves,
        Heightmap::OceanFloor => HeightmapType::OceanFloor,
        _ => return top,
    };
    let bottom = world
        .chunks
        .get(&(x >> 4, z >> 4))
        .map_or(world.layout().min_y(), |c| c.min_y());
    let mut y = top;
    while y > bottom && !counts(kind, world.block(x, y - 1, z)) {
        y -= 1;
    }
    y
}

/// The blocks of one loaded column from `below` under `base` to `above - 1` over it,
/// bottom first, using `air` outside the chunk's vertical range.
#[allow(clippy::too_many_arguments)]
pub fn fill_column_blocks(
    air: BlockStateId,
    chunk: &Chunk,
    level: LevelShape,
    x: i32,
    z: i32,
    base: i32,
    below: u32,
    above: u32,
    out: &mut Vec<BlockStateId>,
) {
    // The chunk's storage may be taller than the level it belongs to.
    let (bottom, top) = (
        chunk.min_y().max(level.min_y),
        chunk.max_y().min(level.max_y()),
    );
    let (lx, lz) = (x & 15, z & 15);
    let start = i64::from(base) - i64::from(below);
    let count = i64::from(below) + i64::from(above);
    for dy in 0..count {
        let y = start + dy;
        let state = if y >= i64::from(bottom) && y < i64::from(top) {
            chunk.block(lx, y as i32, lz)
        } else {
            air
        };
        out.push(state);
    }
}
