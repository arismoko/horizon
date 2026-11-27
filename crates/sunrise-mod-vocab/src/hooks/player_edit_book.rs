//! The `player_edit_book` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player's book edit or signing packet is about to be applied. `pages` is its page
    /// count; `signing` means the packet has a title. Set `cancel` to drop the edit.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_player_edit_book",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's PlayerEditBookEvent)",
        cancellable = true
    )]
    fn player_edit_book(player: u128, name: &str, pages: i32, signing: bool, cancel: &mut bool);
}
