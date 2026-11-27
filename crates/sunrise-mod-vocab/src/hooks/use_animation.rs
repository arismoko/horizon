//! The `use_animation` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Item.getUseAnimation` of `stack`: the animation the renderer draws while the stack is in
    /// use, which `ItemStack::use_animation` also reads. `animation` arrives with the item's
    /// vanilla answer; the item's Part may change it. A `Spear` override on an item without
    /// kinetic data is refused as `None`. Asked on the client when each render consumer
    /// updates, and on either side when a mod reads `ItemStack::use_animation`, with `host`
    /// read-only: answer from the stack alone, the same on both sides. From inside this
    /// callback, a read of `ItemStack::use_animation` for any stack of the same item (a copy
    /// with another count or components included) does not ask its Part again: it gets the
    /// item's own answer. Another item's read asks that item's Parts.
    /// Contract version 3: version 2 asked no Part again for any read from inside the callback;
    /// version 1 was asked on the client only, with a `ClientHost`.
    #[hook(
        target = "sunrise_gameplay_common::game::native_hooks::base::GameHooks::native_use_animation",
        receiver = "read",
        dispatch = "flat",
        side = "both",
        version = 3,
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "none (vanilla Item.getUseAnimation; NeoForge adds no event or extension for it)"
    )]
    fn use_animation(stack: &ItemStack, animation: &mut UseAnimation);
}
