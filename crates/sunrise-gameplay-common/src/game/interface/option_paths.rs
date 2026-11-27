/// This player's `options.txt`: in the game folder, or in `--player-dir` when several
/// players share one (`logging::set_player_dir`).
pub fn options_path() -> std::path::PathBuf {
    sunrise_level::logging::player_dir().join("options.txt")
}

/// The game folder (the working directory): what its players share, saves, packs, mods.
pub fn game_dir() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_default()
}
