//! Snapshot position-owned loot with clone's selected block entities.
use sunrise_core::nbt::Tag;
use sunrise_level::containers::BlockEntity;
use sunrise_sim::save::PendingContainerLoot;

pub(in crate::game) struct CloneBlockPayload {
    entity: Option<BlockEntity>,
    name: Option<Tag>,
    pending: Option<PendingContainerLoot>,
}
impl crate::game::ServerGame<'_> {
    pub(in crate::game::server_side) fn clone_block_payload(
        &self,
        key: (i32, i32, i32),
    ) -> CloneBlockPayload {
        CloneBlockPayload {
            entity: self.common.world.block_entities.get(&key).cloned(),
            name: self.common.world.block_names.get(&key).cloned(),
            pending: self.common.world.block_entity_extras(key).pending_loot,
        }
    }
    /// Replacements discard the previous marker before removal can access it.
    pub(in crate::game::server_side) fn discard_clone_loot(&mut self, pos: [i32; 3]) {
        sunrise_gameplay_common::game::container_loot_level::take_block_pending(
            &mut self.common.world,
            (pos[0], pos[1], pos[2]),
        );
    }
    pub(in crate::game::server_side) fn apply_clone_payload(
        &mut self,
        pos: [i32; 3],
        payload: CloneBlockPayload,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        if let Some(entity) = payload.entity {
            self.common.world.replace_block_entity(key, entity);
            match payload.name {
                Some(name) => self.common.world.block_names.insert(key, name),
                None => self.common.world.block_names.remove(&key),
            };
            self.common.world.with_chunk_extras(key, |extras| {
                extras.pending_loot.remove(&key);
                extras
                    .pending_loot
                    .extend(payload.pending.map(|pending| (key, pending)));
            });
            self.notify_block_entity_load_additional(key);
        }
    }
}

use sunrise_gameplay_common::game::native_hooks::block_entity_load_additional::BlockEntityLoadAdditionalGameExt1 as _;
