//! `Hud.extractRenderState`'s subtitle rule and `Screen.isInGameUi`.
use super::Screen;

/// `Screen.isInGameUi`: the screens that keep the game behind them — the
/// container screens (`AbstractContainerScreen`) and the block editors
/// (`AbstractCommandBlockEditScreen`, structure block, jigsaw, test block) —
/// plus the two Sunrise draws over `Screen::Playing`: a book (`BookViewScreen`)
/// and a sign editor (`AbstractSignEditScreen`).
pub(in crate::game) fn in_game_ui(screen: Screen, book_or_sign: bool) -> bool {
    book_or_sign
        || matches!(
            screen,
            Screen::Inventory | Screen::Creative | Screen::CommandBlock | Screen::BlockEditor
        )
}

pub use sunrise_client_gameplay::subtitles::{Subtitles, subtitles};

#[cfg(test)]
mod tests {
    /// `Hud.extractRenderState`: the subtitles follow the HUD when no screen
    /// is over the game, are deferred over a screen that keeps the game
    /// behind it (so F1 does not take them away), and vanish under F1 alone.
    #[test]
    fn the_subtitles_follow_the_hud_unless_a_screen_keeps_them() {
        use super::{Screen, Subtitles, in_game_ui, subtitles};
        // A container screen defers them to its background, hidden HUD or not.
        assert_eq!(subtitles(true, Some(true)), Subtitles::Deferred);
        assert_eq!(subtitles(false, Some(true)), Subtitles::Deferred);
        // A pause screen leaves them in the HUD's pass. Without a screen the GUI draws them last.
        assert_eq!(subtitles(true, Some(false)), Subtitles::Now);
        assert_eq!(subtitles(false, Some(false)), Subtitles::Hidden);
        assert_eq!(subtitles(true, None), Subtitles::Deferred);
        assert_eq!(subtitles(false, None), Subtitles::Hidden);
        // `Screen.isInGameUi` is `AbstractContainerScreen` and the block
        // editors' answer; a pause menu, the chat and the death screen say no.
        assert!(in_game_ui(Screen::Inventory, false));
        assert!(in_game_ui(Screen::Creative, false));
        assert!(in_game_ui(Screen::CommandBlock, false));
        assert!(in_game_ui(Screen::BlockEditor, false));
        // A book and a sign editor are in-game UI drawn over `Playing`.
        assert!(in_game_ui(Screen::Playing, true));
        assert!(!in_game_ui(Screen::Paused, false));
        assert!(!in_game_ui(Screen::Chat, false));
        assert!(!in_game_ui(Screen::Dead, false));
        assert!(!in_game_ui(Screen::Playing, false));
    }
}
