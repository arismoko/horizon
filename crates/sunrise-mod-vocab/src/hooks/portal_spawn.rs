//! The `portal_spawn` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A fire placed at `pos` (`state`) found an empty obsidian frame, `shape`,
    /// and is about to fill it with a nether portal (`BlockEvent.PortalSpawnEvent`,
    /// in `BaseFireBlock.onPlace`), in the Overworld or the Nether. Set
    /// `cancel` and no portal forms: the fire stays where it can survive, as
    /// without a frame. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockEvent.PortalSpawnEvent",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::portal_spawn::GameLevel<'_>::native_portal_spawn"
    )]
    fn portal_spawn(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        shape: PortalShape,
        cancel: &mut bool,
    );
}
