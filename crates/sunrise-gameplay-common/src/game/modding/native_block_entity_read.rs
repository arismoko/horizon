use crate::game::api_values_common::block_entity_payload;
use mod_api::__private::game::*;
use mod_api::{BlockEntityItemError, BlockEntityRef, BlockEntityTypeKey, LevelRef};
use sunrise_core::nbt::{Compound, Tag};
use sunrise_items::items::ItemStack;
use sunrise_level::{containers::BlockEntity, world::World};
use sunrise_sim::save;

pub trait NativeBlockEntityReadGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// Read a configured mod holder's declared internal item slot, without automation policy.
    fn block_entity_item_slot(
        &self,
        entity: BlockEntityRef,
        slot: u32,
    ) -> Result<Option<ItemStack>, BlockEntityItemError> {
        let (world, BlockEntity::Mod(block_entity)) = self
            .native_block_entity_target(entity)
            .ok_or(BlockEntityItemError::Stale)?
        else {
            return Err(BlockEntityItemError::Unavailable);
        };
        let [x, y, z] = entity.fields().position;
        mod_item_slots(world, (x, y, z), slot)?;
        Ok(block_entity.items.get(slot as usize).cloned().flatten())
    }

    fn native_block_entity_target<'a>(
        &'a self,
        entity: BlockEntityRef,
    ) -> Option<(&'a World, &'a BlockEntity)>
    where
        '__sg: 'a,
    {
        let world = self.native_level_read_world(entity.fields().level)?;
        Some((world, resident_block_entity(world, entity)?))
    }

    /// Find a block entity in a loaded chunk of an open level, or on a client in
    /// the level it shows.
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        world_block_entity(world, level, pos)
    }

    /// `block_entity`, also for a block entity temporarily taken out by an engine operation:
    /// its generation stays in place while it is out.
    fn block_entity_identity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        let world = self.native_level_read_world(level)?;
        if !world.loaded(pos[0], pos[2]) {
            return None;
        }
        let generation = world.block_entities.generation(&(pos[0], pos[1], pos[2]))?;
        Some(mod_api::__private::game::block_entity_ref(
            level, pos, generation,
        ))
    }

    /// The block entity's type (`BlockEntity.getType`).
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let (world, block_entity) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        let block = &self.pack.assets.registry.state(world.block(x, y, z))?.name;
        mod_api::__private::block_entity_type_key(save::block_entity_id(block_entity, block))
    }

    /// The block entity's position.
    fn block_entity_position(&self, entity: BlockEntityRef) -> Option<[i32; 3]> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().position)
    }

    /// The block entity's level.
    fn block_entity_level(&self, entity: BlockEntityRef) -> Option<LevelRef> {
        self.native_block_entity_target(entity)?;
        Some(entity.fields().level)
    }

    /// The data saved without identity or position metadata
    /// (`BlockEntity.saveWithoutMetadata`).
    fn block_entity_data(&self, entity: BlockEntityRef) -> Option<Compound> {
        let world = self.native_level_read_world(entity.fields().level)?;
        world_block_entity_data(world, self.pack.assets.registry, self.res.items(), entity)
    }

    /// The resident lock, independent of the block entity's synthesized save payload.
    fn block_entity_lock<'a>(&'a self, entity: BlockEntityRef) -> Option<&'a Compound>
    where
        '__sg: 'a,
    {
        let (world, _) = self.native_block_entity_target(entity)?;
        let [x, y, z] = entity.fields().position;
        match world.block_lock((x, y, z))? {
            Tag::Compound(lock) => Some(lock),
            _ => None,
        }
    }

    /// A snapshot of a mod block entity's menu data slots, on the server or in the
    /// client's copy. None for a stale/missing/vanilla entity. Item storage is separate.
    fn block_entity_data_slots(&self, entity: BlockEntityRef) -> Option<Vec<i32>> {
        let (_, BlockEntity::Mod(block_entity)) = self.native_block_entity_target(entity)? else {
            return None;
        };
        Some(block_entity.data.clone())
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> NativeBlockEntityReadGameExt1<'__sg> for G {}

/// A mod's data payload as the block entity takes it: without the fields the engine owns, which
/// are its identity and position, and its published shapes (`set_shapes` alone writes those).
pub fn mod_payload(mut data: Compound) -> Compound {
    for field in ["id", "x", "y", "z", sunrise_level::containers::SHAPES_KEY] {
        data.remove(&field.into());
    }
    data
}

/// The block entity as `/data get block` shows it (`saveWithFullMetadata`), with the custom
/// name Sunrise keeps beside the model.
pub fn save_with_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &sunrise_items::items::Items,
) -> Option<Compound> {
    let pending = world
        .chunks
        .get(&(pos.0 >> 4, pos.2 >> 4))
        .and_then(|chunk| chunk.saved.as_ref())
        .and_then(|saved| saved.pending_loot.get(&pos));
    let Tag::Compound(data) = save::block_entity_encode::data(
        pos,
        entity,
        block,
        save::block_entity_encode::DataFields {
            lock: world.block_lock(pos),
            custom_name: world.block_names.get(&pos),
            pending,
            ..Default::default()
        },
        items,
    ) else {
        return None;
    };
    Some(data)
}

pub fn save_without_metadata(
    world: &World,
    pos: (i32, i32, i32),
    entity: &BlockEntity,
    block: &str,
    items: &sunrise_items::items::Items,
) -> Option<Compound> {
    save_with_metadata(world, pos, entity, block, items).map(block_entity_payload)
}

pub fn mod_item_slots(
    world: &World,
    key: (i32, i32, i32),
    slot: u32,
) -> Result<usize, BlockEntityItemError> {
    let slots = world
        .block_entities
        .mod_container(world.block(key.0, key.1, key.2).0)
        .ok_or(BlockEntityItemError::Unavailable)?
        .slots;
    if slot as usize >= slots {
        return Err(BlockEntityItemError::SlotOutOfBounds {
            slot,
            slots: slots as u32,
        });
    }
    Ok(slots)
}

pub(in crate::game) fn part_snapshot(
    world: &World,
    level: Option<LevelRef>,
    authoritative: bool,
    index: Option<
        &std::sync::Arc<
            std::sync::Mutex<
                sunrise_mod_runtime::attachments::index::AttachmentIndex<mod_api::AttachmentHolder>,
            >,
        >,
    >,
    scopes: &sunrise_mod_runtime::attachments::AttachmentQueryScopes,
    entity: BlockEntityRef,
    (namespace, name): (&str, &str),
) -> Result<mod_api::AttachmentBatch, mod_api::PartStateError> {
    use sunrise_mod_runtime::attachments::AttachmentBatch;

    let fields = entity.fields();
    if level != Some(fields.level) {
        return Err(mod_api::PartStateError::OtherLevel);
    }
    let key = (fields.position[0], fields.position[1], fields.position[2]);
    if !world.is_loaded_at(key.0, key.1, key.2)
        || world.block_entities.generation(&key) != Some(fields.generation)
    {
        return Err(mod_api::PartStateError::Stale);
    }
    mod_api::__private::validate_attachment_use(namespace, name, false).map_err(|error| {
        mod_api::PartStateError::Attachment {
            namespace: "",
            name: "",
            error,
        }
    })?;
    let batch = if authoritative {
        index.map_or_else(AttachmentBatch::empty, |index| {
            AttachmentBatch::new(index.lock().unwrap().snapshot_named(namespace, name))
        })
    } else {
        AttachmentBatch::empty()
    };
    Ok(batch.in_scope(scopes).into())
}

/// The handle of the loaded block entity at `pos`, without loading a chunk or taking one
/// temporarily absent from the resident table.
pub fn world_block_entity(world: &World, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
    loaded_block_entity(world, pos)?;
    block_entity_handle(world, level, pos)
}

/// Construct a handle after the caller has checked that the entry is resident.
pub(in crate::game) fn block_entity_handle(
    world: &World,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<BlockEntityRef> {
    let [x, y, z] = pos;
    let generation = world.block_entities.generation(&(x, y, z))?;
    Some(mod_api::__private::game::block_entity_ref(
        level, pos, generation,
    ))
}

/// Read a resident entity's saved data after the caller has resolved its level to `world`.
pub fn world_block_entity_data(
    world: &World,
    registry: &sunrise_world::registry::BlockRegistry,
    items: &sunrise_items::items::Items,
    entity: BlockEntityRef,
) -> Option<Compound> {
    let block_entity = resident_block_entity(world, entity)?;
    let [x, y, z] = entity.fields().position;
    let block = &registry.state(world.block(x, y, z))?.name;
    save_without_metadata(world, (x, y, z), block_entity, block, items)
}

fn resident_block_entity(world: &World, entity: BlockEntityRef) -> Option<&BlockEntity> {
    let fields = entity.fields();
    let [x, y, z] = fields.position;
    if world.block_entities.generation(&(x, y, z)) != Some(fields.generation) {
        return None;
    }
    loaded_block_entity(world, fields.position)
}

pub fn loaded_block_entity(world: &World, [x, y, z]: [i32; 3]) -> Option<&BlockEntity> {
    if !world.loaded(x, z) || !world.shape.contains_y(y) {
        return None;
    }
    world.block_entities.get(&(x, y, z))
}

use crate::game::native_level_refs_common::NativeLevelRefsCommonGameExt1 as _;
