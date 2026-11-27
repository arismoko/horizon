/// The hook point. It does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A player or mob that fell onto farmland is about to turn it to its base block
    /// (`BlockEvent.FarmlandTrampleEvent`): `entity` is the landing living entity, `pos` the
    /// farmland and `state` what it is, `fall_distance` the fall. Cancel and the farmland and
    /// its crop stay as they are. Asked inside the landing before anything is lifted, changed
    /// or hurt by the fall.
    pub(in crate::game) fn native_farmland_trample(
        &mut self,
        entity: mod_api::LivingRef,
        pos: BlockPos,
        state: BlockStateId,
        fall_distance: f64,
        cancel: &mut bool,
    ) {
        self.fire_farmland_trample(entity, pos, state, fall_distance, cancel);
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn apply_farmland_trample(
        &mut self,
        trample: FarmlandTrample,
        still: impl Fn(&Self) -> bool,
        journalled: bool,
    ) -> bool {
        let FarmlandTrample {
            position: pos,
            state,
            base,
            ..
        } = trample;
        if !still(self) {
            return false;
        }
        let at = [pos.x, pos.y, pos.z];
        // A section listener during the lift can change the decided cell too.
        // Validate after all such callbacks, with no callback between this check and the write.
        self.push_entities_up(pos, state, base, |game| {
            still(game)
                && if journalled {
                    game.journal_set_block(at, base)
                } else {
                    game.level_set(at, base).is_ok()
                }
        })
    }
}
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;
