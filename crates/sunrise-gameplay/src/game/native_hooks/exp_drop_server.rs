use super::*;
use sunrise_gameplay_common::game::spill_common::{BlockDropEntity, BlockDrops};

impl crate::game::ServerGame<'_> {
    pub fn entity_block_dropped(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        breaker: Option<u32>,
        block_entity: BlockDropEntity<'_>,
        drops: Vec<ItemStack>,
    ) -> BlockDrops {
        let breaker = if sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::ExpDrop,
            state,
        ) || mod_api::__private::listened(mod_api::HookId::BlockDrops)
        {
            breaker.and_then(|id| self.entity(id))
        } else {
            None
        };
        self.with_level(|_, level, _| {
            let pos = BlockPos::new(pos[0], pos[1], pos[2]);
            // with_level rebuilds GameLevel with both fields None on every call.
            level.drop_entity = breaker;
            level.drop_breaker_at = Some(pos);
            level.block_dropped(pos, state, None, None, block_entity, drops, None)
        })
    }
    /// The block entity at `key` as a player's break finds it before the removal takes it out of
    /// the world. The saved data is read only when a `block_drops` listener will want it.
    pub(in crate::game) fn live_block_entity(
        &self,
        key: (i32, i32, i32),
        block: &str,
    ) -> Option<LiveBlockEntity> {
        let entity = self.common.world.block_entities.get(&key)?.clone();
        let data = (self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::BlockDrops))
        .then(|| self.saved_block_entity_data(key, &entity, block))
        .flatten();
        Some((entity, data))
    }
    /// `Block.dropResources` for a player's break (`block_dropped`), lending the saved data
    /// captured before removing the original block entity. `at` is the position and broken
    /// state; `who` the breaker's UUID, their tool and the block entity's payload; `loot`
    /// the stacks and an experience override.
    pub(in crate::game) fn player_block_dropped(
        &mut self,
        at: ([i32; 3], BlockStateId),
        who: (Option<u128>, Option<&ItemStack>, Option<&Compound>),
        loot: (Vec<ItemStack>, Option<u32>),
    ) -> sunrise_gameplay_common::game::spill_common::BlockDrops {
        let ([x, y, z], state) = at;
        let (breaker, tool, block_entity_data) = who;
        let block_entity = match block_entity_data {
            Some(data) => BlockDropEntity::Saved(data),
            None => BlockDropEntity::None,
        };
        self.with_level(|_, level, _| {
            level.block_dropped(
                BlockPos::new(x, y, z),
                state,
                breaker,
                tool,
                block_entity,
                loot.0,
                loot.1,
            )
        })
    }
    pub(crate) fn prepare_part_break_experience(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        experience: &mut Option<u32>,
    ) {
        if !self.native_authoritative()
            || !sunrise_gameplay_common::game::api_values::block_state::block_listens(
                mod_api::HookId::ExpDrop,
                state,
            )
        {
            return;
        }
        let tool = self.common.held().cloned();
        let breaker = Some(self.native_acting_id());
        *experience = self.with_level(|_, level, _| {
            let pos = BlockPos::new(pos[0], pos[1], pos[2]);
            let block_entity = level.resident_block_entity(pos);
            Some(level.block_break_experience(pos, state, breaker, tool.as_ref(), block_entity))
        });
    }
}

/// A player's break, before the removal: the block entity (for what the drops copy from it),
/// and for the `block_drops` event its saved data with the `CustomName`, which the removal
/// clears from the level.
pub(in crate::game) type LiveBlockEntity =
    (sunrise_level::containers::BlockEntity, Option<Compound>);

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
