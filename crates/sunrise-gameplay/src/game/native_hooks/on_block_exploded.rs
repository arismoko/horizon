use mod_api::{BlockEntityRef, ExplosionStart};
use sunrise_core::position::BlockPos;
use sunrise_world::registry::BlockStateId;

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn native_on_block_exploded(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        block_entity: Option<BlockEntityRef>,
        explosion: &ExplosionStart<'_>,
    ) {
        self.fire_on_block_exploded(pos, state, block_entity, explosion);
    }
}

impl crate::game::ServerGame<'_> {
    pub(in crate::game) fn notify_block_exploded(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        explosion: &mod_api::ExplosionStart<'_>,
    ) {
        if !self.common.is_logical_server()
            || !sunrise_gameplay_common::game::api_values::block_state::block_listens(
                mod_api::HookId::OnBlockExploded,
                state,
            )
        {
            return;
        }
        let block_entity = self
            .level(self.common.dim.id())
            .and_then(|level| self.block_entity(level, pos));
        self.contained_block_exploded(
            BlockPos::new(pos[0], pos[1], pos[2]),
            state,
            block_entity,
            explosion,
        );
    }
}
use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
