pub use sunrise_client_gameplay::motion_ids::motion_at;

/// A block entity's `mark_motion` id: its position, packed into the 32 bits beside the tag
/// (x and z wrap at 2048 blocks and y at 1024; two block entities that far apart in one frame's
/// view are told apart by their vertex counts, as `PackEntities` matches only equal runs).
pub(in crate::game) fn motion_block_entity(pos: [i32; 3]) -> u64 {
    motion_at(sunrise_gameplay_common::game::MOTION_BLOCK_ENTITY, pos)
}
