use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// Per-stack ingredient and fuel remainder, initialized from item data.
    pub(in crate::game) fn native_crafting_remainder(
        &self,
        stack: mod_api::ItemStackView<'_>,
        remainder: &mut Option<ItemStack>,
    ) {
        self.fire_crafting_remainder(stack, remainder);
    }
}
