use super::*;
use crate::game::spill_common::RemovalEffects;
use mod_api::BlockEntityDataView;

impl GameLevel<'_> {
    /// Lend the retiring row and the removal effect sink.
    #[inline(always)]
    pub(in crate::game) fn native_pre_remove_side_effects(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        entity: BlockEntityDataView<'_>,
        effects: &mut RemovalEffects<'_>,
    ) {
        self.fire_pre_remove_side_effects(pos, state, entity, |listen| {
            effects.register_listener(listen);
        });
    }
}
