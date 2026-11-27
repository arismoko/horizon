//! Language map lookup for session and dedicated-server translation resolution.
use sunrise_protocol::text::Language;

/// Resolves session translations from packs without runtime overrides, falling back to `en_us`.
pub fn load_language(resources: &std::path::Path) -> Language {
    if let Some(lang) = crate::lang::current() {
        return lang.pack_map();
    }
    // No language screen ran (a dedicated server's log): en_us, with
    // Sunrise's own strings included — the net thread resolves keys with
    // this map (`Reason::keyed`).
    sunrise_game_data::lang::Language::load(&[&resources.to_path_buf()], "en_us").pack_map()
}
