//! The `perform_bonemeal` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BonemealableBlock.performBonemeal` for a block that answered
    /// `is_valid_bonemeal_target` and whose `is_bonemeal_success`
    /// said yes: grow it here, `state` its state at `pos`. The meal is used up
    /// either way. Change the block with `LevelWriteHost::set_block_property`,
    /// and schedule with `LevelTicksHost::schedule_block_tick`.
    /// Authoritative servers only.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_perform_bonemeal",
        neoforge = "none (vanilla BonemealableBlock.performBonemeal; NeoForge adds no event or extension for it)"
    )]
    fn perform_bonemeal(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        #[engine] ticks: &mut Ticks,
    );
}
