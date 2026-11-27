use super::*;
use crate::game::ServerGame;

impl ServerGame<'_> {
    /// `IBlockExtension.getMapColor` for the block `state` at `pos`, asked by `MapItem.update`.
    #[inline(always)]
    pub(in crate::game) fn native_map_color(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        color: &mut u8,
    ) {
        self.fire_map_color(dimension, pos, state, color);
    }
}
