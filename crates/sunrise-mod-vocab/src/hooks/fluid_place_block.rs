//! The `fluid_place_block` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Liquid at `liquid_pos` is about to place `state` at `pos`. Set
    /// `cancel` to prevent placement or change `new_state` to replace it.
    /// Server-side.
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "BlockEvent.FluidPlaceBlockEvent",
        cancellable = true,
        target = "sunrise_gameplay_common::game::native_hooks::fluid_place_block::GameLevel<'_>::native_fluid_place_block",
        version = 2
    )]
    fn fluid_place_block(
        dimension: LevelDimension,
        pos: BlockPos,
        liquid_pos: BlockPos,
        state: BlockState,
        new_state: &mut BlockState,
        cancel: &mut bool,
    );
}
