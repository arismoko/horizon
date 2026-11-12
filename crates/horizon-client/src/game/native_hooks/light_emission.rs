use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The light the block `state` emits at `pos` in `dimension`
    /// (`IBlockExtension.getLightEmission`), for a block that declares
    /// `dynamic_light: true`. The declaration is per block, not per state as
    /// `hasDynamicLightEmission` is, because it lives on the block's row: every state
    /// of a declared block is asked, and parts on other blocks are never asked, at any
    /// moment (NeoForge asks on a block change whether or not the block declares).
    /// `emission` starts empty: set `Some(level)` to replace the state's own light at
    /// that position, or leave it empty to keep it. An answer above 15 is clamped to
    /// 15 here, so an out-of-range value never reaches the light queue; NeoForge
    /// clamps only the value its light manager stores. Asked only on the thread that
    /// owns the level, both sides, with that level read-only, as a chunk holding the
    /// block comes in and as an edit sets it; the light engine keeps the answer until
    /// the cell's state changes or the chunk unloads, and its workers read only that
    /// answer. Not asked for the block's own rendered light, ambient occlusion or the
    /// spawn rule, which take the state's light. The game may ask more often than
    /// the contract does, so keep the answer a pure function of the level, `pos` and
    /// `state`. Diverges from upstream: new hook.
    #[inline(always)]
    pub(in crate::game) fn native_light_emission(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        emission: &mut Option<u8>,
    ) {
        self.fire_light_emission(dimension, pos, state, emission);
    }
}
