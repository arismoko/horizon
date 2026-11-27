use super::block_entity_client_tick::ClientBlockEntityAccess;
use super::*;
use mod_api::{BlockEntityClientScope, BlockEntitySideScope};

impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn client_fire_pending_block_entity_loads(&mut self) {
        if !self.common.world.block_entities.has_pending_on_load() {
            return;
        }
        let Some(level) = self.native_client_level() else {
            return;
        };
        let mut remaining = self.common.world.block_entities.pending_on_load_len();
        while let Some((pos, generation)) = self
            .common
            .world
            .block_entities
            .next_on_load_in_batch(&mut remaining)
        {
            if !mod_api::__private::has_part_hook(mod_api::HookId::BlockEntityOnLoad) {
                continue;
            }
            let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos) else {
                continue;
            };
            let Some(id) = mod_api::__private::block_entity_type_key(entity.id()) else {
                continue;
            };
            let subject = mod_api::__private::game::block_entity_ref(
                level,
                [pos.0, pos.1, pos.2],
                generation,
            );
            let mut access = ClientBlockEntityAccess {
                game: self,
                subject,
            };
            sunrise_gameplay_common::game::native_hooks::native_block_entity_on_load(
                mod_api::__private::BlockEntityOnLoadCall {
                    id,
                    subject: BlockEntitySideScope::Client(BlockEntityClientScope::new(&mut access)),
                },
            );
        }
    }
}
