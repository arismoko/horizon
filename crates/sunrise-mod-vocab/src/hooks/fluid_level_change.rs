//! The `fluid_level_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A flowing fluid at `pos` is about to change level. `fluid` is `water`
    /// or `lava`; the levels are 0 through 8. Set `cancel` to keep the old
    /// level. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_fluid_level_change"
    )]
    fn fluid_level_change(
        dimension: LevelDimension,
        pos: BlockPos,
        fluid: &str,
        from: i32,
        to: i32,
        cancel: &mut bool,
    );
}
