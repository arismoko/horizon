//! The `block_form` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Precipitation is about to form ice or place or thicken snow at `pos`.
    /// `from` and `to` are block state IDs. Set `cancel` to keep the old state.
    /// Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_block_form"
    )]
    fn block_form(
        dimension: LevelDimension,
        pos: BlockPos,
        from: BlockState,
        to: BlockState,
        cancel: &mut bool,
    );
}
