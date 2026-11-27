//! The `player_item_mend` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Mending repaired a server player's item with experience from an orb. `repair` is durability
    /// restored and `experience` is the orb's value. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_item_mend",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_item_mend(player: u128, name: &str, item: &str, repair: u32, experience: u32);
}
