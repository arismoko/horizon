use super::*;

impl crate::game::ServerGame<'_> {
    /// Sets a block in the current authoritative level through `Level.setBlock`.
    /// Returns false for another dimension, an invalid state, or no change.
    /// `flags` selects vanilla's neighbour, shape and client updates. A block
    /// entity block gets its block entity.
    pub fn native_set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        if !self.common.is_logical_server()
            || dimension != self.common.dim.id()
            || self.common.pack.assets.registry.state(state).is_none()
        {
            return false;
        }
        let changed = self.set_block_live(pos, state, flags, true);
        let _ = self.server_process_level_events();
        changed
    }
}

use sunrise_gameplay_common::game::GameGameExt1 as _;
