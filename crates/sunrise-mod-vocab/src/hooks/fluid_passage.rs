//! The `fluid_passage` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// May `fluid` flow from the cell `from` (`from_state`) one step toward `direction`, into the
    /// cell `to` (`to_state`)? Vanilla's `FlowingFluid.canPassThrough` family has no event; this
    /// is asked where Sunrise checks `canMaybePassThrough`: the downward step of `spread`, the
    /// side choice of `getSpread` and every step of the slope search (`getSlopeDistance`), so
    /// closing a side changes which sides the fluid picks, not only whether one cell fills.
    /// `fluid` is `water`, `lava` or a mod fluid's id; `direction` says which step is asked, and
    /// a listener may answer only for the horizontal ones.
    ///
    /// `passes` starts as true and is asked only after the game's own answer was true (the
    /// target can hold a fluid, is not a source of it and its shapes leave the face open):
    /// a listener can close a side, never open a wall. The answer is not kept: it never enters
    /// the occlusion cache or the published passage cache, which stay shape-only, and it is not
    /// asked from the other uses of `canPassThroughWall` (`getNewLiquid`, `isWaterHole`), which
    /// decide a cell's own level, not where it flows.
    ///
    /// Read-only, and asked often while a fluid spreads: answer from reads, fast. A changed
    /// answer reschedules no fluid tick by itself; when it follows a block change, that change's
    /// neighbour updates already reschedule the adjacent fluids. `block_spread` is unchanged.
    /// Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)",
        target = "sunrise_gameplay_common::game::native_hooks::fluid_passage::GameLevel<'_>::native_fluid_passage"
    )]
    fn fluid_passage(
        dimension: LevelDimension,
        fluid: &str,
        from: BlockPos,
        from_state: BlockState,
        direction: Direction,
        to: BlockPos,
        to_state: BlockState,
        passes: &mut bool,
    );
}
