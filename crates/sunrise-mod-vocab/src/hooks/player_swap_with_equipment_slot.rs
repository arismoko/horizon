//! The `player_swap_with_equipment_slot` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player swapped their main and off hands, after both inventory slots changed.
    /// Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_swap_with_equipment_slot",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_swap_with_equipment_slot(player: u128, name: &str, slot: &str);
}
