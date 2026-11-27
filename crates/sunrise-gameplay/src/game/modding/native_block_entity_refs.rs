//! Block entity handles: storage writes join the journal; explicit world notifications happen
//! immediately and survive abort. Raw mod storage writes and restores synthesize no notice.
use crate::game::{ServerGame, server_side::native_menus::PendingMenuOpen};
use mod_api::__private::game::*;
use mod_api::{
    BlockEntityAt, BlockEntityDataError, BlockEntityError, BlockEntityItemError, BlockEntityRef,
    BlockEntityShapes, BlockEntityShapesError, LevelRef, LevelWriteError, PlayerRef,
    TransactionContext,
};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_core::position::BlockPos;
use sunrise_game_data::block_entity_nbt;
use sunrise_gameplay_common::game::container_loot_level::LootTarget;
use sunrise_gameplay_common::game::native_block_entity_read::{
    loaded_block_entity, mod_item_slots, mod_payload, save_with_metadata, save_without_metadata,
};
use sunrise_items::items::ItemStack;
use sunrise_level::block_entities::TakenType;
use sunrise_level::containers::{BlockEntity, ModBlockEntity};
use sunrise_sim::save;
use sunrise_world::registry::BlockStateId;

impl ServerGame<'_> {
    pub fn block_entity_sync(&mut self, entity: BlockEntityRef) -> Result<(), LevelWriteError> {
        if !self.common.is_logical_server() {
            return Err(LevelWriteError::Unavailable);
        }
        let fields = entity.fields();
        if self.block_entity_identity(fields.level, fields.position) != Some(entity) {
            return Err(LevelWriteError::Stale);
        }
        let world = if fields.level.dimension() == self.common.dim.key() {
            &mut self.common.world
        } else {
            self.server
                .levels
                .0
                .parked_world_mut_of(fields.level.dimension_id())
                .ok_or(LevelWriteError::Stale)?
        };
        let [x, y, z] = fields.position;
        sunrise_gameplay_common::game::block_notifications::notify_block_clients(world, (x, y, z));
        Ok(())
    }
    /// The data of the block entity at `pos`, as saved without identity or position
    /// metadata, for the hook that hears its block's removal: the entity is gone from
    /// the world by then, so it is read first. None for a block without one, or while
    /// no mod listens.
    pub(in crate::game::server_side) fn removed_block_entity_data(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> Option<Compound> {
        if !mod_api::__private::listened(mod_api::HookId::PlayerDestroyedBlock) {
            return None;
        }
        let entity = loaded_block_entity(&self.common.world, pos)?;
        let block = &self.common.pack.assets.registry.state(state)?.name;
        save_without_metadata(
            &self.common.world,
            (pos[0], pos[1], pos[2]),
            entity,
            block,
            self.common.res.items(),
        )
    }

    /// The saved data of `entity` at `key` without identity or position metadata, with its
    /// `CustomName`, for a block `block`: what a handle to it reads.
    pub(in crate::game) fn saved_block_entity_data(
        &self,
        key: (i32, i32, i32),
        entity: &BlockEntity,
        block: &str,
    ) -> Option<Compound> {
        save_without_metadata(
            &self.common.world,
            key,
            entity,
            block,
            self.common.res.items(),
        )
    }

    /// Opens a block entity's menu for a connected player (`ServerPlayer.openMenu`).
    /// Returns Stale for an invalidated entity; Ok(false) for a wrong-level player,
    /// non-provider, cancelled open, or an unconnected player such as a fake player.
    pub(in crate::game) fn player_open_menu(
        &mut self,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        self.native_block_entity_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if self.player_handle_read(player).is_none() || !self.level_in_place(entity.fields().level)
        {
            return Ok(false);
        }
        let Some(conn) = self.server.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| {
                    guest.uuid == player.fields().uuid
                        && guest.placed
                        && !guest.disconnect_pending
                        && guest.dimension == entity.fields().level.dimension_id()
                })
                .map(|guest| guest.conn)
        }) else {
            return Ok(false);
        };
        if self.menu_click_in_progress(conn) {
            return Ok(false);
        }
        if self.menu_open_waits(conn) {
            self.defer_menu_open(conn, PendingMenuOpen::Block(player, entity));
            return Ok(true);
        }
        self.finish_player_open_menu(conn, player, entity)
    }

    pub(in crate::game) fn finish_player_open_menu(
        &mut self,
        conn: u32,
        player: PlayerRef,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        if !self.level_in_place(entity.fields().level)
            || self
                .server
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(conn))
                .is_none_or(|guest| {
                    guest.disconnect_pending
                        || guest.uuid != player.uuid()
                        || guest.dimension != entity.fields().level.dimension_id()
                })
            || self.player_handle_read(player).is_none()
        {
            return Ok(false);
        }
        self.native_block_entity_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let [x, y, z] = entity.fields().position;
        match self.open_block_menu(conn, (x, y, z)) {
            sunrise_gameplay_common::game::lan_actions::MenuOpen::Stale => {
                Err(BlockEntityError::Stale)
            }
            opened => Ok(opened == sunrise_gameplay_common::game::lan_actions::MenuOpen::Opened),
        }
    }

    /// The block entities of an open server level, moving pistons' included, in the box
    /// `min..max` and the build height, read from loaded chunks only (`ServerChunkCache.getChunkNow`,
    /// `LevelChunk.getBlockEntities`). None for reversed bounds, a stale handle or a client.
    pub(in crate::game) fn level_block_entities(
        &self,
        level: LevelRef,
        min: [i32; 3],
        max: [i32; 3],
    ) -> Option<Vec<BlockEntityAt>> {
        let world = self.native_level_world(level)?;
        // The level's moving pistons as its ticks hold them now, not the copy collision reads.
        let ticks = if level.dimension() == self.common.dim.key() {
            &self.common.block_ticks
        } else {
            self.server
                .levels
                .0
                .parked_block_ticks_of(level.dimension_id())?
        };
        let registry = self.common.pack.assets.registry;
        Some(
            world
                .block_entities_in_box(min, max)?
                .into_iter()
                .filter_map(|(x, y, z)| {
                    let pos = (x, y, z);
                    let block = || registry.state(world.block(x, y, z)).map(|s| &*s.name);
                    let id = match world.block_entities.taken_type(&pos) {
                        // Taken out: it keeps its own type, whatever stands there now.
                        Some(TakenType::Mod(kind)) => {
                            return Some(BlockEntityAt::new([x, y, z], kind?));
                        }
                        Some(TakenType::Vanilla(id)) => id,
                        Some(TakenType::Container(id)) => save::chest_backed_id(id, block()?),
                        None => save::block_entity_id(world.block_entities.get(&pos)?, block()?),
                    };
                    let kind = mod_api::__private::block_entity_type_key(id)?;
                    Some(BlockEntityAt::new([x, y, z], kind))
                })
                .chain(
                    ticks
                        .moving_pistons
                        .iter()
                        .map(|(pos, _)| [pos.x, pos.y, pos.z])
                        .filter(|&pos| world.in_box(min, max, pos) && world.loaded(pos[0], pos[2]))
                        .filter_map(|pos| {
                            let kind =
                                mod_api::__private::block_entity_type_key("minecraft:piston")?;
                            Some(BlockEntityAt::new(pos, kind))
                        }),
                )
                .collect(),
        )
    }

    /// Change the lock through the same storage used by container opening and saving.
    pub(in crate::game) fn block_entity_set_lock(
        &mut self,
        entity: BlockEntityRef,
        lock: Option<Compound>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if matches!((self.world.block_lock(key), &lock), (None, None))
            || matches!((self.world.block_lock(key), &lock), (Some(Tag::Compound(old)), Some(new)) if old == new)
        {
            return Ok(false);
        }
        let Some(mut tag) = self
            .common
            .world
            .block_entities
            .get(&key)
            .and_then(|current| {
                save_with_metadata(
                    &self.common.world,
                    key,
                    current,
                    &block,
                    self.common.res.items(),
                )
            })
        else {
            return Ok(false);
        };
        match lock {
            Some(lock) => {
                tag.insert("lock".into(), Tag::Compound(lock));
            }
            None => {
                tag.remove(&"lock".into());
            }
        }
        self.journal_block_entity(key);
        self.common.world.set_block_lock(key, &Tag::Compound(tag));
        self.block_entity_updated(key);
        self.container_changed(key);
        Ok(true)
    }

    /// A loaded block entity of the server level in place, when a write may change it.
    pub(in crate::game) fn native_block_entity_write_key(
        &self,
        entity: BlockEntityRef,
    ) -> Option<(i32, i32, i32)> {
        if !self.level_in_place(entity.fields().level) {
            return None;
        }
        let pos = entity.fields().position;
        self.native_block_entity_target(entity)?;
        Some((pos[0], pos[1], pos[2]))
    }

    /// The block entity's key and block name when a write may change it.
    fn native_block_entity_write_target(
        &self,
        entity: BlockEntityRef,
    ) -> Option<((i32, i32, i32), String)> {
        let key = self.native_block_entity_write_key(entity)?;
        let block = self
            .common
            .pack
            .assets
            .registry
            .state(self.common.world.block(key.0, key.1, key.2))?
            .name
            .clone();
        Some((key, block))
    }

    /// Set a mod block entity's menu data slot silently in the current transaction.
    /// Ok(false) for a vanilla entity, invalid slot or equal value; Stale for an invalidated
    /// or unwritable handle. Notification is explicit; this does not resize slots.
    pub(in crate::game) fn block_entity_set_data_slot(
        &mut self,
        entity: BlockEntityRef,
        slot: u32,
        value: i32,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        let Some(block_entity) = self.mod_block_entity_write_target(key) else {
            return Ok(false);
        };
        let index = slot as usize;
        if block_entity
            .data
            .get(index)
            .is_none_or(|current| *current == value)
        {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data[index] = value);
        Ok(true)
    }

    /// Sizes a mod block entity's transient menu data using the same silent
    /// write and journal as scalar data-slot writes.
    pub(in crate::game) fn block_entity_set_data_slots(
        &mut self,
        entity: BlockEntityRef,
        values: Vec<i32>,
    ) -> Result<bool, BlockEntityDataError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityDataError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityDataError::Unavailable)?;
        if block_entity.data == values {
            return Ok(false);
        }
        self.write_mod_block_entity(key, |block_entity| block_entity.data = values);
        Ok(true)
    }

    fn mod_block_entity_write_target(&self, key: (i32, i32, i32)) -> Option<&ModBlockEntity> {
        let BlockEntity::Mod(block_entity) = self.common.world.block_entities.get(&key)? else {
            return None;
        };
        Some(block_entity)
    }

    // The target was resolved above, and journaling invokes no listener that could replace it.
    fn write_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        self.journal_block_entity(key);
        self.edit_mod_block_entity(key, edit);
    }

    /// `edit` on a resolved mod holder; the caller journaled the storage. Notification is the
    /// mod's explicit responsibility, including any notice it wants while restoring storage.
    fn edit_mod_block_entity(
        &mut self,
        key: (i32, i32, i32),
        edit: impl FnOnce(&mut ModBlockEntity),
    ) {
        let BlockEntity::Mod(block_entity) =
            self.common.world.block_entities.get_mut(&key).unwrap()
        else {
            unreachable!("resolved mod holder replaced before its write")
        };
        edit(block_entity);
    }

    /// Replace internal storage under the existing transaction; external ITEMS routing is separate.
    pub(in crate::game) fn block_entity_set_item_slot(
        &mut self,
        entity: BlockEntityRef,
        tx: &TransactionContext,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<bool, BlockEntityItemError> {
        if !self.transaction_context_is_current(tx) {
            return Err(BlockEntityItemError::InvalidContext);
        }
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityItemError::Stale)?;
        let block_entity = self
            .mod_block_entity_write_target(key)
            .ok_or(BlockEntityItemError::Unavailable)?;
        mod_item_slots(&self.common.world, key, slot)?;
        if let Some(stack) = &stack {
            if stack.count == 0
                || stack.item.as_ref() == "minecraft:air"
                || self.items().get(&stack.item).is_none()
            {
                return Err(BlockEntityItemError::InvalidStack);
            }
            let limit = self.stack_max_size(stack);
            if stack.count > limit {
                return Err(BlockEntityItemError::StackTooLarge {
                    count: stack.count,
                    limit,
                });
            }
        }
        let index = slot as usize;
        if block_entity.items.get(index).and_then(Option::as_ref) == stack.as_ref()
            && !block_entity.has_opaque_item_slot(index)
        {
            return Ok(false);
        }
        // A slot of a list already its full length: the journal keeps that slot, not the
        // entity's whole saved data. A list still to grow is kept whole, length and all.
        if index < block_entity.items.len() {
            self.journal_block_entity_slot(key, index);
            self.edit_mod_block_entity(key, |block_entity| {
                *block_entity.item_slot_mut(index) = stack;
            });
        } else {
            self.write_mod_block_entity(key, |block_entity| {
                *block_entity.item_slot_mut(index) = stack;
            });
        }
        self.journal_chunk_save(
            (key.0 >> 4, key.2 >> 4),
            sunrise_gameplay_common::game::transactions::SaveChunkScope::Level,
        );
        Ok(true)
    }

    /// Merges `data` into the block entity's saved data and loads it, as `/data merge block`
    /// does (`BlockDataAccessor.setData`: `loadWithComponents`, `setChanged`, then
    /// `sendBlockUpdated`, so comparators re-read it and clients get the update). `id`, `x`,
    /// `y` and `z` in `data` are ignored. Ok(true) when data changed; Ok(false) for data
    /// nested too deep, no change, or data the entity cannot load. Stale or unwritable
    /// handles return BlockEntityError::Stale. Inside a transaction an
    /// abort puts the block entity, its name, `lock`, loot table roll and attachments back.
    pub(in crate::game) fn block_entity_merge_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, false, true)
    }

    /// `merge_data` with `BlockEntity.setChanged` only: no `Level.sendBlockUpdated`.
    pub(in crate::game) fn block_entity_merge_data_quiet(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, false, false)
    }

    /// Replaces the saved payload, retaining identity and position metadata.
    /// Uses the same transactional load and client update path as `merge_data`.
    /// Ok(false) for unsupported data or no change; Stale for an invalidated handle.
    pub(in crate::game) fn block_entity_replace_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, true, true)
    }

    /// `replace_data` with `BlockEntity.setChanged` only: no `Level.sendBlockUpdated`.
    pub(in crate::game) fn block_entity_replace_data_quiet(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
    ) -> Result<bool, BlockEntityError> {
        self.write_block_entity_data(entity, data, true, false)
    }

    fn write_block_entity_data(
        &mut self,
        entity: BlockEntityRef,
        data: Compound,
        replace: bool,
        send: bool,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let nbt = Tag::Compound(mod_payload(data));
        if sunrise_game_data::nbt_path::too_deep(&nbt, 0) {
            return Ok(false);
        }
        let Some(old) = self
            .common
            .world
            .block_entities
            .get(&key)
            .and_then(|current| {
                save_with_metadata(
                    &self.common.world,
                    key,
                    current,
                    &block,
                    self.common.res.items(),
                )
            })
            .map(Tag::Compound)
        else {
            return Ok(false);
        };
        let merged = loaded_payload(&old, &nbt, replace);
        if merged == old {
            return Ok(false);
        }
        self.journal_block_entity(key);
        let cleared = matches!(
            self.world.block_entities.get(&key),
            Some(BlockEntity::Mod(current)) if current.shapes_cleared()
        );
        if !self.load_block_entity_data(key, &merged) {
            return Ok(false);
        }
        if cleared
            && let Some(BlockEntity::Mod(loaded)) = self.common.world.block_entities.get_mut(&key)
        {
            loaded.keep_shapes_cleared(true);
        }
        // `BlockDataAccessor.setData`: `setChanged`, then `sendBlockUpdated` unless the mod
        // keeps the send for `block_entity_sync`.
        if send {
            self.block_entity_updated(key);
        } else {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        self.container_changed(key);
        Ok(true)
    }

    /// `RandomizableContainer.setLootTable(table, seed)` on a mod container: kept beside the
    /// block entity as a vanilla chest's is, and rolled by its first access
    /// (`container_loot::unpack_loot`).
    pub(in crate::game) fn block_entity_set_loot_table(
        &mut self,
        entity: BlockEntityRef,
        table: &str,
        seed: i64,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let Some(entity) = self.common.world.block_entities.get(&key) else {
            return Err(BlockEntityError::Stale);
        };
        let Some(table) = sunrise_game_data::identifier::normalize_id(table) else {
            return Ok(false);
        };
        if !matches!(entity, BlockEntity::Mod(_)) || !save::randomizable_container(entity, &block) {
            return Ok(false);
        }
        // The table and seed as `trySaveLootTable` writes them and a reload reads them back:
        // the identifier in full, and no seed for 0.
        let pending = save::PendingContainerLoot {
            table: Tag::String(table.as_str().into()),
            seed: (seed != 0).then_some(Tag::Long(seed)),
        };
        let current = self
            .common
            .world
            .chunks
            .get(&(key.0 >> 4, key.2 >> 4))
            .and_then(|chunk| chunk.saved.as_ref())
            .and_then(|saved| saved.pending_loot.get(&key));
        if current.is_some_and(|current| {
            current.table_id() == table && current.seed_value().unwrap_or(0) == seed
        }) {
            return Ok(false);
        }
        self.journal_block_entity(key);
        self.common.world.with_chunk_extras(key, |extras| {
            extras.pending_loot.insert(key, pending);
        });
        self.common.world.modified.insert((key.0 >> 4, key.2 >> 4));
        Ok(true)
    }

    /// `RandomizableContainer.unpackLootTable(null)`, what vanilla's `getItem`, `setItem`,
    /// `removeItem` and `isEmpty` run first: a pending table rolls into the container's empty
    /// slots now, with no player. False when no table waits.
    pub(in crate::game) fn block_entity_unpack_loot_table(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        let (key, _) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        if !self.block_loot_pending(key) {
            return Ok(false);
        }
        self.journal_block_entity(key);
        self.unpack_loot(LootTarget::Block(key), None);
        Ok(true)
    }

    /// `BlockEntity.setChanged`: notify at once, including inside a transaction. An abort
    /// restores storage, not notifications already delivered to observers or comparators.
    pub(in crate::game) fn block_entity_set_changed(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<bool, BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        self.container_changed(key);
        Ok(true)
    }

    /// Removes any resident block entity in place (`Level.removeBlockEntity`,
    /// `LevelChunk.removeBlockEntity`, `BlockEntity.setRemoved`), without changing its
    /// block or state and without running `pre_remove_side_effects`.
    pub(in crate::game) fn block_entity_remove(
        &mut self,
        entity: BlockEntityRef,
    ) -> Result<(), BlockEntityError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityError::Stale)?;
        let state = self.common.world.block(key.0, key.1, key.2);
        let current = self
            .common
            .world
            .block_entities
            .get(&key)
            .ok_or(BlockEntityError::Stale)?;
        let effects = sunrise_gameplay_common::game::transactions::RemovedBlockEntityEffects {
            pos: key,
            state,
            sculk: matches!(current, BlockEntity::Sculk(_)),
            jukebox: matches!(current, BlockEntity::Jukebox(_)),
            beacon: matches!(current, BlockEntity::Beacon(_)),
            gateway: matches!(current, BlockEntity::EndGateway(_)),
            had_shapes: matches!(current, BlockEntity::Mod(entity) if entity.shapes().is_some()),
        };
        let deferred = self.journal_explicit_remove_block_entity(key, effects);
        // LevelChunk.removeBlockEntity (i) + (iv): remove from map, retire attachments and
        // watches, invalidate generation, and free/remove the ticker.
        self.common.world.block_entities.remove(&key);
        self.common.world.block_names.remove(&key);
        self.common.world.forget_block_entity_source_unmodified(key);
        self.server.hoppers.remove(&key);
        if !deferred {
            self.apply_removed_block_entity_effects(effects);
        }
        Ok(())
    }

    /// Runs the `LevelChunk.removeBlockEntity` / `BlockEntity.setRemoved` effects
    /// (steps ii and iii, plus gateway cache cleanup and neighbour updates when a mod
    /// block entity's published shapes disappeared): immediately outside a transaction,
    /// or at root commit when deferred.
    pub(in crate::game) fn apply_removed_block_entity_effects(
        &mut self,
        effects: sunrise_gameplay_common::game::transactions::RemovedBlockEntityEffects,
    ) {
        // (ii) Server-side game-event listener unregister (LevelChunk.java:486-493).
        if effects.sculk {
            self.sculk_unregister_listener(effects.pos);
        }
        if effects.gateway {
            self.server.be_anims.gateway_servers.remove(&[
                effects.pos.0,
                effects.pos.1,
                effects.pos.2,
            ]);
        }
        let at = BlockPos::new(effects.pos.0, effects.pos.1, effects.pos.2);
        // (iii) BlockEntity.setRemoved subclass effects: JukeboxBlockEntity stops its
        // song, BeaconBlockEntity plays its deactivation sound.
        if effects.jukebox {
            self.common
                .level_events
                .push(sunrise_world::level::LevelEvent::GameEvent {
                    pos: at,
                    event: "minecraft:jukebox_stop_play",
                    state: Some(effects.state),
                });
            self.common
                .level_events
                .push(sunrise_world::level::LevelEvent::LevelEvent {
                    id: 1011,
                    pos: at,
                    data: 0,
                    source: None,
                });
        }
        if effects.beacon {
            self.common
                .level_events
                .push(sunrise_world::level::LevelEvent::Sound {
                    pos: at,
                    sound: "minecraft:block.beacon.deactivate".into(),
                    source: None,
                    volume: 1.0,
                    pitch: 1.0,
                });
        }
        if effects.jukebox || effects.beacon {
            let _ = self.server_process_level_events();
        }
        // Fluids and neighbours read published shapes; notify them when shapes disappear,
        // matching `block_entity_set_shapes(..., None)`.
        if effects.had_shapes {
            let state = self
                .common
                .world
                .block(effects.pos.0, effects.pos.1, effects.pos.2);
            self.with_level(|b, level, ticks| {
                let block = b.block(state);
                b.update_neighbors_at(level, ticks, at, block);
            });
            let _ = self.server_process_level_events();
        }
    }

    /// Register a generation-bound observer in its level's keyed contents index.
    pub(in crate::game) fn block_entity_watch_contents(
        &mut self,
        entity: BlockEntityRef,
        source: mod_api::BlockPos,
        enabled: bool,
    ) -> Result<(), mod_api::ContentsWatchError> {
        let fields = entity.fields();
        if !self.level_in_place(fields.level)
            || self.block_entity_identity(fields.level, fields.position) != Some(entity)
        {
            return Err(mod_api::ContentsWatchError::Stale);
        }
        let [x, y, z] = fields.position;
        if !sunrise_gameplay_common::game::api_values::block_state::block_listens(
            mod_api::HookId::ContentsChanged,
            self.common.world.block(x, y, z),
        ) {
            return Err(mod_api::ContentsWatchError::NotSubscribed);
        }
        self.journal_block_entity((x, y, z));
        let observer = self
            .common
            .world
            .block_entities
            .identity((x, y, z))
            .unwrap();
        self.common.world.block_entities.contents_watches.set(
            observer,
            (source.x, source.y, source.z),
            enabled,
        );
        Ok(())
    }

    /// Publishes the shapes the block entity's block has in place of its declared ones, or
    /// clears them with None. Applied at once, to the queries that follow, and the block's
    /// neighbours are notified; clients get them with the block entity update. Inside a
    /// transaction an abort puts the old ones back.
    pub fn block_entity_set_shapes(
        &mut self,
        entity: BlockEntityRef,
        shapes: Option<BlockEntityShapes>,
    ) -> Result<bool, BlockEntityShapesError> {
        let key = self
            .native_block_entity_write_key(entity)
            .ok_or(BlockEntityShapesError::Stale)?;
        let state = self.common.world.block(key.0, key.1, key.2);
        if !self.common.collision.publishes_shapes(state) {
            return Err(BlockEntityShapesError::NotDeclared);
        }
        let shapes = shapes
            .map(sunrise_level::containers::PublishedShapes::new)
            .transpose()?
            .map(std::sync::Arc::new);
        let Some(current) = self.mod_block_entity_write_target(key) else {
            return Err(BlockEntityShapesError::Stale);
        };
        if current.shapes() == shapes.as_deref() {
            return Ok(false);
        }
        if cfg!(debug_assertions)
            && shapes
                .as_ref()
                .is_some_and(|shapes| shapes.reaches_past_one_cell())
        {
            static WARNED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            let block = self
                .common
                .pack
                .assets
                .registry
                .state(state)
                .map(|s| s.name.clone());
            if let (Some(block), Ok(mut warned)) = (block, WARNED.lock())
                && !warned.contains(&block)
            {
                log::warn!(
                    "{block}: a published shape has a box beyond one block of the block; collision never finds it"
                );
                warned.push(block);
            }
        }
        self.write_mod_block_entity(key, |block_entity| {
            block_entity.set_shapes(shapes);
        });
        self.block_entity_updated(key);
        // Fluids and other neighbours read the shape: tell them it changed (the shulker lid's
        // `doNeighborUpdates`, without its shape updates).
        let at = BlockPos::new(key.0, key.1, key.2);
        self.with_level(|b, level, ticks| {
            let block = b.block(state);
            b.update_neighbors_at(level, ticks, at, block);
        });
        Ok(true)
    }

    /// The custom name (`Nameable.getCustomName`) as plain text. None when the block entity
    /// has none or isn't nameable (chests, barrels, furnaces, hoppers, brewing stands,
    /// banners, beacons, enchanting tables, command blocks...), or for a stale handle or a
    /// client.
    pub(in crate::game) fn block_entity_name(&self, entity: BlockEntityRef) -> Option<String> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match block_entity {
            BlockEntity::Banner(banner) => banner.name.clone(),
            _ => world.block_names.get(&(x, y, z)).map(|name| match name {
                Tag::String(text) => text.to_string_lossy(),
                component => sunrise_game_data::component::name_text(component).plain(),
            }),
        }
    }

    /// The players whose open menu holds this block entity's container (for openers'
    /// containers, `ContainerOpenersCounter.getEntitiesWithContainerOpen`'s players), from the menus the opener counts read
    /// (`tick_mod_container_viewers`, `ChestBlockEntity$1.isOwnContainer`). Guests in
    /// its level, not spectators nor fake players, then the acting player for a menu
    /// their action opened, which the hub records when the action ends. None for a
    /// stale handle or a client.
    pub(in crate::game) fn block_entity_viewers(
        &self,
        entity: BlockEntityRef,
    ) -> Option<Vec<PlayerRef>> {
        if !self.common.is_logical_server() {
            return None;
        }
        self.native_block_entity_target(entity)?;
        let epoch = self.common.native_lifecycle.epoch()?;
        let fields = entity.fields();
        let level = fields.level;
        let [x, y, z] = fields.position;
        let key = (x, y, z);
        let mut viewers: Vec<PlayerRef> = self
            .server
            .lan
            .iter()
            .flat_map(|host| host.hub.guests.simulation_iter().map(move |g| (host, g)))
            .filter(|(host, g)| {
                // The acting guest's mode is live in `Game` during its turn; the hub
                // hears of a change only in the put-back at the end of `guest_turn`.
                let spectator = if self.server.lan_acting_uuid == Some(g.uuid) {
                    self.common.spectator()
                } else {
                    g.spectator()
                };
                g.participates_online()
                    && g.placed
                    && !spectator
                    && g.dimension == level.dimension_id()
                    && host.menus.get(&g.conn).is_some_and(|menus| {
                        menus.menu_holds(g.menu.as_ref(), key, fields.generation)
                    })
            })
            .filter_map(|(_, g)| {
                let id = u32::try_from(g.entity_id).ok()?;
                Some(player_ref(mod_api::EntityRef::new(epoch, id), g.uuid))
            })
            .collect();
        if self.level_in_place(level)
            && !self.server.lan_acting_fake
            && !self.common.spectator()
            && self
                .common
                .container
                .is_some_and(|open| open == key || self.common.chest_other_half(open) == Some(key))
            && let Some(player) = self.native_acting_player()
            && !viewers.contains(&player)
        {
            viewers.push(player);
        }
        Some(viewers)
    }

    /// Sets the custom name, or clears it with None, as an anvil-renamed item placed with
    /// `components` sets `CustomName`. Plain text only. Clients get a block entity update.
    /// Ok(false) for an entity that is not nameable; Stale for an invalidated or
    /// unwritable handle. Inside a transaction an abort puts the name back.
    pub(in crate::game) fn block_entity_set_name(
        &mut self,
        entity: BlockEntityRef,
        name: Option<&str>,
    ) -> Result<bool, BlockEntityError> {
        let (key, block) = self
            .native_block_entity_write_target(entity)
            .ok_or(BlockEntityError::Stale)?;
        let nameable = self
            .common
            .world
            .block_entities
            .get(&key)
            .is_some_and(|current| nameable(key, current, &block, self.common.res.items()));
        if !nameable {
            return Ok(false);
        }
        self.journal_block_entity(key);
        match self.common.world.block_entities.get_mut(&key) {
            Some(BlockEntity::Banner(banner)) => banner.name = name.map(str::to_owned),
            _ => {
                match name {
                    Some(name) => self
                        .common
                        .world
                        .block_names
                        .insert(key, Tag::String(name.into())),
                    None => self.common.world.block_names.remove(&key),
                };
            }
        }
        self.block_entity_updated(key);
        Ok(true)
    }
}

/// The compound a merge (or, with `replace`, a replace) of `payload` loads over the block entity
/// saved as `old`. A replace keeps the identity, position and published shapes, which are not
/// the payload's to replace.
fn loaded_payload(old: &Tag, payload: &Tag, replace: bool) -> Tag {
    let base = if replace {
        let mut metadata = Compound::new();
        if let Tag::Compound(fields) = old {
            for key in ["id", "x", "y", "z", sunrise_level::containers::SHAPES_KEY] {
                if let Some(value) = fields.get(&key.into()) {
                    metadata.insert(key.into(), value.clone());
                }
            }
        }
        Tag::Compound(metadata)
    } else {
        old.clone()
    };
    sunrise_sim::data_cmd::merged(&base, payload)
}

#[cfg(test)]
mod payload_tests {
    use super::*;

    #[test]
    fn merge_and_replace_keep_the_published_shapes_whatever_the_payload_carries() {
        use sunrise_level::containers::SHAPES_KEY;
        let shapes = Tag::Compound(Compound::from([(
            "collision".into(),
            Tag::LongArray(vec![1, 2, 3, 4, 5, 6]),
        )]));
        let old = Tag::Compound(Compound::from([
            ("id".into(), Tag::String("test:bus".into())),
            ("x".into(), Tag::Int(1)),
            ("progress".into(), Tag::Int(7)),
            ("kept".into(), Tag::Int(9)),
            (SHAPES_KEY.into(), shapes.clone()),
        ]));
        // A payload that carries the key, to overwrite it or to remove it.
        let colliding = || {
            mod_payload(Compound::from([
                ("progress".into(), Tag::Int(8)),
                ("id".into(), Tag::String("other:type".into())),
                (SHAPES_KEY.into(), Tag::Compound(Compound::new())),
            ]))
        };
        for replace in [false, true] {
            let loaded = loaded_payload(&old, &Tag::Compound(colliding()), replace);
            assert_eq!(loaded.get(SHAPES_KEY), Some(&shapes), "replace: {replace}");
            // The ordinary field applies, and the identity stays.
            assert_eq!(loaded.get("progress"), Some(&Tag::Int(8)));
            assert_eq!(loaded.get("id"), Some(&Tag::String("test:bus".into())));
            // A merge keeps the fields the payload omits; a replace drops them.
            assert_eq!(loaded.get("kept").is_some(), !replace);
        }
    }
}

/// Whether the block entity keeps a `CustomName` (`Nameable`): its saved form does.
fn nameable(
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &sunrise_items::items::Items,
) -> bool {
    let mut tag = save::block_entity_encode::encode(pos, entity, block, items);
    if let Tag::Compound(map) = &mut tag {
        map.insert("CustomName".into(), Tag::String("x".into()));
    }
    block_entity_nbt::canonical(&tag).is_some_and(|tag| tag.get("CustomName").is_some())
}

#[cfg(test)]
mod tests {
    use sunrise_gameplay_common::game::server_side::native_block_entity_refs::tests::compound;

    use super::*;
    use mod_api::HostTransaction;
    #[cfg(test)]
    use mod_api::ServerHost;
    use mod_api::TransactionError;
    use std::sync::Arc;
    use sunrise_level::world::World;
    use sunrise_level::{chunk::Chunk, containers::ModBlockEntityFields};
    #[cfg(test)]
    use sunrise_world::registry::BlockStateId;

    #[test]
    fn saved_data_omits_metadata_but_keeps_contents() {
        let world = World::new(BlockStateId(0), 0);
        let entity = BlockEntity::Chest(vec![None; 27]);
        let data = save_without_metadata(
            &world,
            (1, 64, 2),
            &entity,
            "minecraft:chest",
            &Default::default(),
        )
        .unwrap();
        for key in ["id", "x", "y", "z"] {
            assert!(!data.contains_key(&key.into()));
        }
        assert!(data.contains_key(&"Items".into()));
    }

    /// What a reader of a vanilla machine's progress (a crafting CPU's estimate, a progress
    /// line) needs is in `BlockEntity.saveWithoutMetadata`, which `block_entity_data` returns:
    /// a furnace's cooking and burning timers and the fuel's speed, a brewing stand's brew
    /// timer counting down, its total, its fuel and its bottles.
    #[test]
    fn saved_data_carries_vanilla_machine_progress() {
        use sunrise_level::{
            brewing::BrewingStand,
            containers::{Furnace, FurnaceKind},
        };
        let world = World::new(BlockStateId(0), 0);
        let int = |data: &Compound, key: &str| match data.get(&key.into()) {
            Some(Tag::Int(value)) => *value,
            other => panic!("{key} is {other:?}"),
        };
        let float = |data: &Compound, key: &str| match data.get(&key.into()) {
            Some(Tag::Float(bits)) => f32::from_bits(*bits),
            other => panic!("{key} is {other:?}"),
        };
        let slots = |data: &Compound| match data.get(&"Items".into()) {
            Some(Tag::List(items)) => items
                .iter()
                .map(|item| match item.get("Slot") {
                    Some(Tag::Byte(slot)) => *slot,
                    other => panic!("slot is {other:?}"),
                })
                .collect::<Vec<_>>(),
            other => panic!("Items is {other:?}"),
        };

        let furnace = BlockEntity::Furnace(Furnace {
            kind: FurnaceKind::BlastFurnace,
            items: [Some(ItemStack::new("minecraft:raw_iron", 3)), None, None],
            lit_remaining: 750,
            lit_total: 800,
            cooking_timer: 40,
            cooking_total: 100,
            speed: 2.0,
            recipes_used: Default::default(),
        });
        let data = save_without_metadata(
            &world,
            (0, 64, 0),
            &furnace,
            "minecraft:blast_furnace",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(int(&data, "cooking_time_spent"), 40);
        assert_eq!(int(&data, "cooking_total_time"), 100);
        assert_eq!(int(&data, "lit_time_remaining"), 750);
        assert_eq!(int(&data, "lit_total_time"), 800);
        assert_eq!(float(&data, "speed_multiplier"), 2.0);
        assert_eq!(slots(&data), vec![0], "the input slot is held");

        let mut stand = BrewingStand::new();
        stand.brew_time = 150;
        stand.total_brew_time = 400;
        stand.fuel = 12;
        stand.items[0] = Some(ItemStack::new("minecraft:potion", 1));
        stand.items[2] = Some(ItemStack::new("minecraft:potion", 1));
        stand.items[3] = Some(ItemStack::new("minecraft:nether_wart", 1));
        let stand = BlockEntity::Brewing(Box::new(stand));
        let data = save_without_metadata(
            &world,
            (1, 64, 0),
            &stand,
            "minecraft:brewing_stand",
            &Default::default(),
        )
        .unwrap();
        assert_eq!(int(&data, "BrewTime"), 150);
        assert_eq!(int(&data, "total_brew_time"), 400);
        assert_eq!(int(&data, "Fuel"), 12);
        assert_eq!(float(&data, "speed_multiplier"), 1.0);
        assert_eq!(
            slots(&data),
            vec![0, 2, 3],
            "two bottles and the ingredient"
        );
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_sync_targets_the_live_holder_even_when_ticking_or_parked() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        use sunrise_gameplay_common::game::Dim;

        if run_fresh_cases(
            "game::server_side::native_block_entity_refs::tests::block_entity_sync_targets_the_live_holder_even_when_ticking_or_parked",
            "SUNRISE_BLOCK_ENTITY_SYNC_CHILD",
            &["holder"],
        ) {
            return;
        }
        with_game(|game| {
            let key = (1, 64, 1);
            let pos = [key.0, key.1, key.2];
            let level = game.level("minecraft:overworld").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
            game.world.block_log = Some(Vec::new());
            let holder = game.block_entity(level, pos).unwrap();
            let (lease, entity) = game.world.block_entities.take(&key).unwrap();
            assert!(game.block_entity(level, pos).is_none());
            assert_eq!(holder.sync(game), Ok(()));
            assert_eq!(game.world.block_log.as_deref(), Some(&[key][..]));
            assert!(game.world.block_entities.put(lease, entity));
            game.world.block_log = Some(Vec::new());

            game.enter_command_level(Dim::Nether).unwrap();
            game.world.block_log = Some(Vec::new());
            assert_eq!(holder.sync(game), Ok(()));
            assert_eq!(game.dim, Dim::Nether);
            assert!(game.world.block_log.as_ref().unwrap().is_empty());
            assert_eq!(
                game.native_level_world(level).unwrap().block_log.as_deref(),
                Some(&[key][..]),
            );

            let world = game
                .server
                .levels
                .0
                .parked_world_mut_of("minecraft:overworld")
                .unwrap();
            world.block_entities.remove(&key);
            world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
            assert_eq!(holder.sync(game), Err(LevelWriteError::Stale));
            assert_eq!(
                game.native_level_world(level).unwrap().block_log.as_deref(),
                Some(&[key][..]),
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn retained_handles_reject_replacements_and_rollback_restores_identity() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game};
        if run_fresh_cases(
            "game::server_side::native_block_entity_refs::tests::retained_handles_reject_replacements_and_rollback_restores_identity",
            "SUNRISE_RETAINED_HANDLES_CHILD",
            &["holder"],
        ) {
            return;
        }
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let pos = (1, 64, 1);
            let holder = || {
                BlockEntity::Mod(Box::new(ModBlockEntity::new(
                    "test:holder".into(),
                    ModBlockEntityFields {
                        nbt: compound("{count:3}"),
                        ..Default::default()
                    },
                )))
            };
            game.world.block_entities.insert(pos, holder());
            let original = game.block_entity(level, [1, 64, 1]).unwrap();
            game.with_level(|_, host, ticks| {
                let wrapped = super::super::mod_api_host::LevelTicks::new(&*host, ticks);
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 64, 1]),
                    mod_api::LevelHost::block_entity(&*host, [1, 64, 1]),
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 64, 1]),
                    Some(original),
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity(&wrapped, [1, 400, 1]),
                    None,
                );
                // The ticks wrapper hands the same reads through, saved data included.
                let data = mod_api::LevelHost::block_entity_data(&*host, original);
                assert_eq!(
                    data.as_ref().and_then(|data| data.get(&"count".into())),
                    Some(&mod_api::Tag::Int(3))
                );
                assert_eq!(
                    mod_api::LevelHost::block_entity_data(&wrapped, original),
                    data,
                );
            });
            assert_eq!(original.set_data_slots(game, vec![7]), Ok(true));
            let mut tentative = None;
            let _: Result<(), ()> = game.storage_transaction(|game, _| {
                game.journal_block_entity(pos);
                game.world
                    .block_entities
                    .insert(pos, BlockEntity::Chest(vec![]));
                assert_eq!(original.data_slots(game), None);
                tentative = game.block_entity(level, [1, 64, 1]);
                Err(())
            });
            assert_eq!(original.data_slots(game), Some(vec![7]));
            assert_eq!(tentative.unwrap().data(game), None);
            let (lease, taken) = game.world.block_entities.take(&pos).unwrap();
            assert_eq!(original.data_slots(game), None);
            assert!(game.world.block_entities.put(lease, taken));
            assert_eq!(original.data_slots(game), Some(vec![7]));
            game.world.block_entities.remove(&pos);
            game.world.block_entities.insert(pos, holder());
            let replacement = game.block_entity(level, [1, 64, 1]).unwrap();
            assert_ne!(original, replacement);
            let stale = Err(BlockEntityError::Stale);
            assert_eq!(original.set_lock(game, None), stale);
            assert_eq!(original.set_name(game, Some("stale")), stale);
            assert_eq!(original.merge_data(game, Compound::new()), stale);
            assert_eq!(original.replace_data(game, Compound::new()), stale);
            assert_eq!(original.set_changed(game), stale);
            assert_eq!(original.set_data_slot(game, 0, 99), stale);
            assert_eq!(
                original.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
            assert_eq!(
                original.item_slot(game, 0),
                Err(BlockEntityItemError::Stale)
            );
            assert_eq!(replacement.data_slots(game), Some(vec![]));
            assert_eq!(replacement.set_data_slots(game, vec![8]), Ok(true));
            game.world.block_entities.load_chunk(vec![(pos, holder())]);
            assert_eq!(
                replacement.set_data_slots(game, vec![99]),
                Err(BlockEntityDataError::Stale)
            );
        });
    }

    #[test]
    fn lookup_requires_loaded_chunk_and_tracks_removal_and_replacement() {
        let mut world = World::new(BlockStateId(0), 0);
        let pos = [-1, 64, 0];
        let key = (-1, 64, 0);
        world.block_entities.insert(key, BlockEntity::Chest(vec![]));
        assert!(loaded_block_entity(&world, pos).is_none());
        world.chunks.insert((-1, 0), Arc::new(Chunk::empty(-1, 0)));
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::Chest(_))
        ));
        world.block_entities.remove(&key);
        assert!(loaded_block_entity(&world, pos).is_none());
        world
            .block_entities
            .insert(key, BlockEntity::DaylightDetector);
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::DaylightDetector)
        ));
        let (lease, detached) = world.block_entities.take(&key).unwrap();
        assert!(loaded_block_entity(&world, pos).is_none());
        assert!(world.block_entities.put(lease, detached));
        assert!(matches!(
            loaded_block_entity(&world, pos),
            Some(BlockEntity::DaylightDetector)
        ));
        let outside_pos = [-1, world.shape.max_y(), 0];
        world
            .block_entities
            .insert((-1, outside_pos[1], 0), BlockEntity::DaylightDetector);
        assert!(loaded_block_entity(&world, outside_pos).is_none());
        let below_pos = [-1, world.shape.min_y - 1, 0];
        world
            .block_entities
            .insert((-1, below_pos[1], 0), BlockEntity::DaylightDetector);
        assert!(loaded_block_entity(&world, below_pos).is_none());
    }

    #[test]
    #[ignore = "daily"]
    fn an_abort_puts_back_the_lock_and_the_loot_table_roll() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let chest = game.block_entity(level, [0, 64, 0]).unwrap();
            let key = (0, 64, 0);
            let lock = compound("{lock:{items:\"minecraft:stick\"}}");
            let table = compound("{LootTable:\"minecraft:chests/simple_dungeon\"}");
            let before = game.world.block_entity_extras(key);

            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(
                    chest
                        .set_lock(host, Some(compound("{items:\"minecraft:stick\"}")))
                        .unwrap()
                );
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Err(())
            });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!(game.world.block_entity_extras(key), before);
            assert!(game.world.block_lock(key).is_none());

            let host: &mut dyn ServerHost = game;
            let committed: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert!(chest.merge_data(host, lock.clone()).unwrap());
                assert!(chest.merge_data(host, table.clone()).unwrap());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert!(game.world.block_lock(key).is_some());
            assert_ne!(game.world.block_entity_extras(key), before);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_remove_mod_and_vanilla_and_stale_cases() {
        use super::super::native_entity_refs::tests::with_game;
        use mod_api::BlockUpdateFlags;
        use sunrise_gameplay_common::game::Dim;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let registry = game.pack.assets.registry;

            // 1. Mod block entity: leaves block & state untouched, invalidates handle, clears
            //    name/lock/watches, does not mark chunk unsaved, does not drop items.
            let mod_key = (1, 64, 1);
            let mod_pos = [1, 64, 1];
            let stone = registry.default_state("minecraft:stone").unwrap();
            game.world.set(1, 64, 1, stone).unwrap();
            let mut mod_be = ModBlockEntity::new(
                "test:holder".into(),
                ModBlockEntityFields {
                    nbt: compound("{count:9}"),
                    ..Default::default()
                },
            );
            *mod_be.item_slot_mut(0) = Some(ItemStack::new("minecraft:diamond", 4));
            game.world
                .block_entities
                .insert(mod_key, BlockEntity::Mod(Box::new(mod_be)));
            let mod_ref = game.block_entity(level, mod_pos).unwrap();
            game.world
                .block_names
                .insert(mod_key, Tag::String("Holder".into()));
            assert_eq!(mod_ref.set_lock(game, Some(Compound::new())), Ok(true));
            game.world.modified.clear();
            game.dropped.clear();

            assert_eq!(mod_ref.remove(game), Ok(()));
            assert_eq!(game.world.block(1, 64, 1), stone);
            assert!(game.block_entity(level, mod_pos).is_none());
            assert_eq!(mod_ref.remove(game), Err(BlockEntityError::Stale));
            assert_eq!(mod_ref.name(game), None);
            assert_eq!(mod_ref.lock(game), None);
            assert!(!game.world.block_names.contains_key(&mod_key));
            assert!(game.world.block_lock(mod_key).is_none());
            assert!(game.dropped.is_empty(), "preRemoveSideEffects must not run");
            assert!(
                !game.world.modified.contains(&(0, 0)),
                "LevelChunk.removeBlockEntity does not mark the chunk unsaved"
            );

            // 2. Vanilla chest: leaves chest block untouched, does not drop contents, does not
            //    mark chunk modified, and next container access creates a fresh empty entity.
            let chest_key = (2, 64, 1);
            let chest_pos = [2, 64, 1];
            let chest_state = registry.default_state("minecraft:chest").unwrap();
            game.world.set(2, 64, 1, chest_state).unwrap();
            let mut chest_slots = vec![None; 27];
            chest_slots[0] = Some(ItemStack::new("minecraft:diamond", 3));
            game.world
                .block_entities
                .insert(chest_key, BlockEntity::Chest(chest_slots));
            let chest_ref = game.block_entity(level, chest_pos).unwrap();
            assert_eq!(chest_ref.set_name(game, Some("Stash")), Ok(true));
            game.world.modified.clear();
            game.dropped.clear();

            assert_eq!(chest_ref.remove(game), Ok(()));
            assert_eq!(game.world.block(2, 64, 1), chest_state);
            assert!(game.block_entity(level, chest_pos).is_none());
            assert_eq!(chest_ref.remove(game), Err(BlockEntityError::Stale));
            assert!(game.dropped.is_empty(), "chest contents must not drop");
            assert!(!game.world.modified.contains(&(0, 0)));

            // Accessing the still-chest block recreates a fresh, empty block entity.
            game.spectator_open_menu(chest_pos);
            let fresh_chest = game
                .block_entity(level, chest_pos)
                .expect("accessing an entity-bearing chest creates a fresh entity");
            assert_ne!(fresh_chest, chest_ref);
            let fresh_data = fresh_chest.data(game).unwrap();
            assert!(
                match fresh_data.get(&"Items".into()) {
                    None => true,
                    Some(Tag::List(items)) => items.is_empty(),
                    _ => false,
                },
                "freshly recreated chest must be empty, got {fresh_data:?}"
            );

            // 3. Same-block property change followed by remove keeps the new property state
            //    while removing the kept block entity.
            let prop_pos = [3, 64, 1];
            game.world.set(3, 64, 1, chest_state).unwrap();
            game.world
                .block_entities
                .insert((3, 64, 1), BlockEntity::Chest(vec![None; 27]));
            assert_eq!(
                level.set_block_property(game, prop_pos, "facing", "east", BlockUpdateFlags::NONE),
                Ok(true)
            );
            let after_prop_state = game.world.block(3, 64, 1);
            assert_ne!(after_prop_state, chest_state);
            let prop_ref = game.block_entity(level, prop_pos).unwrap();
            assert_eq!(prop_ref.remove(game), Ok(()));
            assert_eq!(game.world.block(3, 64, 1), after_prop_state);
            assert!(game.block_entity(level, prop_pos).is_none());

            // 4. Jukebox setRemoved: posts jukebox_stop_play and emits level event 1011.
            let jukebox_pos = [4, 64, 1];
            let jukebox_state = registry.default_state("minecraft:jukebox").unwrap();
            game.world.set(4, 64, 1, jukebox_state).unwrap();
            game.world.block_entities.insert(
                (4, 64, 1),
                BlockEntity::Jukebox(sunrise_level::containers::Jukebox::default()),
            );
            let jukebox_ref = game.block_entity(level, jukebox_pos).unwrap();
            game.start_audio_capture();
            game.server.lan_events.clear();
            crate::game::sculk_server::TEST_POSTED_EVENTS
                .with(|e| *e.borrow_mut() = Some(Vec::new()));
            assert_eq!(jukebox_ref.remove(game), Ok(()));
            let posted = crate::game::sculk_server::TEST_POSTED_EVENTS
                .with(|e| e.borrow_mut().take().unwrap_or_default());
            assert!(
                posted
                    .iter()
                    .any(|(ev, _, _)| *ev == "minecraft:jukebox_stop_play"),
                "removing a jukebox must post jukebox_stop_play, got {posted:?}"
            );
            assert!(
                game.server
                    .lan_events
                    .iter()
                    .any(|e| e.kind == 1011 && e.pos == jukebox_pos),
                "removing a jukebox must emit level event 1011, got {:?}",
                game.server.lan_events
            );

            // 5. Beacon setRemoved: plays block.beacon.deactivate sound.
            let beacon_pos = [5, 64, 1];
            let beacon_state = registry.default_state("minecraft:beacon").unwrap();
            game.world.set(5, 64, 1, beacon_state).unwrap();
            game.world
                .block_entities
                .insert((5, 64, 1), BlockEntity::Beacon(Default::default()));
            let beacon_ref = game.block_entity(level, beacon_pos).unwrap();
            game.start_audio_capture();
            assert_eq!(beacon_ref.remove(game), Ok(()));
            let sounds = std::mem::take(&mut game.server.pending_sounds);
            game.stop_audio_capture();
            assert!(
                sounds
                    .iter()
                    .any(|s| s.event == "minecraft:block.beacon.deactivate"),
                "removing a beacon must play block.beacon.deactivate, got {sounds:?}"
            );

            // 6. Stale, taken-out, unloaded, parked-level, and client-side refusals.
            let refuse_key = (6, 64, 1);
            let refuse_pos = [6, 64, 1];
            game.world
                .block_entities
                .insert(refuse_key, BlockEntity::Chest(vec![None; 27]));
            let refuse_ref = game.block_entity(level, refuse_pos).unwrap();
            let (lease, taken) = game.world.block_entities.take(&refuse_key).unwrap();
            assert_eq!(refuse_ref.remove(game), Err(BlockEntityError::Stale));
            assert!(game.world.block_entities.put(lease, taken));

            // Parked level is not writable in place -> Stale.
            game.enter_command_level(Dim::Nether).unwrap();
            assert_eq!(refuse_ref.remove(game), Err(BlockEntityError::Stale));
            game.enter_command_level(Dim::Overworld).unwrap();

            // Unloaded chunk -> Stale.
            game.world.chunks.remove(&(0, 0));
            assert_eq!(refuse_ref.remove(game), Err(BlockEntityError::Stale));
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));

            // Client side -> Stale.
            game.options.execution_role =
                sunrise_gameplay_common::game::execution_role::ExecutionRole::Client;
            game.options.dedicated = None;
            assert_eq!(refuse_ref.remove(game), Err(BlockEntityError::Stale));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_remove_unregisters_sculk_sensor_listener() {
        use super::super::native_entity_refs::tests::with_game;
        use sunrise_gameplay_common::game::chat_common::ChatCommonGameExt1 as _;

        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.build_registries();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(sunrise_core::position::ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level("minecraft:overworld").unwrap();

            let active_pos = [4, 64, 4];
            let removed_pos = [8, 64, 4];
            assert!(game.run_command("setblock 4 64 4 sculk_sensor strict"));
            assert!(game.run_command("setblock 8 64 4 sculk_sensor strict"));
            game.sculk_prepare();
            game.sculk_sync_index();

            let removed_ref = game.block_entity(level, removed_pos).unwrap();
            assert_eq!(removed_ref.remove(game), Ok(()));
            assert!(game.block_entity(level, removed_pos).is_none());

            // Post a vibration equidistant from both sensors: the live one hears it,
            // while the sensor whose block entity was removed does not.
            game.post_block_game_event("minecraft:block_place", [6, 64, 4], None, None);
            let live_has_vibration = matches!(
                game.world.block_entities.get(&(4, 64, 4)),
                Some(BlockEntity::Sculk(s)) if s.vibration.selector.current.is_some()
            );
            assert!(
                live_has_vibration,
                "live sculk sensor must still receive game events"
            );
            assert!(
                game.world.block_entities.get(&(8, 64, 4)).is_none(),
                "removed sculk sensor must stay removed and not receive game events"
            );
            let phase = game
                .pack
                .assets
                .registry
                .state(game.world.block(8, 64, 4))
                .and_then(|s| s.properties.get("sculk_sensor_phase"))
                .map(String::as_str);
            assert_eq!(phase, Some("inactive"));
            assert!(game.block_entity(level, active_pos).is_some());
        });
    }

    #[test]
    #[ignore = "daily"]
    fn block_entity_remove_transactions_and_self_removal_in_tick() {
        use super::super::native_entity_refs::tests::{run_fresh_cases, with_game_at};
        use mod_api::{
            self, BlockEntityPart, BlockEntityTickEvent, BlockPart, BlockUpdateFlags, Hooks,
            ModEntry, Mods, Registrar, RegistrationContext,
        };
        use std::sync::atomic::{AtomicU32, Ordering};
        use sunrise_gameplay_common::game::api_values::TryIntoApi;
        use sunrise_level::containers::PublishedShapes;

        if run_fresh_cases(
            "game::server_side::native_block_entity_refs::tests::block_entity_remove_transactions_and_self_removal_in_tick",
            "SUNRISE_BLOCK_ENTITY_REMOVE_TX_CHILD",
            &["tx_and_tick"],
        ) {
            return;
        }

        const BLOCK: &str = "be_remove_test:ticker";
        #[derive(Clone, Default)]
        struct SelfRemovingTicker {
            ticks: Arc<AtomicU32>,
            after_remove_stale: Arc<AtomicU32>,
        }
        #[mod_api::__subscriptions]
        impl Hooks for SelfRemovingTicker {
            fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
                let kind = r.block_entity_type("ticker").blocks(&[BLOCK]).build();
                r.block_entity_part(&kind, self.clone()).unwrap();
            }
            fn blocks(&self, _: &RegistrationContext, event: &mut mod_api::BlocksEvent<'_>) {
                event
                    .register(mod_api::Block::new(BLOCK, "minecraft:stone"))
                    .unwrap();
            }
        }
        #[mod_api::sunrise_part]
        impl BlockPart for SelfRemovingTicker {}
        #[mod_api::sunrise_part]
        impl BlockEntityPart for SelfRemovingTicker {
            fn block_entity_tick(&self, event: &mut BlockEntityTickEvent<'_>) {
                self.ticks.fetch_add(1, Ordering::Relaxed);
                let handle = event.subject.with_view(|view| view.handle()).unwrap();
                let host = event.subject.host();
                handle.remove(host).unwrap();
                if event.subject.with_view(|view| view.handle()).is_none() {
                    self.after_remove_stale.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let part = SelfRemovingTicker::default();
        let mods: Mods = vec![ModEntry::new("be_remove_test", Box::new(part.clone()))].into();
        mod_platform::__private::install(mods.clone());
        sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
        let base = std::path::PathBuf::from(std::env::var_os("SUNRISE_TEST_RESOURCES").unwrap());
        let root =
            sunrise_gameplay_common::game::mods::content_root(&base, &Default::default()).unwrap();

        with_game_at(root.prepared().to_path_buf(), |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let registry = game.pack.assets.registry;
            let chest = registry.default_state("minecraft:chest").unwrap();
            let barrel = registry.default_state("minecraft:barrel").unwrap();
            let air = registry.default_state("minecraft:air").unwrap();
            let beacon = registry.default_state("minecraft:beacon").unwrap();
            let api_air = air.try_into_api().unwrap();
            let api_barrel = barrel.try_into_api().unwrap();

            // 1. Transaction abort restores entity, generation, name, lock, published shapes,
            //    and never runs deferred setRemoved effects (beacon sound).
            let abort_key = (1, 64, 1);
            let abort_pos = [1, 64, 1];
            let mut mod_be = ModBlockEntity::new(
                "be_remove_test:ticker".into(),
                ModBlockEntityFields {
                    nbt: compound("{kept:1}"),
                    ..Default::default()
                },
            );
            let published = PublishedShapes::new(BlockEntityShapes {
                outline: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
                collision: vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]],
            })
            .unwrap();
            mod_be.set_shapes(Some(Arc::new(published)));
            game.world
                .block_entities
                .insert(abort_key, BlockEntity::Mod(Box::new(mod_be)));
            let abort_ref = game.block_entity(level, abort_pos).unwrap();
            game.world
                .block_names
                .insert(abort_key, Tag::String("KeepMe".into()));
            assert_eq!(abort_ref.set_lock(game, Some(Compound::new())), Ok(true));

            let beacon_pos = [2, 64, 1];
            game.world.set(2, 64, 1, beacon).unwrap();
            game.world
                .block_entities
                .insert((2, 64, 1), BlockEntity::Beacon(Default::default()));
            let beacon_ref = game.block_entity(level, beacon_pos).unwrap();
            game.start_audio_capture();

            let host: &mut dyn ServerHost = game;
            let aborted: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                abort_ref.remove(host).unwrap();
                beacon_ref.remove(host).unwrap();
                assert!(host.block_entity(level, abort_pos).is_none());
                assert_eq!(abort_ref.remove(host), Err(BlockEntityError::Stale));
                Err(())
            });
            assert_eq!(aborted, Err(TransactionError::Aborted(())));
            assert_eq!(game.block_entity(level, abort_pos), Some(abort_ref));
            assert_eq!(abort_ref.name(game).as_deref(), Some("KeepMe"));
            assert!(abort_ref.lock(game).is_some());
            assert!(matches!(
                game.world.block_entities.get(&abort_key),
                Some(BlockEntity::Mod(m)) if m.shapes().is_some()
            ));
            assert_eq!(game.block_entity(level, beacon_pos), Some(beacon_ref));
            let sounds = std::mem::take(&mut game.server.pending_sounds);
            game.stop_audio_capture();
            assert!(
                sounds.is_empty(),
                "aborted beacon removal must not emit deactivate sound, got {sounds:?}"
            );

            // 2. Transaction commit: remove followed by set_block(air) skips preRemoveSideEffects.
            let suppress_pos = [3, 64, 1];
            game.world.set(3, 64, 1, chest).unwrap();
            let mut chest_slots = vec![None; 27];
            chest_slots[0] = Some(ItemStack::new("minecraft:emerald", 5));
            game.world
                .block_entities
                .insert((3, 64, 1), BlockEntity::Chest(chest_slots));
            let suppress_ref = game.block_entity(level, suppress_pos).unwrap();
            game.dropped.clear();

            let host: &mut dyn ServerHost = game;
            let committed: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                suppress_ref.remove(host).unwrap();
                assert_eq!(
                    level.set_block(host, suppress_pos, api_air, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(game.world.block(3, 64, 1), air);
            assert!(game.block_entity(level, suppress_pos).is_none());
            assert!(
                game.dropped.is_empty(),
                "remove followed by set_block(air) in one tx must not drop chest items"
            );

            // 3. Transaction commit: set_block(barrel) followed by barrel.remove() still replays
            //    the first block's (chest's) preRemoveSideEffects and leaves the barrel entity gone.
            let replay_pos = [4, 64, 1];
            game.world.set(4, 64, 1, chest).unwrap();
            let mut replay_slots = vec![None; 27];
            replay_slots[0] = Some(ItemStack::new("minecraft:gold_ingot", 2));
            game.world
                .block_entities
                .insert((4, 64, 1), BlockEntity::Chest(replay_slots));
            game.dropped.clear();

            let host: &mut dyn ServerHost = game;
            let committed: Result<(), TransactionError<()>> = host.transaction(|host, _| {
                assert_eq!(
                    level.set_block(host, replay_pos, api_barrel, BlockUpdateFlags::NEIGHBORS),
                    Ok(true)
                );
                let barrel_ref = host
                    .block_entity(level, replay_pos)
                    .expect("set_block(barrel) must create barrel entity");
                barrel_ref.remove(host).unwrap();
                assert!(host.block_entity(level, replay_pos).is_none());
                Ok(())
            });
            assert_eq!(committed, Ok(()));
            assert_eq!(game.world.block(4, 64, 1), barrel);
            assert!(
                game.block_entity(level, replay_pos).is_none(),
                "barrel block entity removed after set_block must remain removed at commit"
            );
            assert!(
                game.dropped
                    .iter()
                    .any(|item| &*item.stack.item == "minecraft:gold_ingot"),
                "first chest's preRemoveSideEffects must still replay at commit"
            );

            // 4. Self-removal inside block_entity_tick stops further loans and removes the ticker.
            let tick_pos = [5, 64, 1];
            let ticker_state = registry.default_state(BLOCK).unwrap();
            let api_ticker = ticker_state.try_into_api().unwrap();
            assert_eq!(
                level.set_block(game, tick_pos, api_ticker, BlockUpdateFlags::NONE),
                Ok(true)
            );
            assert!(game.block_entity(level, tick_pos).is_some());
            game.tick_block_entities_with(&mut ()).unwrap();
            assert_eq!(part.ticks.load(Ordering::Relaxed), 1);
            assert_eq!(part.after_remove_stale.load(Ordering::Relaxed), 1);
            assert!(game.block_entity(level, tick_pos).is_none());
            // Subsequent ticks do not invoke the removed ticker again.
            game.tick_block_entities_with(&mut ()).unwrap();
            assert_eq!(part.ticks.load(Ordering::Relaxed), 1);
        });
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::transactions::TransactionsGameExt as _;

use sunrise_gameplay_common::game::GameGameExt1 as _;

use sunrise_gameplay_common::game::commands_common::CommandsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_block_entity_read::NativeBlockEntityReadGameExt1 as _;

#[cfg(test)]
use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
