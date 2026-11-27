//! Ticking eligibility through the same predicates as the server's tick phases.
use crate::game::ServerGame;
use mod_api::LevelRef;

impl ServerGame<'_> {
    fn native_tick_position(&self, level: LevelRef, pos: [i32; 3]) -> Option<bool> {
        if !self.level_in_place(level) {
            return None;
        }
        Some(
            self.level_in_world_bounds(level, pos)?
                && self.chunk(level, [pos[0] >> 4, pos[2] >> 4]).is_some(),
        )
    }

    pub(in crate::game) fn level_entity_ticking(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        Some(self.native_tick_position(level, pos)? && self.entity_ticking_area()(pos[0], pos[2]))
    }

    pub(in crate::game) fn level_should_tick_blocks_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        Some(self.native_tick_position(level, pos)? && self.block_ticking_at(pos[0], pos[2]))
    }
}

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
