//! An `[[inject]]`-only mod: no hooks, one listener at the head of `Game::tick`.
use mod_api::Hooks;

#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {}

pub fn on_tick(_: mod_api::HostSide<'_>, _: &mut mod_points::TickArgs) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("slice: slice_inject listener at Game::tick head called"));
}
