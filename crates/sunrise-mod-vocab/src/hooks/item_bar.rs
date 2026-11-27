//! The `item_bar` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The small bar `GuiGraphicsExtractor.itemDecorations` draws under an
    /// item, wherever the game shows one: `bar` arrives holding vanilla's value
    /// (the damage bar, or a bundle's fullness already applied) and may be
    /// replaced (signed pixel width and RGB colour) or set to `None` to hide it.
    /// Vanilla's bar is 13 pixels wide when full; the foreground endpoint uses
    /// wrapping integer addition, as in `GuiGraphicsExtractor.itemBar`.
    /// Asked each frame a stack is drawn. Client-side.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_item_bar",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (vanilla Item.isBarVisible/getBarWidth/getBarColor; RegisterItemDecorationsEvent registers decorators, not bar queries)",
        version = 2
    )]
    fn item_bar(stack: &ItemStack, bar: &mut Option<(i32, u32)>);
}
