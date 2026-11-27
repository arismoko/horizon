//! The `living_entity_use_item_stop` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `LivingEntityUseItemEvent.Stop`: `entity` lets go of
    /// `stack` in `hand` before its use ran its time
    /// (`LivingEntity.releaseUsingItem`: the use button let go, a screen
    /// opened), `duration` the ticks left. Set `canceled` to skip the item's
    /// release (a bow doesn't shoot, a spyglass makes no sound,
    /// `release_using` isn't called); the use stops anyway. Not when
    /// the hand changed item, which stops the use without a release. Both
    /// sides, as `living_entity_use_item_tick`.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_living_entity_use_item_stop",
        dispatch = "flat",
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "LivingEntityUseItemEvent.Stop",
        cancellable = true,
        version = 2
    )]
    fn living_entity_use_item_stop(
        stack: &ItemStack,
        entity: Option<i32>,
        hand: Hand,
        duration: i32,
        canceled: &mut bool,
    );
}
