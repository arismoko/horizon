use sunrise_items::items::ItemStack;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn native_player_inventory_changed(
        &mut self,
        player: u128,
        stacks: &[Option<ItemStack>],
    ) {
        self.fire_player_inventory_changed(player, stacks);
    }
}
