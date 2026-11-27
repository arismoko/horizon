use super::*;
use mod_api::BlockEntityDataSource;

#[cfg(feature = "client")]
impl crate::game::ClientGame<'_> {
    /// `handleBlockEntityData`: the block entity at `pos`, when it is of the
    /// packet's type, loads the tag (`loadWithComponents`). An empty compound
    /// leaves stored fields unchanged; update packets still send a notice.
    pub fn remote_block_entity_data(
        &mut self,
        pos: [i32; 3],
        kind: i32,
        source: BlockEntityDataSource,
        tag: &Tag,
    ) {
        let key = (pos[0], pos[1], pos[2]);
        let Some(name) = self
            .client
            .remote
            .as_ref()
            .and_then(|r| r.builtin.get("minecraft:block_entity_type")?.name(kind))
        else {
            return;
        };
        // A decorated pot is no block entity here: its tag is kept with the chunk.
        if name == "minecraft:decorated_pot" {
            if let Tag::Compound(fields) = tag
                && !fields.is_empty()
            {
                let mut full = fields.clone();
                full.insert("id".into(), Tag::String(name.into()));
                for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
                    full.insert(k.into(), Tag::Int(v));
                }
                sunrise_gameplay_common::game::spill_common::put_saved_entity(
                    &mut self.common.world,
                    pos,
                    Tag::Compound(full),
                );
                // The section is drawn again with the pot's sherds.
                let state = self.common.world.block(pos[0], pos[1], pos[2]);
                let _ = self.common.world.set(pos[0], pos[1], pos[2], state);
            }
            return;
        }
        let Tag::Compound(fields) = tag else {
            return;
        };
        // Only the declared type on one of its valid blocks accepts an update tag.
        // Nothing is drawn from it yet, so its section stays as it is.
        let block = self.common.world.block(pos[0], pos[1], pos[2]);
        if let Some(BlockEntity::Mod(m)) = self.common.world.block_entities.get_mut(&key) {
            if m.id() == name
                && sunrise_game_data::block_entity_types::valid(
                    &self.common.res.facts().get(block).name,
                    name,
                ) == Some(true)
            {
                m.merge_update_tag(fields);
                if source == BlockEntityDataSource::UpdatePacket || !fields.is_empty() {
                    self.notify_block_entity_data_received(pos, source, fields);
                }
            }
            return;
        }
        let Some(entity) = self.common.world.block_entities.get(&key) else {
            return;
        };
        if entity.id() != name {
            return;
        }
        if fields.is_empty() && !matches!(entity, BlockEntity::Conduit(_)) {
            if source == BlockEntityDataSource::UpdatePacket
                && let Some(BlockEntity::TrialSpawner(s)) =
                    self.common.world.block_entities.get_mut(&key)
            {
                s.next_spawn_data = None;
                self.side_update_spawner_display(key);
                let state = self.common.world.block(pos[0], pos[1], pos[2]);
                let _ = self.common.world.set(pos[0], pos[1], pos[2], state);
            }
            return;
        }
        let mut full = fields.clone();
        full.insert("id".into(), Tag::String(name.into()));
        for (k, v) in ["x", "y", "z"].into_iter().zip(pos) {
            full.insert(k.into(), Tag::Int(v));
        }
        if let Some((_, mut loaded)) = sunrise_sim::save::decode_block_entity(&Tag::Compound(full))
        {
            keep_client_state(self.common.world.block_entities.get(&key), &mut loaded);
            let command_block = matches!(loaded, BlockEntity::CommandBlock(_));
            self.common.world.block_entities.insert(key, loaded);
            if source == BlockEntityDataSource::Chunk {
                sunrise_level::spans::note_block_entity_loads(1);
            }
            self.side_update_spawner_display(key);
            // The section is drawn again with what the block entity shows.
            let state = self.common.world.block(pos[0], pos[1], pos[2]);
            let _ = self.common.world.set(pos[0], pos[1], pos[2], state);
            if command_block {
                self.command_block_data_arrived(key);
            }
        }
    }
    pub(in crate::game) fn remote_block_entity_packet(
        &mut self,
        p: sunrise_protocol::packets::play::BlockEntityData,
    ) {
        self.remote_block_entity_data(p.pos, p.kind, BlockEntityDataSource::UpdatePacket, &p.tag);
    }
    pub(in crate::game) fn remote_open_command_block(&mut self, data: &[u8]) {
        assert!(
            !self.is_logical_server(),
            "remote_open_command_block called on Server role"
        );
        let Some(pos) = sunrise_sim::lan::data::decode_open_command_block(data) else {
            return;
        };
        if self.command_block_open_at(pos) {
            self.command_block_data_arrived(pos);
        } else {
            self.open_command_block(pos);
        }
    }
    /// Keep the renderable block entities (chests, furnaces) in step with blocks.
    /// `previous`: the block's state before, when it changed. A column sent again has
    /// none, and keeps its block entities' synced attachments: the server does not send
    /// them again (`remote_chunk` drops those of block entities the column lost).
    pub fn sync_block_entity(
        &mut self,
        pos: (i32, i32, i32),
        previous: Option<BlockStateId>,
        state: BlockStateId,
    ) {
        let name = self.common.res.facts().get(state).name.clone();
        // LevelChunk.setBlockState: another block's block entity goes, even
        // one of the same type (a mod's blocks may share one).
        let other_block = previous.is_some_and(|p| self.common.res.facts().get(p).name != name);
        self.ensure_stateful_entity_table();
        // State-aware factories decide creation only. A valid same-block
        // instance survives even when the new state would create none.
        if !other_block
            && self.common.world.block_entities.stateful_entity(state.0)
            && matches!(
                self.world.block_entities.get(&pos),
                Some(BlockEntity::Mod(_))
            )
        {
            return;
        }
        // A mod block's is the mod's own, as on the server.
        let removed = match self.new_block_entity_for_state(state) {
            Some(mut entity) => {
                // PotentSulfurBlockEntity.setLevel: the eruption clock starts as it joins the
                // level, not at its first tick.
                if let BlockEntity::PotentSulfur(sulfur) = &mut entity {
                    sulfur.eruption_tick = self.common.world.game_time;
                }
                let same = self.common.world.block_entities.get(&pos).is_some_and(|e| {
                    e.id() == entity.id() && !(other_block && matches!(e, BlockEntity::Mod(_)))
                });
                if same {
                    return;
                }
                // With no previous state this is a column load/adoption, not a block edit.
                if previous.is_none() {
                    sunrise_level::spans::note_block_entity_loads(1);
                }
                self.common.world.block_entities.replace(pos, entity)
            }
            None => self.common.world.block_entities.remove(&pos),
        };
        if let Some(previous) = previous {
            self.retire_client_block_entity([pos.0, pos.1, pos.2], previous, removed);
        }
    }
}

use sunrise_gameplay_common::game::side_ops::presentation::PresentationSideGameExt as _;

use sunrise_gameplay_common::game::native_hooks::SharedGameExt1 as _;
