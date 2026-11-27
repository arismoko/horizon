//! Server-side serialization; `data()` still reads storage without re-entering the hook.
use super::{BlockEntity, GameLevel, Pos};
use crate::game::native_level_signals::ReadLevel;
use mod_api::{BlockEntityRef, BlockEntityTypeKey, HookId, LevelRef};
use std::borrow::Cow;
use sunrise_core::nbt::{Compound, JavaString, Limits, Tag};
use sunrise_items::items::Items;
use sunrise_level::{containers::ModBlockEntity, world::World};
use sunrise_mod_runtime::attachments::ATTACHMENTS_KEY;
use sunrise_sim::save::block_entity_encode;
use sunrise_world::registry::BlockRegistry;

fn owns_slots(world: &World, registry: &BlockRegistry, block: &str) -> bool {
    registry
        .default_state(block)
        .and_then(|state| world.block_entities.mod_container(state.0))
        .is_some_and(|container| container.slots != 0)
        // There can be a save before the first pass populates the container table.
        || !world.block_entities.has_states()
            && mod_api::__private::parts_installation::block_container(block)
                .is_some_and(|container| container.slots != 0)
}

fn save_additional(
    level: LevelRef,
    pos: Pos,
    generation: Option<u64>,
    row: &ModBlockEntity,
    items: &Items,
    owns_slots: impl FnOnce() -> bool,
    fire: impl FnOnce(BlockEntityTypeKey, BlockEntityRef, &mut Compound),
) -> Option<Compound> {
    let kind = row.id();
    if kind.is_empty()
        || !mod_api::__private::block_entity_type_listens(HookId::BlockEntitySaveAdditional, kind)
    {
        return None;
    }
    let id = mod_api::__private::block_entity_type_key(kind)?;
    let generation = generation?;
    let entity =
        mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
    let owns_slots = owns_slots();
    let mut input = Cow::Borrowed(&row.nbt);
    if !owns_slots && !row.items.is_empty() {
        // Slotless rows can still have modeled stacks from decoding a save.
        // The listener owns that field; don't normalize the resident row.
        block_entity_encode::write_mod_items(input.to_mut(), row, items);
    }
    // The raw attachment field stays available for the disk overlay (including
    // opaque or malformed rows), but is never part of the Part's payload.
    let key = ATTACHMENTS_KEY.into();
    let mut output = input.as_ref().clone();
    output.remove(&key);
    fire(id, entity, &mut output);
    // Admit the listener's output once, before any consumer sees it. Disk
    // encoding still owns the size limit for its complete record.
    let max_depth = Limits::saved().max_depth;
    let fits = |s: &JavaString| u16::try_from(s.modified_utf8_len()).is_ok();
    let mut pending: Vec<_> = output.iter().map(|(k, v)| (Some(k), v, 1)).collect();
    while let Some((name, tag, depth)) = pending.pop() {
        if name.is_none_or(fits) {
            match tag {
                Tag::List(items) if depth < max_depth => {
                    pending.extend(items.iter().map(|v| (None, v, depth + 1)));
                    continue;
                }
                Tag::Compound(items) if depth < max_depth => {
                    pending.extend(items.iter().map(|(k, v)| (Some(k), v, depth + 1)));
                    continue;
                }
                Tag::End | Tag::List(_) | Tag::Compound(_) => {}
                Tag::String(value) if !fits(value) => {}
                _ => continue,
            }
        }
        log::warn!("block_entity_save_additional: refused malformed payload for {kind} at {pos:?}");
        sunrise_core::nbt::discard_tag(Tag::Compound(output));
        return None;
    }
    output.remove(&key);
    let mut output = crate::game::native_block_entity_read::mod_payload(output);
    if let Some(attachments) = input.get(&key) {
        output.insert(key, attachments.clone());
    }
    if output == *input {
        return None;
    }
    if owns_slots {
        block_entity_encode::write_mod_items(&mut output, row, items);
    }
    Some(output)
}

pub trait BlockEntitySaveAdditionalGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    fn capture_block_entity_payload(
        &self,
        pos: Pos,
        block: &str,
        row: &ModBlockEntity,
        items: &Items,
    ) -> Option<Compound> {
        // A detached unload column still has its live slots until after capture.
        self.capture_block_entity_payload_at(
            pos,
            self.world.block_entities.generation(&pos),
            block,
            row,
            items,
        )
    }

    fn capture_block_entity_payload_at(
        &self,
        pos: Pos,
        generation: Option<u64>,
        block: &str,
        row: &ModBlockEntity,
        items: &Items,
    ) -> Option<Compound> {
        if self.is_logical_server()
            && mod_api::__private::has_part_hook(HookId::BlockEntitySaveAdditional)
            && let Some(level) = self.level(self.dim.id())
        {
            let host = ReadLevel::new(self, level, &self.world);
            save_additional(
                level,
                pos,
                generation,
                row,
                items,
                || owns_slots(&self.world, self.pack.assets.registry, block),
                |id, entity, tag| {
                    let pos = [pos.0, pos.1, pos.2];
                    mod_api::dispatch_block_entity_save_additional(
                        &host, id, entity, pos, level, tag,
                    );
                },
            )
        } else {
            None
        }
    }

    /// Save the retained block entity only once the removed block's loot will roll.
    /// The handle still names the captured generation, but its level slot may be gone.
    fn encode_removed_block_entity(
        &self,
        pos: Pos,
        entity: &BlockEntity,
        generation: Option<u64>,
        block: &str,
        items: &sunrise_items::items::Items,
    ) -> Tag {
        block_entity_encode::encode_with(pos, entity, block, Default::default(), items, |row| {
            self.capture_block_entity_payload_at(pos, generation, block, row, items)
        })
    }

    /// Save the live entity for a command, a template, an item or a loot copy.
    fn encode_live_block_entity(
        &self,
        pos: Pos,
        entity: &BlockEntity,
        block: &str,
        items: &sunrise_items::items::Items,
    ) -> Tag {
        block_entity_encode::encode_with(pos, entity, block, Default::default(), items, |row| {
            self.capture_block_entity_payload(pos, block, row, items)
        })
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockEntitySaveAdditionalGameExt1<'__sg> for G {}

impl GameLevel<'_> {
    /// `BlockEntity.saveAdditional` for a live block entity used in a loot roll.
    pub fn encode_live_block_entity(&self, pos: Pos, entity: &BlockEntity, block: &str) -> Tag {
        block_entity_encode::encode_with(
            pos,
            entity,
            block,
            Default::default(),
            self.items,
            |row| {
                if self.native_authoritative
                    && let Some(level) = self.api_level
                {
                    save_additional(
                        level,
                        pos,
                        self.world.block_entities.generation(&pos),
                        row,
                        self.items,
                        || owns_slots(self.world, self.registry, block),
                        |id, entity, tag| {
                            let pos = [pos.0, pos.1, pos.2];
                            self.native_block_entity_save_additional(id, entity, pos, level, tag);
                        },
                    )
                } else {
                    None
                }
            },
        )
    }

    pub(in crate::game) fn native_block_entity_save_additional(
        &self,
        id: mod_api::BlockEntityTypeKey,
        entity: BlockEntityRef,
        pos: [i32; 3],
        level: LevelRef,
        tag: &mut Compound,
    ) {
        self.fire_block_entity_save_additional(id, entity, pos, level, tag);
    }
}

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
