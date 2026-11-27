//! The `player_open_sign` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player opened a sign for editing, after the editor state was made. `front` selects the
    /// front or back side. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_open_sign",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_open_sign(player: u128, name: &str, pos: [i32; 3], front: bool);
}
