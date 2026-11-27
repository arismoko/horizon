//! The `inventory_move_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A hopper is about to move one item between containers, a composter's
    /// bone meal included. Set `cancel` to leave it in its source slot. A chest
    /// boat or minecart is reported at the block it is in.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_inventory_move_item",
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)",
        cancellable = true
    )]
    fn inventory_move_item(source: [i32; 3], destination: [i32; 3], item: &str, cancel: &mut bool);
}
