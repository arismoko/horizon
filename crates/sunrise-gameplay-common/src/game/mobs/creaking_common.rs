use sunrise_core::position::BlockPos;
use sunrise_level::containers::BlockEntity;

/// Comparator input of a heart (`getAnalogOutputSignal`).
pub(in crate::game) fn heart_signal(
    world: &sunrise_level::world::World,
    pos: BlockPos,
) -> Option<i32> {
    match world.block_entities.get(&(pos.x, pos.y, pos.z))? {
        BlockEntity::CreakingHeart(h) => Some(h.output_signal),
        _ => None,
    }
}
