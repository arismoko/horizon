use super::*;

impl GameLevel<'_> {
    /// Whether a piston may move the block `state` at `pos`, with its block
    /// entity if it has one (`IBlockExtension.getRelocability`), asked at the
    /// last step of `PistonBaseBlock.isPushable` for a block that got past its
    /// push reaction: `Some(false)` keeps it in place, `Some(true)` moves it
    /// with its block entity, and left `None` vanilla's rule holds (a
    /// block with a block entity does not move). Both sides; only the block's
    /// own part is asked.
    #[inline(always)]
    pub(in crate::game) fn native_relocability(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        movable: &mut Option<bool>,
    ) {
        self.fire_relocability(dimension, pos, state, movable);
    }
}
