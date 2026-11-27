use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `UseItemOnBlockEvent`: after a phase was chosen, before its Part or
    /// vanilla behaviour. Cancel skips that behaviour and returns
    /// `cancellation_result` (initially `TryWithEmptyHand`). In `ItemBeforeBlock`,
    /// `Pass` continues to the block, skipping the item's `on_item_use_first`
    /// Part; only `Fail` or a consuming result ends the block interaction.
    /// A block-phase `TryWithEmptyHand` can still try the empty-hand behaviour.
    fn native_use_item_on_block(
        &mut self,
        click: &BlockClick<'_>,
        phase: UsePhase,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_use_item_on_block(
            self,
            click,
            phase,
            cancellation_result,
            canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
