//! The `sponge_absorb` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sponge at `pos` absorbed water and became wet. Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_sponge_absorb"
    )]
    fn sponge_absorb(dimension: LevelDimension, pos: BlockPos);
}
