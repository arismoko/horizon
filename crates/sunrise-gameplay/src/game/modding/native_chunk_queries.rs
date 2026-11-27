//! Read-only chunk facts that never request residency.
use crate::game::ServerGame;
use mod_api::{BlockKey, ChunkRef};
use sunrise_core::random::{RandomSource, slime_chunk_random};

impl ServerGame<'_> {
    pub(in crate::game) fn chunk_is_slime_chunk(&self, chunk: ChunkRef) -> Option<bool> {
        let seed = self.level_seed(chunk.level())?;
        let [x, z] = chunk.pos();
        // Reuse the same world-bound validation as handles for unloaded chunks.
        self.level_chunk_ref(chunk.level(), [x, z])?;
        Some(
            slime_chunk_random(x, z, seed, 987_234_911)
                .next_int_bound(10)
                .unwrap_or(1)
                == 0,
        )
    }

    /// The sections of a loaded chunk whose palette may hold a state of `block` (`maybeHas`),
    /// by section Y, bottom up.
    pub(in crate::game) fn chunk_sections_with_block(
        &self,
        chunk: ChunkRef,
        block: BlockKey,
    ) -> Option<Vec<i32>> {
        if !self.common.is_logical_server() {
            return None;
        }
        let [x, z] = chunk.pos();
        let column = self
            .native_level_world(chunk.level())?
            .chunks
            .get(&(x, z))?;
        let states = sunrise_gameplay_common::game::api_values::block_state::current().ok()?;
        let of_block = |id: u32| {
            states
                .to_api(sunrise_world::registry::BlockStateId(id))
                .is_ok_and(|state| state.block() == block)
        };
        let bottom = column.min_y() >> 4;
        Some(
            column
                .sections
                .iter()
                .zip(bottom..)
                .filter(|(section, _)| section.blocks.maybe_has(of_block))
                .map(|(_, y)| y)
                .collect(),
        )
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
