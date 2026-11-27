//! The `player_flower_pot_manipulate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player put a plant into a flower pot, after the potted block state was stored and the
    /// item consumed. `plant` is the item id and `pos` the pot position. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_flower_pot_manipulate",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_flower_pot_manipulate(player: u128, name: &str, pos: [i32; 3], plant: &str);
}
