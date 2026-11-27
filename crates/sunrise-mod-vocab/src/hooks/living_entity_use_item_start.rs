//! The `living_entity_use_item_start` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Start`: `entity` starts using
    /// `stack` in `hand` (`LivingEntity.startUsingItem`), for `duration`
    /// ticks (`use_duration`'s answer). `entity` is the user's
    /// `Entity.getId`: a player's (the acting one, `host.players().acting_player()`,
    /// on the server; the local player on a server's client) or a mob's; a mob fires it only for `LivingRef::start_using_item`. Change `duration` (consumables, potions,
    /// spyglasses and mods' items count to it; below 0 refuses the use, and
    /// 0 runs it out on its first tick, as `living_entity_use_item_tick`
    /// running it out, unless the item's own is 0 already), or set
    /// `canceled` and the use doesn't start. Both sides, as
    /// vanilla's: on the server for every player of the world, and on a
    /// server's client for the uses it predicts. Answer the same on both,
    /// and change the world only on the server (`native_window` is false
    /// there), not on a server's client.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_living_entity_use_item_start",
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Start",
        cancellable = true,
        version = 2
    )]
    fn living_entity_use_item_start(
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: &mut i32,
        canceled: &mut bool,
    );
}
