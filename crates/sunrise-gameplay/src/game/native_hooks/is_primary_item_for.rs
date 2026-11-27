use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// `IItemExtension.isPrimaryItemFor`: is `stack`'s item primary for `enchantment`, so the
    /// enchanting table, loot `enchant_with_levels`, trades and spawn gear may offer it?
    /// `primary` arrives true for a plain book. For any other item, it starts with the
    /// resolved support answer and membership in `primary_items`, if present. The part may
    /// change the answer. Asked once per enchantment on the server when parts install, with
    /// the item's default stack (`resolve_primary_items`).
    #[inline(never)]
    pub(in crate::game) fn native_is_primary_item_for(
        &self,
        stack: &ItemStack,
        enchantment: &str,
        primary: &mut bool,
    ) {
        self.fire_is_primary_item_for(stack, enchantment, primary);
    }
}
