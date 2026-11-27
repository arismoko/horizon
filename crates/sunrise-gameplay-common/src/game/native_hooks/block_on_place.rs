use super::*;

impl GameLevel<'_> {
    /// `BlockBehaviour.onPlace`, from `LevelChunk.setBlockState`: the block `state` was written
    /// at `pos` over `old`, before the block's own `onPlace` handling. A part's writes go
    /// through the level's `ticks`. Server-side.
    #[inline(always)]
    pub fn native_on_place(
        &mut self,
        ticks: &mut Ticks,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        old_state: BlockStateId,
        moved_by_piston: bool,
    ) {
        self.fire_on_place(ticks, dimension, pos, state, old_state, moved_by_piston);
    }
}
