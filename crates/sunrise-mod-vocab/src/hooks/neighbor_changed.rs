//! The `neighbor_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An authoritative server is about to deliver `neighborChanged`.
    /// State and block IDs are valid in this level's registries for this call.
    /// The host is the level writer vanilla's `neighborChanged` gets: a part may read
    /// its inputs (`signal`, `has_neighbor_signal`), schedule its tick, as a diode does
    /// (`DiodeBlock.checkTickOnNeighbor`), and set blocks, as `RedstoneLampBlock` lights
    /// itself at once (`level.setBlock(pos, state.cycle(LIT), 2)`). A write's own
    /// neighbour updates run next, before the rest of the update that delivered this
    /// call (`CollectingNeighborUpdater`), as `Level.setBlock` from inside
    /// `neighborChanged` runs them. The block's own handling then runs with the state
    /// it was called with.
    #[hook(
        dispatch = "flat",
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        version = 3,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_neighbor_changed",
        neoforge = "none (vanilla Block.neighborChanged; NeoForge adds no event or extension for it); BlockEvent.NeighborNotifyEvent is the neighbour broadcast, a different point"
    )]
    fn neighbor_changed(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        changed: mod_api::BlockKey,
        moved: bool,
        #[engine] ticks: &mut Ticks,
    );
}
