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
    fn tell(&mut self, uuid: u128, line: horizon_client::text::Text) -> bool {
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
}

pub(super) struct LevelTicks<'a, 'b, 'c> {
    pub level: &'a super::GameLevel<'b>,
    pub ticks: &'c mut Ticks,
}

pub(super) struct LevelWrite<'a, 'b, 'c> {
    pub level: &'a mut super::GameLevel<'b>,
    pub ticks: &'c mut Ticks,
}

macro_rules! level_ticks_host {
    ($ty:ident) => {
        impl horizon_client::mod_api::LevelHost for $ty<'_, '_, '_> {
            fn block_state(&self, pos: BlockPos) -> BlockStateId {
                horizon_world::level::LevelAccess::block_state(self.level, pos)
            }
        }
        impl horizon_client::mod_api::LevelTicksHost for $ty<'_, '_, '_> {
            fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
                self.level
                    .native_schedule_block_tick(self.ticks, pos, block, delay)
            }
            fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
                self.level
                    .native_schedule_fluid_tick(self.ticks, pos, fluid, delay)
            }
        }
    };
}
level_ticks_host!(LevelTicks);
level_ticks_host!(LevelWrite);

impl horizon_client::mod_api::LevelWriteHost for LevelWrite<'_, '_, '_> {
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(self.ticks, pos, key, value, flags)
    }
}
