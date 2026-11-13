use super::*;

impl GameLevel<'_> {
    /// `IBlockExtension.onBlockStateChange`: a write through `Level.setBlock` held and the
    /// block at `pos` went from `old` to `new`; the parts of `state`'s block hear it, at the
    /// end of the write. Both sides; the level is read-only.
    #[inline(always)]
    pub(in crate::game) fn native_on_block_state_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        self.fire_on_block_state_change(dimension, pos, state, old, new);
    }

    /// `LevelAccess::observe_block_state_change`, and a client's own block writes: the parts
    /// of `old`'s block hear the change, then the parts of `new`'s block when the block
    /// changed. A panicking part is contained with a warning.
    pub(in crate::game) fn block_state_change(
        &self,
        pos: BlockPos,
        old: BlockStateId,
        new: BlockStateId,
    ) {
        let changed = self.behaviours.block(old) != self.behaviours.block(new);
        for state in [Some(old), changed.then_some(new)].into_iter().flatten() {
            if !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::OnBlockStateChange,
                state,
            ) {
                continue;
            }
            if let Err(error) = horizon_client::crash::contain(|| {
                self.native_on_block_state_change(self.dimension, pos, state, old, new);
            }) {
                log::warn!("on_block_state_change callback failed: {error}");
            }
        }
    }
}
