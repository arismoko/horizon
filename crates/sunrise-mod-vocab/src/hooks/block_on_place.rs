//! The `on_place` hook: a declaration the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A block of `state` was just written into the level on an authoritative server
    /// (`BlockBehaviour.onPlace`, called from `LevelChunk.setBlockState`). It is heard for every
    /// write that sets a new state and does not skip it: a player's placement, `/setblock`,
    /// `/fill`, a structure, a piston landing a block. `old_state` is the state the write
    /// replaced (the same block when only a property changed) and `moved_by_piston` says the
    /// write came from a piston. A write whose flags skip `onPlace`
    /// (`Block.UPDATE_SKIP_ON_PLACE`), and one whose block was already replaced again by the
    /// removal of the old state, are not heard.
    /// State IDs are valid in this level's registries for this call.
    ///
    /// The host is the level writer `neighbor_changed` gets: a part may read its inputs
    /// (`signal`, `has_neighbor_signal`), schedule its tick and set blocks, as `CopperBulbBlock`
    /// does through `checkAndFlip` (`level.setBlockAndUpdate`). A write
    /// goes through the level's ticks, so its neighbour updates run as vanilla's do, before the
    /// rest of the write that delivered this call. The part runs before the block's own
    /// `onPlace` handling, which then runs with the state it was called with. Dispatched only
    /// to the parts on the placed state.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::block_on_place::GameLevel<'_>::native_on_place",
        neoforge = "none (vanilla BlockBehaviour.onPlace; NeoForge adds no event or extension for it)"
    )]
    fn on_place(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        old_state: BlockState,
        moved_by_piston: bool,
        #[engine] ticks: &mut Ticks,
    );
}
