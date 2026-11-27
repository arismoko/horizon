//! The `analog_output` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `BlockState.getAnalogOutputSignal`: the signal a comparator reads from
    /// the block `state` at `pos`, `face` the side that touches the comparator
    /// (or the conductor in front of it). `output` holds vanilla's answer:
    /// change it when the block is yours (0 to 15; anything else is clamped).
    /// Asked only for blocks `has_analog_output` answered for, from
    /// inside the level's update: don't change the world here. When the
    /// answer changes without the block's state changing, call
    /// `level.update_neighbors(host, pos, state, NeighbourUpdate::OutputSignal)`; a change of
    /// state through `LevelRef::set_block` or `LevelRef::set_block_property`
    /// (or `LevelWriteHost` inside a block tick, with neighbour updates on)
    /// tells the comparators itself, as vanilla's `Level.setBlock` does.
    /// Authoritative servers only. Version 2 supplies `&mut dyn LevelRandomHost`: use
    /// `container_signal` for vanilla's comparator read, which may unpack unopened loot.
    /// The level's random stream remains in vanilla order. Other world edits stay forbidden.
    #[hook(
        dispatch = "flat",
        side = "server",
        version = 2,
        contract = "collect",
        out = "output",
        scope = "block",
        key = "state",
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_analog_output",
        neoforge = "none (vanilla BlockBehaviour.getAnalogOutputSignal; NeoForge adds no event or extension for it)"
    )]
    fn analog_output(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        face: Direction,
        output: &mut i32,
    );
}
