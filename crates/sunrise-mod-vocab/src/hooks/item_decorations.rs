//! The `item_decorations` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Append an item's custom decorations after its default bar, cooldown and count.
    /// `draw` lends a [`GuiCanvas`](crate::GuiCanvas), not a full drawing context. The icon's
    /// top-left `(x, y)` is in the caller's local GUI pixels, outside the model-only squeeze.
    /// Inventory, hotbar and tooltip draws lend an identity pose. Appended draws belong to
    /// the enclosing group; use `draw.scope` to make just your decoration draws all-or-nothing.
    /// `draw.item_decorations` on this canvas or its children paints only the standard suffix,
    /// including the `item_bar` override. It does not invoke item decorators again.
    /// Use ordinary HUD or screen hooks for widgets, scenes, focus and narration.
    /// The NeoForge counterpart is `IItemDecorator.render`; registration through
    /// `RegisterItemDecorationsEvent` is a separate moment.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_item_decorations",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "IItemDecorator.render",
        version = 3
    )]
    fn item_decorations(stack: &ItemStack, x: f32, y: f32, draw: &mut dyn GuiCanvas);
}
