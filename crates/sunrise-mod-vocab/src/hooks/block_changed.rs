//! The `block_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// An authoritative server committed a changed state through `Level.setBlock`.
    /// The world contains `new`; block side effects and neighbour updates follow.
    /// `flags` are the [`BlockUpdateFlags`](crate::BlockUpdateFlags) applied to the write that
    /// reached the level, not to any intermediate changes staged in a transaction. `KNOWN_SHAPE`
    /// skips the neighbour shape passes, not this notice. Each listener gets its own event.
    #[notes = r##"### Server block changes

`block_changed` observes a committed authoritative server `Level.setBlock` change before block side effects and
neighbour updates. It supplies the dimension, position, old and new states, and the `BlockUpdateFlags` applied to the
write. `KNOWN_SHAPE` skips the shape passes but does not suppress this notice. The hook does not observe equal-state
writes, chunk loading or direct storage writes that bypass `Level.setBlock`. Each listener gets its own event."##]
    #[hook(
        receiver = "read",
        dispatch = "flat",
        side = "server",
        contract = "listen",
        version = 2,
        event_per_listener = true,
        target = "sunrise_gameplay_common::game::native_hooks::base::GameLevel<'_>::native_block_changed",
        neoforge = "none (NeoForge has no set-block event; IBlockExtension.onBlockStateChange is the block part hook on_block_state_change, at the end of the write)"
    )]
    fn block_changed(
        dimension: LevelDimension,
        pos: BlockPos,
        old: BlockState,
        new: BlockState,
        flags: BlockUpdateFlags,
    );
}
