use super::*;

/// The hook points. Each does nothing in Sunrise.
pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// `PlayerInteractEvent.RightClickItem`, immediately before `Item.use`.
    /// Cancel skips its Part and vanilla behaviour; its result defaults to
    /// `Pass`. On the client `Success`, `SuccessServer` and `Consume` end the
    /// hand loop; only `Success` predicts a swing.
    fn native_right_click_item(
        &mut self,
        click: &ItemClick<'_>,
        cancellation_result: &mut mod_api::InteractionResult,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_right_click_item(
            self,
            click,
            cancellation_result,
            canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}
