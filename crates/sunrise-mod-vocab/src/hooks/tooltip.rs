//! The `tooltip` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `Item.appendHoverText`: an item's own lines in its tooltip, after its
    /// name and before its components' (enchantments, lore, attributes). Add
    /// lines to `lines`, which arrives empty, each with a colour as `rgba` packs
    /// it (`u32::MAX` keeps the text's own); `Text::legacy` reads `§` codes.
    /// `context` is the same [`TooltipAppendContext`] passed to component appenders:
    /// player, advanced/creative/nested flags and vanilla `Item.TooltipContext.tickRate()`
    /// (the client level's rate, or 20 without a level).
    /// The stack has its components (`stack.data()`). Asked when lines are built for
    /// display or `ItemResource::tooltip`, except during reentrant line queries, when
    /// the stack hides its tooltip or for a crossbow's loaded projectile. NeoForge's
    /// `ItemTooltipEvent` is the corresponding line-building event; this scoped hook adds
    /// attached parts' lines before component lines are assembled.
    /// Afterwards [`Hooks::item_tooltip`]
    /// sees the finished list, including these parts' lines.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_tooltip",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "collect",
        out = "lines",
        scope = "item",
        key = "stack",
        neoforge = "ItemTooltipEvent",
        version = 3
    )]
    fn tooltip(
        stack: &ItemStack,
        context: &mod_api::TooltipAppendContext,
        lines: &mut Vec<(Text, u32)>,
    );
}
