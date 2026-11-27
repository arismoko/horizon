pub(crate) use super::block_entity_tick::ResidentBlockEntityAccess;
use super::*;
use mod_api::{BlockEntityScope, BlockEntitySideScope};

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn server_fire_pending_block_entity_loads(&mut self) {
        self.fire_pending_block_entity_data_loads();
        if !self.common.world.block_entities.has_pending_on_load() {
            return;
        }
        let Some(level) = self.level(self.common.dim.id()) else {
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
            let mut access = ResidentBlockEntityAccess::on_load(self, subject);
            sunrise_gameplay_common::game::native_hooks::native_block_entity_on_load(
                mod_api::__private::BlockEntityOnLoadCall {
                    id,
                    subject: BlockEntitySideScope::Server(BlockEntityScope::new(&mut access)),
                },
            );
        }
    }

    /// A replacement can inherit a ticker slot later in the current pass.
    /// Deliver its attachment notice before lending its resident row to that tick.
    /// Returns whether the resident ticker was dispatched.
    pub fn tick_mod_block_entity(&mut self, pos: Pos) -> bool {
        if mod_api::__private::any_mod_disabled()
            && let Some(BlockEntity::Mod(entity)) = self.common.world.block_entities.get(&pos)
            && mod_api::__private::block_entity_owner_disabled(entity.id())
        {
            return false;
        }
        if self.common.world.block_entities.has_pending_on_load()
            || self.common.world.block_entities.has_pending_data_load()
        {
            let generation = self.common.world.block_entities.generation(&pos);
            self.fire_pending_block_entity_loads();
            if self.common.world.block_entities.generation(&pos) != generation {
                return false;
            }
        }
        self.tick_mod_block_entity_loaded(pos)
    }
}

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_load_additional::BlockEntityLoadAdditionalGameExt1 as _;

use sunrise_gameplay_common::game::native_hooks::block_entity_on_load::BlockEntityOnLoadGameExt1 as _;
