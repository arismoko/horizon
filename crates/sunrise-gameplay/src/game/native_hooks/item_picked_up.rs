impl crate::game::ServerGame<'_> {
    /// `player` touches the ground item entity `item`
    /// (`ItemEntity.playerTouch`), before its pickup delay and target are
    /// checked: it is asked on every tick the two touch, even while the item
    /// can't be taken. Leave `can_pickup` at `Default` for vanilla's rule,
    /// set `True` to take it despite its delay or another player's target
    /// (a full inventory still leaves it), or `False` to leave it there: the
    /// fork sets a vanilla eligibility delay of at least 10 ticks. The hook
    /// still runs on each touch, and a later `True` overrides that delay. The last mod to set it
    /// decides. The handle reads the whole stack on the ground; for the
    /// window's own player of a world it ticks itself, whose pickup pass
    /// lifts the items out of the level, it reads as unavailable. Server-side.
    pub(in crate::game) fn native_item_picked_up(
        &mut self,
        player: u128,
        item: ItemEntityRef,
        can_pickup: &mut TriState,
    ) {
        self.fire_item_picked_up(player, item, can_pickup);
    }

    /// `player` picked up all or part of the stack of the ground item entity
    /// `item` (`ItemEntity.playerTouch`, right after `Inventory.add` took some
    /// of it), the entity `item_picked_up` named. `original` is a copy of the
    /// stack as it lay before the pickup: changing it does nothing.
    /// `remaining` is what the item entity still holds: `None` (empty) after a
    /// whole pickup, the part that did not fit after a partial one. Set a
    /// stack there to leave the entity holding it, or `None` to empty it: the
    /// pickup then counts `original`'s count less what is left. The amount
    /// taken is `original`'s count less `remaining`'s. A mod that removes the
    /// item entity meanwhile keeps it removed: no take animation, the stat and
    /// criteria still run. Not called when `item_picked_up` refused the
    /// pickup or the inventory took nothing; a forced `True` from
    /// `item_picked_up` changes none of this. A creative player always takes
    /// the whole stack. Server-side.
    pub(in crate::game) fn native_item_picked_up_post(
        &mut self,
        player: u128,
        item: ItemEntityRef,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    ) {
        self.fire_item_picked_up_post(player, item, original, remaining);
    }

    /// The item entity `id` as the hooks name it. It need not be in the
    /// level: a canceled toss and the host player's pickup hand out one the
    /// handle reads cannot find.
    pub(in crate::game) fn native_item_handle(&self, id: u32) -> Option<ItemEntityRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        Some(ItemEntityRef::new(EntityRef::new(epoch, id)))
    }
}
