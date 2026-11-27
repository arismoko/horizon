use crate::game::ClientGame;

impl ClientGame<'_> {
    /// `Component.translatable`: the selected language, falling back to `fallback`.
    pub fn translate_with_fallback(&self, key: &str, fallback: &str) -> String {
        sunrise_game_data::lang::get_or(key, fallback)
    }
}
