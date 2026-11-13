//! The `item_container` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Item `item` (`mod_id:name`) holds items (`Capabilities.Item.ITEM`): set
    /// `slots` when the item is yours, for a backpack, a storage cell or a
    /// toolbox. Clamped to 1..=256, `ItemContainerContents`' limit; 0 leaves
    /// the item without slots, and vanilla's items have none. Asked at each
    /// call of the item container helpers (`native_item_container_slots`,
    /// `native_item_container_insert`, `native_item_container_extract`, in
    /// `item_container.rs`). The contents are kept in the stack's
    /// `minecraft:container`, as a shulker box item keeps them.
    #[hook(
        side = "both",
        contract = "replace",
        out = "slots",
        host = "read",
        neoforge = "Capabilities.Item.ITEM"
    )]
    fn item_container();
}
