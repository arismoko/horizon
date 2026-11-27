//! The `item_name` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Vanilla's `Item.getName(ItemStack)`: the name of `stack` from its
    /// components, so one mod item has many names (a data model named after
    /// the mob in it, a filled container after its contents). `name` arrives
    /// as `None`, which keeps the default: the stack's `minecraft:item_name`
    /// component, else the item's translated name. Set it to `Some(text)` to
    /// name the stack; a translatable `Text` (`Text::translate`) renders in
    /// each viewer's own language. It replaces the default rather than
    /// filling in for a missing `item_name`, as the vanilla items that
    /// override `getName` do. A `minecraft:custom_name`, or a written book's
    /// title, still wins over it (`ItemStack.getHoverName`), and the part
    /// isn't asked then.
    ///
    /// Read wherever the game names a stack, once per naming: tooltips, the
    /// selected item's name above the hotbar, command feedback such as
    /// `/give`'s, the anvil's rename box and its comparison on both sides,
    /// stats, the recipe book's and the creative inventory's search, recipe
    /// screens and `Host::hover_name`. It is given the loop `context` of the
    /// side naming the stack and takes no `Host`, so it can't name a stack
    /// again: answer from the stack and `context` alone, and fast. Only the
    /// `ItemPart`s attached to `stack`'s item are asked, and the last to set
    /// `name` wins; an item with none costs one table lookup and the stack
    /// isn't converted.
    /// Read the borrowed stack with `get`; call `project` only to keep an owned copy.
    #[hook(
        side = "both",
        contract = "collect",
        out = "name",
        context = "loop",
        scope = "item",
        key = "stack",
        target = "sunrise_sim::stations::native_item_name",
        version = 2,
        neoforge = "Item.getName"
    )]
    fn item_name(stack: ItemStackView<'_>, name: &mut Option<Text>);
}
