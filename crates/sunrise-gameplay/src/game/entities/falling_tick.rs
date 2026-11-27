//! `falling_tick`: a falling block's parts hear each tick it ends still in the air, once the
//! ordered pass that ticked it is over (the falling block is lent out during its own tick).
use crate::game::ServerGame;
use sunrise_core::position::BlockPos;
use sunrise_sim::join_seq::JoinSeq;
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    /// Record that the falling block `id` (`join`) of `state` ended its tick still falling,
    /// when a part listens to its state.
    pub(in crate::game::server_side) fn note_falling_tick(
        &mut self,
        id: u32,
        join: JoinSeq,
        state: BlockStateId,
    ) {
        if sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::FallingTick,
            state,
        ) {
            self.server.ordered.falling_ticked.push((id, join));
        }
    }

    /// Deliver the recorded falling ticks in order, each to the parts of the falling block's
    /// state as it is now; one gone since is skipped.
    pub fn deliver_falling_ticks(&mut self) {
        if self.server.ordered.falling_ticked.is_empty() {
            return;
        }
        let Some(epoch) = self.common.native_lifecycle.epoch() else {
            self.server.ordered.falling_ticked.clear();
            return;
        };
        let mut ticked = std::mem::take(&mut self.server.ordered.falling_ticked);
        for (id, join) in ticked.drain(..) {
            let Some(i) = self
                .common
                .falling
                .locate(join)
                .filter(|&i| self.common.falling[i].id == id)
            else {
                continue;
            };
            let (state, p) = (
                self.common.falling[i].state,
                self.common.falling[i].position,
            );
            let pos = BlockPos::new(p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32);
            self.native_falling_tick(state, pos, mod_api::EntityRef::new(epoch, id));
        }
        self.server.ordered.falling_ticked = ticked;
    }
}
