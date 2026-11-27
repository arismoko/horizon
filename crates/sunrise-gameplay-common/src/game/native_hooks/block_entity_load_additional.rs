//! Saved-data loads on the owning server thread, distinct from attachment and client sync.
use super::block_entity_read::ResidentReadBlockEntity;
use super::{BlockEntity, GameLevel, Pos};
use crate::game::{api_values::TryIntoApi, native_level_signals::ReadLevel};
use mod_api::{
    BlockEntityLevelReadAccess, BlockEntityLevelReadScope, BlockEntityReadView, BlockEntityRef,
    BlockEntityTypeKey, HookId, LevelHost, LevelRef,
};
use sunrise_core::nbt::Compound;
use sunrise_level::world::World;

fn loaded_payload(
    world: &World,
    level: LevelRef,
    pos: Pos,
) -> Option<(BlockEntityTypeKey, BlockEntityRef, &Compound)> {
    let BlockEntity::Mod(data) = world.block_entities.get(&pos)? else {
        return None;
    };
    let id = mod_api::__private::block_entity_type_key(data.id())?;
    let generation = world.block_entities.generation(&pos)?;
    let entity =
        mod_api::__private::game::block_entity_ref(level, [pos.0, pos.1, pos.2], generation);
    Some((id, entity, &data.nbt))
}

struct LevelReadBlockEntityAccess<'a> {
    world: &'a World,
    host: &'a dyn LevelHost,
    subject: BlockEntityRef,
}

impl BlockEntityLevelReadAccess for LevelReadBlockEntityAccess<'_> {
    fn with_view(&self, call: &mut dyn FnMut(BlockEntityReadView<'_>)) {
        use mod_api::__private::game::Fields;
        let target = self.subject.fields();
        let [x, y, z] = target.position;
        let pos = (x, y, z);
        if self.world.block_entities.generation(&pos) != Some(target.generation) {
            return;
        }
        let Some(BlockEntity::Mod(entity)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let state_id = self.world.block(x, y, z);
        let Ok(state) = state_id.try_into_api() else {
            return;
        };
        let slot_count = self
            .world
            .block_entities
            .mod_container(state_id.0)
            .map_or(0, |c| c.slots);
        let data = ResidentReadBlockEntity { entity, slot_count };
        call(BlockEntityReadView::new(self.subject, state, &data));
    }

    fn host(&self) -> &dyn LevelHost {
        self.host
    }
}

pub trait BlockEntityLoadAdditionalGameExt1<'__sg>: crate::game::SideGame<'__sg> {
    /// An explicit engine load into an attached instance.
    fn notify_block_entity_load_additional(&self, pos: Pos) {
        if self.is_logical_server()
            && mod_api::__private::has_part_hook(HookId::BlockEntityLoadAdditional)
            && let Some(level) = self.level(self.dim.id())
            && let Some((id, subject, tag)) = loaded_payload(&self.world, level, pos)
        {
            let host = ReadLevel::new(self, level, &self.world);
            let access = LevelReadBlockEntityAccess {
                world: &self.world,
                host: &host,
                subject,
            };
            mod_api::dispatch_block_entity_load_additional(
                id,
                BlockEntityLevelReadScope::new(&access),
                tag,
            );
        }
    }

    /// Chunk workers decode data; only the level thread tells the Parts, after
    /// the instances and their attachments are in the world.
    fn fire_pending_block_entity_data_loads(&mut self) {
        if !self.world.block_entities.has_pending_data_load() {
            return;
        }
        while let Some((pos, generation)) = self.world.block_entities.next_data_load() {
            if self.world.block_entities.generation(&pos) == Some(generation) {
                self.notify_block_entity_load_additional(pos);
            }
        }
    }
}
impl<'__sg, G: crate::game::SideGame<'__sg>> BlockEntityLoadAdditionalGameExt1<'__sg> for G {}

impl GameLevel<'_> {
    pub(in crate::game) fn notify_block_entity_load_additional(&self, pos: Pos) {
        if self.native_authoritative
            && mod_api::__private::has_part_hook(HookId::BlockEntityLoadAdditional)
            && let Some(level) = self.api_level
            && let Some((id, subject, tag)) = loaded_payload(self.world, level, pos)
        {
            let access = LevelReadBlockEntityAccess {
                world: self.world,
                host: self,
                subject,
            };
            native_block_entity_load_additional(id, BlockEntityLevelReadScope::new(&access), tag);
        }
    }
}

/// The level's `BlockEntity.loadAdditional` notice once the payload is installed.
pub(in crate::game) fn native_block_entity_load_additional(
    id: mod_api::BlockEntityTypeKey,
    subject: mod_api::BlockEntityLevelReadScope<'_>,
    tag: &Compound,
) {
    crate::game::native_hooks::fire::fire_block_entity_load_additional(id, subject, tag);
}

use crate::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;
