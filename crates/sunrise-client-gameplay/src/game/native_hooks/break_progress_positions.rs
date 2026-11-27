#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn with_break_progress_positions(
        &mut self,
        pos: [i32; 3],
        progress: u8,
        publish: impl FnOnce(&mut Self, Vec<[i32; 3]>),
    ) {
        let state = self.common.world.block(pos[0], pos[1], pos[2]);
        if !self.common.native_window()
            || !block_listens(mod_api::HookId::BreakProgressPositions, state)
        {
            publish(self, Vec::new());
            return;
        }
        let Some(level) = self.native_client_level() else {
            publish(self, Vec::new());
            return;
        };
        self.storage_scope(|game, _| {
            let mut positions = Vec::new();
            game.native_break_progress_positions(super::fire::BreakProgressPositionsCall {
                level,
                pos: mod_api::BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                progress,
                positions: &mut positions,
            });
            let mut extras = Vec::new();
            let mut refused = 0_usize;
            let mut remaining = positions.into_iter();
            while let Some(candidate) = remaining.next() {
                let candidate = [candidate.x, candidate.y, candidate.z];
                if candidate == pos || extras.contains(&candidate) {
                    refused += 1;
                } else {
                    extras.push(candidate);
                    if extras.len() == 64 {
                        refused += remaining.len();
                        break;
                    }
                }
            }
            if refused > 0 {
                log::debug!(
                    "break_progress_positions refused {refused} extras: the origin, duplicates and those past the 64 kept"
                );
            }
            if game.native_client_level() == Some(level) {
                publish(game, extras);
            } else {
                log::debug!(
                    "break_progress_positions discarded extras after the client level changed"
                );
            }
            ((), true)
        });
    }

    pub(in crate::game) fn native_break_progress_positions(
        &mut self,
        call: super::fire::BreakProgressPositionsCall<'_>,
    ) {
        self.fire_break_progress_positions(call);
    }
}
use sunrise_gameplay_common::game::api_values::block_state::block_listens;
use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
