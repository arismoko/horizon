//! Client synchronization notices for mod block entities.
use super::BlockEntity;
use mod_api::{
    BlockEntityClientReadAccess, BlockEntityClientReadScope, BlockEntityDataSource,
    BlockEntityReadView, BlockEntityRef, ClientHost,
};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::native_hooks::block_entity_read::ResidentReadBlockEntity;

struct ClientDataReadAccess<'g, 'w> {
    game: &'g crate::game::ClientGame<'w>,
    subject: BlockEntityRef,
}

impl BlockEntityClientReadAccess for ClientDataReadAccess<'_, '_> {
    fn route_kind(&self) -> Option<&str> {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let pos = (target.position[0], target.position[1], target.position[2]);
        if self.game.native_client_level() != Some(target.level)
            || self.game.world.block_entities.generation(&pos) != Some(target.generation)
        {
            return None;
        }
        match self.game.world.block_entities.get(&pos) {
            Some(BlockEntity::Mod(entity)) => Some(entity.id()),
            _ => None,
        }
    }

    fn with_view(&self, call: &mut dyn FnMut(BlockEntityReadView<'_>)) {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let [x, y, z] = target.position;
        let pos = (x, y, z);
        if self.game.native_client_level() != Some(target.level)
            || !self.game.world.loaded(x, z)
            || self.game.world.block_entities.generation(&pos) != Some(target.generation)
        {
            return;
        }
        let Some(BlockEntity::Mod(entity)) = self.game.world.block_entities.get(&pos) else {
            return;
        };
        let state_id = self.game.world.block(x, y, z);
        let Ok(state) = state_id.try_into_api() else {
            return;
        };
        let slot_count = self
            .game
            .world
            .block_entities
            .mod_container(state_id.0)
            .map_or(0, |c| c.slots);
        let data = ResidentReadBlockEntity { entity, slot_count };
        call(BlockEntityReadView::new(self.subject, state, &data));
    }

    fn host(&self) -> &dyn ClientHost {
        self.game
    }
}

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn notify_block_entity_data_received(
        &self,
        pos: [i32; 3],
        source: BlockEntityDataSource,
        tag: &sunrise_core::nbt::Compound,
    ) {
        if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityDataReceived) {
            return;
        }
        let key = (pos[0], pos[1], pos[2]);
        let Some(level) = self.native_client_level() else {
            return;
        };
        let (Some(BlockEntity::Mod(_)), Some(generation)) = (
            self.common.world.block_entities.get(&key),
            self.common.world.block_entities.generation(&key),
        ) else {
            return;
        };
        let subject = mod_api::__private::game::block_entity_ref(level, pos, generation);
        let access = ClientDataReadAccess {
            game: self,
            subject,
        };
        native_block_entity_data_received(BlockEntityClientReadScope::new(&access), source, tag);
    }
}

/// Notice after a matching mod block entity has merged its synchronized tag.
pub(in crate::game) fn native_block_entity_data_received(
    subject: mod_api::BlockEntityClientReadScope<'_>,
    source: BlockEntityDataSource,
    tag: &sunrise_core::nbt::Compound,
) {
    crate::game::native_hooks::fire::fire_block_entity_data_received(subject, source, tag);
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
