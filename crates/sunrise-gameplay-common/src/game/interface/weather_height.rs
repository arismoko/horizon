use sunrise_level::{assets::Properties, world::World};
use sunrise_world::{heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates};

/// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)` from the chunk's
/// heightmap: one above the highest block in `blocks_motion_in_heightmap` or
/// holding a fluid, however far above the camera it is.
pub fn motion_blocking_height(world: &World, properties: &Properties, x: i32, z: i32) -> i32 {
    world.motion_blocking(x, z, |s| {
        HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
    })
}
