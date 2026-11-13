//! API-only mod: its constructor runs when the mods library hands the game its entries, and its
//! hooks write one line each when the game fires them.
use mod_api::{Hooks, HostSide, ServerHost};

struct Mod;

impl Default for Mod {
    fn default() -> Self {
        eprintln!("slice: slice_smoke constructed (mods library entries)");
        Mod
    }
}

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn server_started(&self, _: &mut dyn ServerHost, _: &mut mod_api::ServerStartedEvent) {
        eprintln!("slice: slice_smoke hook server_started fired");
    }
    fn server_tick(&self, _: HostSide<'_>, _: &mut mod_api::ServerTickEvent) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| eprintln!("slice: slice_smoke hook server_tick fired"));
    }
}
