use sunrise_items::items::ItemStack;

/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// A brewing stand at `pos` is about to brew (`PotionBrewEvent.Pre`,
    /// in `BrewingStandBlockEntity.doBrew`): `stacks` is a copy of its five
    /// slots (three bottles, the ingredient, the fuel). Set `cancel` to skip
    /// vanilla's brew: the stand then takes `stacks` as the mods left them,
    /// uses no ingredient and plays no brew sound, and `potion_brew_post`
    /// sees those slots if at least one slot still matches its prior value.
    /// Uncanceled, changes to `stacks` are dropped. Server-side.
    pub(in crate::game) fn native_potion_brew_pre(
        &mut self,
        pos: [i32; 3],
        stacks: &mut [Option<ItemStack>; 5],
        cancel: &mut bool,
    ) {
        self.fire_potion_brew_pre(pos, stacks, cancel);
    }
}
