//! The `trade_select` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player selected trade `index` from `villager` in the trading menu.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_trade_select",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn trade_select(player: u128, name: &str, villager: Option<u32>, index: usize);
}
