//! The `player_take_lectern_book` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player took the book from a lectern, after the menu and block entity changed. `book`
    /// is the removed item id. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_take_lectern_book",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_take_lectern_book(player: u128, name: &str, pos: [i32; 3], book: &str);
}
