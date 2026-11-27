//! The `block_receive_game` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sculk sensor or shrieker received a vibration for `event` at `pos`.
    /// Set `cancel` to suppress its response. Server-side.
    #[hook(
        target = "sunrise_gameplay::game::native_hooks::base::ServerGame<'_>::native_block_receive_game",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true
    )]
    fn block_receive_game(
        dimension: &str,
        pos: [i32; 3],
        block: &str,
        event: &str,
        cancel: &mut bool,
    );
}
