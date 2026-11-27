use sunrise_items::items::ItemStack;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// NeoForge's `PotionBrewEvent.Post` in `BrewingStandBlockEntity.doBrew`.
    /// `stacks` contains the renewed bottles and the as-yet unconsumed ingredient.
    /// The stand takes the edited slots before consuming the original ingredient
    /// and producing its remainder and brew sound. If Pre cancels and at least
    /// one slot matches its prior value, Post sees the replacement slots; a
    /// canceled Pre never consumes the ingredient. Server-side.
    pub(in crate::game) fn native_potion_brew_post(
        &mut self,
        pos: [i32; 3],
        stacks: &mut [Option<ItemStack>; 5],
    ) {
        self.fire_potion_brew_post(pos, stacks);
    }
}
