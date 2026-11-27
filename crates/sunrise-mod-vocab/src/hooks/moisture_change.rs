//! The `moisture_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Farmland at `pos` is about to change moisture from `from` to `to`.
    /// Set `cancel` to keep its old moisture. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "MoistureChangeEvent",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_moisture_change"
    )]
    fn moisture_change(
        dimension: LevelDimension,
        pos: BlockPos,
        from: i32,
        to: i32,
        cancel: &mut bool,
    );
}
