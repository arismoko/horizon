use super::*;

impl GameLevel<'_> {
    /// A neighbour's `FenceGateBlock.isWall` answer, on both logical sides.
    #[inline(always)]
    pub(in crate::game) fn native_fence_gate_in_wall(
        &self,
        state: BlockStateId,
        answer: &mut Option<bool>,
    ) {
        self.fire_fence_gate_in_wall(state, answer);
    }
}
