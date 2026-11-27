use super::*;

impl GameLevel<'_> {
    /// The fluid at `pos` is about to run its scheduled tick (`FlowingFluid.tick`). `fluid` is
    /// `water`, `lava` or a mod fluid's id. Set `cancel` and the tick does nothing at all: no
    /// level change, no spreading and no new tick. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_tick(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        cancel: &mut bool,
    ) {
        self.fire_fluid_tick(dimension, pos, fluid, cancel);
    }
}
