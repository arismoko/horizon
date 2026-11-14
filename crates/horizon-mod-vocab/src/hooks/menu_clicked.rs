//! The `menu_clicked` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0034

pub trait Hooks {
    /// A click in a menu a mod opened (`native_open_menu`), which moved
    /// nothing: the menu's `slot` (its own first, then the player's
    /// inventory, `native_menu_inventory_slot`; -999 outside), the `button`
    /// and the click's `mode` (`ClickType`'s id) of the player `uuid`.
    /// No player's state is swapped in: the menu helpers take the player's
    /// UUID. For a player still connected, the mods are told inside an
    /// action: what a listener writes through the `Host` stays when it
    /// returns, and is undone, with the rest of the action, when a listener
    /// fails (panics or traps). A click drained after its player left is
    /// still told, outside an action.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (vanilla AbstractContainerMenu.clicked; NeoForge adds no event or extension for it)"
    )]
    fn menu_clicked();
}
