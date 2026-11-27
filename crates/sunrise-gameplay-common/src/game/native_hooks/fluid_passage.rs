use super::*;

impl GameLevel<'_> {
    /// May `fluid` flow from `from` one step toward `direction`, into `to`
    /// (`FlowingFluid.canMaybePassThrough`)? Asked only after the game's own
    /// answer was true, so `passes` starts true and a listener can only close
    /// the side. Server-side.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::game) fn native_fluid_passage(
        &self,
        dimension: LevelDimension,
        fluid: &str,
        from: BlockPos,
        from_state: BlockStateId,
        direction: Direction,
        to: BlockPos,
        to_state: BlockStateId,
        passes: &mut bool,
    ) {
        self.fire_fluid_passage(
            dimension, fluid, from, from_state, direction, to, to_state, passes,
        );
    }
}
