//! The `hopper_inventory_search` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A hopper selected a container while searching for items to push or
    /// pull. Exactly one of `container` and `entity` identifies the target.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_hopper_inventory_search",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's InventoryMoveItemEvent)"
    )]
    fn hopper_inventory_search(hopper: [i32; 3], container: Option<[i32; 3]>, entity: Option<u32>);
}
