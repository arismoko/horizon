use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    /// `IItemExtension.getXpRepairRatio`: the durability points Mending
    /// restores to `stack` per point of `experience`, arriving as vanilla's.
    /// Server-side, at orb pickup.
    pub(in crate::game) fn native_xp_repair_ratio(
        &self,
        player: u128,
        stack: &ItemStack,
        experience: u32,
        ratio: &mut f32,
    ) {
        self.fire_xp_repair_ratio(player, stack, experience, ratio);
    }
}
