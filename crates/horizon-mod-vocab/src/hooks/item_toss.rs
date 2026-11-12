//! The `item_toss` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `ItemTossEvent`: `player` tosses `count` of `item` (`Player.drop`: the drop key, a menu's throw, what no longer fits in the inventory), not what scatters when they die. Set `canceled` and no item entity is made; the items are already out of the inventory and are lost. A canceled toss awards no drop statistic, as vanilla awards one only when an item entity came of it. Server-side. Diverges from upstream: replaces the listen-only `item_dropped`, which fired for the drop key and menu throws only.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemTossEvent",
        cancellable = true
    )]
    fn item_toss();
}

/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"`item_dropped_v2(player, stack, cancel)`: an additive, cancellable version of `native_item_dropped` (NeoForge's `ItemTossEvent`) with the whole stack thrown, its components included (enchantments, unbreakable, attribute modifiers...). Heard just before v1, which keeps its signature and firing point. Set `cancel` and nothing is thrown: the stack goes back into the player's inventory (what does not fit is thrown, unheard) and v1 hears nothing. Bundles that use it need `"hooks": @API:item_dropped_v2@`."###]
    fn item_dropped_v2();
}
