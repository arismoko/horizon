//! The `item_expire` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A dropped item reaches its lifespan on the server (`ItemEntity.tick`,
    /// NeoForge's `EventHooks.onItemExpire` / `ItemExpireEvent`), before
    /// removal. Every mod may set or add to `extra_life`, in ticks, which
    /// starts at zero, and may query or mutate the live item entity through
    /// `event.item` (including lending its stack with
    /// `event.item.with_stack(host, ...)` and reading entity state via
    /// `event.item.entity()`). This is an expiry attempt, not a removal notice:
    /// it runs even when a listener extends the item's life. The item's
    /// `lifespan` (default 6000) gets `extra_life` by signed 32-bit wrapping
    /// addition, then is clamped to `0..=32766` (`Short.MAX_VALUE - 1`)
    /// without rewinding `age`. If `age >= lifespan`, the item is discarded
    /// and `entity_leave_level` fires before the next drop ticks; otherwise
    /// this hook fires again at the updated lifespan. Extensions through this
    /// event cannot exceed a total lifespan of 32766 ticks; use unlimited or
    /// extended lifetime (`ItemDrop::UNLIMITED_LIFETIME`) for longer lifetimes.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_item_expire",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "ItemExpireEvent",
        cancellable = false,
        version = 2
    )]
    fn item_expire(item: ItemEntityRef, extra_life: &mut i32);
}
