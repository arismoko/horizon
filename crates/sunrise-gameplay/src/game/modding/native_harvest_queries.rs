//! The correct-tool queries a mod can call: `BlockStates::is_correct_tool_for_drops` (a stack
//! alone) and `PlayerRef::has_correct_tool_for_drops` (the answer the game's own digs and
//! breaks use, `Game::acting_can_harvest`).
use crate::game::ServerGame;
use mod_api::{LevelRef, PlayerRef};
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    pub(in crate::game) fn server_player_destroy_progress(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<f32> {
        if self.native_level_world(level).is_none() || level.dimension() != self.common.dim.key() {
            return None;
        }
        let mut answer = None;
        self.native_as_player_handle(player, |game| {
            answer = Some(game.destroy_progress_for(pos, state));
        });
        answer
    }

    pub(in crate::game) fn server_player_has_correct_tool_for_drops(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<bool> {
        if self.native_level_world(level).is_none() || level.dimension() != self.common.dim.key() {
            return None;
        }
        let mut answer = None;
        self.native_as_player_handle(player, |game| {
            answer = Some(game.acting_can_harvest(pos, state));
        });
        answer
    }
}

use sunrise_gameplay_common::game::held_items::HeldItemsGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
