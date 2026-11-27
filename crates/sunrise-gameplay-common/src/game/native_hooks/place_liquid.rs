use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_can_place_liquid(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        fluid: mod_api::FluidKey,
        user: Option<mod_api::LivingRef>,
        answer: &mut Option<bool>,
    ) {
        self.fire_can_place_liquid(pos, state, fluid, user, answer);
    }

    pub(in crate::game) fn native_place_liquid(
        &mut self,
        ticks: &mut Ticks,
        pos: BlockPos,
        state: BlockStateId,
        fluid: mod_api::FluidState,
        answer: &mut Option<bool>,
    ) {
        self.fire_place_liquid(ticks, pos, state, fluid, answer);
    }
}
