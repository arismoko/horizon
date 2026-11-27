/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
impl crate::game::CommonGame<'_> {
    /// The selected language's text for a translation key (`Component.translatable`'s lookup), `None`
    /// when no language layer defines it.
    pub fn translate(&self, key: &str) -> Option<String> {
        sunrise_game_data::lang::has(key).then(|| sunrise_game_data::lang::get(key))
    }
}
