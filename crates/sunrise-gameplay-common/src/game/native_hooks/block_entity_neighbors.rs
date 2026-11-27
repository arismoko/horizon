use super::*;
impl GameLevel<'_> {
    pub(in crate::game) fn native_on_neighbor_change(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        neighbor: BlockPos,
    ) {
        self.fire_on_neighbor_change(dimension, pos, state, neighbor);
    }
    pub(in crate::game) fn native_weak_changes(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        receive: &mut bool,
    ) {
        self.fire_weak_changes(dimension, pos, state, receive);
    }
}

impl sunrise_world::level::BlockEntityNeighbors for GameLevel<'_> {
    #[inline]
    fn observe_block_entity_neighbor(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        neighbor: BlockPos,
    ) {
        if self.native_authoritative {
            self.native_on_neighbor_change(self.dimension, pos, state, neighbor);
        }
    }
    #[inline]
    fn receives_weak_changes(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let mut receive = false;
        if self.native_authoritative {
            self.native_weak_changes(self.dimension, pos, state, &mut receive);
        }
        receive
    }
}
