use super::{BlockStateId, Game};
use horizon_client::mod_api::BlockPos;
use horizon_world::level::Ticks;
impl horizon_client::mod_api::Host for Game<'_> {
    fn attachments(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        use horizon_client::mod_api::{AttachmentError, Holder};
        if self.serving.is_none() && self.options.dedicated.is_none() {
            return Err(AttachmentError::MissingHolder);
        }
        let Holder::Chunk([x, z]) = holder else {
            return Err(AttachmentError::MissingHolder);
        };
        let chunk = self
            .world
            .chunks
            .get_mut(&(x, z))
            .ok_or(AttachmentError::MissingHolder)?;
        let chunk = std::sync::Arc::make_mut(chunk);
        let extras = std::sync::Arc::make_mut(chunk.saved.get_or_insert_with(Default::default));
        if extras
            .root
            .get(&"horizon:attachments".into())
            .is_some_and(|v| !matches!(v, horizon_world::nbt::Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        self.world.modified.insert((x, z));
        Ok(&mut extras.attachments)
    }
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
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn is_client_side(&self) -> bool {
        horizon_world::level::LevelAccess::is_client_side(self)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        self.native_state_with_property(state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for super::GameLevel<'_> {
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        self.native_level_random()
    }
}

pub(super) struct LevelTicks<'a, L> {
    pub level: L,
    pub ticks: &'a mut Ticks,
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelHost
    for LevelTicks<'_, L>
{
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_client::mod_api::LevelHost::block_state(&*self.level, pos)
    }
    fn is_client_side(&self) -> bool {
        horizon_client::mod_api::LevelHost::is_client_side(&*self.level)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        horizon_client::mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        horizon_client::mod_api::LevelRandomHost::random(&mut *self.level)
    }
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelTicksHost
    for LevelTicks<'_, L>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(self.ticks, pos, fluid, delay)
    }
}

impl horizon_client::mod_api::LevelWriteHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(self.ticks, pos, key, value, flags)
    }
}
