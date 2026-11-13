use super::*;

impl Game<'_> {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override: a pickup click on a menu
    /// slot, with the cursor and both stacks frozen in `event`. A listener sets `canceled` to skip
    /// the bundle override and answers in `result`: `handled` for a cancelled click that is done
    /// (nothing else runs), and the stacks the click leaves in the cursor and the slot. The click
    /// validates the answer once and applies it once. Both sides.
    #[inline(never)]
    pub(in crate::game) fn native_item_stacked_on_other(
        &mut self,
        event: &mod_api::item::menu::StackedOnOther,
        result: &mut mod_api::item::menu::StackedOnOtherResult,
        canceled: &mut bool,
    ) {
        self.fire_item_stacked_on_other(event, result, canceled);
    }
}
