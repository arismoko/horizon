use super::{BlockStateId, Game};
use horizon_client::mod_api::{BlockPos, Ticks};
impl horizon_client::mod_api::Host for Game<'_> {
    fn block(&self, pos: [i32; 3]) -> String {
        self.native_block(pos)
    }
    fn set_block(
        &mut self,
        dimension: &str,
        pos: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> bool {
        self.native_set_block(dimension, pos, state, flags)
    }
    fn feedback(&mut self, text: horizon_client::text::Text) {
        self.native_feedback(text);
    }
    fn acting_player(&self) -> Option<horizon_client::mod_api::NativeServerPlayer> {
        self.native_acting_player()
    }
    fn tell(&self, uuid: u128, line: horizon_client::text::Text) -> bool {
        self.native_tell(uuid, line)
    }
    fn action_bar(&mut self, uuid: u128, line: horizon_client::text::Text) -> bool {
        self.native_action_bar(uuid, line)
    }
}

impl horizon_client::mod_api::LevelHost for super::GameLevel<'_> {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        horizon_world::level::LevelAccess::block_state(self, pos)
    }
    fn set_block_property(
        &mut self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        key: &str,
        value: &str,
        flags: i32,
    ) -> bool {
        self.native_set_block_property(ticks, pos, key, value, flags)
    }
    fn schedule_block_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        block: &str,
        delay: i32,
    ) -> bool {
        self.native_schedule_block_tick(ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(
        &self,
        ticks: &mut Ticks,
        pos: [i32; 3],
        fluid: &str,
        delay: i32,
    ) -> bool {
        self.native_schedule_fluid_tick(ticks, pos, fluid, delay)
    }
}
