use super::*;

impl crate::game::CommonGame<'_> {}

pub trait GameHooks<'__sg>: crate::game::SideGame<'__sg> {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override: a pickup click on a menu
    /// slot, with its cursor and clicked slot lent in `click`. A listener sets `canceled` to skip
    /// both item overrides and the bundle override; `click.handled` then says whether the click is
    /// done or plain pickup still runs. Authoritative server side only; only the `ItemPart`
    /// callback below runs on both sides.
    #[inline(never)]
    fn native_item_stacked_on_other(
        &mut self,
        click: &mut mod_api::item::menu::StackedOnOther<'_>,
        canceled: &mut bool,
    ) {
        crate::game::native_hooks::fire::FireHooks::fire_item_stacked_on_other(
            self, click, canceled,
        );
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> GameHooks<'__sg> for G {}

pub trait ItemStackedPartGameExt<'g>: crate::game::SideGame<'g> {
    /// `ItemPart::item_stacked_on_other` for `item` (either the carried stack in
    /// `CarriedOnOther` or the clicked stack in `OtherStackedOnMe`), sharing `HookId`, the lent
    /// click, and transaction with `native_item_stacked_on_other`. Both sides (server clicks,
    /// client prediction, and creative inventory-tab/hotbar pickups).
    #[inline(never)]
    fn item_stacked_on_other_part(
        &mut self,
        item: &str,
        click: &mut mod_api::item::menu::StackedOnOther<'_>,
        canceled: &mut bool,
    ) {
        mod_api::dispatch_item_stacked_on_other_part(
            execution_role::host_side(self),
            item,
            click,
            canceled,
        );
    }
}
impl<'g, G: crate::game::SideGame<'g>> ItemStackedPartGameExt<'g> for G {}
