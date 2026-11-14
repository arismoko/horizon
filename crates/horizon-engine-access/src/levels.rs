use horizon_world::registry::BlockStateId;
use mod_api::{BlockUpdateFlags, ClipBlock, ClipFluid, ClipHit, LevelRef, LevelWriteError};

/// Reads of the levels a world holds.
pub trait LevelRead {
    /// The level named `dimension` (`minecraft:the_nether`) as a handle, or None where it is not
    /// loaded.
    fn level_of(&self, dimension: &str) -> Option<LevelRef>;
    /// Whether the block at `pos` of the level in place carries the block tag `tag`.
    fn block_has_tag(&self, pos: [i32; 3], tag: &str) -> bool;
    /// An empty-context ray against block shapes, as `Entity.pick` makes it.
    fn clip(
        &self,
        level: LevelRef,
        from: [f64; 3],
        to: [f64; 3],
        blocks: ClipBlock,
        fluids: ClipFluid,
    ) -> Option<ClipHit>;
}

/// Writes to the levels a world holds.
pub trait LevelWrite {
    /// `Level.setBlock`: `Ok(false)` where vanilla refuses an unchanged, unloaded or out-of-height
    /// write; an error for a stale level.
    fn set_block(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError>;
}
