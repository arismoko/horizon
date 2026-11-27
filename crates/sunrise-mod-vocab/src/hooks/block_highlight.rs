//! The `block_highlight` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Before an eligible targeted block's selection outline is built.
    /// Cancel skips only the default block highlight, after normal HUD and game-mode gates.
    #[hook(
        target = "sunrise_client_gameplay::game::native_hooks::base::ClientGame<'_>::native_block_highlight",
        receiver = "read",
        dispatch = "flat",
        side = "client",
        contract = "cancel",
        out = "canceled",
        cancellable = true,
        neoforge = "ExtractBlockOutlineRenderStateEvent"
    )]
    fn block_highlight(pos: [i32; 3], state: BlockState, partial: f32, canceled: &mut bool);
}
