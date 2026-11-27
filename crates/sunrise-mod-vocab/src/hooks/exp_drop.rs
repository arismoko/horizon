//! Per-block experience before enchantments.
pub trait Hooks {
    /// Experience before enchantments (`IBlockExtension.getExpDrop`). First answer
    /// replaces vanilla sampling; the global break_block event may override it.
    /// Negative answers are clamped to 0; positive values are awarded as given.
    /// A panic unwinds before the answer is applied.
    #[hook(
        receiver = "read",
        side = "server",
        contract = "replace",
        out = "experience",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::exp_drop::GameLevel<'_>::native_exp_drop",
        neoforge = "IBlockExtension.getExpDrop",
        version = 2
    )]
    fn exp_drop(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        block_entity: Option<BlockEntityRef>,
        breaker: Option<EntityRef>,
        tool: Option<&ItemStack>,
        experience: &mut Option<i32>,
    );
}
