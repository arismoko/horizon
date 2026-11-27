use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `BlockBehaviour.useWithoutItem`: the captured state's empty-hand method,
    /// after useItemOn asks for it, on both logical sides and the main hand only.
    fn native_use_without_item(
        &mut self,
        state: BlockStateId,
        click: &BlockClick<'_>,
        answer: &mut Option<mod_api::InteractionResult>,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_use_without_item(
            self, state, click, answer,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
