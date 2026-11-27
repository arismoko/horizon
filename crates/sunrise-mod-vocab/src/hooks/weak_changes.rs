pub trait Hooks {
    /// Whether this block receives block entity changes through an adjacent conductor. Starts false.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "receive",
        scope = "block",
        key = "state",
        neoforge = "IBlockExtension.getWeakChanges",
        target = "sunrise_gameplay_common::game::native_hooks::block_entity_neighbors::GameLevel<'_>::native_weak_changes"
    )]
    fn weak_changes(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        receive: &mut bool,
    );
}
