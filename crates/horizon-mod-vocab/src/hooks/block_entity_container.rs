//! The `block_entity_container` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0026

pub trait Hooks {
    /// Block `block`'s block entity has item slots (a `WorldlyContainer`):
    /// hoppers, comparators and breaking the block use them as vanilla's containers.
    /// Set `container` when the block is yours. Asked once for each mod
    /// block when the ticking table is built; at most 256 slots, and slot
    /// numbers past the count are left out. Its `menu`, one of
    /// `containers::MOD_MENUS` or a content mod's menu type
    /// (`data/<id>/horizon/menu/`) with as many
    /// slots, opens when the block is used (its template must open a menu, as
    /// a chest or a furnace does); another is left out. A mod menu type shows
    /// on Horizon clients through the mod's `screen` only.
    #[hook(
        side = "both",
        contract = "replace",
        out = "container",
        host = "read",
        neoforge = "RegisterCapabilitiesEvent"
    )]
    fn block_entity_container();
}
