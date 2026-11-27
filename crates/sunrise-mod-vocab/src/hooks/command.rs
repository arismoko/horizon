//! The `update_shape` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An authoritative server is about to compute the state the block
    /// `state` at `pos` takes now that its neighbour in `direction` is
    /// `neighbour` (`BlockBehaviour.updateShape`). `state` may not be in the
    /// world yet (a piston landing it): decide from `state`, not from the
    /// block at `pos`. Answer only for your own blocks: put the new state in
    /// `new` (`state` keeps the block as it is; air breaks it, with its drops,
    /// on a neighbour update) and the block's own `updateShape` doesn't run,
    /// nor the ticks it would schedule. The level is read-only, as vanilla's
    /// `LevelReader`; ticks go through `native_schedule_block_tick` and
    /// `native_schedule_fluid_tick` with `ticks` (`ScheduledTickAccess`). It
    /// runs often, at least six times per `setBlock` that updates shapes
    /// (more for redstone wire's diagonals and knock-on updates): keep it
    /// short.
    /// A disabled owner's subscribed update keeps the input state instead of running the template.
    #[hook(
        receiver = "read",
        side = "server",
        contract = "replace",
        inert = "*new = Some(state);",
        out = "new",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_update_shape",
        neoforge = "none (vanilla BlockBehaviour.updateShape; NeoForge adds no event or extension for it)",
        version = 2
    )]
    fn update_shape(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        direction: Direction,
        neighbour_pos: BlockPos,
        neighbour: BlockState,
        new: &mut Option<BlockState>,
        #[engine] ticks: &mut Ticks,
    );
}
