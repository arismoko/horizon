//! The `item_picked_up_post` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `player` picked up all or part of the stack of the ground item entity
    /// `item` (`ItemEntity.playerTouch`, right after `Inventory.add` took some
    /// of it), the entity `item_picked_up` named. `original` is a copy of the
    /// stack as it lay before the pickup: changing it does nothing.
    /// `remaining` is the part of the captured `original` that did not fit,
    /// or `None` if it all fit. If Pre replaced the live stack, that replacement
    /// remains on `item` unless a Post listener changes `remaining`. The amount
    /// taken is `original`'s count less `remaining`'s. A mod that removes the
    /// item entity meanwhile keeps it removed: no take animation, the stat and
    /// criteria still run. Not called when `item_picked_up` refused the
    /// pickup or the inventory took nothing; a forced `True` from
    /// `item_picked_up` changes none of this. A creative player always takes
    /// the whole stack. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::item_picked_up::ServerGame<'_>::native_item_picked_up_post",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ItemEntityPickupEvent.Post",
        version = 6
    )]
    fn item_picked_up_post(
        player: u128,
        item: ItemEntityRef,
        original: &ItemStack,
        remaining: &mut Option<ItemStack>,
    );
}
