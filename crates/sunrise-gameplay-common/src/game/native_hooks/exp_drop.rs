use super::*;
use crate::game::spill_common::BlockDropEntity;
impl GameLevel<'_> {
    pub(in crate::game) fn native_exp_drop(&self, call: super::fire::ExpDropCall<'_>) {
        self.fire_exp_drop(call);
    }
}

impl GameLevel<'_> {
    pub fn part_break_experience(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        breaker: Option<u128>,
        tool: Option<&ItemStack>,
        block_entity: BlockDropEntity<'_>,
    ) -> Option<u32> {
        if !self.native_authoritative
            || !crate::game::api_values::block_state::block_listens(mod_api::HookId::ExpDrop, state)
        {
            return None;
        }
        let (block_entity, breaker) = self.drop_event_refs(pos, breaker, block_entity);
        let mut experience = None;
        self.native_exp_drop(super::fire::ExpDropCall {
            dimension: self.dimension,
            pos,
            state,
            block_entity,
            breaker,
            tool,
            experience: &mut experience,
        });
        experience.map(|value| value.max(0) as u32)
    }
}

impl GameLevel<'_> {
    /// The block entity bound to this break and the entity that broke it, as a block's drop and
    /// experience hooks give them (`BlockDropsEvent.getBlockEntity` and `getBreaker`): the
    /// resident block entity only when its original generation still stands at `pos`, the
    /// player's entity for a player's break (`breaker`, their UUID), or the mob in `drop_entity`
    /// while its break at this position runs.
    pub(in crate::game) fn drop_event_refs(
        &self,
        pos: BlockPos,
        breaker: Option<u128>,
        entity: BlockDropEntity<'_>,
    ) -> (Option<mod_api::BlockEntityRef>, Option<mod_api::EntityRef>) {
        let at = [pos.x, pos.y, pos.z];
        let block_entity = match entity {
            BlockDropEntity::Resident(expected_generation, _) => self.api_level.and_then(|level| {
                let key = (pos.x, pos.y, pos.z);
                self.world.block_entities.get(&key)?;
                let generation = self.world.block_entities.generation(&key)?;
                (generation == expected_generation)
                    .then(|| mod_api::__private::game::block_entity_ref(level, at, generation))
            }),
            BlockDropEntity::None | BlockDropEntity::Saved(_) => None,
        };
        let breaker = if breaker.is_some() {
            self.api_level
                .zip(self.native_player_entity)
                .map(|(level, id)| mod_api::EntityRef::new(level.epoch(), id as u32))
        } else {
            self.drop_entity
                .filter(|_| self.drop_breaker_at == Some(pos))
        };
        (block_entity, breaker)
    }
}

impl GameLevel<'_> {
    /// Capture the resident block entity's generation at `pos`, if one currently exists.
    pub fn resident_block_entity(&self, pos: BlockPos) -> BlockDropEntity<'static> {
        BlockDropEntity::at(self.world, (pos.x, pos.y, pos.z), None)
    }
}
