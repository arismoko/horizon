/// The hook points. Each does nothing in Sunrise.
impl crate::game::ServerGame<'_> {
    /// `player` tosses the item entity `item` (`ItemTossEvent`, from
    /// `Player.drop`: the drop key, a menu's throw, what no longer fits),
    /// not what scatters when they die. The entity is made but not yet in
    /// the level: the `item_entity_*` methods read and change it (its stack,
    /// pickup delay, target), and what they change stays. Set `canceled` and
    /// it never joins the level; the items are already out of the inventory
    /// and are lost. Server-side.
    pub(in crate::game) fn native_item_toss(
        &mut self,
        player: u128,
        item: ItemEntityRef,
        canceled: &mut bool,
    ) {
        self.fire_item_toss(player, item, canceled);
    }
}
