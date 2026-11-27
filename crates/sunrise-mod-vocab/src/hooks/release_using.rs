//! The `release_using` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Item.releaseUsing` of `stack`: `entity` let it go with `time_left`
    /// ticks of its `use_duration` left, so it was held for
    /// `duration - time_left` (a bow-like item shoots by that charge). Set
    /// `used` as vanilla's return, and the stack's use cooldown starts
    /// (`ItemStack.releaseUsing`). After the vanilla item's own release, once
    /// `living_entity_use_item_stop` let it through. Both sides, as
    /// vanilla's: change the world only where this `Game` runs the world,
    /// not on a server's client. Vanilla's, with no NeoForge counterpart
    /// (`LivingEntityUseItemEvent.Stop` is `living_entity_use_item_stop`);
    /// `on_stop_using` hears every stop, this one included.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_release_using",
        dispatch = "flat",
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        version = 2
    )]
    fn release_using(stack: &ItemStack, entity: Option<i32>, time_left: i32, used: &mut bool);
}
