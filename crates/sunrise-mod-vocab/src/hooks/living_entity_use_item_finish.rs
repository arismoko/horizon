//! The `living_entity_use_item_finish` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Finish`: `entity`'s use of
    /// `stack` in `hand` ran its time (`LivingEntity.completeUsingItem`),
    /// `duration` the ticks left, always 0. `entity` is as in
    /// `living_entity_use_item_start`. `result` arrives with what
    /// `finishUsingItem` left in the hand (the bowl of a stew, none for the
    /// last apple; a mod's own item is kept) and may be replaced. Fired for
    /// consumables, potions, spyglasses and mods' items. Both sides, as
    /// vanilla's `completeUsingItem`: answer the same on both, and change the
    /// world only on the server (`native_window` is false there), not on a
    /// server's client. On the server it fires for every player of the world,
    /// `entity` the acting one's id. On a server's client the result
    /// arrives as vanilla's client `finishUsingItem` leaves it: an item that
    /// is not a consumable unchanged (a spyglass), a consumable less one, and
    /// its use remainder (the bowl of the last stew) once that empties the
    /// hand; nothing is used up in creative mode. The hand holds the result,
    /// a mod's replacement included, until the server's inventory update.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_living_entity_use_item_finish",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        neoforge = "LivingEntityUseItemEvent.Finish",
        version = 4
    )]
    fn living_entity_use_item_finish(
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        result: &mut Option<ItemStack>,
    );
}
