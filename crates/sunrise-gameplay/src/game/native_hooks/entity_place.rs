use mod_api::{EntityRef, Placement};
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `BlockEvent.EntityPlaceEvent` (and `EntityMultiPlaceEvent`, which
    /// extends it): `placer` places `state` at `pos` against `placed_against`.
    /// Ordinary item and mob placements fire before the write; chosen player
    /// placements fire after their states and block entities are staged.
    /// `placement` is `Multi` when the placement writes more blocks (bed, door,
    /// tall plant). Cancel stops the whole placement. Server-side.
    pub(in crate::game) fn native_entity_place(
        &mut self,
        placer: Option<EntityRef>,
        pos: BlockPos,
        state: BlockStateId,
        placed_against: BlockStateId,
        placement: Placement,
        canceled: &mut bool,
    ) {
        self.fire_entity_place(placer, pos, state, placed_against, placement, canceled);
    }
}
