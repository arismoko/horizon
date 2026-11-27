//! The `inventory_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server tick of the stack a player carries in `slot`, where `Inventory.tick`
    /// calls `Item.inventoryTick`. `slot` uses the saved inventory numbers (0 to 35;
    /// armor 100 feet to 103 head; -106 off hand), and `selected` marks the held slot.
    /// `place` addresses that live slot: `ItemPlace::with_stack` lends its current
    /// stack, `ItemPlace::edit` edits its components within a transaction, and the
    /// ordinary slot operations can empty it. Later reads see earlier writes, but
    /// only Parts of the original `item` are called for this tick. Server-side,
    /// including player ticks while the world is frozen.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_inventory_tick",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "item",
        key = "item",
        version = 2,
        neoforge = "none (vanilla Item.inventoryTick; NeoForge adds no event or extension for it); PlayerTickEvent ticks the player, not each stack"
    )]
    fn inventory_tick(
        item: mod_api::ItemKey,
        place: &mod_api::ItemPlace,
        slot: i32,
        selected: bool,
    );
}
