//! The `item_stacked_on_other` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `ItemStackedOnOtherEvent`, before either item override. It runs inside a pickup
    /// click, with the cursor and both stacks frozen in `event`. Answer in `result`: `canceled`
    /// skips the bundle override, and a cancelled click that is also `handled` is done, with the
    /// stacks `result` sets (a cancelled click not `handled` goes on to the plain pickup; setting
    /// a stack on any other answer refuses the click). A write through the `Host` to what the
    /// click read makes it stale: the click is refused and every write is undone. A panic or an
    /// invalid answer does the same. A refused click changes nothing.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "canceled",
        neoforge = "ItemStackedOnOtherEvent",
        cancellable = true,
        version = 3
    )]
    fn item_stacked_on_other();
}
