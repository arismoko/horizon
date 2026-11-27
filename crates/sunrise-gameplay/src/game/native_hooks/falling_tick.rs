use mod_api::EntityRef;
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl crate::game::ServerGame<'_> {
    /// `IFallableExtension.fallingTick`: the falling block `entity` of `state`, in the block at
    /// `pos`, ticked and is still falling. Delivered after the ordered pass, with a
    /// `&mut dyn ServerHost`, so the part's writes land on the stored falling block.
    pub(in crate::game) fn native_falling_tick(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        entity: EntityRef,
    ) {
        self.fire_falling_tick(state, pos, entity);
    }
}
