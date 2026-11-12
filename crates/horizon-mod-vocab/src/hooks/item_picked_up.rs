//! The `item_picked_up` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0050

pub trait Hooks {
    /// `player` touches `count` of `item` on the ground
    /// (`ItemEntity.playerTouch`), before its pickup delay and target are
    /// checked: it is asked on every tick the two touch, even while the item
    /// can't be taken. Leave `can_pickup` at `Default` for vanilla's rule,
    /// set `True` to take it despite its delay or another player's target
    /// (a full inventory still leaves it), or `False` to leave it there: the
    /// fork sets a vanilla eligibility delay of at least 10 ticks. The hook
    /// still runs on each touch, and a later `True` overrides that delay. The last mod to set it
    /// decides. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "ItemEntityPickupEvent.Pre"
    )]
    fn item_picked_up();
}
