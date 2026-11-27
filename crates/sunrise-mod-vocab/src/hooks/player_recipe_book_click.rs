//! The `player_recipe_book_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player clicked a known, placeable recipe in their current recipe book.
    /// Runs after display, recipe knowledge and live menu checks, for vanilla and mod books.
    /// Runs before applying placement, including before a mod menu's `recipe_book_place`
    /// decision. `recipe` is its ID and `all` is the Shift-click form. The player is not
    /// swapped in. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_recipe_book_click",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_recipe_book_click(player: u128, name: &str, recipe: &str, all: bool);
}
