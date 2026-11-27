//! The `screen_files_dropped` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Files were dropped on the window while a screen is open (vanilla's
    /// `Screen.onFilesDrop`, called by `MouseHandler.onDrop` with the whole drop as one
    /// list). NeoForge's `ScreenEvent` has no event for it, so this hook has no NeoForge
    /// name.
    /// `paths` are the dropped files as the operating system gave them, in the order it
    /// gave them: a path that is not valid UTF-8 is passed through unchanged, so compare
    /// and open paths as `Path`, not as text. The game reads no file; the mod reads the
    /// ones it wants. `screen` is the screen that is open when the drop ends, a vanilla
    /// screen or a mod's (`screen.mod_screen` is its mod).
    /// Observe only: the hook cannot cancel or mark the drop handled, and the game's own
    /// handling (the resource-pack screens take a dropped pack) always runs. A drop that
    /// arrives with no screen open, or with no listener, fires nothing. Client only: no
    /// packet is sent.
    #[notes = r#"## Dropped files

`screen_files_dropped(screen, paths)` is vanilla's `Screen.onFilesDrop`: it fires once for each drop on the window while a screen is open, with every dropped path in the order the operating system gave them. A path that is not UTF-8 is passed through as it is. The game reads no file, so open the paths yourself. The hook only observes: the game's own handling of the drop (a dropped resource pack) always runs, whatever a mod does."#]
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_screen_files_dropped",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        version = 1,
        contract = "listen",
        cancellable = false
    )]
    fn screen_files_dropped(screen: &ScreenView<'_>, paths: &[std::path::PathBuf]);
}
