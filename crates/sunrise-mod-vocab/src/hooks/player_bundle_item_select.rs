//! The `player_bundle_item_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player selected a bundle item in `slot` at `index`. The guest's
    /// state is not swapped in during packet handling.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_bundle_item_select",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerBundleItemSelect)"
    )]
    fn player_bundle_item_select(player: u128, name: &str, slot: i32, index: i32);
}
