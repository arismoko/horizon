//! The `player_insert_lectern_book` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player inserted a book into a lectern, after the block entity and inventory changed.
    /// `book` is the item id. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_insert_lectern_book",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_insert_lectern_book(player: u128, name: &str, pos: [i32; 3], book: &str);
}
