//! The `is_bonemeal_success` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BonemealableBlock.isBonemealSuccess` for a block that answered
    /// `is_valid_bonemeal_target`, after it did: set `success` to grow
    /// it this time. Vanilla draws the level's random here
    /// (`isBonemealSuccess(Level, RandomSource, BlockPos, BlockState)`).
    /// Use `host.with_random(|rng| rng.next_int_bound(bound))` to draw from that
    /// live stream in vanilla's order.
    /// Authoritative servers only.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "collect",
        out = "success",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_is_bonemeal_success",
        neoforge = "none (vanilla BonemealableBlock.isBonemealSuccess; NeoForge adds no event or extension for it)"
    )]
    fn is_bonemeal_success(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        success: &mut bool,
    );
}
