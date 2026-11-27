//! The `prepare_item_enchant` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Enchanting offers were calculated for a server player at `pos`. Change `costs` before they are sent to the menu.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_prepare_item_enchant",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn prepare_item_enchant(player: u128, name: &str, pos: [i32; 3], costs: &mut [i32; 3]);
}
