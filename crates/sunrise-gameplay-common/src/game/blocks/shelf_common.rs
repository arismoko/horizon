//! Common shelf front-face hit test shared by server swaps and client prediction.
use super::Hit;
use sunrise_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use sunrise_gameplay_common::hoppers::dir_from_name;
use sunrise_level::containers::BlockEntity;
use sunrise_world::level::hit_slot;

impl crate::game::CommonGame<'_> {
    /// Slot, facing, and powered flag for a click on a shelf's front face.
    pub fn shelf_hit(&self, hit: &Hit) -> Option<(usize, Direction, bool)> {
        let p = hit.pos;
        if !matches!(
            self.world.block_entities.get(&(p[0], p[1], p[2])),
            Some(BlockEntity::Shelf(_))
        ) {
            return None;
        }
        let state = self.world.block(p[0], p[1], p[2]);
        let data = self.pack.assets.registry.state(state)?;
        let facing = data
            .properties
            .get("facing")
            .and_then(|s| dir_from_name(s))?;
        let face = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == hit.face)?;
        let slot = hit_slot(
            facing,
            face,
            BlockPos::new(p[0], p[1], p[2]),
            Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            1,
            3,
        )?;
        Some((
            slot,
            facing,
            data.properties.get("powered").is_some_and(|s| s == "true"),
        ))
    }
}
