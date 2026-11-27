//! The `tab_complete` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest asked to complete `command`; owned API `Suggestion` records may be
    /// added to or filtered from `suggestions`. `range` is the replaced byte range.
    /// The game snapshots completions only when a listener subscribes.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_tab_complete",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "suggestions",
        neoforge = "none (NeoForge has no generic tab completion event)",
        version = 2
    )]
    fn tab_complete(
        player: u128,
        name: &str,
        command: &str,
        range: [usize; 2],
        suggestions: &mut Vec<Suggestion>,
    );
}
