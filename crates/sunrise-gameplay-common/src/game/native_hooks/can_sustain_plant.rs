use super::*;

impl LevelView<'_> {
    /// `IBlockExtension.canSustainPlant` for the soil `ground` at `ground_pos`
    /// and the plant `state` above it, asked before the plant answers its own
    /// `mayPlaceOn`: `Some(true)` lets the plant stand there, `Some(false)`
    /// refuses it, and left `None` the plant's own rule answers. Answer only for
    /// your own soil. The level is read-only. Asked on both sides: a client asks
    /// it from its placement prediction and its own shape updates.
    #[inline(always)]
    pub(in crate::game) fn native_can_sustain_plant(
        &self,
        dimension: LevelDimension,
        state: BlockStateId,
        ground: BlockStateId,
        ground_pos: BlockPos,
        sustains: &mut Option<bool>,
        inert: &mut bool,
    ) {
        let pos = BlockPos::new(ground_pos.x, ground_pos.y.saturating_add(1), ground_pos.z);
        let Some(_active) = super::survival_query_guard::enter_part(
            self.world,
            pos,
            state,
            super::survival_query_guard::PartHook::CanSustainPlant,
        ) else {
            return;
        };
        self.fire_can_sustain_plant(dimension, state, ground, ground_pos, sustains, inert);
    }
}
