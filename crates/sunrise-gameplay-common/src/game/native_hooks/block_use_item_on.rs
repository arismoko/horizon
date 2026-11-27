use sunrise_world::registry::BlockStateId;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `BlockBehaviour.useItemOn`: the block's own behaviour, with the
    /// held item, on both logical sides. An answer replaces the
    /// template's behaviour; `TryWithEmptyHand` tries `useWithoutItem` on
    /// the main hand, and `Pass` goes on to the item's `useOn`.
    fn native_use_item_on(
        &mut self,
        state: BlockStateId,
        click: &mod_api::BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_use_item_on(self, state, click, answer);
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
