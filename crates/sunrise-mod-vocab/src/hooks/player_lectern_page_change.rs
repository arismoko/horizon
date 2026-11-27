//! The `player_lectern_page_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player changed the page of a lectern, after its page and redstone pulse changed.
    /// `page` is the zero-based new page. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_lectern_page_change",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_lectern_page_change(player: u128, name: &str, pos: [i32; 3], page: i32);
}
