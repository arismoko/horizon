//! The `block_break_progress_update` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server guest's block-breaking animation changed to `progress` at
    /// `pos` (0 through 9).
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_break_progress_update",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn block_break_progress_update(player: u128, name: &str, pos: [i32; 3], progress: i32);
}
