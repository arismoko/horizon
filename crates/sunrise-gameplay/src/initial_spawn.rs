//! The loaded world as the initial spawn searches query it.
use sunrise_core::{
    geometry::{Direction, ShapeOp, Vec3, VoxelShape},
    position::BlockPos,
};
use sunrise_level::{assets::Properties, collisions::SceneCollisions, world::World};
use sunrise_sim::world_spawn::SpawnLevel;
use sunrise_world::{
    block_collision::BlockCollisionRegistry, collision_context::CollisionContext,
    heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates,
};

/// The loaded world as the spawn searches query it.
pub struct Columns<'a, 'r> {
    pub world: &'a World,
    pub properties: &'a Properties,
    pub collision: &'a BlockCollisionRegistry<'r>,
    pub has_ceiling: bool,
    /// `ChunkGenerator.getSpawnHeight` (64 for the vanilla generators).
    pub spawn_height: i32,
}
impl SpawnLevel for Columns<'_, '_> {
    fn min_y(&self) -> i32 {
        self.world.bottom_y()
    }
    fn height(&self, kind: HeightmapType, x: i32, z: i32) -> i32 {
        (self.world.bottom_y()..self.world.top_y())
            .rev()
            .find(|&y| kind.is_opaque(self.properties.heightmap_state(self.world.block(x, y, z))))
            .map_or(self.world.bottom_y(), |y| y + 1)
    }
    fn has_fluid(&self, x: i32, y: i32, z: i32) -> bool {
        use sunrise_world::heightmap::HeightmapState;
        self.properties
            .heightmap_state(self.world.block(x, y, z))
            .has_fluid()
    }
    fn top_face_full(&self, x: i32, y: i32, z: i32) -> bool {
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let state = self.world.block(x, y, z);
        let Ok(shape) = self.collision.collision_shape_with_context(
            &mut scene,
            state,
            BlockPos::new(x, y, z),
            &CollisionContext::empty(false),
        ) else {
            return false;
        };
        shape.face_shape(Direction::Up).is_ok_and(|face| {
            VoxelShape::block()
                .join_is_not_empty(&face, ShapeOp::XOR)
                .is_ok_and(|differs| !differs)
        })
    }
    fn has_ceiling(&self) -> bool {
        self.has_ceiling
    }
    fn spawn_height(&self) -> i32 {
        self.spawn_height
    }
}

/// Vec3.atBottomCenterOf.
pub fn bottom_center(pos: [i32; 3]) -> Vec3 {
    Vec3::new(
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    )
}
