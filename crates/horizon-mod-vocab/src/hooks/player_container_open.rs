//! The `player_container_open` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0163

pub trait Hooks {
    /// `player` (`name`) opened a menu (NeoForge's `PlayerContainerEvent.Open`,
    /// posted when `ServerPlayer.openMenu` sets the new `containerMenu`): any
    /// menu, vanilla or a mod's. `menu` is the menu type's name
    /// (`minecraft:generic_9x3`, `minecraft:furnace`, `minecraft:merchant`,
    /// `minecraft:crafting`, `minecraft:lectern`, a mod's own), `pos` the
    /// block it is of in `dimension` (none for a trade, a vehicle's, a mod's
    /// native menu or a mount's inventory, which has no menu type in vanilla
    /// and is told as `horizon:mount_inventory`). The player's own inventory
    /// is not one. Heard once a tick, so a menu opened and closed again
    /// inside one tick is not; one that replaces another comes after that
    /// one's close. The player's state is swapped in. `player_container_close`
    /// follows. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerContainerEvent.Open",
        cancellable = false
    )]
    fn player_container_open();
}
